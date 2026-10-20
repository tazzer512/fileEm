//! Drives (the Java `Volumes` polling, `FolderTree.syncVolumes` and `MainWindow.eject`): a
//! thread lists the drives every 5 s (and right after an eject) and hands changed lists to the
//! window; both trees show them with their free space. Ejecting removes the whole drive: refused
//! while an operation uses it, tabs on it move home first, then the blocking eject runs in the
//! background. Tests swap the real eject ([`Ejector`]) and show drives of their own.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fileem_core::fs::volumes::{self, Volume};
use fileem_core::util::{formats, fs_util, os};

use super::commands::{self, Dialog, DialogSpec};
use super::{nav, sync_tree, Shared};
use crate::bg;
use crate::tab::Nav;
use crate::tree::Drive;

const POLL_EVERY: Duration = Duration::from_secs(5);

/// Ejects drives; both calls block and run in the background. Tests put in their own.
pub trait Ejector: Send + Sync {
    /// Every mount point that goes away with the drive holding `root`, `root` first.
    fn drive(&self, root: &Path) -> Vec<PathBuf> {
        vec![root.to_path_buf()]
    }

    fn eject(&self, roots: &[PathBuf]) -> Result<(), String>;
}

/// The real thing: `volumes::same_drive` and `volumes::eject`.
pub struct SystemEjector;

impl Ejector for SystemEjector {
    fn drive(&self, root: &Path) -> Vec<PathBuf> {
        volumes::same_drive(root)
    }

    fn eject(&self, roots: &[PathBuf]) -> Result<(), String> {
        volumes::eject(roots)
    }
}

/// The polling thread; dropping it stops the thread.
pub struct Poller {
    stop: Arc<AtomicBool>,
    wake: Sender<()>,
    /// The newest list, waiting for the UI thread.
    latest: Arc<Mutex<Option<Vec<Volume>>>>,
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.wake.send(());
    }
}

impl Poller {
    /// Asks for a new list now instead of at the next poll (after an eject).
    fn poll_now(&self) {
        let _ = self.wake.send(());
    }
}

/// Starts polling; `first` is the list shown so far (only changes are handed over).
pub fn start(app: &Shared, first: Vec<Volume>) {
    let stop = Arc::new(AtomicBool::new(false));
    let latest = Arc::new(Mutex::new(None));
    let (wake, rx) = mpsc::channel::<()>();
    let ui = app.borrow().ui.clone();
    let (thread_stop, thread_latest) = (stop.clone(), latest.clone());
    let started = std::thread::Builder::new().name("fileem-volumes".into()).spawn(move || {
        let mut last = first;
        loop {
            let list = volumes::list();
            if thread_stop.load(Ordering::Relaxed) {
                return;
            }
            if list != last {
                last = list.clone();
                *thread_latest.lock().unwrap() = Some(list);
                let _ = ui.upgrade_in_event_loop(|ui| ui.invoke_volumes_changed());
            }
            match rx.recv_timeout(POLL_EVERY) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if thread_stop.load(Ordering::Relaxed) {
                return;
            }
        }
    });
    if started.is_ok() {
        app.borrow_mut().poller = Some(Poller { stop, wake, latest });
    }
}

/// The poller found a new list.
pub fn changed(app: &Shared) {
    let list = app.borrow().poller.as_ref().and_then(|p| p.latest.lock().unwrap().take());
    if let Some(list) = list {
        app.borrow_mut().volumes = list;
        sync(app);
    }
}

/// Shows `list` instead of the system's drives and stops following them (tests).
pub fn show(app: &Shared, list: Vec<Volume>) {
    {
        let mut a = app.borrow_mut();
        a.poller = None;
        a.volumes = list;
    }
    sync(app);
}

/// The tree's rows for `list`: the free space, or "Ejecting…".
pub fn drives(list: &[Volume], ejecting: &HashSet<PathBuf>) -> Vec<Drive> {
    list.iter()
        .map(|v| Drive {
            name: v.label.clone(),
            path: v.root.clone(),
            kind: v.kind,
            extra: if ejecting.contains(&v.root) {
                Some("Ejecting…".into())
            } else {
                v.space.map(|(free, total)| format!("{} free of {}", formats::size(Some(free)), formats::size(Some(total))))
            },
            ejectable: v.ejectable,
        })
        .collect()
}

/// Both trees show the current list; a tree whose folder went away with a drive shows the tab's
/// folder again (wherever it is now).
pub fn sync(app: &Shared) {
    let list = {
        let a = app.borrow();
        drives(&a.volumes, &a.ejecting)
    };
    for side in 0..2 {
        let lost = {
            let mut a = app.borrow_mut();
            a.sides[side].tree.set_volumes(&list);
            a.sides[side].tree.current().is_none()
        };
        sync_tree(app, side);
        let dir = app.borrow().sides[side].tab().dir().map(Path::to_path_buf);
        if let (true, Some(d)) = (lost, dir) {
            nav::reveal(app, side, d);
        }
    }
}

pub fn is_ejecting(app: &Shared, root: &Path) -> bool {
    app.borrow().ejecting.contains(root)
}

fn on_any(dir: Option<&Path>, roots: &[PathBuf]) -> bool {
    dir.is_some_and(|d| roots.iter().any(|r| d.starts_with(r)))
}

/// Safely removes the drive at `root`, with all its other mounted partitions (the whole drive
/// goes, as in Windows and macOS). Refused while a file operation uses it. Tabs showing a folder
/// on it move to the home folder first (an open folder keeps the drive busy: watchers,
/// listings); then it is unmounted and powered off in the background.
pub fn eject(app: &Shared, root: PathBuf, label: String) {
    if !app.borrow_mut().ejecting.insert(root.clone()) {
        return;
    }
    sync(app);
    let ejector = app.borrow().ejector.clone();
    let (done_app, asked) = (app.clone(), root.clone());
    bg::run(
        move |_| ejector.drive(&asked),
        move |roots: Vec<PathBuf>| {
            let app = &done_app;
            let busy = app.borrow().ops.running().iter().any(|h| {
                roots.iter().any(|r| h.target_dir().is_some_and(|t| t.starts_with(r)) || h.sources().iter().any(|s| s.starts_with(r)))
            });
            if busy {
                finished(app, std::slice::from_ref(&root));
                let msg = format!(
                    "\u{201c}{label}\u{201d} is in use by a running file operation.\nWait for it to finish or cancel it, then try again."
                );
                return commands::open_dialog(app, Dialog::Info, DialogSpec::info("Eject", &msg));
            }
            app.borrow_mut().ejecting.extend(roots.iter().cloned());
            sync(app);
            let leaving: Vec<u64> = {
                let a = app.borrow();
                a.sides.iter().flat_map(|s| s.tabs.iter()).filter(|t| on_any(t.tab.dir(), &roots)).map(|t| t.id).collect()
            };
            for &id in &leaving {
                nav::go(app, id, os::home(), Nav::Go);
            }
            when_left(app.clone(), leaving, roots, label, 50);
        },
    );
}

/// Runs the eject once no tab in `tabs` shows a folder on `roots` (or after `tries` × 100 ms).
fn when_left(app: Shared, tabs: Vec<u64>, roots: Vec<PathBuf>, label: String, tries: u32) {
    let still = {
        let a = app.borrow();
        tabs.iter().any(|&id| a.find(id).is_some_and(|(s, i)| on_any(a.sides[s].tabs[i].tab.dir(), &roots)))
    };
    if still && tries > 0 {
        slint::Timer::single_shot(Duration::from_millis(100), move || when_left(app, tabs, roots, label, tries - 1));
        return;
    }
    let ejector = app.borrow().ejector.clone();
    let (done_app, ejected) = (app.clone(), roots.clone());
    bg::run(
        move |_| ejector.eject(&ejected),
        move |result: Result<(), String>| {
            let app = &done_app;
            finished(app, &roots);
            let msg = match result {
                Ok(()) => {
                    if let Some(p) = app.borrow().poller.as_ref() {
                        p.poll_now();
                    }
                    let what = match roots.as_slice() {
                        [_] => format!("\u{201c}{label}\u{201d}"),
                        many => format!(
                            "The drive with {}",
                            many.iter().map(|r| format!("\u{201c}{}\u{201d}", fs_util::display_name(r))).collect::<Vec<_>>().join(" and ")
                        ),
                    };
                    format!("{what} can now be safely removed.")
                }
                Err(e) => format!("Cannot eject \u{201c}{label}\u{201d}:\n{e}"),
            };
            commands::open_dialog(app, Dialog::Info, DialogSpec::info("Eject", &msg));
        },
    );
}

fn finished(app: &Shared, roots: &[PathBuf]) {
    {
        let mut a = app.borrow_mut();
        for r in roots {
            a.ejecting.remove(r);
        }
    }
    sync(app);
}
