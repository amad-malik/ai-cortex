//! Search's OWN concurrency cap (S3 r2.7 R-9 case b, R-16's exception; v1 incident 912073e2): at most `n` searches at once, and a search that
//! finds no place is refused AT ONCE with 429 `busy`. It never waits: queueing under overload turned a slow query into an outage in v1.
//! The cap is separate from `cortex-http`'s interim global cap (which queues); it is taken after the request's bounds and before the first
//! database statement, so an invalid request takes no place and a refused one touches no database.

use std::borrow::Cow;
use std::sync::Arc;

use cortex_module::Problem;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The cap: at most `n` searches at once. Cloning shares the places.
#[derive(Clone)]
pub struct Cap(Arc<Semaphore>);

/// A place; it comes back when this is dropped (an answer, a refusal, a panic's unwind or a dropped request).
pub struct Permit(#[allow(dead_code)] OwnedSemaphorePermit);

impl Cap {
    pub fn new(n: usize) -> Cap {
        Cap(Arc::new(Semaphore::new(n)))
    }

    /// A place, or the 429 `busy` problem at once. The chain adds `Retry-After: 1` to every 429.
    pub fn try_enter(&self) -> Result<Permit, Problem> {
        Arc::clone(&self.0)
            .try_acquire_owned()
            .map(Permit)
            .map_err(|_| busy())
    }
}

/// 429 `busy` (a platform code, minted by `cortex-http`) with search's own text.
fn busy() -> Problem {
    Problem {
        code: "busy",
        status: 429,
        title: "Busy",
        detail: Cow::Borrowed("Search is answering as many requests as it may at once."),
        fix: "Retry after the number of seconds in Retry-After; a search takes no Idempotency-Key.",
        scope: None,
    }
}
