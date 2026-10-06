use std::time::Duration;

// One finished request, successful or not
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestObservation {
    pub duration: Duration,
    pub success: bool,
}
