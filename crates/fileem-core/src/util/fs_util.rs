//! Path helpers: names, hidden files, junctions, unique names, name validation and filters.

use std::path::{Component, Path, PathBuf};

use super::os;

/// The last path component, or the whole path for a root ("/", "C:\").
pub fn display_name(p: &Path) -> String {
    match p.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => p.to_string_lossy().into_owned(),
    }
}

/// Dot files on POSIX; the hidden attribute on Windows.
pub fn is_hidden(p: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        p.symlink_metadata().map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
    }
}

/// True for Windows directory junctions / mount points (reparse points that are not symlinks).
/// Recursive operations must not descend into them.
pub fn is_junction(p: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        p.symlink_metadata()
            .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 && !m.file_type().is_symlink())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        false
    }
}

/// Makes `p` absolute and resolves `.` and `..` lexically (links are not followed).
pub fn normalize(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut out = PathBuf::new();
    for c in abs.components() {
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

/// True when `target` equals `folder` or lies somewhere below it (by path, not by name prefix).
pub fn is_same_or_inside(target: &Path, folder: &Path) -> bool {
    normalize(target).starts_with(normalize(folder))
}

/// Whether dragged `sources` may be dropped into `target`: never into one of themselves (or a
/// folder inside one), and not when they all already are in `target`. Paths only: no file system
/// calls, so it is cheap enough for every mouse move.
pub fn can_drop(sources: &[PathBuf], target: &Path) -> bool {
    if sources.is_empty() || sources.iter().any(|s| is_same_or_inside(target, s)) {
        return false;
    }
    let target = normalize(target);
    !sources.iter().all(|s| s.parent().map(normalize).as_deref() == Some(target.as_path()))
}

/// Whether something exists at `p`, without following a final link (a broken link exists).
pub fn exists_nofollow(p: &Path) -> bool {
    p.symlink_metadata().is_ok()
}

fn is_dir_nofollow(p: &Path) -> bool {
    p.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false)
}

/// A free sibling name: "name (2).ext", "name (3).ext" ... or, with `copy_suffix`,
/// "name - Copy.ext", "name - Copy (2).ext" ...
pub fn unique_name(wanted: &Path, copy_suffix: bool) -> PathBuf {
    let name = display_name(wanted);
    let (mut base, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 && !is_dir_nofollow(wanted) => (name[..dot].to_string(), name[dot..].to_string()),
        _ => (name.clone(), String::new()),
    };
    let parent = wanted.parent().unwrap_or(Path::new(""));
    if copy_suffix {
        base.push_str(" - Copy");
        let first = parent.join(format!("{base}{ext}"));
        if !exists_nofollow(&first) {
            return first;
        }
    } else if !exists_nofollow(wanted) {
        return wanted.to_path_buf();
    }
    (2..)
        .map(|i| parent.join(format!("{base} ({i}){ext}")))
        .find(|c| !exists_nofollow(c))
        .expect("an unbounded range finds a free name")
}

/// Returns an error message, or `None` when the name is a valid file name on this OS.
pub fn validate_name(name: &str) -> Option<String> {
    validate_name_for(name, os::is_windows())
}

fn validate_name_for(name: &str, windows: bool) -> Option<String> {
    if name.trim().is_empty() {
        return Some("The name cannot be empty.".into());
    }
    if name == "." || name == ".." {
        return Some(format!("\"{name}\" is not a valid name."));
    }
    if name.contains('/') || name.contains('\0') {
        return Some("A name cannot contain \"/\".".into());
    }
    if windows {
        if name.chars().any(|c| "\\:*?\"<>|".contains(c)) {
            return Some("A name cannot contain any of these characters: \\ / : * ? \" < > |".into());
        }
        if name.ends_with('.') || name.ends_with(' ') {
            return Some("A name cannot end with a dot or a space.".into());
        }
        if name.chars().any(|c| (c as u32) < 32) {
            return Some("A name cannot contain control characters.".into());
        }
        let stem = name.split('.').next().unwrap_or(name);
        if is_windows_reserved(stem.trim()) {
            return Some(format!("\"{stem}\" is reserved by Windows and cannot be used as a name."));
        }
    }
    None
}

/// Device names Windows reserves, with or without an extension ("NUL", "con.txt", "COM1").
fn is_windows_reserved(stem: &str) -> bool {
    let upper = stem.to_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let rest: Vec<char> = chars.collect();
    (prefix == "COM" || prefix == "LPT") && rest.len() == 1 && (rest[0].is_ascii_digit() || "¹²³".contains(rest[0]))
}

/// Filter text to a name matcher: plain text matches as "contains" (case-insensitive), `*` and `?`
/// act as wildcards over the whole name.
pub fn name_matcher(filter: &str) -> Box<dyn Fn(&str) -> bool + Send + Sync> {
    let f = filter.trim();
    if f.is_empty() {
        return Box::new(|_| true);
    }
    let lower = f.to_lowercase();
    if !f.contains(['*', '?']) {
        return Box::new(move |n| n.to_lowercase().contains(&lower));
    }
    let pattern: Vec<char> = lower.chars().collect();
    Box::new(move |n| {
        let name: Vec<char> = n.to_lowercase().chars().collect();
        wildcard_match(&pattern, &name)
    })
}

/// Glob match with `*` (any run) and `?` (one character); linear backtracking on the last star.
pub fn wildcard_match(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || (pattern[p] != '*' && pattern[p] == name[n])) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((sp, sn)) = star {
            p = sp + 1;
            n = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn unique_name_returns_wanted_name_when_free() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        assert_eq!(unique_name(&a, false), a);
    }

    #[test]
    fn unique_name_numbers_before_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "").unwrap();
        fs::write(dir.path().join("a (2).txt"), "").unwrap();
        assert_eq!(unique_name(&dir.path().join("a.txt"), false), dir.path().join("a (3).txt"));
    }

    #[test]
    fn unique_name_copy_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        fs::write(&a, "").unwrap();
        assert_eq!(unique_name(&a, true), dir.path().join("a - Copy.txt"));
        fs::write(dir.path().join("a - Copy.txt"), "").unwrap();
        assert_eq!(unique_name(&a, true), dir.path().join("a - Copy (2).txt"));
    }

    #[test]
    fn unique_name_keeps_dots_in_folder_names() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("v1.2")).unwrap();
        assert_eq!(unique_name(&dir.path().join("v1.2"), false), dir.path().join("v1.2 (2)"));
    }

    #[test]
    fn unique_name_dot_files_are_not_split_at_the_leading_dot() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".bashrc"), "").unwrap();
        assert_eq!(unique_name(&dir.path().join(".bashrc"), false), dir.path().join(".bashrc (2)"));
    }

    #[test]
    fn validate_name_rejects_empty_dots_and_slashes() {
        for bad in ["", "   ", ".", "..", "a/b", "nul\0byte"] {
            assert!(validate_name(bad).is_some(), "{bad:?}");
        }
        assert_eq!(validate_name("normal name.txt"), None);
        assert_eq!(validate_name(".hidden"), None);
    }

    #[test]
    fn validate_name_rejects_windows_reserved_characters_on_windows() {
        // the Windows rules are plain string checks, so they are tested on every platform
        for bad in ["a:b", "a?", "trailing.", "trailing ", "tab\there"] {
            assert!(validate_name_for(bad, true).is_some(), "{bad:?}");
            if !bad.contains(['.', ' ', '\t']) {
                assert!(validate_name_for(bad, false).is_none(), "{bad:?} is fine on POSIX");
            }
        }
    }

    #[test]
    fn validate_name_rejects_windows_device_names_on_windows() {
        for reserved in ["CON", "nul", "Aux.txt", "COM1", "lpt9.log", "PRN.tar.gz", "COM\u{00b9}"] {
            assert!(validate_name_for(reserved, true).is_some(), "{reserved}");
        }
        for fine in ["CONSOLE.txt", "com10", "null"] {
            assert_eq!(validate_name_for(fine, true), None, "{fine}");
        }
    }

    #[test]
    fn name_matcher_plain_text_is_case_insensitive_contains() {
        let m = name_matcher("Rep");
        assert!(m("report.pdf"));
        assert!(m("MY_REPORT"));
        assert!(!m("other"));
        assert!(name_matcher("")("anything"));
        assert!(name_matcher("   ")("anything"));
    }

    #[test]
    fn name_matcher_wildcards_match_the_whole_name() {
        let m = name_matcher("*.TXT");
        assert!(m("a.txt"));
        assert!(!m("a.txt.bak"));
        let q = name_matcher("file?.log");
        assert!(q("file1.log"));
        assert!(!q("file12.log"));
        assert!(name_matcher("a+b(*")("a+b(1)"), "regex characters are literal");
        assert!(name_matcher("*a*b*c")("xxaxxbxxc"));
        assert!(!name_matcher("*a*b*c")("xxaxxcxxb"));
    }

    #[test]
    fn same_or_inside() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert!(is_same_or_inside(&d.join("a/b"), &d.join("a")));
        assert!(is_same_or_inside(&d.join("a"), &d.join("a")));
        assert!(is_same_or_inside(&d.join("a/x/../b"), &d.join("a")));
        assert!(!is_same_or_inside(&d.join("ab"), &d.join("a")), "prefix of name is not inside");
        assert!(!is_same_or_inside(&d.join("a"), &d.join("a/b")));
    }

    #[test]
    fn drops_are_refused_into_the_sources_themselves_and_where_they_already_are() {
        let d = Path::new("/data");
        let (a, b) = (d.join("a"), d.join("b"));
        let just_a = std::slice::from_ref(&a);
        assert!(can_drop(just_a, &b), "a sibling folder");
        assert!(!can_drop(just_a, &a), "onto itself");
        assert!(!can_drop(just_a, &a.join("x/y")), "into a folder inside it");
        assert!(!can_drop(&[a.clone(), d.join("f.txt")], d), "all already in the target");
        assert!(can_drop(&[a.clone(), b.join("f.txt")], d), "one of them comes from elsewhere");
        assert!(can_drop(&[d.join("ab/f")], &d.join("a")), "a name prefix is not inside");
        assert!(!can_drop(&[], d), "nothing dragged");
    }

    #[cfg(unix)]
    #[test]
    fn hidden_follows_platform_rules() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_hidden(&dir.path().join(".x")));
        assert!(!is_hidden(&dir.path().join("x")));
        assert!(!is_junction(dir.path()), "junctions only exist on Windows");
    }

    #[test]
    fn display_name_of_root_is_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().ancestors().last().unwrap();
        assert_eq!(display_name(root), root.to_string_lossy());
        assert_eq!(display_name(&dir.path().join("x")), "x");
    }
}
