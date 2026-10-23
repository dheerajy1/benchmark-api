use std::{
    thread,
    time::{Duration, Instant},
};

use super::live::RunControl;

// Why a step ended without a result of its own
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    // A cancel request was seen, the step has stopped what it was doing
    Cancelled,
    Failed(String),
}

impl From<String> for Stop {
    fn from(message: String) -> Self {
        Stop::Failed(message)
    }
}

impl Stop {
    pub fn into_message(self) -> String {
        match self {
            Stop::Cancelled => "The benchmark was cancelled.".to_string(),
            Stop::Failed(message) => message,
        }
    }
}

// Sleeps for the given time unless a cancel request comes first, and answers whether it did.
// It looks at the request every 25 ms, so a long wait never delays a cancellation by more.
pub fn sleep_unless_cancelled(total: Duration, control: &RunControl) -> bool {
    let deadline = Instant::now() + total;

    loop {
        if control.is_cancel_requested() {
            return true;
        }

        let now = Instant::now();

        if now >= deadline {
            return false;
        }

        thread::sleep((deadline - now).min(Duration::from_millis(25)));
    }
}
