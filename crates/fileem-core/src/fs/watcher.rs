//! Shared directory watcher (the Java `DirWatcher`): one OS watcher for the whole app, listeners
//! per folder, bursts of changes debounced per folder. Listeners run on a background thread; the
//! UI forwards them to its event loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

const DEBOUNCE: Duration = Duration::from_millis(400);

type Listener = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Entry {
    listeners: Vec<(u64, Listener)>,
    pending: bool,
}

struct Shared {
    watcher: Mutex<RecommendedWatcher>,
    entries: Arc<Mutex<HashMap<PathBuf, Entry>>>,
    /// Held while a folder's listeners and its OS watch change together: otherwise the last
    /// handle of a folder, dropped while a new watch on it starts, could remove the OS watch the
    /// new one just made, and the new listener would never hear of a change. (Not `entries`: the
    /// watcher's own thread takes that one, and unwatching waits for that thread.)
    registering: Mutex<()>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn shared() -> Option<&'static Shared> {
    static SHARED: OnceLock<Option<Shared>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            let entries: Arc<Mutex<HashMap<PathBuf, Entry>>> = Arc::default();
            let routed = entries.clone();
            let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event {
                    if !is_change(&event.kind) {
                        return;
                    }
                    for p in &event.paths {
                        // a change of an item concerns its folder; one of the folder itself, the folder
                        changed(&routed, p.parent());
                        changed(&routed, Some(p));
                    }
                }
            })
            .ok()?;
            Some(Shared { watcher: Mutex::new(watcher), entries, registering: Mutex::new(()) })
        })
        .as_ref()
}

/// Whether an event changes what a listing shows. Reads are none: inotify reports opening the
/// folder or a file in it, so every listing (and thumbnail, preview, or another program reading)
/// would otherwise refresh the folder again, endlessly. A file closed after writing is one.
fn is_change(kind: &EventKind) -> bool {
    !matches!(kind, EventKind::Access(a) if *a != AccessKind::Close(AccessMode::Write))
}

/// Debounces: the first change starts a timer; changes until it fires are folded into one call.
fn changed(entries: &Arc<Mutex<HashMap<PathBuf, Entry>>>, dir: Option<&Path>) {
    let Some(dir) = dir else { return };
    let mut map = entries.lock().unwrap();
    let Some(entry) = map.get_mut(dir) else { return };
    if entry.pending {
        return;
    }
    entry.pending = true;
    let (entries, dir) = (entries.clone(), dir.to_path_buf());
    std::thread::spawn(move || {
        std::thread::sleep(DEBOUNCE);
        let listeners: Vec<Listener> = {
            let mut map = entries.lock().unwrap();
            match map.get_mut(&dir) {
                Some(e) => {
                    e.pending = false;
                    e.listeners.iter().map(|(_, l)| l.clone()).collect()
                }
                None => Vec::new(),
            }
        };
        for l in listeners {
            l();
        }
    });
}

/// Stops its listener when dropped.
pub struct WatchHandle {
    dir: PathBuf,
    id: u64,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        let Some(s) = shared() else { return };
        let _registering = s.registering.lock().unwrap();
        let mut map = s.entries.lock().unwrap();
        let Some(entry) = map.get_mut(&self.dir) else { return };
        entry.listeners.retain(|(id, _)| *id != self.id);
        if entry.listeners.is_empty() {
            map.remove(&self.dir);
            drop(map);
            #[cfg(test)]
            tests::pause_point(&self.dir);
            let _ = s.watcher.lock().unwrap().unwatch(&self.dir);
        }
    }
}

/// Calls `on_change` (on a background thread) whenever the content of `dir` changes, until the
/// handle is dropped. Registering touches the file system (it can block on a hung mount): call it
/// off the UI thread. A folder that can't be watched simply never reports.
pub fn watch(dir: &Path, on_change: impl Fn() + Send + Sync + 'static) -> WatchHandle {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let handle = WatchHandle { dir: dir.to_path_buf(), id };
    let Some(s) = shared() else { return handle };
    let _registering = s.registering.lock().unwrap();
    // registered again on every watch: a folder deleted and created again at the same path needs
    // a new OS watch, or it would never report again
    let _ = s.watcher.lock().unwrap().watch(dir, RecursiveMode::NonRecursive);
    s.entries.lock().unwrap().entry(dir.to_path_buf()).or_default().listeners.push((id, Arc::new(on_change)));
    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    const WAIT: Duration = Duration::from_secs(15);

    /// Folders whose last handle, when dropped, waits between forgetting its listeners and
    /// removing the OS watch until the test says so.
    static PAUSES: Mutex<Vec<(PathBuf, mpsc::Receiver<()>)>> = Mutex::new(Vec::new());

    pub(super) fn pause_point(dir: &Path) {
        let pause = {
            let mut p = PAUSES.lock().unwrap();
            p.iter().position(|(d, _)| d == dir).map(|i| p.remove(i).1)
        };
        if let Some(rx) = pause {
            let _ = rx.recv_timeout(WAIT);
        }
    }

    fn counter() -> (Arc<AtomicUsize>, mpsc::Receiver<()>, impl Fn() + Send + Sync + 'static) {
        let calls = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        let c = calls.clone();
        let tx = Mutex::new(tx);
        (calls, rx, move || {
            c.fetch_add(1, Ordering::SeqCst);
            let _ = tx.lock().unwrap().send(());
        })
    }

    #[test]
    fn notifies_on_changes_and_stops_after_the_handle_is_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let (calls, rx, listener) = counter();
        let handle = watch(tmp.path(), listener);
        fs::write(tmp.path().join("new.txt"), "x").unwrap();
        rx.recv_timeout(WAIT).expect("change was reported");

        drop(handle);
        std::thread::sleep(DEBOUNCE + Duration::from_millis(200));
        let before = calls.load(Ordering::SeqCst);
        fs::write(tmp.path().join("after-close.txt"), "x").unwrap();
        std::thread::sleep(DEBOUNCE * 3);
        assert_eq!(calls.load(Ordering::SeqCst), before, "no notifications after the handle is gone");
    }

    #[test]
    fn a_folder_deleted_and_created_again_is_watched_again() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        let (_, first, listener) = counter();
        let handle = watch(&sub, listener);
        fs::write(sub.join("a.txt"), "x").unwrap();
        first.recv_timeout(WAIT).unwrap();
        fs::remove_file(sub.join("a.txt")).unwrap();
        fs::remove_dir(&sub).unwrap();
        std::thread::sleep(DEBOUNCE + Duration::from_millis(200));
        fs::create_dir(&sub).unwrap();

        let (_, again, listener) = counter();
        let _second = watch(&sub, listener);
        fs::write(sub.join("b.txt"), "x").unwrap();
        again.recv_timeout(WAIT).expect("changes in the new folder are reported");
        drop(handle);
    }

    #[test]
    fn a_new_watch_while_the_last_old_one_is_dropped_keeps_reporting() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let (_, _old_rx, old_listener) = counter();
        let old = watch(&dir, old_listener);
        let (go, paused) = mpsc::channel();
        PAUSES.lock().unwrap().push((dir.clone(), paused));
        // the old handle goes (another tab left the folder) ...
        let dropping = std::thread::spawn(move || drop(old));
        std::thread::sleep(Duration::from_millis(100));
        // ... while a new one starts on the same folder
        let (_, rx, listener) = counter();
        let starting = std::thread::spawn({
            let dir = dir.clone();
            move || watch(&dir, listener)
        });
        std::thread::sleep(Duration::from_millis(200));
        go.send(()).unwrap();
        dropping.join().unwrap();
        let _new = starting.join().unwrap();
        fs::write(dir.join("new.txt"), "x").unwrap();
        rx.recv_timeout(Duration::from_secs(3)).expect("the new watch still reports changes");
    }

    #[test]
    fn bursts_of_changes_are_debounced() {
        let tmp = tempfile::tempdir().unwrap();
        let (calls, rx, listener) = counter();
        let _handle = watch(tmp.path(), listener);
        for i in 0..200 {
            fs::write(tmp.path().join(format!("f{i}")), "x").unwrap();
        }
        rx.recv_timeout(WAIT).unwrap();
        std::thread::sleep(DEBOUNCE * 3);
        let n = calls.load(Ordering::SeqCst);
        assert!(n <= 5, "200 writes caused {n} notifications");
    }

    #[test]
    fn reading_the_folder_or_its_files_is_no_change() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), "x").unwrap();
        std::thread::sleep(DEBOUNCE);
        let (calls, _rx, listener) = counter();
        let _handle = watch(tmp.path(), listener);
        // what a listing, a thumbnail or a preview does: open the folder and read files
        for _ in 0..3 {
            let _ = fs::read_dir(tmp.path()).unwrap().count();
            let _ = fs::read(tmp.path().join("a.txt")).unwrap();
        }
        std::thread::sleep(DEBOUNCE * 3);
        // regression: opens were reported, so every listing refreshed the folder again, forever
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn changes_in_a_subfolder_do_not_report_for_the_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        std::thread::sleep(DEBOUNCE);
        let (calls, _rx, listener) = counter();
        let _handle = watch(tmp.path(), listener);
        fs::write(sub.join("deep.txt"), "x").unwrap();
        std::thread::sleep(DEBOUNCE * 3);
        // the folder's own modification time may change, which some systems report; the file
        // inside it is not the parent's business
        assert!(calls.load(Ordering::SeqCst) <= 1);
    }
}
