//! Finds files and folders by name in a folder and everything below it (the Java `FileSearch`).
//! Patterns use the wildcards `*` (any text) and `?` (one character) and ignore case. Several can
//! be given, separated by spaces or `;` (`*.jpg *.png`); `!` in front excludes (`a*.* !*.mp3`). A
//! pattern without wildcards matches anywhere in the name, like Explorer's search box. Links are
//! not followed; unreadable folders are reported and skipped.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::FileEntry;
use crate::util::fs_util;

/// The name test of a pattern (see [`matcher`]).
#[derive(Debug, Clone)]
pub struct Matcher {
    include: Vec<Vec<char>>,
    exclude: Vec<Vec<char>>,
}

impl Matcher {
    /// A name matches when it matches any plain term and no `!` term; with only `!` terms,
    /// everything else matches.
    pub fn matches(&self, name: &str) -> bool {
        let name: Vec<char> = name.to_lowercase().chars().collect();
        let any = |terms: &[Vec<char>]| terms.iter().any(|t| fs_util::wildcard_match(t, &name));
        (self.include.is_empty() || any(&self.include)) && !any(&self.exclude)
    }
}

/// The name test for `pattern`; `None` when it has nothing to look for. Terms are separated by
/// spaces or `;`; quotes keep spaces in a term (`"my file*"`, `!"old copy"`).
pub fn matcher(pattern: &str) -> Option<Matcher> {
    let mut m = Matcher { include: Vec::new(), exclude: Vec::new() };
    for term in terms(pattern) {
        let (not, glob) = match term.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, term.as_str()),
        };
        if glob.is_empty() {
            continue;
        }
        let glob = if glob.contains(['*', '?']) { glob.to_string() } else { format!("*{glob}*") };
        let chars = glob.to_lowercase().chars().collect();
        if not { &mut m.exclude } else { &mut m.include }.push(chars);
    }
    (!m.include.is_empty() || !m.exclude.is_empty()).then_some(m)
}

/// Splits at spaces and `;` outside quotes; the quotes themselves are dropped.
pub fn terms(pattern: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in pattern.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && (c.is_whitespace() || c == ';') {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// What a search reports, on the searching thread.
pub trait Listener {
    fn found(&mut self, entry: FileEntry);

    /// A folder is about to be searched (for progress).
    fn entering(&mut self, _dir: &Path) {}

    /// A folder that couldn't be read (or only partly).
    fn failed(&mut self, _path: &Path) {}

    /// False ends the search early (enough results).
    fn wants_more(&self) -> bool {
        true
    }
}

/// Searches everything below `root` (not `root` itself) for names `m` accepts. Hidden items, and
/// everything inside hidden folders, are skipped unless `include_hidden`. A linked `root` is
/// searched through its target; results keep the path that was asked for. Blocks; `stop` ends it
/// early.
///
/// Returns true when the whole tree was searched, false when stopped or the listener had enough;
/// an error when `root` isn't a folder.
pub fn search(root: &Path, m: &Matcher, include_hidden: bool, listener: &mut dyn Listener, stop: &AtomicBool) -> io::Result<bool> {
    if !root.metadata()?.is_dir() {
        return Err(io::Error::other(format!("Not a folder: {}", root.display())));
    }
    let more = |l: &dyn Listener| !stop.load(Ordering::Relaxed) && l.wants_more();
    // depth first, a folder's subfolders in name order
    let mut pending: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if !more(listener) {
            return Ok(false);
        }
        listener.entering(&dir);
        let Ok(list) = std::fs::read_dir(&dir) else {
            listener.failed(&dir);
            continue;
        };
        let mut subfolders = Vec::new();
        for item in list {
            if !more(listener) {
                return Ok(false);
            }
            let Ok(item) = item else {
                listener.failed(&dir); // a listing that broke off halfway
                break;
            };
            let path = item.path();
            if !include_hidden && fs_util::is_hidden(&path) {
                continue;
            }
            // the entry's own type: links are reported, never followed
            let folder = item.file_type().is_ok_and(|t| t.is_dir()) && !fs_util::is_junction(&path);
            if m.matches(&item.file_name().to_string_lossy()) {
                listener.found(FileEntry::read(&path));
            }
            if folder {
                subfolders.push(path);
            }
        }
        subfolders.sort();
        pending.extend(subfolders.into_iter().rev());
    }
    Ok(!stop.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn m(p: &str) -> Matcher {
        matcher(p).unwrap()
    }

    #[derive(Default)]
    struct Collect {
        found: Vec<PathBuf>,
        failed: Vec<PathBuf>,
        limit: Option<usize>,
    }

    impl Listener for Collect {
        fn found(&mut self, e: FileEntry) {
            self.found.push(e.path().to_path_buf());
        }
        fn failed(&mut self, p: &Path) {
            self.failed.push(p.to_path_buf());
        }
        fn wants_more(&self) -> bool {
            self.limit.is_none_or(|n| self.found.len() < n)
        }
    }

    struct Result {
        found: Vec<String>,
        failed: Vec<String>,
        complete: bool,
    }

    fn rel(root: &Path, list: &[PathBuf]) -> Vec<String> {
        let mut v: Vec<String> =
            list.iter().map(|p| p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/")).collect();
        v.sort();
        v
    }

    fn run(root: &Path, pattern: &str, hidden: bool) -> Result {
        let mut c = Collect::default();
        let complete = search(root, &m(pattern), hidden, &mut c, &AtomicBool::new(false)).unwrap();
        Result { found: rel(root, &c.found), failed: rel(root, &c.failed), complete }
    }

    fn tree(dir: &Path, paths: &[&str]) {
        for p in paths {
            let f = dir.join(p);
            if p.ends_with('/') {
                fs::create_dir_all(&f).unwrap();
            } else {
                fs::create_dir_all(f.parent().unwrap()).unwrap();
                fs::write(&f, p).unwrap();
            }
        }
    }

    #[test]
    fn wildcards_and_plain_text() {
        let jpg = m("*.jpg");
        assert!(jpg.matches("a.jpg"));
        assert!(jpg.matches("PHOTO.JPG"), "case doesn't matter");
        assert!(!jpg.matches("a.jpg.txt"), "the whole name has to match");
        assert!(!jpg.matches("ajpg"));

        let img = m("IMG_????.*");
        assert!(img.matches("img_1234.cr2"));
        assert!(!img.matches("IMG_123.cr2"), "? is exactly one character");

        let plain = m("report");
        assert!(plain.matches("Annual Report 2024.pdf"), "no wildcards: anywhere in the name");
        assert!(!plain.matches("repo.txt"));

        let several = m(" *.png ; *.gif ;");
        assert!(several.matches("x.png"));
        assert!(several.matches("y.GIF"));
        assert!(!several.matches("z.jpg"));

        let special = m("\"a+b (1)[x].txt\"");
        assert!(special.matches("a+b (1)[x].txt"), "regex characters are plain text");
        assert!(!special.matches("aab (1)x.txt"));

        assert!(m("*").matches("anything"));
        assert!(matcher("").is_none());
        assert!(matcher("  ;  ").is_none());
    }

    #[test]
    fn spaces_separate_terms_and_an_exclamation_mark_excludes() {
        let a = m("a*.* !*.mp3");
        assert!(a.matches("abc.txt"));
        assert!(a.matches("Album.FLAC"));
        assert!(!a.matches("anthem.mp3"), "excluded");
        assert!(!a.matches("ANTHEM.MP3"), "excluding ignores case too");
        assert!(!a.matches("b.txt"), "not included");

        let any = m("*.jpg   *.png");
        assert!(any.matches("x.jpg"));
        assert!(any.matches("y.png"), "plain terms: any of them");
        assert!(!any.matches("z.gif"));

        let only_exclude = m("!*.tmp !~*");
        assert!(only_exclude.matches("report.doc"), "only exclusions: everything else");
        assert!(!only_exclude.matches("x.TMP"));
        assert!(!only_exclude.matches("~lock"));

        let both = m("*.txt;!old* *.md");
        assert!(both.matches("new.txt"));
        assert!(both.matches("notes.md"));
        assert!(!both.matches("old.txt"), "; and spaces mix");

        let quoted = m("\"my file*\" !\"my file copy*\"");
        assert!(quoted.matches("my file.txt"), "quotes keep the space");
        assert!(!quoted.matches("my file copy.txt"));
        assert!(!quoted.matches("file.txt"), "not split into \"my\" and \"file*\"");

        let plain = m("report !draft");
        assert!(plain.matches("Report final.pdf"));
        assert!(!plain.matches("report draft.pdf"), "plain text excludes anywhere in the name too");

        assert!(matcher("!").is_none(), "a lone ! has nothing to look for");
        assert!(matcher("  \"\" ").is_none());
    }

    #[test]
    fn exclusions_apply_in_every_folder() {
        let t = tempfile::tempdir().unwrap();
        tree(t.path(), &["a1.txt", "a2.mp3", "b.mp3", "sub/a3.MP3", "sub/a4.jpg"]);
        assert_eq!(run(t.path(), "a*.* !*.mp3", false).found, ["a1.txt", "sub/a4.jpg"]);
    }

    #[test]
    fn searches_every_folder_below_and_finds_folders_too() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        tree(dir, &["top.jpg", "notes.txt", "a/b/c/deep.JPG", "a/b/other.png", "photos.jpg/", "photos.jpg/inside.txt"]);
        let r = run(dir, "*.jpg", false);
        assert_eq!(r.found, ["a/b/c/deep.JPG", "photos.jpg", "top.jpg"]);
        assert!(r.complete);
        assert!(r.failed.is_empty());
        assert!(run(dir, "*.doc", false).found.is_empty());
        assert_eq!(run(&dir.join("a"), "deep", false).found, ["b/c/deep.JPG"], "only below the folder searched");
    }

    #[test]
    fn the_folder_searched_is_not_a_result_itself() {
        let t = tempfile::tempdir().unwrap();
        tree(t.path(), &["match/", "match/match.txt"]);
        assert_eq!(run(&t.path().join("match"), "match", false).found, ["match.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn hidden_items_and_everything_in_hidden_folders_only_when_asked() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        tree(dir, &["a.txt", ".b.txt", ".cache/c.txt", "sub/.d.txt"]);
        assert_eq!(run(dir, "*.txt", false).found, ["a.txt"]);
        assert_eq!(run(dir, "*.txt", true).found, [".b.txt", ".cache/c.txt", "a.txt", "sub/.d.txt"]);
        assert_eq!(run(&dir.join(".cache"), "*.txt", false).found, ["c.txt"], "searching inside a hidden folder itself works");
    }

    #[cfg(unix)]
    #[test]
    fn links_are_found_but_not_followed() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        tree(dir, &["real/inside.txt", "elsewhere/far.txt"]);
        std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join("real/link")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("real/loop")).unwrap();
        let r = run(&dir.join("real"), "*", false);
        assert_eq!(r.found, ["inside.txt", "link", "loop"], "no far.txt, and the loop ends");
        assert!(r.complete);
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_folder_is_searched_through_its_target_under_its_own_path() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        tree(dir, &["target/sub/x.txt"]);
        let link = dir.join("shortcut");
        std::os::unix::fs::symlink(dir.join("target"), &link).unwrap();
        let mut c = Collect::default();
        search(&link, &m("x.txt"), false, &mut c, &AtomicBool::new(false)).unwrap();
        assert_eq!(c.found, [link.join("sub/x.txt")]);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_folders_are_skipped_and_reported() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        tree(dir, &["open/a.txt", "locked/b.txt"]);
        let locked = dir.join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::read_dir(&locked).is_ok(); // running as root
        let r = run(dir, "*.txt", false);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        if readable {
            return;
        }
        assert_eq!(r.found, ["open/a.txt"]);
        assert_eq!(r.failed, ["locked"]);
        assert!(r.complete, "the rest is still searched");
    }

    #[test]
    fn stops_when_the_listener_has_enough_or_when_asked() {
        let t = tempfile::tempdir().unwrap();
        for i in 0..20 {
            tree(t.path(), &[&format!("f{i}/x{i}.txt")]);
        }
        let mut c = Collect { limit: Some(5), ..Collect::default() };
        assert!(!search(t.path(), &m("*.txt"), false, &mut c, &AtomicBool::new(false)).unwrap());
        assert_eq!(c.found.len(), 5);

        let mut none = Collect::default();
        assert!(!search(t.path(), &m("*"), false, &mut none, &AtomicBool::new(true)).unwrap());
        assert!(none.found.is_empty());
    }

    #[test]
    fn a_missing_folder_or_a_file_is_an_error() {
        let t = tempfile::tempdir().unwrap();
        tree(t.path(), &["file.txt"]);
        let stop = AtomicBool::new(false);
        assert!(search(&t.path().join("gone"), &m("*"), false, &mut Collect::default(), &stop).is_err());
        assert!(search(&t.path().join("file.txt"), &m("*"), false, &mut Collect::default(), &stop).is_err());
    }
}
