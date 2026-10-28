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

/// macOS's FSEvents backend restarts its one stream on every watch and unwatch, "since now":
/// changes not yet delivered are dropped. Each restart then counts as a change of every folder
/// already watched (one debounced refresh each).
const RESTARTS_LOSE_EVENTS: bool = cfg!(target_os = "macos");

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
            let watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| route(&routed, event)).ok()?;
            Some(Shared { watcher: Mutex::new(watcher), entries, registering: Mutex::new(()) })
        })
        .as_ref()
}

/// Hands an event to the folders it concerns.
fn route(entries: &Arc<Mutex<HashMap<PathBuf, Entry>>>, event: notify::Result<notify::Event>) {
    match event {
        // the OS dropped events (inotify's queue overflowed in a big copy) or reported an error:
        // any folder may have changed
        Ok(event) if event.need_rescan() => changed_all(entries),
        Err(_) => changed_all(entries),
        Ok(event) => {
            if !is_change(&event.kind) {
                return;
            }
            for p in &event.paths {
                // a change of an item concerns its folder; one of the folder itself, the folder
                changed(entries, p.parent());
                changed(entries, Some(p));
            }
        }
    }
}

fn changed_all(entries: &Arc<Mutex<HashMap<PathBuf, Entry>>>) {
    let dirs: Vec<PathBuf> = entries.lock().unwrap().keys().cloned().collect();
    for dir in dirs {
        changed(entries, Some(&dir));
    }
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
            if RESTARTS_LOSE_EVENTS {
                changed_all(&s.entries);
            }
        }
    }
}

/// Windows can't rename, move or delete a folder while a folder in it (or itself) is watched: the
/// watch holds it open ("Access denied"). Releases the OS watches on `path` and below until the
/// guard is dropped, which watches the folders still wanted and existing again and reports them
/// changed. notify closes a released watch on its own thread, moments later: callers retry
/// briefly ([`retry_while_busy`]). Elsewhere it does nothing.
pub fn release_under(path: &Path) -> Released {
    let mut released = Released { dirs: Vec::new() };
    if !RELEASE_TO_CHANGE {
        return released;
    }
    let Some(s) = shared() else { return released };
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let _registering = s.registering.lock().unwrap();
    released.dirs = s.entries.lock().unwrap().keys().filter(|d| d.starts_with(&path)).cloned().collect();
    let mut watcher = s.watcher.lock().unwrap();
    for d in &released.dirs {
        let _ = watcher.unwatch(d);
    }
    released
}

/// Watches held open block changing their folders on Windows only.
const RELEASE_TO_CHANGE: bool = cfg!(windows);

/// Watches given back when dropped (see [`release_under`]).
pub struct Released {
    dirs: Vec<PathBuf>,
}

impl Drop for Released {
    fn drop(&mut self) {
        if self.dirs.is_empty() {
            return;
        }
        let Some(s) = shared() else { return };
        let _registering = s.registering.lock().unwrap();
        let wanted: Vec<PathBuf> =
            { s.entries.lock().unwrap().keys().filter(|d| self.dirs.contains(d)).cloned().collect() };
        for d in &wanted {
            if d.is_dir() {
                let _ = s.watcher.lock().unwrap().watch(d, RecursiveMode::NonRecursive);
            }
            changed(&s.entries, Some(d));
        }
    }
}

/// Runs `f` again while it fails with "Access denied", for up to a second, on Windows: a watch
/// just released ([`release_under`]) or a virus scanner may still hold the folder for a moment.
pub fn retry_while_busy<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let start = std::time::Instant::now();
    loop {
        match f() {
            Err(e)
                if RELEASE_TO_CHANGE
                    && e.kind() == std::io::ErrorKind::PermissionDenied
                    && start.elapsed() < Duration::from_secs(1) =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            r => return r,
        }
    }
}

/// Calls `on_change` (on a background thread) whenever the content of `dir` changes, until the
/// handle is dropped. Registering touches the file system (it can block on a hung mount): call it
/// off the UI thread. A folder that can't be watched simply never reports.
pub fn watch(dir: &Path, on_change: impl Fn() + Send + Sync + 'static) -> WatchHandle {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    // by real path: a folder opened through a link and directly is one folder, and the OS has one
    // watch for it (removing it for one path would end it for the other)
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let dir = dir.as_path();
    let handle = WatchHandle { dir: dir.to_path_buf(), id };
    let Some(s) = shared() else { return handle };
    let _registering = s.registering.lock().unwrap();
    // registered again on every watch: a folder deleted and created again at the same path needs
    // a new OS watch, or it would never report again
    let _ = s.watcher.lock().unwrap().watch(dir, RecursiveMode::NonRecursive);
    if RESTARTS_LOSE_EVENTS {
        changed_all(&s.entries); // before adding the new one: its listing follows anyway
    }
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
    fn a_folder_with_a_watched_folder_inside_can_be_renamed_while_released() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, inner) = (tmp.path().join("a"), tmp.path().join("a/inner"));
        fs::create_dir_all(&inner).unwrap();
        let _watching = watch(&inner, || {});
        let released = release_under(&a);
        // without the release, Windows refuses: the watch holds a/inner open
        retry_while_busy(|| fs::rename(&a, tmp.path().join("b"))).expect("renamed");
        drop(released);
        assert!(tmp.path().join("b/inner").is_dir());
    }

    #[test]
    fn released_watches_report_again_once_given_back() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("inner");
        fs::create_dir(&inner).unwrap();
        let (_, rx, listener) = counter();
        let _watching = watch(&inner, listener);
        drop(release_under(tmp.path()));
        std::thread::sleep(DEBOUNCE * 2); // giving back reports the folder once
        while rx.try_recv().is_ok() {}
        fs::write(inner.join("x.txt"), "x").unwrap();
        rx.recv_timeout(WAIT).expect("watched again");
    }

    #[test]
    fn a_change_right_before_another_folder_is_watched_is_not_lost() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (_, rx, listener) = counter();
        let _watching_a = watch(a.path(), listener);
        std::thread::sleep(DEBOUNCE); // settled: only the write below reports
        while rx.try_recv().is_ok() {}
        fs::write(a.path().join("new.txt"), "x").unwrap();
        // on macOS this restarts the OS stream, which dropped the write's undelivered event
        let _watching_b = watch(b.path(), || {});
        drop(watch(a.path().join("..").as_path(), || {}));
        rx.recv_timeout(WAIT).expect("the change in a was reported");
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

    #[cfg(unix)]
    #[test]
    fn a_folder_watched_through_a_link_and_directly_reports_to_both() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let (_, via_link, listener) = counter();
        let linked = watch(&link, listener);
        let (_, direct, listener) = counter();
        let _direct = watch(&real, listener);
        fs::write(real.join("a.txt"), "x").unwrap();
        via_link.recv_timeout(WAIT).expect("reported through the link");
        direct.recv_timeout(WAIT).expect("reported directly");
        std::thread::sleep(DEBOUNCE + Duration::from_millis(200));
        while direct.try_recv().is_ok() {}
        // regression: dropping the link's watch removed the OS watch the other path shared
        drop(linked);
        fs::write(real.join("b.txt"), "x").unwrap();
        direct.recv_timeout(WAIT).expect("still reported after the link's watch is gone");
    }

    #[test]
    fn dropped_events_report_every_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, rx, listener) = counter();
        let _handle = watch(tmp.path(), listener);
        // what the OS sends when its queue overflowed: no paths, only a flag
        let overflow = notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
        route(&shared().unwrap().entries, Ok(overflow));
        rx.recv_timeout(WAIT).expect("an overflow refreshes the folder");
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
    // on macOS every watch another test adds restarts the stream, which reports every folder
    #[cfg_attr(target_os = "macos", ignore = "macOS reports every folder on each stream restart")]
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
    // on macOS every watch another test adds restarts the stream, which reports every folder
    #[cfg_attr(target_os = "macos", ignore = "macOS reports every folder on each stream restart")]
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
