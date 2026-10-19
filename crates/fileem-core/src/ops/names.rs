//! Renaming and creating single items (F2, F7): quick enough for the UI thread, and never
//! replacing anything that exists.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use super::file_operation::rename_noreplace;
use crate::util::fs_util;

/// Why `name` can't be used for a new item in `dir` (or for renaming `current` there), or `None`.
pub fn name_problem(dir: &Path, name: &str, current: Option<&Path>) -> Option<String> {
    let name = name.trim();
    if let Some(problem) = fs_util::validate_name(name) {
        return Some(problem);
    }
    let target = dir.join(name);
    let taken = fs_util::exists_nofollow(&target);
    // renaming to the same name, or a case-only change on a case-insensitive drive, is fine
    let itself = current.is_some_and(|c| c == target || same_entry(c, &target));
    (taken && !itself).then(|| format!("An item named \u{201c}{name}\u{201d} already exists."))
}

/// Renames `src` to `new_name` in the same folder. Never replaces an existing item; a case-only
/// change on a case-insensitive drive goes through a temporary name.
pub fn rename(src: &Path, new_name: &str) -> io::Result<PathBuf> {
    let new_name = new_name.trim();
    if let Some(problem) = fs_util::validate_name(new_name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, problem));
    }
    let target = src.with_file_name(new_name);
    if target == src {
        return Ok(target);
    }
    if fs_util::exists_nofollow(&target) && same_entry(src, &target) {
        let tmp = fs_util::unique_name(&src.with_file_name(format!("{new_name}.renaming")), false);
        rename_noreplace(src, &tmp)?;
        if let Err(e) = rename_noreplace(&tmp, &target) {
            let _ = rename_noreplace(&tmp, src);
            return Err(e);
        }
        return Ok(target);
    }
    rename_noreplace(src, &target)?;
    Ok(target)
}

pub fn create_folder(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let p = dir.join(name.trim());
    fs::create_dir(&p)?;
    Ok(p)
}

pub fn create_file(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let p = dir.join(name.trim());
    OpenOptions::new().write(true).create_new(true).open(&p)?;
    Ok(p)
}

/// Whether two paths are the same directory entry (not following links): a case-only rename
/// target on a case-insensitive drive.
fn same_entry(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (a.symlink_metadata(), b.symlink_metadata()) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_and_never_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let a = d.join("a.txt");
        fs::write(&a, "A").unwrap();
        fs::write(d.join("b.txt"), "B").unwrap();
        let renamed = rename(&a, " c.txt ").unwrap();
        assert_eq!(renamed, d.join("c.txt"), "names are trimmed");
        assert_eq!(fs::read_to_string(&renamed).unwrap(), "A");

        let err = rename(&renamed, "b.txt").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(d.join("b.txt")).unwrap(), "B", "the existing file is intact");
        assert_eq!(fs::read_to_string(&renamed).unwrap(), "A", "and so is the one being renamed");

        assert_eq!(rename(&renamed, "bad/name").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(rename(&renamed, "c.txt").unwrap(), renamed, "the same name is a no-op");
    }

    #[test]
    fn case_only_renames_work() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("readme.md");
        fs::write(&a, "x").unwrap();
        let r = rename(&a, "README.md").unwrap();
        let names: Vec<String> =
            fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["README.md"]);
        assert_eq!(fs::read_to_string(r).unwrap(), "x");
    }

    #[test]
    fn name_problems() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        fs::write(d.join("taken.txt"), "").unwrap();
        assert_eq!(name_problem(d, "free.txt", None), None);
        assert!(name_problem(d, "taken.txt", None).unwrap().contains("already exists"));
        assert_eq!(name_problem(d, "taken.txt", Some(&d.join("taken.txt"))), None, "renaming to itself");
        assert!(name_problem(d, "", None).is_some());
        assert!(name_problem(d, "a/b", None).is_some());
    }

    #[test]
    fn creating_never_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let folder = create_folder(d, "New folder").unwrap();
        assert!(folder.is_dir());
        assert_eq!(create_folder(d, "New folder").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        fs::write(d.join("f.txt"), "keep").unwrap();
        assert_eq!(create_file(d, "f.txt").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(d.join("f.txt")).unwrap(), "keep");
        assert!(create_file(d, "g.txt").unwrap().is_file());
    }
}
