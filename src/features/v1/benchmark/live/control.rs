use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

const RUNNING: u8 = 0;
const CANCEL_REQUESTED: u8 = 1;
const FINALIZING: u8 = 2;

// What asking a run to stop led to
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelRequest {
    // This request started the cancellation
    Requested,
    // Cancellation was already requested, nothing changes
    AlreadyRequested,
    // The run has its result and is saving it, too late to cancel
    Finalizing,
}

// The one switch between a run that is working, a run that was asked to stop and a run that
// is saving its result. Every change is a single atomic compare-and-swap away from `Running`,
// so cancelling and finishing can never both win:
//
//   Running -> CancelRequested   a cancel request got there first, the run ends as cancelled
//   Running -> Finalizing        the benchmark finished first, cancelling is refused from then on
//
// The execution reads it to stop early. The supervisor decides the end of the run with it.
#[derive(Clone, Default)]
pub struct RunControl {
    state: Arc<AtomicU8>,
}

impl RunControl {
    pub fn request_cancel(&self) -> CancelRequest {
        match self.state.compare_exchange(
            RUNNING,
            CANCEL_REQUESTED,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => CancelRequest::Requested,
            Err(CANCEL_REQUESTED) => CancelRequest::AlreadyRequested,
            Err(_) => CancelRequest::Finalizing,
        }
    }

    // True when the run may go on to save its result, false when a cancel request won
    pub fn begin_finalizing(&self) -> bool {
        self.state
            .compare_exchange(RUNNING, FINALIZING, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn is_cancel_requested(&self) -> bool {
        self.state.load(Ordering::SeqCst) == CANCEL_REQUESTED
    }
}
