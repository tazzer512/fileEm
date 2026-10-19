//! Runs file operations with a row in the progress window and inline conflict questions (the
//! Java `OperationUi`). Each operation runs on its own thread; quick ones never show up, slower
//! ones get a row after 350 ms. Conflicts are asked in the operation's row, never with a modal
//! dialog: the operation's thread waits on a channel (checking for cancel), the UI never blocks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, VecModel};

use fileem_core::ops::{Decision, FileOperation, Handle, Kind, Resolution, State};

use crate::progress::{self, Conflict, Question, Row};
use crate::{OpRow, ProgressWindow};

/// How long an operation may run before it gets a row in the progress window.
pub const SHOW_AFTER: Duration = Duration::from_millis(350);

/// What a finished operation reports to whoever started it.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub title: String,
    pub state: State,
    pub errors: Vec<String>,
    /// Folders whose contents changed (the target and the sources' parents).
    pub affected: Vec<PathBuf>,
    /// What `Compress` / `Extract` created.
    pub result: Option<PathBuf>,
}

type OnDone = Box<dyn FnOnce(Outcome)>;

struct Running {
    handle: Handle,
    title: String,
    on_done: Option<OnDone>,
}

struct Inner {
    running: HashMap<u64, Running>,
    /// Rows shown, in the order they were added.
    rows: Vec<Row>,
    window: Option<ProgressWindow>,
    model: Rc<VecModel<OpRow>>,
    ticker: slint::Timer,
    started: Instant,
    ticks: u32,
}

/// The operations of one main window and their progress window. Cheap to clone.
#[derive(Clone)]
pub struct OpsUi {
    id: u64,
    inner: Rc<RefCell<Inner>>,
}

thread_local! {
    /// Lets operation threads reach their `OpsUi` on the UI thread by id.
    static ALL: RefCell<HashMap<u64, std::rc::Weak<RefCell<Inner>>>> = RefCell::new(HashMap::new());
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Default for OpsUi {
    fn default() -> Self {
        OpsUi::new()
    }
}

impl OpsUi {
    pub fn new() -> OpsUi {
        let id = next_id();
        let inner = Rc::new(RefCell::new(Inner {
            running: HashMap::new(),
            rows: Vec::new(),
            window: None,
            model: Rc::new(VecModel::default()),
            ticker: slint::Timer::default(),
            started: Instant::now(),
            ticks: 0,
        }));
        ALL.with(|all| all.borrow_mut().insert(id, Rc::downgrade(&inner)));
        OpsUi { id, inner }
    }

    fn find(id: u64) -> Option<OpsUi> {
        ALL.with(|all| all.borrow().get(&id).and_then(|w| w.upgrade())).map(|inner| OpsUi { id, inner })
    }

    /// Starts `kind` on `sources`, asking conflicts in the progress window; `on_done` runs on the
    /// UI thread when it ends.
    pub fn run(
        &self,
        kind: Kind,
        sources: Vec<PathBuf>,
        target: Option<PathBuf>,
        on_done: impl FnOnce(Outcome) + 'static,
    ) -> Handle {
        let mut op = FileOperation::new(kind, sources, target);
        let (ui_id, op_id, handle) = (self.id, next_id(), op.handle());
        op.on_conflict(move |src, dst| ask_from_worker(ui_id, op_id, &handle, src, dst));
        self.start_with_id(op_id, op, on_done)
    }

    /// Starts an operation built elsewhere (tests pass one with their own conflict handler).
    pub fn start(&self, op: FileOperation, on_done: impl FnOnce(Outcome) + 'static) -> Handle {
        self.start_with_id(next_id(), op, on_done)
    }

    fn start_with_id(&self, op_id: u64, op: FileOperation, on_done: impl FnOnce(Outcome) + 'static) -> Handle {
        let handle = op.handle();
        let title = progress::title(handle.kind(), handle.sources(), handle.target_dir());
        self.inner.borrow_mut().running.insert(
            op_id,
            Running { handle: handle.clone(), title, on_done: Some(Box::new(on_done)) },
        );
        let ui_id = self.id;
        std::thread::Builder::new()
            .name("fileem-operation".into())
            .spawn(move || {
                op.run();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = OpsUi::find(ui_id) {
                        ui.finished(op_id);
                    }
                });
            })
            .expect("spawn an operation thread");
        let me = self.clone();
        slint::Timer::single_shot(SHOW_AFTER, move || me.show_row(op_id));
        handle
    }

    /// Operations started and not yet finished (running, paused or asking).
    pub fn running(&self) -> Vec<Handle> {
        self.inner.borrow().running.values().map(|r| r.handle.clone()).collect()
    }

    /// Cancels every running operation and waits up to `timeout` for all of them to stop,
    /// including their cleanup. Call off the UI thread (with handles from [`OpsUi::running`]).
    pub fn cancel_all_and_wait(handles: &[Handle], timeout: Duration) -> bool {
        handles.iter().for_each(Handle::cancel);
        let deadline = Instant::now() + timeout;
        handles.iter().all(|h| h.await_stopped(deadline.saturating_duration_since(Instant::now())))
    }

    /// The progress window while it shows rows (tests).
    pub fn window(&self) -> Option<ProgressWindow> {
        let inner = self.inner.borrow();
        inner.window.as_ref().filter(|_| !inner.rows.is_empty()).map(|w| w.clone_strong())
    }

    pub fn rows(&self) -> Vec<OpRow> {
        use slint::Model;
        self.inner.borrow().model.iter().collect()
    }

    // ---- rows ----

    fn show_row(&self, op_id: u64) {
        let (handle, title) = {
            let inner = self.inner.borrow();
            match inner.running.get(&op_id) {
                Some(r) if !r.handle.is_done() => (r.handle.clone(), r.title.clone()),
                _ => return,
            }
        };
        if self.inner.borrow().rows.iter().any(|r| r.id == op_id) {
            return;
        }
        self.inner.borrow_mut().rows.push(Row::new(op_id, handle, title));
        self.open_window();
        self.sync();
    }

    fn open_window(&self) {
        if self.inner.borrow().window.is_none() {
            let Ok(w) = ProgressWindow::new() else { return };
            w.set_rows(ModelRc::from(self.inner.borrow().model.clone()));
            let me = self.clone();
            w.on_pause(move |id| me.with_row(id, Row::toggle_pause));
            let me = self.clone();
            w.on_cancel(move |id| me.with_row(id, Row::cancel));
            let me = self.clone();
            w.on_answer(move |id, res, all| {
                let resolution = match res {
                    0 => Resolution::Overwrite,
                    1 => Resolution::Skip,
                    _ => Resolution::KeepBoth,
                };
                me.with_row(id, |r| r.answer(resolution, all));
            });
            let me = self.clone();
            // closing the window cancels everything in it
            w.window().on_close_requested(move || {
                let ids: Vec<u64> = me.inner.borrow().rows.iter().map(|r| r.id).collect();
                for id in ids {
                    me.with_row(id as i32, Row::cancel);
                }
                slint::CloseRequestResponse::HideWindow
            });
            self.inner.borrow_mut().window = Some(w);
        }
        let inner = self.inner.borrow();
        if let Some(w) = &inner.window {
            let _ = w.show();
        }
        if !inner.ticker.running() {
            let me = self.clone();
            inner.ticker.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || me.tick());
        }
    }

    fn with_row(&self, id: i32, f: impl FnOnce(&mut Row)) {
        if let Some(r) = self.inner.borrow_mut().rows.iter_mut().find(|r| r.id == id as u64) {
            f(r);
        }
        self.sync();
    }

    /// Every 200 ms: progress and messages; once a second: speed and time left.
    fn tick(&self) {
        {
            let mut inner = self.inner.borrow_mut();
            inner.ticks += 1;
            if inner.ticks.is_multiple_of(5) {
                let now = inner.started.elapsed();
                inner.rows.iter_mut().for_each(|r| r.tick(now));
            }
        }
        self.sync();
    }

    fn sync(&self) {
        let inner = self.inner.borrow();
        let rows: Vec<OpRow> = inner
            .rows
            .iter()
            .map(|r| {
                let q = r.question.as_ref().map(|q| &q.conflict);
                OpRow {
                    id: r.id as i32,
                    title: r.title.clone().into(),
                    message: r.handle.message().into(),
                    progress: r.fraction(),
                    detail: r.detail().into(),
                    paused: r.handle.is_paused(),
                    cancelling: r.cancelling,
                    asking: q.is_some(),
                    question: q.map(|c| c.question.clone()).unwrap_or_default().into(),
                    source: q.map(|c| c.source.clone()).unwrap_or_default().into(),
                    existing: q.map(|c| c.existing.clone()).unwrap_or_default().into(),
                }
            })
            .collect();
        inner.model.set_vec(rows);
        if let Some(w) = &inner.window {
            w.set_window_title(progress::window_title(&inner.rows).into());
        }
    }

    /// A conflict question from an operation's thread: shown in its row (added if needed).
    fn ask(&self, op_id: u64, conflict: Conflict, answer: mpsc::Sender<Decision>) {
        let handle = match self.inner.borrow().running.get(&op_id) {
            Some(r) if !r.handle.is_cancelled() => r.handle.clone(),
            _ => return, // dropping `answer` reads as Cancel
        };
        if !self.inner.borrow().rows.iter().any(|r| r.id == op_id) {
            let title = progress::title(handle.kind(), handle.sources(), handle.target_dir());
            self.inner.borrow_mut().rows.push(Row::new(op_id, handle, title));
        }
        if let Some(r) = self.inner.borrow_mut().rows.iter_mut().find(|r| r.id == op_id) {
            r.question = Some(Question { conflict, answer });
        }
        self.open_window();
        self.sync();
    }

    fn finished(&self, op_id: u64) {
        let Some(mut running) = self.inner.borrow_mut().running.remove(&op_id) else { return };
        {
            let mut inner = self.inner.borrow_mut();
            inner.rows.retain(|r| r.id != op_id);
            if inner.rows.is_empty() {
                inner.ticker.stop();
                if let Some(w) = &inner.window {
                    let _ = w.hide();
                }
            }
        }
        self.sync();
        let h = &running.handle;
        let mut affected: Vec<PathBuf> = h.target_dir().map(Path::to_path_buf).into_iter().collect();
        for s in h.sources() {
            if let Some(parent) = s.parent() {
                if !affected.iter().any(|a| a == parent) {
                    affected.push(parent.to_path_buf());
                }
            }
        }
        let outcome = Outcome { title: running.title.clone(), state: h.state(), errors: h.errors(), affected, result: h.result() };
        if let Some(on_done) = running.on_done.take() {
            on_done(outcome);
        }
    }
}

/// The conflict handler of operations started with [`OpsUi::run`]: runs on the operation's thread,
/// reads the file details there, and waits for the answer from the progress window. A cancel
/// (from the row, the window, or quitting) ends the wait with Cancel.
fn ask_from_worker(ui_id: u64, op_id: u64, handle: &Handle, src: &Path, dst: &Path) -> Decision {
    let conflict = Conflict::describe(src, dst);
    let (tx, rx) = mpsc::channel();
    let posted = slint::invoke_from_event_loop(move || match OpsUi::find(ui_id) {
        Some(ui) => ui.ask(op_id, conflict, tx),
        None => drop(tx),
    });
    if posted.is_err() {
        return Decision::once(Resolution::Cancel);
    }
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(d) => return d,
            Err(mpsc::RecvTimeoutError::Timeout) if !handle.is_cancelled() => continue,
            Err(_) => return Decision::once(Resolution::Cancel),
        }
    }
}
