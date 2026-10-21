//! Copy/move/delete/trash semantics and integrity on failure and cancel (port of `FileOperationTest`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::file_operation::{archive_name, FAST_COPY_LIMIT};
use super::{Decision, FileOperation, Handle, Kind, Resolution, State};

struct Dirs {
    _tmp: tempfile::TempDir,
    base: PathBuf,
    src: PathBuf,
    dst: PathBuf,
}

fn dirs() -> Dirs {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_path_buf();
    let src = base.join("src");
    let dst = base.join("dst");
    fs::create_dir(&src).unwrap();
    fs::create_dir(&dst).unwrap();
    Dirs { _tmp: tmp, base, src, dst }
}

fn fail_on_conflict(s: &Path, t: &Path) -> Decision {
    panic!("unexpected conflict {} -> {}", s.display(), t.display());
}

fn op(kind: Kind, sources: &[PathBuf], target: Option<&Path>) -> FileOperation {
    let mut op = FileOperation::new(kind, sources.to_vec(), target.map(Path::to_path_buf));
    op.on_conflict(fail_on_conflict);
    op
}

fn run(kind: Kind, sources: &[PathBuf], target: Option<&Path>) -> Handle {
    op(kind, sources, target).run()
}

fn run_with(kind: Kind, sources: &[PathBuf], target: &Path, r: Resolution) -> Handle {
    let mut op = op(kind, sources, Some(target));
    op.on_conflict(move |_, _| Decision::once(r));
    op.run()
}

fn assert_progress_complete(h: &Handle) {
    let (done, total) = h.progress();
    assert_eq!(done, total, "progress completes");
}

fn random_bytes(size: usize, seed: u64) -> Vec<u8> {
    let mut s = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    (0..size)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s as u8
        })
        .collect()
}

/// Relative paths below `root`, sorted, joined with "," (links are not followed).
fn tree(root: &Path) -> String {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            if p.symlink_metadata().unwrap().is_dir() {
                walk(root, &p, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out.join(",")
}

fn assert_no_temp_files(base: &Path) {
    let leftovers: Vec<String> = tree(base).split(',').filter(|p| p.contains(".fileem-")).map(String::from).collect();
    assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");
}

/// Larger than the fast-copy limit, so the chunked (cancellable, fsync-able) path is used.
fn big_file(p: &Path, seed: u64) -> PathBuf {
    fs::write(p, random_bytes(FAST_COPY_LIMIT as usize + 3 * 1024 * 1024 + 17, seed)).unwrap();
    p.to_path_buf()
}

fn write(p: &Path, text: &str) -> PathBuf {
    fs::write(p, text).unwrap();
    p.to_path_buf()
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap()
}

fn set_mtime(p: &Path, t: SystemTime) {
    filetime::set_file_mtime(p, filetime::FileTime::from_system_time(t)).unwrap();
}

fn mtime(p: &Path) -> SystemTime {
    fs::metadata(p).unwrap().modified().unwrap()
}

#[cfg(unix)]
fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

// ---- copy ----

#[test]
fn copies_trees_byte_for_byte_and_keeps_modification_times() {
    let d = dirs();
    fs::create_dir_all(d.src.join("sub/deep")).unwrap();
    let small = random_bytes(4096, 1);
    fs::write(d.src.join("a.bin"), &small).unwrap();
    write(&d.src.join("sub/b.txt"), "B");
    big_file(&d.src.join("sub/deep/big.bin"), 2);
    fs::create_dir(d.src.join("empty")).unwrap();
    let time = UNIX_EPOCH + Duration::from_millis(1_600_000_000_000);
    set_mtime(&d.src.join("a.bin"), time);
    set_mtime(&d.src.join("sub/deep/big.bin"), time);

    let h = run(Kind::Copy, &[d.src.join("a.bin"), d.src.join("sub"), d.src.join("empty")], Some(&d.dst));

    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(h.state(), State::Succeeded);
    assert_eq!(tree(&d.dst), "a.bin,empty,sub,sub/b.txt,sub/deep,sub/deep/big.bin");
    assert_eq!(fs::read(d.dst.join("a.bin")).unwrap(), small);
    assert_eq!(fs::read(d.src.join("sub/deep/big.bin")).unwrap(), fs::read(d.dst.join("sub/deep/big.bin")).unwrap());
    assert_eq!(mtime(&d.dst.join("a.bin")), time);
    assert_eq!(mtime(&d.dst.join("sub/deep/big.bin")), time);
    assert!(d.src.join("sub/deep/big.bin").exists(), "copy keeps the source");
    assert_progress_complete(&h);
    assert_no_temp_files(&d.base);
}

#[cfg(unix)]
#[test]
fn copy_keeps_posix_permissions_on_both_copy_paths() {
    use std::os::unix::fs::PermissionsExt;
    let d = dirs();
    let small = write(&d.src.join("run.sh"), "#!/bin/sh");
    let big = big_file(&d.src.join("big.run"), 3);
    for p in [&small, &big] {
        fs::set_permissions(p, fs::Permissions::from_mode(0o750)).unwrap();
    }
    run(Kind::Copy, &[small, big], Some(&d.dst));
    for name in ["run.sh", "big.run"] {
        assert_eq!(fs::metadata(d.dst.join(name)).unwrap().permissions().mode() & 0o777, 0o750, "{name}");
    }
}

#[test]
fn copy_into_same_folder_makes_a_copy_name() {
    let d = dirs();
    let a = write(&d.src.join("a.txt"), "A");
    run(Kind::Copy, std::slice::from_ref(&a), Some(&d.src));
    run(Kind::Copy, &[a], Some(&d.src));
    assert_eq!(tree(&d.src), "a - Copy (2).txt,a - Copy.txt,a.txt");
}

#[test]
fn copy_folder_into_itself_is_refused() {
    let d = dirs();
    fs::create_dir_all(d.src.join("sub/deep")).unwrap();
    let h = run(Kind::Copy, &[d.src.join("sub")], Some(&d.src.join("sub/deep")));
    assert_eq!(h.errors().len(), 1);
    assert_eq!(tree(&d.src), "sub,sub/deep");
}

#[cfg(unix)]
#[test]
fn copying_or_moving_a_folder_into_itself_through_a_link_is_refused() {
    let d = dirs();
    fs::create_dir_all(d.src.join("sub/deep")).unwrap();
    write(&d.src.join("sub/file.txt"), "f");
    let via_link = d.dst.join("link");
    std::os::unix::fs::symlink(d.src.join("sub"), &via_link).unwrap();
    let before = tree(&d.src);
    for kind in [Kind::Copy, Kind::Move] {
        let h = run(kind, &[d.src.join("sub")], Some(&via_link.join("deep")));
        assert_eq!(h.errors().len(), 1, "{kind:?}: {:?}", h.errors());
        assert!(h.errors()[0].contains("into itself"), "{:?}", h.errors());
        assert_eq!(tree(&d.src), before, "{kind:?} left everything as it was");
    }
    assert_no_temp_files(&d.base);
}

// ---- conflicts ----

#[test]
fn conflict_resolutions() {
    let d = dirs();
    let a = write(&d.src.join("a.txt"), "new");
    write(&d.dst.join("a.txt"), "old");

    run_with(Kind::Copy, std::slice::from_ref(&a), &d.dst, Resolution::Skip);
    assert_eq!(read(&d.dst.join("a.txt")), "old");

    run_with(Kind::Copy, std::slice::from_ref(&a), &d.dst, Resolution::KeepBoth);
    assert_eq!(read(&d.dst.join("a (2).txt")), "new");
    assert_eq!(read(&d.dst.join("a.txt")), "old");

    run_with(Kind::Copy, std::slice::from_ref(&a), &d.dst, Resolution::Overwrite);
    assert_eq!(read(&d.dst.join("a.txt")), "new");

    write(&d.dst.join("a.txt"), "old");
    let cancelled = run_with(Kind::Copy, &[a], &d.dst, Resolution::Cancel);
    assert!(cancelled.is_cancelled());
    assert_eq!(cancelled.state(), State::Cancelled);
    assert_eq!(read(&d.dst.join("a.txt")), "old");
    assert_no_temp_files(&d.base);
}

#[test]
fn apply_to_all_asks_only_once() {
    let d = dirs();
    let mut sources = Vec::new();
    for n in ["1", "2", "3"] {
        sources.push(write(&d.src.join(n), &format!("new{n}")));
        write(&d.dst.join(n), "old");
    }
    let asked = Arc::new(AtomicUsize::new(0));
    let mut o = op(Kind::Copy, &sources, Some(&d.dst));
    let counter = asked.clone();
    o.on_conflict(move |_, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        Decision::for_all(Resolution::Overwrite)
    });
    o.run();
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(read(&d.dst.join("3")), "new3");
}

#[test]
fn folders_merge_and_only_conflicting_files_ask() {
    let d = dirs();
    fs::create_dir_all(d.src.join("d")).unwrap();
    fs::create_dir_all(d.dst.join("d")).unwrap();
    write(&d.src.join("d/new.txt"), "n");
    write(&d.src.join("d/same.txt"), "src");
    write(&d.dst.join("d/same.txt"), "dst");
    write(&d.dst.join("d/keep.txt"), "k");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut o = op(Kind::Copy, &[d.src.join("d")], Some(&d.dst));
    let log = asked.clone();
    o.on_conflict(move |_, t| {
        log.lock().unwrap().push(t.to_path_buf());
        Decision::once(Resolution::Skip)
    });
    o.run();
    assert_eq!(*asked.lock().unwrap(), [d.dst.join("d/same.txt")]);
    assert_eq!(tree(&d.dst), "d,d/keep.txt,d/new.txt,d/same.txt");
    assert_eq!(read(&d.dst.join("d/same.txt")), "dst");
}

#[test]
fn replacing_a_folder_with_a_file_and_back() {
    let d = dirs();
    let x = write(&d.src.join("x"), "file");
    fs::create_dir_all(d.dst.join("x/inner")).unwrap();
    run_with(Kind::Copy, &[x], &d.dst, Resolution::Overwrite);
    assert_eq!(read(&d.dst.join("x")), "file");

    let src_dir = d.base.join("other/y/inner");
    fs::create_dir_all(&src_dir).unwrap();
    write(&d.dst.join("y"), "file");
    run_with(Kind::Copy, &[src_dir.parent().unwrap().to_path_buf()], &d.dst, Resolution::Overwrite);
    assert!(d.dst.join("y/inner").is_dir());
    assert_no_temp_files(&d.base);
}

// ---- integrity on failure / cancel ----

#[test]
fn cancelling_during_an_overwrite_keeps_the_original_intact() {
    let d = dirs();
    let big = big_file(&d.src.join("big.bin"), 4);
    let original = random_bytes(1000, 5);
    fs::write(d.dst.join("big.bin"), &original).unwrap();
    let mut o = op(Kind::Copy, &[big], Some(&d.dst));
    let h = o.handle();
    o.on_conflict(move |_, _| {
        h.cancel(); // cancel right as the replacement copy starts
        Decision::once(Resolution::Overwrite)
    });
    let h = o.run();
    assert!(h.is_cancelled());
    assert_eq!(h.state(), State::Cancelled);
    assert_eq!(fs::read(d.dst.join("big.bin")).unwrap(), original);
    assert_no_temp_files(&d.base);
}

#[cfg(unix)]
#[test]
fn failed_overwrite_keeps_the_original_intact() {
    use std::os::unix::fs::PermissionsExt;
    if is_root() {
        return; // root reads everything
    }
    let d = dirs();
    let unreadable = write(&d.src.join("a.txt"), "secret");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o200)).unwrap();
    write(&d.dst.join("a.txt"), "original");
    let h = run_with(Kind::Copy, std::slice::from_ref(&unreadable), &d.dst, Resolution::Overwrite);
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(h.errors().len(), 1, "{:?}", h.errors());
    assert!(h.errors()[0].ends_with(": Access denied"), "{:?}", h.errors());
    assert_eq!(read(&d.dst.join("a.txt")), "original");
    assert_no_temp_files(&d.base);
}

/// Runs `f` on its own thread and fails the test if it takes longer than `limit` (a hang).
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit).expect("timed out")
}

#[cfg(unix)]
#[test]
fn special_files_are_skipped_instead_of_hanging() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let d = dirs();
    let fifo = CString::new(d.src.join("pipe").as_os_str().as_bytes()).unwrap();
    // SAFETY: a valid path string.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    write(&d.src.join("ok.txt"), "x");
    let (src, dst) = (d.src.clone(), d.dst.clone());
    let h = within(Duration::from_secs(20), move || run(Kind::Copy, &[src], Some(&dst)));
    assert_eq!(h.errors().len(), 1, "{:?}", h.errors());
    assert!(h.errors()[0].contains("Special file skipped"), "{:?}", h.errors());
    assert!(d.dst.join("src/ok.txt").exists());
}

// ---- links ----

#[cfg(unix)]
#[test]
fn symlinks_are_copied_as_links() {
    use std::os::unix::fs::symlink;
    let d = dirs();
    write(&d.src.join("t.txt"), "t");
    symlink("t.txt", d.src.join("rel-link")).unwrap();
    symlink(&d.base, d.src.join("dir-link")).unwrap();
    let h = run(Kind::Copy, &[d.src.join("rel-link"), d.src.join("dir-link")], Some(&d.dst));
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert!(d.dst.join("rel-link").symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(d.dst.join("rel-link")).unwrap(), Path::new("t.txt"));
    assert!(
        d.dst.join("dir-link").symlink_metadata().unwrap().file_type().is_symlink(),
        "linked folder is not copied recursively"
    );
    assert_no_temp_files(&d.base);
}

#[cfg(unix)]
#[test]
fn deleting_a_folder_never_follows_links_out_of_it() {
    use std::os::unix::fs::symlink;
    let d = dirs();
    let outside = d.base.join("outside");
    fs::create_dir(&outside).unwrap();
    write(&outside.join("precious.txt"), "keep me");
    symlink(&outside, d.src.join("link-to-outside")).unwrap();
    symlink(outside.join("precious.txt"), d.src.join("file-link")).unwrap();

    let h = run(Kind::Delete, std::slice::from_ref(&d.src), None);
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert!(d.src.symlink_metadata().is_err());
    assert_eq!(read(&outside.join("precious.txt")), "keep me");
}

#[cfg(unix)]
#[test]
fn deleting_a_link_itself_keeps_its_target() {
    let d = dirs();
    let outside = d.base.join("outside");
    fs::create_dir(&outside).unwrap();
    write(&outside.join("f"), "x");
    let link = d.src.join("l");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    run(Kind::Delete, std::slice::from_ref(&link), None);
    assert!(link.symlink_metadata().is_err());
    assert!(outside.join("f").exists());
}

// ---- move ----

#[cfg(unix)]
#[test]
fn move_on_the_same_file_system_renames_without_copying() {
    use std::os::unix::fs::MetadataExt;
    let d = dirs();
    let dir = d.src.join("d");
    fs::create_dir_all(&dir).unwrap();
    big_file(&dir.join("big.bin"), 6);
    let inode = fs::metadata(dir.join("big.bin")).unwrap().ino();
    let h = run(Kind::Move, std::slice::from_ref(&dir), Some(&d.dst));
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert!(!dir.exists());
    assert_eq!(fs::metadata(d.dst.join("d/big.bin")).unwrap().ino(), inode, "same inode: renamed, not copied");
}

#[test]
fn move_merges_and_keeps_sources_that_were_skipped() {
    let d = dirs();
    fs::create_dir_all(d.src.join("d")).unwrap();
    fs::create_dir_all(d.dst.join("d")).unwrap();
    write(&d.src.join("d/new.txt"), "n");
    write(&d.src.join("d/conflict.txt"), "src");
    write(&d.dst.join("d/conflict.txt"), "dst");

    let h = run_with(Kind::Move, &[d.src.join("d")], &d.dst, Resolution::Skip);
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(read(&d.dst.join("d/new.txt")), "n");
    assert_eq!(read(&d.dst.join("d/conflict.txt")), "dst");
    assert_eq!(tree(&d.src), "d,d/conflict.txt", "skipped file and its folder stay at the source");
}

// ---- moves between file systems (forced: tests run on one) ----

/// `folders` × `per_folder` small files under src/many.
fn many_small_files(src: &Path, folders: usize, per_folder: usize) -> PathBuf {
    let many = src.join("many");
    fs::create_dir(&many).unwrap();
    for f in 0..folders {
        let d = many.join(format!("sub{f}"));
        fs::create_dir(&d).unwrap();
        for i in 0..per_folder {
            write(&d.join(format!("f{i}.txt")), &format!("content {f}/{i}"));
        }
    }
    many
}

fn count_files(root: &Path) -> u64 {
    if !root.exists() {
        return 0;
    }
    let mut n = 0;
    for e in fs::read_dir(root).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            n += count_files(&p);
        } else {
            n += 1;
        }
    }
    n
}

fn copying_move(sources: &[PathBuf], target: &Path, syncer: Option<super::file_operation::Syncer>) -> FileOperation {
    let mut o = op(Kind::Move, sources, Some(target));
    o.force_copy_moves();
    o.syncer(syncer);
    o
}

fn assert_moved_intact(moved_root: &Path, folders: usize, per_folder: usize) {
    for f in 0..folders {
        for i in 0..per_folder {
            assert_eq!(read(&moved_root.join(format!("sub{f}/f{i}.txt"))), format!("content {f}/{i}"));
        }
    }
}

#[test]
fn a_move_between_drives_flushes_in_batches_and_deletes_sources_only_after_each_flush() {
    let d = dirs();
    let many = many_small_files(&d.src, 5, 500); // 2500 files: batches of 1000, 1000, 500
    let left_at_each_sync = Arc::new(Mutex::new(Vec::new()));
    let (log, watched, expected_dir) = (left_at_each_sync.clone(), many.clone(), d.dst.clone());
    let o = copying_move(
        std::slice::from_ref(&many),
        &d.dst,
        Some(Box::new(move |dir: &Path| {
            assert_eq!(dir, expected_dir, "the destination's file system is flushed");
            log.lock().unwrap().push(count_files(&watched));
            true
        })),
    );
    let h = o.run();
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(
        *left_at_each_sync.lock().unwrap(),
        [2500, 1500, 500],
        "one flush per batch, and no source of a batch is deleted before its flush"
    );
    assert!(!many.exists(), "the emptied source folders are removed too");
    assert_moved_intact(&d.dst.join("many"), 5, 500);
    assert_progress_complete(&h);
    assert_no_temp_files(&d.base);
}

#[test]
fn when_the_batch_flush_fails_every_copy_is_fsynced_before_its_source_goes() {
    let d = dirs();
    let many = many_small_files(&d.src, 2, 30);
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let o = copying_move(
        std::slice::from_ref(&many),
        &d.dst,
        Some(Box::new(move |_: &Path| {
            counter.fetch_add(1, Ordering::SeqCst);
            false // e.g. no syncfs support: fall back to fsync per file
        })),
    );
    let h = o.run();
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!many.exists());
    assert_moved_intact(&d.dst.join("many"), 2, 30);
    assert_no_temp_files(&d.base);
}

#[test]
fn without_batching_each_moved_file_is_fsynced_and_moved_as_before() {
    let d = dirs();
    let many = many_small_files(&d.src, 2, 20);
    let h = copying_move(std::slice::from_ref(&many), &d.dst, None).run();
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert!(!many.exists());
    assert_moved_intact(&d.dst.join("many"), 2, 20);
    assert_no_temp_files(&d.base);
}

#[test]
fn cancelling_a_batched_move_loses_nothing_and_removes_only_flushed_sources() {
    let d = dirs();
    let many = many_small_files(&d.src, 5, 500);
    let mut o = op(Kind::Move, std::slice::from_ref(&many), Some(&d.dst));
    o.force_copy_moves();
    let h = o.handle();
    let mut syncs = 0;
    o.syncer(Some(Box::new(move |_: &Path| {
        syncs += 1;
        if syncs == 1 {
            h.cancel(); // the user cancels while the first batch is being written
        }
        true
    })));
    let h = o.run();
    assert_eq!(h.state(), State::Cancelled);
    for f in 0..5 {
        for i in 0..500 {
            let rel = format!("sub{f}/f{i}.txt");
            let (in_src, in_dst) = (many.join(&rel), d.dst.join("many").join(&rel));
            assert!(in_src.exists() || in_dst.exists(), "{rel} was lost");
            if !in_src.exists() {
                assert_eq!(read(&in_dst), format!("content {f}/{i}"), "{rel} complete at the destination");
            }
        }
    }
    assert!(count_files(&many) > 0, "the rest of the move was cancelled");
    assert_no_temp_files(&d.base);
}

#[test]
fn move_with_overwrite_replaces_and_removes_the_source() {
    let d = dirs();
    let a = write(&d.src.join("a.txt"), "new");
    write(&d.dst.join("a.txt"), "old");
    run_with(Kind::Move, std::slice::from_ref(&a), &d.dst, Resolution::Overwrite);
    assert_eq!(read(&d.dst.join("a.txt")), "new");
    assert!(!a.exists());
}

#[test]
fn copying_move_with_overwrite_replaces_and_removes_the_source() {
    let d = dirs();
    let a = write(&d.src.join("a.txt"), "new");
    write(&d.dst.join("a.txt"), "old");
    let mut o = copying_move(std::slice::from_ref(&a), &d.dst, None);
    o.on_conflict(|_, _| Decision::once(Resolution::Overwrite));
    let h = o.run();
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(read(&d.dst.join("a.txt")), "new");
    assert!(!a.exists());
    assert_no_temp_files(&d.base);
}

#[test]
fn move_into_own_folder_or_same_folder_does_nothing() {
    let d = dirs();
    fs::create_dir_all(d.src.join("d/inner")).unwrap();
    let f = write(&d.src.join("f"), "x");
    let into = run(Kind::Move, &[d.src.join("d")], Some(&d.src.join("d/inner")));
    assert_eq!(into.errors().len(), 1);
    let same = run(Kind::Move, &[f], Some(&d.src));
    assert!(same.errors().is_empty());
    assert_eq!(tree(&d.src), "d,d/inner,f");
}

#[cfg(unix)]
#[test]
fn moving_a_file_onto_itself_through_another_path_is_refused() {
    let d = dirs();
    let a = write(&d.src.join("a.txt"), "data");
    let alias = d.base.join("alias");
    std::os::unix::fs::symlink(&d.src, &alias).unwrap();
    let h = run_with(Kind::Move, std::slice::from_ref(&a), &alias, Resolution::Overwrite);
    assert_eq!(h.errors().len(), 1, "{:?}", h.errors());
    assert_eq!(read(&a), "data", "file must survive");

    run(Kind::Copy, &[a], Some(&alias));
    assert_eq!(read(&d.src.join("a - Copy.txt")), "data", "copying onto itself makes a copy");
}

#[test]
fn roots_are_refused_for_move_delete_and_trash() {
    let root = std::path::absolute(Path::new("/")).unwrap();
    for kind in [Kind::Move, Kind::Delete, Kind::Trash] {
        let d = dirs();
        let target = if kind == Kind::Move { Some(d.dst.as_path()) } else { None };
        let h = run(kind, std::slice::from_ref(&root), target);
        assert_eq!(h.errors().len(), 1, "{kind:?}: {:?}", h.errors());
        assert!(h.errors()[0].contains("drive or file system root"), "{:?}", h.errors());
    }
}

// ---- delete / trash ----

#[test]
fn delete_removes_trees_and_reports_progress() {
    let d = dirs();
    fs::create_dir_all(d.src.join("a/b/c")).unwrap();
    for i in 0..50 {
        write(&d.src.join(format!("a/b/f{i}")), "x");
    }
    let h = run(Kind::Delete, &[d.src.join("a")], None);
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert!(!d.src.join("a").exists());
    assert_progress_complete(&h);
}

#[cfg(unix)]
#[test]
fn delete_continues_past_failures_and_keeps_what_it_cannot_delete() {
    use std::os::unix::fs::PermissionsExt;
    if is_root() {
        return;
    }
    let d = dirs();
    let locked = d.src.join("locked");
    fs::create_dir_all(&locked).unwrap();
    write(&locked.join("f"), "x");
    let free = write(&d.src.join("free.txt"), "x");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let h = run(Kind::Delete, &[locked.clone(), free.clone()], None);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!h.errors().is_empty());
    assert!(locked.join("f").exists());
    assert!(!free.exists(), "other items are still deleted");
}

#[test]
fn trash_of_a_missing_item_reports_an_error_and_touches_nothing() {
    let d = dirs();
    let h = run(Kind::Trash, &[d.base.join("does-not-exist")], None);
    assert_eq!(h.errors().len(), 1, "{:?}", h.errors());
}

#[test]
fn not_enough_space_fails_before_writing_anything() {
    let d = dirs();
    let sparse = d.src.join("huge.bin");
    let available = fs4::available_space(&d.dst).unwrap();
    fs::File::create(&sparse).unwrap().set_len(available + (1 << 30)).unwrap(); // sparse: costs no space
    let h = run(Kind::Copy, &[sparse], Some(&d.dst));
    match h.state() {
        State::Failed(msg) => assert!(msg.contains("Not enough free space"), "{msg}"),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(tree(&d.dst), "");
}

// ---- archives ----

#[test]
fn archive_names() {
    let d = dirs();
    let file = write(&d.src.join("report.final.txt"), "x");
    let dir = d.src.join("my.folder");
    fs::create_dir(&dir).unwrap();
    assert_eq!(archive_name(std::slice::from_ref(&file)), "report.final.zip");
    assert_eq!(archive_name(std::slice::from_ref(&dir)), "my.folder.zip");
    assert_eq!(archive_name(&[file, dir]), "Archive.zip");
}

#[test]
fn compress_and_extract_round_trips_content_and_times() {
    let d = dirs();
    fs::create_dir_all(d.src.join("d/inner/empty")).unwrap();
    let data = random_bytes(300_000, 7);
    fs::write(d.src.join("d/inner/data.bin"), &data).unwrap();
    let top = write(&d.src.join("top.txt"), "top");
    let time = UNIX_EPOCH + Duration::from_millis(1_600_000_000_000);
    set_mtime(&top, time);

    let zip = run(Kind::Compress, &[d.src.join("d"), top], Some(&d.src));
    assert!(zip.errors().is_empty(), "{:?}", zip.errors());
    assert_eq!(zip.result(), Some(d.src.join("Archive.zip")));
    assert_progress_complete(&zip);

    fs::rename(zip.result().unwrap(), d.dst.join("Archive.zip")).unwrap();
    let unzip = run(Kind::Extract, &[d.dst.join("Archive.zip")], Some(&d.dst));
    assert!(unzip.errors().is_empty(), "{:?}", unzip.errors());
    let out = d.dst.join("Archive");
    assert_eq!(unzip.result(), Some(out.clone()));
    assert_eq!(tree(&out), "d,d/inner,d/inner/data.bin,d/inner/empty,top.txt");
    assert_eq!(fs::read(out.join("d/inner/data.bin")).unwrap(), data);
    let delta = mtime(&out.join("top.txt")).duration_since(time).unwrap_or_else(|e| e.duration());
    assert!(delta <= Duration::from_secs(2), "modification time kept (zip has 2 s precision), off by {delta:?}");
    assert_no_temp_files(&d.base);
}

#[test]
fn extracting_a_single_top_level_item_does_not_nest_it_and_never_overwrites() {
    let d = dirs();
    fs::create_dir_all(d.src.join("project/sub")).unwrap();
    write(&d.src.join("project/sub/a.txt"), "a");
    let zip = run(Kind::Compress, &[d.src.join("project")], Some(&d.src));
    assert_eq!(zip.result(), Some(d.src.join("project.zip")));

    fs::rename(zip.result().unwrap(), d.dst.join("project.zip")).unwrap();
    fs::create_dir(d.dst.join("project")).unwrap(); // already there: must not be merged into
    let unzip = run(Kind::Extract, &[d.dst.join("project.zip")], Some(&d.dst));
    assert_eq!(unzip.result(), Some(d.dst.join("project (2)")));
    assert_eq!(read(&d.dst.join("project (2)/sub/a.txt")), "a");
    assert_eq!(tree(&d.dst.join("project")), "");
}

/// A zip with the given entries, each holding `data`.
fn make_zip(path: &Path, names: &[&str], data: &[u8]) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for name in names {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn extract_refuses_entries_that_escape_the_target() {
    let d = dirs();
    let zip = d.dst.join("evil.zip");
    make_zip(&zip, &["../escaped.txt", "a/../../escaped2.txt", "/abs-escaped.txt", "ok.txt"], b"x");
    let h = run(Kind::Extract, &[zip], Some(&d.dst));
    assert_eq!(h.errors().len(), 3, "{:?}", h.errors());
    assert!(!d.base.join("escaped.txt").exists());
    assert!(!d.base.join("escaped2.txt").exists());
    assert!(!Path::new("/abs-escaped.txt").exists());
    assert_eq!(read(&d.dst.join("ok.txt")), "x", "safe entries are extracted");
    assert_no_temp_files(&d.base);
}

#[test]
fn corrupt_archive_fails_without_leaving_anything() {
    let d = dirs();
    let zip = write(&d.dst.join("broken.zip"), "this is not a zip file");
    let h = run(Kind::Extract, &[zip], Some(&d.dst));
    assert!(matches!(h.state(), State::Failed(_)) || !h.errors().is_empty(), "{:?}", h.state());
    assert_eq!(tree(&d.dst), "broken.zip");
    assert_no_temp_files(&d.base);
}

/// A zip whose entry "big.bin" really holds `real_size` zero bytes but declares only 10 in its directory.
fn zip_with_understated_size(zip: &Path, real_size: usize) -> PathBuf {
    make_zip(zip, &["big.bin"], &vec![0u8; real_size]);
    let mut b = fs::read(zip).unwrap();
    let mut patched = false;
    for i in 0..b.len().saturating_sub(28) {
        if b[i..i + 4] == [0x50, 0x4b, 0x01, 0x02] {
            // central directory header: uncompressed size at offset 24
            b[i + 24..i + 28].copy_from_slice(&10u32.to_le_bytes());
            patched = true;
        }
    }
    assert!(patched, "central directory found");
    fs::write(zip, b).unwrap();
    zip.to_path_buf()
}

#[test]
fn extract_stops_at_the_declared_entry_size_so_a_zip_bomb_cannot_fill_the_disk() {
    let d = dirs();
    let zip = zip_with_understated_size(&d.dst.join("bomb.zip"), 1 << 20);
    let h = run(Kind::Extract, &[zip], Some(&d.dst));
    assert_eq!(h.errors().len(), 1, "{:?}", h.errors());
    assert!(h.errors()[0].contains("larger than the archive says"), "{:?}", h.errors());
    assert_eq!(h.result(), None);
    assert_eq!(tree(&d.dst), "bomb.zip", "nothing placed");
    assert_no_temp_files(&d.base);
}

#[test]
fn one_failing_archive_does_not_stop_the_others() {
    let d = dirs();
    let bomb = zip_with_understated_size(&d.dst.join("bomb.zip"), 1 << 20);
    let good = d.dst.join("good.zip");
    make_zip(&good, &["fine.txt"], b"ok");
    let h = run(Kind::Extract, &[bomb, good], Some(&d.dst));
    assert_eq!(h.state(), State::Succeeded);
    assert_eq!(h.errors().len(), 1, "{:?}", h.errors());
    assert_eq!(read(&d.dst.join("fine.txt")), "ok");
    assert_no_temp_files(&d.base);
}

#[test]
fn extract_skips_entries_whose_name_is_not_a_valid_path() {
    let d = dirs();
    let zip = d.dst.join("odd.zip");
    make_zip(&zip, &["bad\0name.txt", "ok.txt"], b"x");
    let h = run(Kind::Extract, &[zip], Some(&d.dst));
    assert_eq!(h.state(), State::Succeeded, "an invalid name does not abort the extraction");
    assert_eq!(h.errors().len(), 1, "{:?}", h.errors());
    assert_eq!(read(&d.dst.join("ok.txt")), "x");
    assert_no_temp_files(&d.base);
}

#[cfg(unix)]
#[test]
fn archives_skip_links_instead_of_following_them() {
    let d = dirs();
    fs::create_dir(d.src.join("d")).unwrap();
    write(&d.src.join("d/real.txt"), "r");
    std::os::unix::fs::symlink(&d.src, d.src.join("d/loop")).unwrap();
    let zip = run(Kind::Compress, &[d.src.join("d")], Some(&d.dst));
    assert_eq!(zip.errors().len(), 1, "{:?}", zip.errors());
    let unzip = run(Kind::Extract, &[zip.result().unwrap()], Some(&d.base));
    assert_eq!(tree(&unzip.result().unwrap()), "real.txt");
}

// ---- pause / resume ----

/// Runs the operation on its own thread, as the app does.
fn start(o: FileOperation) -> (Handle, thread::JoinHandle<Handle>) {
    let h = o.handle();
    (h, thread::spawn(move || o.run()))
}

fn await_paused(h: &Handle) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !h.is_waiting_while_paused() {
        assert!(Instant::now() < deadline, "the operation never paused");
        thread::yield_now();
    }
}

fn join(t: thread::JoinHandle<Handle>) -> Handle {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !t.is_finished() {
        assert!(Instant::now() < deadline, "the operation never finished");
        thread::sleep(Duration::from_millis(5));
    }
    t.join().unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn a_paused_operation_waits_and_then_finishes_on_resume() {
    let d = dirs();
    let files: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"].iter().map(|n| write(&d.src.join(n), n)).collect();
    let o = op(Kind::Copy, &files, Some(&d.dst));
    o.handle().pause();
    let (h, t) = start(o);
    await_paused(&h);
    assert!(h.is_paused());
    assert_eq!(h.message(), "Paused");
    assert!(names(&d.dst).is_empty(), "nothing is copied while paused");

    h.resume();
    let h = join(t);
    assert!(!h.is_paused());
    assert_eq!(names(&d.dst), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(h.state(), State::Succeeded);
    assert_progress_complete(&h);
}

#[test]
fn pausing_stops_between_items_and_resume_continues_with_the_next_one() {
    let d = dirs();
    let files: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"].iter().map(|n| write(&d.src.join(n), n)).collect();
    write(&d.dst.join("b.txt"), "existing");
    let mut o = op(Kind::Copy, &files, Some(&d.dst));
    let pauser = o.handle();
    // the conflict on b.txt pauses the operation, as a click on Pause would at that moment
    o.on_conflict(move |_, _| {
        pauser.pause();
        Decision::once(Resolution::Skip)
    });
    let (h, t) = start(o);
    await_paused(&h);
    assert_eq!(names(&d.dst), ["a.txt", "b.txt"], "a copied, b skipped, c waits");
    assert_eq!(read(&d.dst.join("b.txt")), "existing");

    h.resume();
    join(t);
    assert_eq!(read(&d.dst.join("c.txt")), "c.txt");
}

#[test]
fn cancelling_a_paused_operation_stops_it_and_leaves_no_partial_files() {
    let d = dirs();
    let big = d.src.join("big.bin");
    fs::write(&big, random_bytes(1 << 16, 7)).unwrap();
    let small = write(&d.src.join("small.txt"), "small");
    fs::create_dir(d.dst.join("sub")).unwrap();
    let o = op(Kind::Move, &[big.clone(), small.clone()], Some(&d.dst.join("sub")));
    o.handle().pause();
    let (h, t) = start(o);
    await_paused(&h);

    h.cancel();
    let h = join(t);
    assert_eq!(h.state(), State::Cancelled);
    assert!(names(&d.dst.join("sub")).is_empty(), "nothing moved, no .part files");
    assert!(big.exists() && small.exists(), "sources intact");
}

#[test]
fn await_stopped_returns_only_after_the_cancelled_worker_cleaned_up() {
    let d = dirs();
    let a = write(&d.src.join("a.txt"), "a");
    let b = write(&d.src.join("b.txt"), "b");
    let o = op(Kind::Copy, &[a, b], Some(&d.dst));
    o.handle().pause();
    let (h, t) = start(o);
    await_paused(&h);
    assert!(!h.await_stopped(Duration::from_millis(50)), "still running while paused");

    h.cancel();
    assert!(h.await_stopped(Duration::from_secs(10)));
    assert!(!h.is_waiting_while_paused(), "worker left the pause");
    assert!(names(&d.dst).is_empty(), "no files and no .part leftovers");
    join(t);
}

#[test]
fn an_operation_cancelled_before_it_started_counts_as_stopped() {
    let d = dirs();
    let a = write(&d.src.join("a.txt"), "a");
    let o = op(Kind::Copy, &[a], Some(&d.dst));
    o.handle().cancel();
    assert!(o.handle().await_stopped(Duration::from_secs(1)));
    let h = o.run(); // a cancelled operation never runs
    assert_eq!(h.state(), State::Cancelled);
    assert!(names(&d.dst).is_empty());
}

// ---- names ----

/// Linux names are bytes: names that aren't UTF-8 must arrive exactly as they are. Two of them
/// once both became "\u{fffd}.txt" in the copy, so the second asked to replace the first (and a
/// move had already deleted the first one's source).
#[cfg(target_os = "linux")]
#[test]
fn names_that_are_not_utf8_are_copied_and_moved_unchanged() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let names = [OsStr::from_bytes(b"\xff.txt"), OsStr::from_bytes(b"\xfe.txt")];
    for (kind, copy_style) in [(Kind::Copy, false), (Kind::Move, false), (Kind::Move, true)] {
        let d = dirs();
        let folder = d.src.join(OsStr::from_bytes(b"caf\xe9"));
        fs::create_dir(&folder).unwrap();
        for (i, n) in names.iter().enumerate() {
            fs::write(folder.join(n), format!("file {i}")).unwrap();
        }
        let mut o = op(kind, std::slice::from_ref(&folder), Some(&d.dst));
        if copy_style {
            o.force_copy_moves();
        }
        let h = o.run();
        assert_eq!(h.state(), State::Succeeded, "{kind:?}: {:?}", h.errors());
        assert!(h.errors().is_empty(), "{kind:?}: {:?}", h.errors());
        let copied = d.dst.join(OsStr::from_bytes(b"caf\xe9"));
        for (i, n) in names.iter().enumerate() {
            assert_eq!(fs::read_to_string(copied.join(n)).unwrap(), format!("file {i}"), "{kind:?} {copy_style}");
        }
        assert_eq!(fs::read_dir(&copied).unwrap().count(), 2);
        assert_eq!(folder.exists(), kind == Kind::Copy, "{kind:?} {copy_style}");
        assert_no_temp_files(&d.base);
    }
}
