use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::{
        State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
    routing::get,
};
use serde_json::{Value, json};
use tokio::time::{Instant, MissedTickBehavior, interval, interval_at, timeout};

use crate::{
    app_state::AppState,
    errors::{api_error::ApiError, catalog::ErrorCode, fallback::method_not_allowed},
    features::v1::{benchmark::live::handler::active_view, health::handler::check},
    origin,
};

// How often the server looks at the execution slot. A frame is only sent when the answer
// changed, so an idle API sends nothing.
const ACTIVE_CHECK: Duration = Duration::from_secs(1);
// How often the database is asked
const HEALTH_EVERY: Duration = Duration::from_secs(15);
// The database may take this long to answer before the API counts as unhealthy
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

const MAX_STATUS_SOCKETS: usize = 16;
const CLOSE_GOING_AWAY: u16 = 1001;
const CLOSE_TOO_MANY_SOCKETS: u16 = 4429;

// Status sockets of the whole process
static OPEN_SOCKETS: AtomicUsize = AtomicUsize::new(0);

// A place among the status sockets, given back when the socket ends however it ends
struct StatusPermit;

impl StatusPermit {
    fn acquire() -> Option<Self> {
        let taken = OPEN_SOCKETS.fetch_add(1, Ordering::SeqCst);

        if taken >= MAX_STATUS_SOCKETS {
            OPEN_SOCKETS.fetch_sub(1, Ordering::SeqCst);

            return None;
        }

        Some(Self)
    }
}

impl Drop for StatusPermit {
    fn drop(&mut self) {
        OPEN_SOCKETS.fetch_sub(1, Ordering::SeqCst);
    }
}

// GET /api/v1/status/ws
//
// One socket that keeps the dashboard current: the active run when it changes, the API
// health every 15 seconds, and an answer to every ping so the page can measure the round trip.
// The origin is checked before the upgrade. A full server accepts the socket and closes it
// with its own code, because a browser cannot read the status of a failed handshake.
async fn connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !origin::websocket_origin_allowed(&headers) {
        return Err(ApiError::new(
            ErrorCode::Forbidden,
            "The origin of this request is not allowed.",
        ));
    }

    let config = state.live.ws_config();
    let permit = StatusPermit::acquire();

    Ok(upgrade
        .max_message_size(config.max_client_message_bytes)
        .max_frame_size(config.max_client_message_bytes)
        .on_upgrade(move |socket| serve(socket, state, permit)))
}

async fn serve(mut socket: WebSocket, state: AppState, permit: Option<StatusPermit>) {
    // The permit is held until the socket ends
    let Some(_permit) = permit else {
        close(&mut socket, CLOSE_TOO_MANY_SOCKETS, "too_many_sockets").await;

        return;
    };

    let config = state.live.ws_config();

    let mut active_tick = interval(ACTIVE_CHECK);
    let mut health_tick = interval(HEALTH_EVERY);
    let mut ping = interval_at(Instant::now() + config.ping_interval, config.ping_interval);

    active_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    health_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut last_active: Option<String> = None;
    let mut last_pong = Instant::now();

    loop {
        tokio::select! {
            _ = active_tick.tick() => {
                let frame = json!({ "type": "active", "data": active_view(&state) }).to_string();

                // Only a changed answer is sent, the first one always is
                if last_active.as_deref() != Some(frame.as_str()) {
                    if send(&mut socket, frame.clone()).await.is_err() {
                        return;
                    }

                    last_active = Some(frame);
                }
            }
            _ = health_tick.tick() => {
                if send(&mut socket, health_frame(&state).await.to_string()).await.is_err() {
                    return;
                }
            }
            _ = ping.tick() => {
                if last_pong.elapsed() >= config.pong_timeout {
                    close(&mut socket, CLOSE_GOING_AWAY, "pong_timeout").await;

                    return;
                }

                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    return;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Pong(_))) => last_pong = Instant::now(),
                Some(Ok(Message::Text(text))) => {
                    if let Some(id) = ping_id(text.as_str()) {
                        let pong = json!({ "type": "pong", "id": id }).to_string();

                        if send(&mut socket, pong).await.is_err() {
                            return;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

// The `id` of a client frame `{"type":"ping","id":<number>}`, anything else is ignored
fn ping_id(text: &str) -> Option<u64> {
    let value: Value = serde_json::from_str(text).ok()?;

    if value.get("type")?.as_str()? != "ping" {
        return None;
    }

    value.get("id")?.as_u64()
}

// The health of the API now. A database that does not answer in time is reported, not an error.
async fn health_frame(state: &AppState) -> Value {
    let database = matches!(timeout(HEALTH_TIMEOUT, check(state)).await, Ok(Ok(_)));

    json!({
        "type": "health",
        "data": {
            "ok": database,
            "database": database,
            "version": env!("CARGO_PKG_VERSION"),
        }
    })
}

async fn send(socket: &mut WebSocket, text: String) -> Result<(), ()> {
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ())
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

pub fn router() -> Router<AppState> {
    Router::new().route("/status/ws", get(connect).fallback(method_not_allowed))
}
