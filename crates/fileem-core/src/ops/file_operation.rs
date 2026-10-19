//! Copy / move / delete / trash / compress / extract of a list of paths, run on a worker thread.
//! Progress is in "work units": bytes plus one unit per entry.
//!
//! Integrity rules (each has a test in `tests.rs`):
//! - Files are written to a hidden temp file next to the target (`.fileem-<random>.part`),
//!   size-checked, then renamed into place, so an existing target is only replaced by a complete
//!   copy and a failed or cancelled copy never leaves a truncated file behind.
//! - A move between file systems makes the copy durable before the source is deleted, and a
//!   source is only deleted after its copy succeeded. On Linux the copies are flushed in batches
//!   (one `syncfs` of the destination per batch, then the batch's sources are deleted) instead of
//!   one fsync per file.
//! - Symbolic links are copied/deleted as links and never followed; Windows junctions are never
//!   descended into; special files (FIFOs, devices, sockets) are skipped.
//! - Something displaced by "Replace" (e.g. a folder replaced by a file) is moved aside first and
//!   restored if the replacement fails.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::fs::trash;
use crate::util::fs_util::{self, exists_nofollow};
use crate::util::{formats, os};

/// `Compress` zips all sources into one archive in the target folder; `Extract` unpacks each zip into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Move,
    Delete,
    Trash,
    Compress,
    Extract,
}

impl Kind {
    fn verb(self) -> &'static str {
        match self {
            Kind::Copy => "copy",
            Kind::Move => "move",
            Kind::Delete => "delete",
            Kind::Trash => "trash",
            Kind::Compress => "compress",
            Kind::Extract => "extract",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Overwrite,
    Skip,
    KeepBoth,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub resolution: Resolution,
    pub apply_to_all: bool,
}

impl Decision {
    pub fn once(resolution: Resolution) -> Decision {
        Decision { resolution, apply_to_all: false }
    }

    pub fn for_all(resolution: Resolution) -> Decision {
        Decision { resolution, apply_to_all: true }
    }
}

/// Called on the worker thread when a target already exists; may block for user input.
pub type ConflictHandler = Box<dyn FnMut(&Path, &Path) -> Decision + Send>;

/// Makes a whole file system durable; false when it couldn't (callers then fsync file by file).
pub type Syncer = Box<dyn FnMut(&Path) -> bool + Send>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Ready,
    Running,
    Succeeded,
    Cancelled,
    /// The operation as a whole failed (e.g. not enough free space); per-item problems are in `errors()`.
    Failed(String),
}

/// Files up to this size use `fs::copy` (fast, keeps permissions); bigger ones are chunked for progress.
pub(crate) const FAST_COPY_LIMIT: u64 = 8 << 20;
const CHUNK: u64 = 16 << 20;
const TRASH_BATCH: usize = 100;
/// Flush a batch once it holds this many sources, this many bytes, or has waited this long.
pub(crate) const BATCH_ITEMS: usize = 1000;
const BATCH_BYTES: u64 = 1 << 30;
const BATCH_TIME: Duration = Duration::from_secs(15);

/// State shared between the worker and everyone watching or steering it.
struct Shared {
    kind: Kind,
    sources: Vec<PathBuf>,
    target_dir: Option<PathBuf>,
    cancelled: AtomicBool,
    paused_flag: AtomicBool,
    paused: Mutex<bool>,
    unpaused: Condvar,
    waiting_while_paused: AtomicBool,
    started: AtomicBool,
    stopped: Mutex<bool>,
    stopped_cv: Condvar,
    done: AtomicU64,
    total: AtomicU64,
    message: Mutex<String>,
    errors: Mutex<Vec<String>>,
    result: Mutex<Option<PathBuf>>,
    state: Mutex<State>,
}

/// A cheap, cloneable view of a running (or finished) operation: progress, pause, resume, cancel.
#[derive(Clone)]
pub struct Handle(Arc<Shared>);

impl Handle {
    pub fn kind(&self) -> Kind {
        self.0.kind
    }

    pub fn sources(&self) -> &[PathBuf] {
        &self.0.sources
    }

    pub fn target_dir(&self) -> Option<&Path> {
        self.0.target_dir.as_deref()
    }

    /// Stops the operation at its next check (between items and chunks); a paused worker wakes up
    /// and stops. Cleanup (temp files, displaced items) still runs: see [`Handle::await_stopped`].
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        let _guard = self.0.paused.lock().unwrap();
        self.0.unpaused.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Pauses at the next item or chunk (a file under 8 MB being copied is finished first); the
    /// worker then waits, holding no files open, until [`Handle::resume`] or cancel.
    pub fn pause(&self) {
        *self.0.paused.lock().unwrap() = true;
        self.0.paused_flag.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        let mut paused = self.0.paused.lock().unwrap();
        *paused = false;
        self.0.paused_flag.store(false, Ordering::SeqCst);
        self.0.unpaused.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        self.0.paused_flag.load(Ordering::SeqCst)
    }

    /// Whether the worker is stopped at a pause point (it may still finish a step after `pause()`).
    pub fn is_waiting_while_paused(&self) -> bool {
        self.0.waiting_while_paused.load(Ordering::SeqCst)
    }

    /// Waits until the worker has stopped, including the cleanup after a cancel. Returns false on
    /// timeout. An operation that never started counts as stopped once it is cancelled.
    pub fn await_stopped(&self, timeout: Duration) -> bool {
        if !self.0.started.load(Ordering::SeqCst) && self.is_cancelled() {
            return true;
        }
        let stopped = self.0.stopped.lock().unwrap();
        let (stopped, _) = self.0.stopped_cv.wait_timeout_while(stopped, timeout, |s| !*s).unwrap();
        *stopped
    }

    /// (done, total) work units.
    pub fn progress(&self) -> (u64, u64) {
        (self.0.done.load(Ordering::Relaxed), self.0.total.load(Ordering::Relaxed))
    }

    /// What the worker is doing now (a path, "Counting…", "Paused", …).
    pub fn message(&self) -> String {
        self.0.message.lock().unwrap().clone()
    }

    /// One line per item that could not be processed.
    pub fn errors(&self) -> Vec<String> {
        self.0.errors.lock().unwrap().clone()
    }

    /// What `Compress` / `Extract` created (none otherwise, or when it failed).
    pub fn result(&self) -> Option<PathBuf> {
        self.0.result.lock().unwrap().clone()
    }

    pub fn state(&self) -> State {
        self.0.state.lock().unwrap().clone()
    }

    pub fn is_done(&self) -> bool {
        matches!(self.state(), State::Succeeded | State::Cancelled | State::Failed(_))
    }
}

/// Why the worker stopped a step: an I/O error (reported, then the next item) or a cancel.
enum Fail {
    Io(io::Error),
    Cancelled,
}

impl From<io::Error> for Fail {
    fn from(e: io::Error) -> Self {
        Fail::Io(e)
    }
}

type R<T> = Result<T, Fail>;

pub struct FileOperation {
    shared: Arc<Shared>,
    conflicts: ConflictHandler,
    syncer: Option<Syncer>,
    force_copy_moves: bool,
}

impl FileOperation {
    /// An operation on `sources`; `target_dir` is needed for copy, move, compress and extract.
    /// Conflicts cancel until a handler is set with [`FileOperation::on_conflict`].
    pub fn new(kind: Kind, sources: Vec<PathBuf>, target_dir: Option<PathBuf>) -> FileOperation {
        FileOperation {
            shared: Arc::new(Shared {
                kind,
                sources,
                target_dir,
                cancelled: AtomicBool::new(false),
                paused_flag: AtomicBool::new(false),
                paused: Mutex::new(false),
                unpaused: Condvar::new(),
                waiting_while_paused: AtomicBool::new(false),
                started: AtomicBool::new(false),
                stopped: Mutex::new(false),
                stopped_cv: Condvar::new(),
                done: AtomicU64::new(0),
                total: AtomicU64::new(1),
                message: Mutex::new(String::new()),
                errors: Mutex::new(Vec::new()),
                result: Mutex::new(None),
                state: Mutex::new(State::Ready),
            }),
            conflicts: Box::new(|_, _| Decision::once(Resolution::Cancel)),
            syncer: default_syncer(),
            force_copy_moves: false,
        }
    }

    pub fn handle(&self) -> Handle {
        Handle(self.shared.clone())
    }

    pub fn on_conflict(&mut self, handler: impl FnMut(&Path, &Path) -> Decision + Send + 'static) {
        self.conflicts = Box::new(handler);
    }

    /// Test hook: moves copy and delete like moves between drives do (tests can't span two).
    #[cfg(test)]
    pub(crate) fn force_copy_moves(&mut self) {
        self.force_copy_moves = true;
    }

    /// Test hook: how a batch of moved files is flushed (`None`: fsync every file, no batching).
    #[cfg(test)]
    pub(crate) fn syncer(&mut self, syncer: Option<Syncer>) {
        self.syncer = syncer;
    }

    /// Runs the operation on the calling thread (start it on its own thread to keep the UI free).
    /// An operation cancelled before it started does nothing.
    pub fn run(self) -> Handle {
        let handle = self.handle();
        let shared = self.shared.clone();
        // marks the operation stopped even if the worker panics
        struct Stopped(Arc<Shared>);
        impl Drop for Stopped {
            fn drop(&mut self) {
                *self.0.stopped.lock().unwrap() = true;
                self.0.stopped_cv.notify_all();
            }
        }
        let _stopped = Stopped(shared.clone());
        if handle.is_cancelled() {
            *shared.state.lock().unwrap() = State::Cancelled;
            return handle;
        }
        shared.started.store(true, Ordering::SeqCst);
        *shared.state.lock().unwrap() = State::Running;

        let target = shared.target_dir.clone().unwrap_or_default();
        let mut w = Worker {
            kind: shared.kind,
            target,
            conflicts: self.conflicts,
            syncer: self.syncer,
            force_copy_moves: self.force_copy_moves,
            s: shared.clone(),
            conflict_for_all: None,
            rename_moves: false,
            pending: Vec::new(),
            pending_files: 0,
            pending_bytes: 0,
            pending_since: Instant::now(),
            done: 0,
            total: 1,
            store_cache: HashMap::new(),
            counted_items: 0,
            counted_bytes: 0,
            last_count_report: None,
            last_message: String::new(),
        };
        let outcome = w.work();
        // moves that completed (also before a cancel or failure) lose their source
        w.flush_moves();
        let state = match outcome {
            _ if handle.is_cancelled() => State::Cancelled,
            Ok(()) => {
                w.done = w.total;
                w.report_progress();
                State::Succeeded
            }
            Err(Fail::Cancelled) => {
                shared.cancelled.store(true, Ordering::SeqCst);
                State::Cancelled
            }
            Err(Fail::Io(e)) => State::Failed(error_text(&e)),
        };
        *shared.state.lock().unwrap() = state;
        handle
    }
}

/// Linux: `syncfs(2)` on the file system holding the target folder.
fn default_syncer() -> Option<Syncer> {
    #[cfg(target_os = "linux")]
    {
        Some(Box::new(|dir: &Path| {
            use std::os::fd::AsRawFd;
            match File::open(dir) {
                // SAFETY: the descriptor is valid while `f` lives.
                Ok(f) => unsafe { libc::syncfs(f.as_raw_fd()) == 0 },
                Err(_) => false,
            }
        }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// The worker's private state for one run.
struct Worker {
    kind: Kind,
    target: PathBuf,
    conflicts: ConflictHandler,
    syncer: Option<Syncer>,
    force_copy_moves: bool,
    s: Arc<Shared>,
    conflict_for_all: Option<Resolution>,
    /// The current top-level move stays on one file system, so items can simply be renamed.
    rename_moves: bool,
    /// Sources of a cross-file-system move whose copies are complete and in place but not yet
    /// flushed, in the order they finished (a folder, with no copy, after its contents).
    pending: Vec<(PathBuf, Option<PathBuf>)>,
    pending_files: usize,
    pending_bytes: u64,
    pending_since: Instant,
    done: u64,
    total: u64,
    store_cache: HashMap<PathBuf, Option<u64>>,
    counted_items: u64,
    counted_bytes: u64,
    last_count_report: Option<Instant>,
    last_message: String,
}

impl Worker {
    fn work(&mut self) -> R<()> {
        self.message("Preparing…");
        let mut work = Vec::new();
        for src in self.s.sources.clone() {
            let is_root = std::path::absolute(&src).map(|a| a.parent().is_none()).unwrap_or(false);
            if self.kind != Kind::Copy && is_root {
                self.error_text(&src, &format!("Cannot {} a drive or file system root", self.kind.verb()));
            } else {
                work.push(src);
            }
        }
        self.total = self.measure(&work)?.max(1);
        self.report_progress();
        match self.kind {
            Kind::Trash => self.trash_all(&work)?,
            Kind::Compress => self.compress(&work)?,
            Kind::Extract => {
                for zip in &work {
                    self.check_cancel()?;
                    match self.extract(zip) {
                        Err(Fail::Io(e)) => self.error(zip, &e),
                        other => other?,
                    }
                }
            }
            _ => {
                for src in &work {
                    self.check_cancel()?;
                    let r = match self.kind {
                        Kind::Copy => self.copy_top(src),
                        Kind::Move => self.move_top(src),
                        _ => self.delete_tree(src, true),
                    };
                    match r {
                        Err(Fail::Io(e)) => self.error(src, &e),
                        other => other?,
                    }
                }
            }
        }
        Ok(())
    }

    // ---- archives ----

    fn compress(&mut self, work: &[PathBuf]) -> R<()> {
        if work.is_empty() {
            return Ok(());
        }
        let out = fs_util::unique_name(&self.target.join(archive_name(work)), false);
        let tmp = temp_sibling(&out);
        let r = (|| -> R<()> {
            let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            let mut zip = zip::ZipWriter::new(BufWriter::with_capacity(1 << 16, file));
            for src in work {
                let name = fs_util::display_name(src);
                self.add_to_zip(&mut zip, src, &name)?;
            }
            let mut inner = zip.finish().map_err(zip_error)?;
            inner.flush()?;
            drop(inner);
            place(&tmp, &out, false)?;
            Ok(())
        })();
        match r {
            Ok(()) => {
                *self.s.result.lock().unwrap() = Some(out);
                Ok(())
            }
            Err(e) => {
                self.delete_quietly(&tmp);
                Err(e)
            }
        }
    }

    fn add_to_zip(&mut self, zip: &mut zip::ZipWriter<BufWriter<File>>, p: &Path, entry_name: &str) -> R<()> {
        self.check_cancel()?;
        let a = p.symlink_metadata()?;
        let ft = a.file_type();
        if ft.is_symlink() || is_other(&a) || (a.is_dir() && fs_util::is_junction(p)) {
            self.error_text(p, "Links and special files are not added to archives");
            self.advance(1);
            return Ok(());
        }
        self.message(&p.to_string_lossy());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip_time(a.modified().ok()))
            .large_file(a.len() >= u32::MAX as u64);
        if a.is_dir() {
            zip.add_directory(format!("{entry_name}/"), options).map_err(zip_error)?;
            self.advance(1);
            let mut children: Vec<PathBuf> = match fs::read_dir(p) {
                Ok(rd) => rd.filter_map(Result::ok).map(|e| e.path()).collect(),
                Err(e) => {
                    self.error(p, &e);
                    Vec::new()
                }
            };
            children.sort();
            for child in children {
                let name = format!("{entry_name}/{}", fs_util::display_name(&child));
                self.add_to_zip(zip, &child, &name)?;
            }
            return Ok(());
        }
        // open before writing the entry header, so an unreadable file is skipped cleanly;
        // a failure while reading aborts the whole archive (the entry would be truncated)
        let mut input = match File::open(p) {
            Ok(f) => f,
            Err(e) => {
                self.error(p, &e);
                self.advance(a.len() + 1);
                return Ok(());
            }
        };
        zip.start_file(entry_name, options).map_err(zip_error)?;
        self.copy_stream(&mut input, zip, None, entry_name)?;
        self.advance(1);
        Ok(())
    }

    /// (uncompressed bytes, entries) of a zip, from its central directory.
    fn zip_size(zip_file: &Path) -> io::Result<(u64, u64)> {
        let mut archive = zip::ZipArchive::new(File::open(zip_file)?).map_err(zip_io_error)?;
        let mut bytes = 0;
        for i in 0..archive.len() {
            bytes += archive.by_index_raw(i).map_err(zip_io_error)?.size();
        }
        Ok((bytes, archive.len() as u64))
    }

    /// Extracts into a hidden temp folder, then renames it into place: "name/" for archives with
    /// several top-level entries, or the single top-level item itself. Entries that would land
    /// outside the target (zip-slip) or whose names aren't valid paths are skipped and reported.
    fn extract(&mut self, zip_file: &Path) -> R<()> {
        let mut base = fs_util::display_name(zip_file);
        if let Some(dot) = base.rfind('.').filter(|&d| d > 0) {
            base.truncate(dot);
        }
        let tmp = temp_sibling(&self.target.join(&base));
        fs::create_dir(&tmp)?;
        let r = self.extract_into(zip_file, &tmp, &base);
        match r {
            Ok(out) => {
                *self.s.result.lock().unwrap() = Some(out);
                Ok(())
            }
            Err(e) => {
                self.delete_quietly(&tmp);
                Err(e)
            }
        }
    }

    fn extract_into(&mut self, zip_file: &Path, tmp: &Path, base: &str) -> R<PathBuf> {
        let root = fs_util::normalize(tmp);
        let label = zip_file.to_string_lossy().into_owned();
        let mut archive = zip::ZipArchive::new(File::open(zip_file)?).map_err(zip_io_error)?;
        for i in 0..archive.len() {
            self.check_cancel()?;
            let mut entry = archive.by_index(i).map_err(zip_io_error)?;
            let name = entry.name().to_string();
            if !valid_entry_name(&name) {
                self.push_error(format!("{label}: Skipped entry with an invalid name \"{}\"", name.escape_debug()));
                self.advance(1);
                continue;
            }
            let dest = lexical_normalize(&root.join(name.replace('\\', "/")));
            if !dest.starts_with(&root) || dest == root {
                if dest != root {
                    self.push_error(format!("{label}: Skipped unsafe entry \"{name}\""));
                }
                self.advance(1);
                continue;
            }
            self.message(&name);
            if entry.is_dir() {
                fs::create_dir_all(&dest)?;
            } else {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut out = match OpenOptions::new().write(true).create_new(true).open(&dest) {
                    Ok(f) => f,
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        self.push_error(format!("{label}: Duplicate entry \"{name}\" skipped"));
                        self.advance(1);
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                };
                let declared = entry.size();
                self.copy_stream(&mut entry, &mut out, Some(declared), &name)?;
                drop(out);
                if let Some(t) = entry.last_modified().and_then(from_zip_time) {
                    let _ = filetime::set_file_mtime(&dest, filetime::FileTime::from_system_time(t));
                }
            }
            self.advance(1);
        }
        let top: Vec<PathBuf> = fs::read_dir(tmp)?.filter_map(Result::ok).map(|e| e.path()).collect();
        let out = if top.len() == 1 {
            let out = fs_util::unique_name(&self.target.join(fs_util::display_name(&top[0])), false);
            rename_noreplace(&top[0], &out)?;
            fs::remove_dir(tmp)?;
            out
        } else {
            let out = fs_util::unique_name(&self.target.join(base), false);
            rename_noreplace(tmp, &out)?;
            out
        };
        Ok(out)
    }

    /// Copies with progress and cancel checks. With `declared`, never writes more than that many
    /// bytes: the free space check trusts the archive's sizes, and a crafted archive (zip bomb)
    /// could otherwise fill the disk.
    fn copy_stream(&mut self, input: &mut impl Read, out: &mut impl Write, declared: Option<u64>, name: &str) -> R<()> {
        let mut buf = vec![0u8; 1 << 16];
        let mut written = 0u64;
        loop {
            let n = match input.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            self.check_cancel()?;
            if declared.is_some_and(|d| written + n as u64 > d) {
                return Err(io::Error::other(format!(
                    "Entry \"{name}\" is larger than the archive says; extraction stopped."
                ))
                .into());
            }
            out.write_all(&buf[..n])?;
            written += n as u64;
            self.advance(n as u64);
        }
    }

    // ---- measuring ----

    fn measure(&mut self, work: &[PathBuf]) -> R<u64> {
        let mut units = 0;
        let mut new_bytes = 0;
        for src in work {
            self.check_cancel()?;
            match self.kind {
                Kind::Trash => units += 1,
                Kind::Compress => {
                    let (bytes, entries) = self.tree_size(src)?;
                    units += bytes + entries;
                }
                Kind::Extract => {
                    let (bytes, entries) = Self::zip_size(src)?;
                    units += bytes + entries;
                    new_bytes += bytes;
                }
                Kind::Delete => units += self.tree_size(src)?.1,
                Kind::Copy | Kind::Move => {
                    if self.kind == Kind::Move && self.is_rename(src) {
                        units += 1;
                    } else {
                        let (bytes, entries) = self.tree_size(src)?;
                        units += bytes + entries;
                        if !exists_nofollow(&self.target.join(file_name(src))) {
                            new_bytes += bytes;
                        }
                    }
                }
            }
        }
        if new_bytes > 0 {
            let usable = fs4::available_space(&self.target).unwrap_or(u64::MAX);
            if new_bytes > usable {
                return Err(io::Error::other(format!(
                    "Not enough free space on the destination: {} needed, {} available.",
                    formats::size(Some(new_bytes)),
                    formats::size(Some(usable))
                ))
                .into());
            }
        }
        Ok(units)
    }

    fn is_rename(&mut self, src: &Path) -> bool {
        self.same_store(src) && !exists_nofollow(&self.target.join(file_name(src)))
    }

    /// (bytes, entries) below `root`, not following links or junctions. Only an estimate:
    /// unreadable parts are left out.
    fn tree_size(&mut self, root: &Path) -> R<(u64, u64)> {
        let mut sum = (0, 0);
        let mut stack = vec![root.to_path_buf()];
        while let Some(p) = stack.pop() {
            let Ok(m) = p.symlink_metadata() else { continue };
            if m.is_dir() {
                self.check_cancel()?;
                sum.1 += 1;
                if !fs_util::is_junction(&p) {
                    if let Ok(rd) = fs::read_dir(&p) {
                        stack.extend(rd.filter_map(Result::ok).map(|e| e.path()));
                    }
                }
            } else {
                let bytes = if m.is_file() { m.len() } else { 0 };
                sum.0 += bytes;
                sum.1 += 1;
                self.counted(1, bytes);
            }
        }
        Ok(sum)
    }

    /// Live feedback while measuring, so a big job doesn't look stuck before copying starts.
    fn counted(&mut self, items: u64, bytes: u64) {
        self.counted_items += items;
        self.counted_bytes += bytes;
        let now = Instant::now();
        if self.last_count_report.is_none_or(|t| now - t > Duration::from_millis(200)) {
            self.last_count_report = Some(now);
            let msg = format!(
                "Counting… {} {}, {}",
                formats::count(self.counted_items),
                if self.counted_items == 1 { "item" } else { "items" },
                formats::size(Some(self.counted_bytes))
            );
            self.message(&msg);
        }
    }

    /// The file system holding `p` (cached: on Linux every lookup is a stat, mount points rarely change).
    fn store(&mut self, p: &Path) -> Option<u64> {
        let key = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
        *self.store_cache.entry(key.clone()).or_insert_with(|| store_id(&key))
    }

    fn same_store(&mut self, src: &Path) -> bool {
        let parent = std::path::absolute(src).ok().and_then(|a| a.parent().map(Path::to_path_buf));
        let a = parent.and_then(|p| self.store(&p));
        let target = self.target.clone();
        let b = self.store(&target);
        a.is_some() && a == b
    }

    // ---- copy / move ----

    fn copy_top(&mut self, src: &Path) -> R<()> {
        let mut dest = self.target.join(file_name(src));
        if fs_util::normalize(&dest) == fs_util::normalize(src) {
            dest = fs_util::unique_name(src, true);
        } else if self.copies_into_itself(src)? {
            return Err(io::Error::other("Cannot copy a folder into itself.").into());
        }
        self.rename_moves = false;
        self.transfer(src, &dest, false)
    }

    fn move_top(&mut self, src: &Path) -> R<()> {
        let dest = self.target.join(file_name(src));
        if fs_util::normalize(&dest) == fs_util::normalize(src) {
            self.advance(1);
            return Ok(());
        }
        if self.copies_into_itself(src)? {
            return Err(io::Error::other("Cannot move a folder into itself.").into());
        }
        self.rename_moves = !self.force_copy_moves && self.same_store(src);
        self.transfer(src, &dest, true)
    }

    /// Whether the target folder is the folder `src` or lies inside it, also when the target path
    /// reaches it through a link (lexical checks alone miss that, and the copy would recurse into
    /// its own output until paths get too long).
    fn copies_into_itself(&self, src: &Path) -> io::Result<bool> {
        if !src.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            return Ok(false);
        }
        if fs_util::is_same_or_inside(&self.target, src) {
            return Ok(true);
        }
        match (fs::canonicalize(&self.target), fs::canonicalize(src)) {
            (Ok(t), Ok(s)) => Ok(t.starts_with(s)),
            // a missing folder is reported by the copy itself
            _ => Ok(false),
        }
    }

    /// Copies (or moves, when `mv`) `src` to `dst`, merging folders and asking on conflicts.
    fn transfer(&mut self, src: &Path, dst: &Path, mv: bool) -> R<()> {
        self.check_cancel()?;
        let a = src.symlink_metadata()?;
        let mut dst = dst.to_path_buf();
        let mut exists = exists_nofollow(&dst);

        if exists && same_file(src, &dst) {
            if mv {
                return Err(io::Error::other("Source and destination are the same item.").into());
            }
            dst = fs_util::unique_name(&dst, true);
            exists = false;
        }

        if mv && self.rename_moves && !exists {
            self.message(&src.to_string_lossy());
            if try_rename(src, &dst, false)? {
                self.advance(if a.is_dir() { 1 } else { a.len() + 1 });
                return Ok(());
            }
            exists = exists_nofollow(&dst);
        }

        if (a.is_dir() || is_other(&a)) && fs_util::is_junction(src) {
            return Err(io::Error::other("Junctions are not copied (move it within the same drive instead).").into());
        }
        if is_other(&a) {
            return Err(io::Error::other("Special file skipped (pipe, socket or device).").into());
        }

        if a.is_dir() {
            self.transfer_directory(src, dst, &a, exists, mv)
        } else {
            self.transfer_file(src, dst, &a, exists, mv)
        }
    }

    fn transfer_directory(&mut self, src: &Path, mut dst: PathBuf, a: &Metadata, mut exists: bool, mv: bool) -> R<()> {
        let mut displaced = None;
        if exists && !dst.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            match self.resolve(src, &dst)? {
                Resolution::Skip => return Ok(()),
                Resolution::KeepBoth => dst = fs_util::unique_name(&dst, false),
                _ => displaced = Some(displace(&dst)?),
            }
            exists = false;
        }
        if !exists {
            if let Err(e) = fs::create_dir(&dst) {
                restore(displaced.as_deref(), &dst);
                return Err(e.into());
            }
        }
        if let Some(d) = displaced {
            self.delete_quietly(&d);
        }
        self.advance(1);
        let children: Vec<PathBuf> = fs::read_dir(src)?.filter_map(Result::ok).map(|e| e.path()).collect();
        for child in children {
            match self.transfer(&child, &dst.join(file_name(&child)), mv) {
                Err(Fail::Io(e)) => self.error(&child, &e),
                other => other?,
            }
        }
        if let Ok(t) = a.modified() {
            let _ = filetime::set_file_mtime(&dst, filetime::FileTime::from_system_time(t));
        }
        if mv {
            if !self.pending.is_empty() {
                self.pending.push((src.to_path_buf(), None)); // after its contents, once they are flushed
            } else {
                delete_source_folder(src)?;
            }
        }
        Ok(())
    }

    fn transfer_file(&mut self, src: &Path, mut dst: PathBuf, a: &Metadata, mut exists: bool, mv: bool) -> R<()> {
        if exists {
            match self.resolve(src, &dst)? {
                Resolution::Skip => {
                    self.advance(a.len() + 1);
                    return Ok(());
                }
                Resolution::KeepBoth => {
                    dst = fs_util::unique_name(&dst, false);
                    exists = false;
                }
                _ => {} // replace, below
            }
        }

        if mv && self.rename_moves {
            self.message(&src.to_string_lossy());
            let displaced =
                if exists && dst.symlink_metadata().is_ok_and(|m| m.is_dir()) { Some(displace(&dst)?) } else { None };
            match try_rename(src, &dst, exists) {
                Ok(true) => {
                    if let Some(d) = displaced {
                        self.delete_quietly(&d);
                    }
                    self.advance(a.len() + 1);
                    return Ok(());
                }
                Ok(false) => restore(displaced.as_deref(), &dst),
                Err(e) => {
                    restore(displaced.as_deref(), &dst);
                    return Err(e.into());
                }
            }
        }

        let batch = mv && self.syncer.is_some();
        if a.file_type().is_symlink() {
            self.copy_symlink(src, &dst, exists)?;
        } else {
            self.copy_file(src, &dst, a, exists, mv && !batch)?; // batched: flushed with the others
        }
        if batch {
            self.delete_after_flush(src, &dst, a.len());
        } else if mv {
            fs::remove_file(src)?;
        }
        Ok(())
    }

    /// Queues a moved file's source for deletion once its copy is flushed; flushes full batches.
    fn delete_after_flush(&mut self, src: &Path, dst: &Path, size: u64) {
        if self.pending.is_empty() {
            self.pending_since = Instant::now();
        }
        self.pending.push((src.to_path_buf(), Some(dst.to_path_buf())));
        self.pending_files += 1;
        self.pending_bytes += size;
        if self.pending_files >= BATCH_ITEMS
            || self.pending_bytes >= BATCH_BYTES
            || self.pending_since.elapsed() >= BATCH_TIME
        {
            self.flush_moves();
        }
    }

    /// Makes the queued copies durable, then deletes their sources (folders after their contents).
    /// One syncfs for the whole batch; if that fails, every copy is fsynced on its own; a copy that
    /// can't be flushed keeps its source. Never fails: problems are reported per item. Runs also
    /// after a cancel, so completed moves never leave a duplicate behind.
    fn flush_moves(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        self.pending_files = 0;
        self.pending_bytes = 0;
        let message = self.last_message.clone();
        self.message("Writing to disk…");
        let target = self.target.clone();
        let synced = self.syncer.as_mut().is_some_and(|s| s(&target));
        let mut unflushed: HashSet<PathBuf> = HashSet::new();
        for (src, dst) in batch {
            let r = match dst {
                None => {
                    if !unflushed.iter().any(|u| u.starts_with(&src)) {
                        delete_source_folder(&src)
                    } else {
                        Ok(())
                    }
                }
                Some(dst) => (|| {
                    if !synced && !dst.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
                        force(&dst)?;
                    }
                    fs::remove_file(&src)
                })(),
            };
            if let Err(e) = r {
                self.error(&src, &e);
                unflushed.insert(src);
            }
        }
        self.message(&message);
    }

    fn copy_symlink(&mut self, src: &Path, dst: &Path, replace: bool) -> R<()> {
        let tmp = temp_sibling(dst);
        copy_link(src, &tmp)?;
        if let Err(e) = place(&tmp, dst, replace) {
            self.delete_quietly(&tmp);
            return Err(e.into());
        }
        self.advance(1);
        Ok(())
    }

    fn copy_file(&mut self, src: &Path, dst: &Path, a: &Metadata, replace: bool, durable: bool) -> R<()> {
        self.message(&src.to_string_lossy());
        let tmp = temp_sibling(dst);
        let r = (|| -> R<()> {
            if a.len() <= FAST_COPY_LIMIT {
                fs::copy(src, &tmp)?;
                copy_times(&tmp, a);
                self.advance(a.len());
                if durable {
                    force(&tmp)?;
                }
            } else {
                self.chunked_copy(src, &tmp, durable)?;
                copy_metadata(src, &tmp, a);
            }
            let copied = fs::metadata(&tmp)?.len();
            if copied != a.len() {
                return Err(io::Error::other(format!(
                    "Copy is incomplete ({} of {}); the source may have changed during the copy.",
                    formats::exact(copied),
                    formats::exact(a.len())
                ))
                .into());
            }
            let displaced =
                if replace && dst.symlink_metadata().is_ok_and(|m| m.is_dir()) { Some(displace(dst)?) } else { None };
            if let Err(e) = place(&tmp, dst, replace) {
                restore(displaced.as_deref(), dst);
                return Err(e.into());
            }
            if let Some(d) = displaced {
                self.delete_quietly(&d);
            }
            Ok(())
        })();
        if r.is_err() {
            self.delete_quietly(&tmp);
        }
        r?;
        self.advance(1);
        Ok(())
    }

    /// Kernel copy in chunks (copy_file_range / sendfile under `io::copy`), cancellable, with progress.
    fn chunked_copy(&mut self, src: &Path, tmp: &Path, durable: bool) -> R<()> {
        let input = File::open(src)?;
        let mut out = OpenOptions::new().write(true).create_new(true).open(tmp)?;
        let size = input.metadata()?.len();
        let mut pos = 0;
        while pos < size {
            self.check_cancel()?;
            let n = io::copy(&mut (&input).take(CHUNK.min(size - pos)), &mut out)?;
            if n == 0 {
                break; // source shrank; the size check reports it
            }
            pos += n;
            self.advance(n);
        }
        if durable {
            out.sync_all()?;
        }
        Ok(())
    }

    fn delete_quietly(&mut self, p: &Path) {
        match p.symlink_metadata() {
            Ok(m) if m.is_dir() => {
                // best effort cleanup; never reported, never cancelled half-way
                let _ = remove_tree_quietly(p);
            }
            Ok(_) => {
                let _ = fs::remove_file(p);
            }
            Err(_) => {}
        }
    }

    fn resolve(&mut self, src: &Path, dst: &Path) -> R<Resolution> {
        if let Some(r) = self.conflict_for_all {
            return Ok(r);
        }
        let d = (self.conflicts)(src, dst);
        if d.resolution == Resolution::Cancel {
            return Err(Fail::Cancelled);
        }
        if d.apply_to_all {
            self.conflict_for_all = Some(d.resolution);
        }
        Ok(d.resolution)
    }

    // ---- trash ----

    fn trash_all(&mut self, work: &[PathBuf]) -> R<()> {
        let mut present = Vec::new();
        for p in work {
            if exists_nofollow(p) {
                present.push(p.clone());
            } else {
                self.error_text(p, "Not found");
                self.advance(1);
            }
        }
        for batch in present.chunks(TRASH_BATCH) {
            self.check_cancel()?;
            self.message(&batch[0].to_string_lossy());
            if batch.len() > 1 && trash::move_all_to_trash(batch) {
                self.advance(batch.len() as u64);
                continue;
            }
            // single item, or the batch failed part-way: items still present are retried one by
            // one so each failure gets its own error message
            for p in batch {
                self.check_cancel()?;
                if batch.len() == 1 || exists_nofollow(p) {
                    self.message(&p.to_string_lossy());
                    if let Err(e) = trash::move_to_trash(p) {
                        self.error(p, &e);
                    }
                }
                self.advance(1);
            }
        }
        Ok(())
    }

    // ---- delete ----

    /// Deletes `p` and everything below it, never following links or junctions. Failures are
    /// reported per item; the rest is still deleted.
    fn delete_tree(&mut self, p: &Path, report: bool) -> R<()> {
        self.check_cancel()?;
        let m = match p.symlink_metadata() {
            Ok(m) => m,
            Err(e) => {
                self.error(p, &e);
                return Ok(());
            }
        };
        if m.is_dir() && !fs_util::is_junction(p) {
            match fs::read_dir(p) {
                Ok(rd) => {
                    let children: Vec<PathBuf> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
                    for child in children {
                        self.delete_tree(&child, report)?;
                    }
                }
                Err(e) => self.error(p, &e),
            }
            self.check_cancel()?;
            match delete_one(p, true) {
                Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => {} // a child failed and was reported
                Err(e) => self.error(p, &e),
                Ok(()) => {}
            }
        } else {
            if report {
                self.message(&p.to_string_lossy());
            }
            // a junction is removed itself, never what it points to
            if let Err(e) = delete_one(p, m.is_dir()) {
                self.error(p, &e);
            }
        }
        if report {
            self.advance(1);
        }
        Ok(())
    }

    // ---- helpers ----

    fn advance(&mut self, units: u64) {
        self.done += units;
        self.report_progress();
    }

    fn report_progress(&self) {
        self.s.total.store(self.total, Ordering::Relaxed);
        self.s.done.store(self.done.min(self.total), Ordering::Relaxed);
    }

    fn message(&mut self, m: &str) {
        self.last_message = m.to_string();
        *self.s.message.lock().unwrap() = m.to_string();
    }

    /// The worker's checkpoint between items and chunks: stops when cancelled, waits while paused.
    fn check_cancel(&mut self) -> R<()> {
        if self.s.cancelled.load(Ordering::SeqCst) {
            return Err(Fail::Cancelled);
        }
        if self.s.paused_flag.load(Ordering::SeqCst) {
            self.wait_while_paused()?;
        }
        Ok(())
    }

    fn wait_while_paused(&mut self) -> R<()> {
        let message = self.last_message.clone();
        self.message("Paused");
        {
            let mut paused = self.s.paused.lock().unwrap();
            self.s.waiting_while_paused.store(true, Ordering::SeqCst);
            while *paused && !self.s.cancelled.load(Ordering::SeqCst) {
                paused = self.s.unpaused.wait(paused).unwrap();
            }
            self.s.waiting_while_paused.store(false, Ordering::SeqCst);
        }
        if self.s.cancelled.load(Ordering::SeqCst) {
            return Err(Fail::Cancelled);
        }
        self.message(&message);
        Ok(())
    }

    fn error(&self, p: &Path, e: &io::Error) {
        self.error_text(p, &error_text(e));
    }

    fn error_text(&self, p: &Path, reason: &str) {
        self.push_error(format!("{}: {reason}", p.display()));
    }

    fn push_error(&self, line: String) {
        self.s.errors.lock().unwrap().push(line);
    }
}

/// A short reason for an error line: "Access denied", "Not found", or the message without the
/// "(os error N)" suffix.
fn error_text(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => "Access denied".into(),
        io::ErrorKind::NotFound => "Not found".into(),
        io::ErrorKind::DirectoryNotEmpty => "Folder is not empty".into(),
        io::ErrorKind::AlreadyExists => "Already exists".into(),
        _ => {
            let text = e.to_string();
            match text.find(" (os error") {
                Some(i) => text[..i].to_string(),
                None => text,
            }
        }
    }
}

/// Name for a new archive: "name.zip" for one item (extension dropped for files), else "Archive.zip".
pub fn archive_name(items: &[PathBuf]) -> String {
    let [p] = items else {
        return "Archive.zip".into();
    };
    let mut name = fs_util::display_name(p);
    if let Some(dot) = name.rfind('.').filter(|&d| d > 0) {
        if !p.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            name.truncate(dot);
        }
    }
    name + ".zip"
}

fn file_name(p: &Path) -> String {
    fs_util::display_name(p)
}

/// FIFOs, sockets, devices: neither file, folder nor link.
fn is_other(m: &Metadata) -> bool {
    let ft = m.file_type();
    !ft.is_file() && !ft.is_dir() && !ft.is_symlink()
}

/// Whether two paths are the same file (following links), as `Files.isSameFile`.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::metadata(a), fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
    }
}

/// An id of the file system holding `p` (device number; the drive on Windows).
fn store_id(p: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(p).ok().map(|m| m.dev())
    }
    #[cfg(not(unix))]
    {
        use std::hash::{Hash, Hasher};
        let abs = fs::canonicalize(p).ok()?;
        let prefix = abs.components().next()?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        prefix.as_os_str().to_string_lossy().to_uppercase().hash(&mut h);
        Some(h.finish())
    }
}

fn delete_source_folder(src: &Path) -> io::Result<()> {
    match fs::remove_dir(src) {
        // something inside was skipped or failed: keep the source folder
        Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
        other => other,
    }
}

/// Removes one item: a folder (must be empty), a file or a link (as the link).
fn delete_one(p: &Path, dir: bool) -> io::Result<()> {
    let r = if dir { fs::remove_dir(p) } else { remove_file_or_link(p) };
    match r {
        #[cfg(windows)]
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            // read-only files cannot be deleted on Windows
            let mut perms = fs::symlink_metadata(p)?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            fs::set_permissions(p, perms)?;
            if dir {
                fs::remove_dir(p)
            } else {
                remove_file_or_link(p)
            }
        }
        other => other,
    }
}

fn remove_file_or_link(p: &Path) -> io::Result<()> {
    match fs::remove_file(p) {
        // a link to a folder is a directory entry on Windows
        #[cfg(windows)]
        Err(e) if p.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) => fs::remove_dir(p).map_err(|_| e),
        other => other,
    }
}

/// Deletes a tree without following links; used for our own temp files only.
fn remove_tree_quietly(p: &Path) -> io::Result<()> {
    for e in fs::read_dir(p)?.filter_map(Result::ok) {
        let child = e.path();
        match child.symlink_metadata() {
            Ok(m) if m.is_dir() && !fs_util::is_junction(&child) => remove_tree_quietly(&child)?,
            Ok(m) => delete_one(&child, m.is_dir())?,
            Err(_) => {}
        }
    }
    fs::remove_dir(p)
}

/// Renames within one file system. Returns false when the rename is impossible and copying should
/// be used instead; errors that copying would not fix (access denied, missing source) are returned.
fn try_rename(src: &Path, dst: &Path, replace: bool) -> io::Result<bool> {
    let r = if replace { fs::rename(src, dst) } else { rename_noreplace(src, dst) };
    match r {
        Ok(()) => Ok(true),
        Err(e) if matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound) => Err(e),
        Err(_) => Ok(false),
    }
}

/// Moves a finished temp file to its final name, atomically replacing the target when allowed.
fn place(tmp: &Path, dst: &Path, replace: bool) -> io::Result<()> {
    if replace {
        fs::rename(tmp, dst)
    } else {
        rename_noreplace(tmp, dst)
    }
}

/// Renames `p` to a hidden sibling so it can be restored if replacing it fails.
fn displace(p: &Path) -> io::Result<PathBuf> {
    let aside = temp_sibling(p);
    rename_noreplace(p, &aside)?;
    Ok(aside)
}

fn restore(displaced: Option<&Path>, original: &Path) {
    if let Some(d) = displaced {
        // on failure it stays under the temp name rather than being lost
        let _ = rename_noreplace(d, original);
    }
}

/// `rename` that fails with `AlreadyExists` instead of replacing an existing target.
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let a = CString::new(from.as_os_str().as_bytes())?;
        let b = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both strings are valid, NUL-terminated paths.
        let rc = unsafe {
            libc::renameat2(libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), libc::RENAME_NOREPLACE)
        };
        if rc == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        // file systems without RENAME_NOREPLACE (some FUSE, older NFS) fall through to check + rename
        if !matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EOPNOTSUPP)) {
            return Err(e);
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let a = CString::new(from.as_os_str().as_bytes())?;
        let b = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both strings are valid, NUL-terminated paths.
        let rc = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_EXCL) };
        if rc == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if !matches!(e.raw_os_error(), Some(libc::ENOTSUP) | Some(libc::EINVAL)) {
            return Err(e);
        }
    }
    if exists_nofollow(to) {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Already exists"));
    }
    fs::rename(from, to)
}

/// Copies a link as a link (same target text), with the link's own times where supported.
fn copy_link(src: &Path, dst: &Path) -> io::Result<()> {
    let target = fs::read_link(src)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, dst)?;
    #[cfg(windows)]
    {
        if src.metadata().is_ok_and(|m| m.is_dir()) {
            std::os::windows::fs::symlink_dir(&target, dst)?;
        } else {
            std::os::windows::fs::symlink_file(&target, dst)?;
        }
    }
    if let Ok(m) = src.symlink_metadata() {
        let mtime = filetime::FileTime::from_last_modification_time(&m);
        let atime = filetime::FileTime::from_last_access_time(&m);
        let _ = filetime::set_symlink_file_times(dst, atime, mtime);
    }
    Ok(())
}

fn copy_times(dst: &Path, a: &Metadata) {
    // the target file system may not support it; not worth failing the copy
    let _ = filetime::set_file_mtime(dst, filetime::FileTime::from_last_modification_time(a));
}

fn copy_metadata(src: &Path, dst: &Path, a: &Metadata) {
    copy_times(dst, a);
    if let Ok(m) = fs::metadata(src) {
        let _ = fs::set_permissions(dst, m.permissions());
    }
}

/// Flushes a finished copy to disk (used before deleting the source of a cross-device move).
fn force(p: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        match OpenOptions::new().write(true).open(p) {
            Ok(f) => f.sync_all(),
            // read-only copy: cannot open for flushing on Windows
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Ok(()),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(windows))]
    {
        File::open(p)?.sync_all()
    }
}

/// A hidden sibling for temp copies and displaced items: `.fileem-<random hex>.part`.
pub(crate) fn temp_sibling(target: &Path) -> PathBuf {
    target.with_file_name(format!(".fileem-{:016x}.part", random_u64()))
}

fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // RandomState is seeded from the OS once per process and varied per instance
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u128(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.finish()
}

/// Zip names that can't be paths here: NUL bytes anywhere, and on Windows its reserved characters.
fn valid_entry_name(name: &str) -> bool {
    if name.contains('\0') {
        return false;
    }
    if os::is_windows() {
        let reserved = |part: &str| part.contains([':', '*', '?', '"', '<', '>', '|']);
        let drive = |part: &str| part.len() == 2 && part.as_bytes()[0].is_ascii_alphabetic() && part.ends_with(':');
        // a leading drive ("C:") is an absolute path: the zip-slip check reports that one
        return !name.split(['/', '\\']).enumerate().any(|(i, part)| reserved(part) && !(i == 0 && drive(part)));
    }
    true
}

/// `.` and `..` resolved lexically, for a path that is already absolute.
fn lexical_normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Zip times are local wall-clock times with 2 s precision, from 1980 on.
fn zip_time(t: Option<SystemTime>) -> zip::DateTime {
    use chrono::{Datelike, Timelike};
    t.map(chrono::DateTime::<chrono::Local>::from)
        .and_then(|d| {
            zip::DateTime::from_date_and_time(
                u16::try_from(d.year()).ok()?,
                d.month() as u8,
                d.day() as u8,
                d.hour() as u8,
                d.minute() as u8,
                d.second() as u8,
            )
            .ok()
        })
        .unwrap_or_default()
}

fn from_zip_time(t: zip::DateTime) -> Option<SystemTime> {
    use chrono::TimeZone;
    let date = chrono::NaiveDate::from_ymd_opt(t.year() as i32, t.month() as u32, t.day() as u32)?;
    let local = date.and_hms_opt(t.hour() as u32, t.minute() as u32, t.second() as u32)?;
    chrono::Local.from_local_datetime(&local).earliest().map(SystemTime::from)
}

fn zip_error(e: zip::result::ZipError) -> Fail {
    Fail::Io(zip_io_error(e))
}

fn zip_io_error(e: zip::result::ZipError) -> io::Error {
    match e {
        zip::result::ZipError::Io(e) => e,
        other => io::Error::other(format!("Not a readable zip archive: {other}")),
    }
}
