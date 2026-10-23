use std::sync::Arc;

use axum::{
    Router,
    body::Bytes,
    extract::{
        Path, Query, State,
        rejection::QueryRejection,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Instant, interval_at};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    errors::{
        api_error::ApiError, catalog::ErrorCode, fallback::method_not_allowed, json::ApiJson,
        response::ApiSuccess,
    },
    features::v1::benchmark::model::BenchmarkRequest,
    origin,
};

use super::{
    event::RunEvent,
    log::RunStatus,
    registry::{CancelOutcome, RunEntry},
    sockets::{SocketPermit, WsConfig},
};

// Close codes of the live-run WebSocket
pub const CLOSE_NORMAL: u16 = 1000;
pub const CLOSE_GOING_AWAY: u16 = 1001;
pub const CLOSE_UNKNOWN_RUN: u16 = 4404;
pub const CLOSE_TOO_MANY_SOCKETS: u16 = 4429;

#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub cancel: bool,
}

// What the 202 of POST /api/v1/live-runs carries. The cancel token is shown here and nowhere
// else, so a debug print redacts it.
#[derive(Serialize)]
pub struct LiveRunAccepted {
    pub run_id: Uuid,
    pub ws_path: String,
    pub status_path: String,
    pub cancel_token: String,
    pub capabilities: Capabilities,
}

impl std::fmt::Debug for LiveRunAccepted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveRunAccepted")
            .field("run_id", &self.run_id)
            .field("ws_path", &self.ws_path)
            .field("status_path", &self.status_path)
            .field("cancel_token", &"<redacted>")
            .field("capabilities", &self.capabilities)
            .finish()
    }
}

// Body of POST /api/v1/live-runs/{run_id}/cancel. A missing token is a refusal like a wrong one.
#[derive(Deserialize)]
pub struct CancelBody {
    #[serde(default)]
    pub cancel_token: Option<String>,
}

// What the 202 of a cancel request carries
#[derive(Debug, Serialize)]
pub struct CancelAccepted {
    pub state: &'static str,
    pub already_requested: bool,
}

// The JSON replay of a run, the same events and rules as the WebSocket
#[derive(Debug, Serialize)]
pub struct LiveRunReplay {
    pub run_id: Uuid,
    pub status: &'static str,
    pub last_seq: u64,
    pub dropped_through_seq: u64,
    pub truncated: bool,
    pub events: Vec<RunEvent>,
}

#[derive(Debug, Deserialize)]
pub struct AfterQuery {
    pub after: Option<u64>,
}

// POST /api/v1/live-runs
async fn create(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<BenchmarkRequest>,
) -> Result<ApiSuccess<LiveRunAccepted>, ApiError> {
    let accepted = state.live.submit(request).map_err(ApiError::from)?;
    let run_id = accepted.run_id;

    Ok(ApiSuccess::accepted(
        "Benchmark accepted.",
        LiveRunAccepted {
            run_id,
            ws_path: format!("/api/v1/live-runs/{run_id}/ws"),
            status_path: format!("/api/v1/live-runs/{run_id}"),
            cancel_token: accepted.cancel_token,
            capabilities: Capabilities { cancel: true },
        },
    ))
}

// GET /api/v1/live-runs/{run_id}?after=
async fn replay(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    query: Result<Query<AfterQuery>, QueryRejection>,
) -> Result<ApiSuccess<LiveRunReplay>, ApiError> {
    let Query(query) = query
        .map_err(|rejection| ApiError::new(ErrorCode::ValidationError, rejection.body_text()))?;

    let run_id = parse_run_id(&run_id)?;

    let entry = state.live.registry().get(run_id).ok_or_else(|| {
        ApiError::new(
            ErrorCode::NotFound,
            format!("Live run {run_id} was not found."),
        )
    })?;

    let replay = entry.replay(query.after.unwrap_or(0));

    Ok(ApiSuccess::ok(
        "Live run retrieved.",
        LiveRunReplay {
            run_id,
            status: replay.status.as_str(),
            last_seq: replay.last_seq,
            dropped_through_seq: replay.dropped_through_seq,
            truncated: replay.truncated(),
            events: replay.events,
        },
    ))
}

// POST /api/v1/live-runs/{run_id}/cancel
//
// Asks the run to stop and answers at once: the run is not cancelled yet. `run.cancelled` is
// recorded by the run itself once its work has really stopped and its cleanup is done.
async fn cancel(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    ApiJson(body): ApiJson<CancelBody>,
) -> Result<ApiSuccess<CancelAccepted>, ApiError> {
    let run_id = parse_run_id(&run_id)?;

    let entry = state.live.registry().get(run_id).ok_or_else(|| {
        ApiError::new(
            ErrorCode::NotFound,
            format!("Live run {run_id} was not found."),
        )
    })?;

    // Never says which part was wrong, and never repeats a token
    if !entry.verify_token(body.cancel_token.as_deref().unwrap_or_default()) {
        return Err(ApiError::new(
            ErrorCode::Forbidden,
            "The cancel token is missing or not valid.",
        ));
    }

    match entry.request_cancel() {
        CancelOutcome::Requested => Ok(cancel_accepted(false)),
        CancelOutcome::AlreadyRequested => Ok(cancel_accepted(true)),
        CancelOutcome::Saving => Err(ApiError::new(
            ErrorCode::Conflict,
            "The run is already saving its result and can no longer be cancelled.",
        )),
        CancelOutcome::Finished(status) => Err(ApiError::new(
            ErrorCode::Conflict,
            format!("The run already finished ({}).", status.as_str()),
        )),
    }
}

fn cancel_accepted(already_requested: bool) -> ApiSuccess<CancelAccepted> {
    ApiSuccess::accepted(
        "Cancellation requested.",
        CancelAccepted {
            state: "cancelling",
            already_requested,
        },
    )
}

fn parse_run_id(run_id: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(run_id).map_err(|_| {
        ApiError::new(
            ErrorCode::BadRequest,
            format!("'{run_id}' is not a valid run id."),
        )
    })
}

// What a handshake leads to, decided before the upgrade
enum Admission {
    Serve(Arc<RunEntry>, SocketPermit),
    UnknownRun,
    TooManySockets,
}

// GET /api/v1/live-runs/{run_id}/ws?after=
//
// The origin and the run id are checked before the upgrade, so a refusal is a plain HTTP
// error. An unknown run and a full run are accepted and closed with their own close
// code, because a browser cannot read the status of a failed handshake.
async fn connect(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    query: Result<Query<AfterQuery>, QueryRejection>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !origin::websocket_origin_allowed(&headers) {
        return Err(ApiError::new(
            ErrorCode::Forbidden,
            "The origin of this request is not allowed.",
        ));
    }

    let Query(query) = query
        .map_err(|rejection| ApiError::new(ErrorCode::ValidationError, rejection.body_text()))?;

    let run_id = parse_run_id(&run_id)?;
    let after = query.after.unwrap_or(0);
    let config = state.live.ws_config();

    let admission = match state.live.registry().get(run_id) {
        None => Admission::UnknownRun,
        Some(entry) => match state.live.sockets().try_acquire(&entry) {
            Some(permit) => Admission::Serve(entry, permit),
            None => Admission::TooManySockets,
        },
    };

    Ok(upgrade
        .max_message_size(config.max_client_message_bytes)
        .max_frame_size(config.max_client_message_bytes)
        .on_upgrade(move |socket| serve_socket(socket, admission, after, config)))
}

async fn serve_socket(mut socket: WebSocket, admission: Admission, after: u64, config: WsConfig) {
    match admission {
        Admission::UnknownRun => close(&mut socket, CLOSE_UNKNOWN_RUN, "unknown_run").await,
        Admission::TooManySockets => {
            close(&mut socket, CLOSE_TOO_MANY_SOCKETS, "too_many_sockets").await
        }
        // The permit is held until the socket ends, whichever way it ends
        Admission::Serve(entry, _permit) => stream_run(&mut socket, &entry, after, &config).await,
    }
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

async fn send_json(socket: &mut WebSocket, value: &Value) -> Result<(), ()> {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .map_err(|_| ())
}

enum Drain {
    // Everything stored so far was sent, the run goes on
    Waiting,
    // The terminal event was sent
    Finished,
    // The socket is gone
    Failed,
}

// Sends the events after the cursor and moves the cursor. The log is the only source, so
// a slow client simply reads later and nothing can overflow.
async fn drain(
    socket: &mut WebSocket,
    entry: &RunEntry,
    cursor: &mut u64,
    announced: &mut u64,
) -> Drain {
    let replay = entry.replay(*cursor);

    // Samples between the cursor and this boundary were evicted before they were read
    if replay.dropped_through_seq > *announced && replay.dropped_through_seq > *cursor {
        *announced = replay.dropped_through_seq;

        let frame = json!({ "type": "samples.dropped", "through_seq": *announced });

        if send_json(socket, &frame).await.is_err() {
            return Drain::Failed;
        }
    }

    for event in &replay.events {
        let Ok(text) = serde_json::to_string(event) else {
            return Drain::Failed;
        };

        if socket.send(Message::Text(text.into())).await.is_err() {
            return Drain::Failed;
        }

        *cursor = event.seq;
    }

    if replay.status != RunStatus::Running && *cursor >= replay.last_seq {
        Drain::Finished
    } else {
        Drain::Waiting
    }
}

// stream.opened, then the replay after `after`, then the live events, then a normal
// close after the terminal event. The run itself never depends on this socket.
async fn stream_run(socket: &mut WebSocket, entry: &RunEntry, after: u64, config: &WsConfig) {
    // Subscribed before the first read, so no event falls between the read and the wait
    let mut changes = entry.subscribe();

    let opening = entry.replay(after);
    let mut announced = opening.dropped_through_seq;
    let mut cursor = after;

    let opened = json!({
        "type": "stream.opened",
        "run_id": entry.run_id(),
        "status": opening.status.as_str(),
        "last_seq": opening.last_seq,
        "truncated": opening.truncated(),
        "dropped_through_seq": opening.dropped_through_seq,
    });

    if send_json(socket, &opened).await.is_err() {
        return;
    }

    let mut ping = interval_at(Instant::now() + config.ping_interval, config.ping_interval);
    let mut last_pong = Instant::now();

    loop {
        match drain(socket, entry, &mut cursor, &mut announced).await {
            Drain::Failed => return,
            Drain::Finished => {
                close(socket, CLOSE_NORMAL, "run_finished").await;

                return;
            }
            Drain::Waiting => {}
        }

        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            _ = ping.tick() => {
                if last_pong.elapsed() >= config.pong_timeout {
                    close(socket, CLOSE_GOING_AWAY, "pong_timeout").await;

                    return;
                }

                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    return;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Pong(_))) => last_pong = Instant::now(),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                // Nothing a client sends changes the run
                Some(Ok(_)) => {}
            },
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/live-runs", post(create).fallback(method_not_allowed))
        .route(
            "/live-runs/{run_id}",
            get(replay).fallback(method_not_allowed),
        )
        .route(
            "/live-runs/{run_id}/ws",
            get(connect).fallback(method_not_allowed),
        )
        .route(
            "/live-runs/{run_id}/cancel",
            post(cancel).fallback(method_not_allowed),
        )
}
