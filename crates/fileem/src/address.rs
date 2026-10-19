//! The address bar's logic (the Java `AddressBar`): breadcrumb segments of a path, and turning
//! typed text into a path.

use std::path::{Path, PathBuf};

use fileem_core::util::fs_util;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub path: PathBuf,
    /// The file system root (drawn as a drive).
    pub drive: bool,
}

/// One crumb per folder from the root down to `p`.
pub fn crumbs(p: &Path) -> Vec<Crumb> {
    let mut chain: Vec<&Path> = p.ancestors().collect();
    chain.reverse();
    chain
        .into_iter()
        .map(|seg| match seg.file_name() {
            Some(name) => Crumb { label: name.to_string_lossy().into_owned(), path: seg.to_path_buf(), drive: false },
            // "C:\" shows as "C:"; "/" as a drive icon
            None => Crumb { label: seg.to_string_lossy().trim_end_matches(['\\', '/']).to_string(), path: seg.to_path_buf(), drive: true },
        })
        .collect()
}

/// Typed text to a path: `~` is the home folder, relative paths start at `current`, and `.` /
/// `..` are resolved. `None` for empty text.
pub fn resolve(text: &str, current: Option<&Path>, home: &Path) -> Option<PathBuf> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let path = match text.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => home.join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(text),
    };
    let path = match current {
        Some(cur) if path.is_relative() => cur.join(path),
        _ => path,
    };
    Some(fs_util::normalize(&path))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn crumbs_go_from_the_root_down() {
        let c = crumbs(Path::new("/home/u/2019_09_08"));
        let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["", "home", "u", "2019_09_08"], "names are shown as they are");
        assert!(c[0].drive && !c[1].drive);
        assert_eq!(c[2].path, Path::new("/home/u"));
        assert_eq!(crumbs(Path::new("/")).len(), 1);
    }

    #[test]
    fn typed_text_resolves_home_relative_and_dots() {
        let home = Path::new("/home/u");
        let cur = Some(Path::new("/data/x"));
        assert_eq!(resolve("~", cur, home).unwrap(), home);
        assert_eq!(resolve(" ~/docs ", cur, home).unwrap(), Path::new("/home/u/docs"));
        assert_eq!(resolve("~other", cur, home).unwrap(), Path::new("/data/x/~other"), "only ~ and ~/ are home");
        assert_eq!(resolve("sub/../y", cur, home).unwrap(), Path::new("/data/x/y"));
        assert_eq!(resolve("..", cur, home).unwrap(), Path::new("/data"));
        assert_eq!(resolve("/etc/./hosts", cur, home).unwrap(), Path::new("/etc/hosts"));
        assert_eq!(resolve("   ", cur, home), None);
    }
}
