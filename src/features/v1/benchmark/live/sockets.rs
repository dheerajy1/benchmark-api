use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use super::registry::RunEntry;

pub const MAX_SOCKETS_PER_RUN: usize = 8;
pub const MAX_SOCKETS_TOTAL: usize = 32;
pub const PING_INTERVAL: Duration = Duration::from_secs(20);
pub const PONG_TIMEOUT: Duration = Duration::from_secs(45);
pub const MAX_CLIENT_MESSAGE_BYTES: usize = 1024;

// The WebSocket settings of the live runs, tests shorten the heartbeat
#[derive(Debug, Clone, Copy)]
pub struct WsConfig {
    // A ping is sent every interval
    pub ping_interval: Duration,
    // The socket is closed at the first ping tick after this long without a pong
    pub pong_timeout: Duration,
    // Larger frames from a client close the socket, the server only needs pongs
    pub max_client_message_bytes: usize,
    pub max_sockets_per_run: usize,
    pub max_sockets_total: usize,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            ping_interval: PING_INTERVAL,
            pong_timeout: PONG_TIMEOUT,
            max_client_message_bytes: MAX_CLIENT_MESSAGE_BYTES,
            max_sockets_per_run: MAX_SOCKETS_PER_RUN,
            max_sockets_total: MAX_SOCKETS_TOTAL,
        }
    }
}

// Counts the sockets of the whole process, the per-run count lives in the run's entry
pub struct SocketLimits {
    per_run: usize,
    total: usize,
    active: AtomicUsize,
}

impl SocketLimits {
    pub fn new(config: &WsConfig) -> Arc<Self> {
        Arc::new(Self {
            per_run: config.max_sockets_per_run,
            total: config.max_sockets_total,
            active: AtomicUsize::new(0),
        })
    }

    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    // A place for one more socket on this run, or None when the run or the process is full
    pub fn try_acquire(self: &Arc<Self>, entry: &Arc<RunEntry>) -> Option<SocketPermit> {
        if self.active.fetch_add(1, Ordering::SeqCst) >= self.total {
            self.active.fetch_sub(1, Ordering::SeqCst);

            return None;
        }

        if !entry.try_acquire_socket(self.per_run) {
            self.active.fetch_sub(1, Ordering::SeqCst);

            return None;
        }

        Some(SocketPermit {
            limits: Arc::clone(self),
            entry: Arc::clone(entry),
        })
    }
}

// Held for as long as a socket is attached, released on every way a socket can end
pub struct SocketPermit {
    limits: Arc<SocketLimits>,
    entry: Arc<RunEntry>,
}

impl Drop for SocketPermit {
    fn drop(&mut self) {
        self.entry.release_socket();
        self.limits.active.fetch_sub(1, Ordering::SeqCst);
    }
}
