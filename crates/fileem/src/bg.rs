//! Background work with results back on the UI thread (the Java `Bg.run`).
//!
//! Each job gets its own thread, so a hung network mount never delays anything else. The
//! callback runs on the Slint event loop; it is dropped, never called, when the job is cancelled
//! (the generation counters in the callers drop stale results too).

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

type Callback = Box<dyn FnOnce(Box<dyn Any + Send>)>;

thread_local! {
    /// Callbacks waiting for their job, by job id. Lives on the UI thread only.
    static PENDING: RefCell<HashMap<u64, Callback>> = RefCell::new(HashMap::new());
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A running job. Cancelling sets the flag the work can poll and drops the callback.
#[derive(Clone)]
pub struct Job {
    id: u64,
    cancelled: Arc<AtomicBool>,
}

impl Job {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        PENDING.with(|p| p.borrow_mut().remove(&self.id));
    }
}

/// Runs `work` on a new thread and `done` with its result on the UI thread. `work` gets the
/// cancel flag to stop long loops early. Call on the UI thread.
pub fn run<T, W, D>(work: W, done: D) -> Job
where
    T: Send + 'static,
    W: FnOnce(&AtomicBool) -> T + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let cancelled = Arc::new(AtomicBool::new(false));
    PENDING.with(|p| {
        p.borrow_mut().insert(
            id,
            Box::new(move |any: Box<dyn Any + Send>| {
                if let Ok(value) = any.downcast::<T>() {
                    done(*value);
                }
            }),
        );
    });
    let flag = cancelled.clone();
    std::thread::Builder::new()
        .name("fileem-io".into())
        .spawn(move || {
            let result = work(&flag);
            if flag.load(Ordering::SeqCst) {
                return;
            }
            // fails only when the event loop is gone (app closing): nothing to deliver to then
            let _ = slint::invoke_from_event_loop(move || deliver(id, Box::new(result)));
        })
        .expect("spawn a background thread");
    Job { id, cancelled }
}

fn deliver(id: u64, result: Box<dyn Any + Send>) {
    let callback = PENDING.with(|p| p.borrow_mut().remove(&id));
    if let Some(callback) = callback {
        callback(result);
    }
}
