//! Window layout, open tabs and other settings, persisted as a Java `.properties` file in the OS
//! config dir: the same file and format the Java app uses, so both versions share settings.
//! Writes go to a temp file that is renamed into place, so a crash mid-write never leaves a broken
//! session, and unchanged contents are not written again.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::util::os;

type Props = BTreeMap<String, String>;

pub struct Session {
    file: PathBuf,
    inner: Arc<Inner>,
}

/// The contents last handed to a write, numbered: a write only goes ahead while its number is
/// still the newest, so an older background write never replaces a newer save.
#[derive(Default)]
struct Last {
    n: u64,
    /// `None` after a failed write, so the next save writes even if nothing changed.
    saved: Option<Props>,
}

struct Inner {
    props: Mutex<Props>,
    last: Mutex<Last>,
    /// Serializes the actual file writes.
    writing: Mutex<()>,
}

impl Session {
    /// The user's session (`<config dir>/session.properties`).
    pub fn load_default() -> Session {
        Session::load(&os::config_dir().join("session.properties"))
    }

    /// Loads (or starts empty) a session stored at `file`. An unreadable or malformed file starts
    /// empty instead of failing.
    pub fn load(file: &Path) -> Session {
        let props = match fs::read_to_string(file) {
            Ok(text) => parse(&text).unwrap_or_else(|e| {
                eprintln!("Could not read session: {e}");
                Props::new()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Props::new(),
            Err(e) => {
                eprintln!("Could not read session: {e}");
                Props::new()
            }
        };
        Session {
            file: file.to_path_buf(),
            inner: Arc::new(Inner {
                props: Mutex::new(props),
                last: Mutex::new(Last::default()),
                writing: Mutex::new(()),
            }),
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.inner.props.lock().unwrap().get(key).cloned()
    }

    pub fn get_or(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or_else(|| default.to_string())
    }

    pub fn get_int(&self, key: &str, default: i64) -> i64 {
        self.get(key).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
    }

    pub fn get_f64(&self, key: &str, default: f64) -> f64 {
        self.get(key).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
    }

    /// Like Java's `Boolean.parseBoolean`: "true" in any case is true, anything else false.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.get(key).map(|v| v.eq_ignore_ascii_case("true")).unwrap_or(default)
    }

    pub fn put(&self, key: &str, value: impl ToString) {
        self.inner.props.lock().unwrap().insert(key.to_string(), value.to_string());
    }

    pub fn remove_prefix(&self, prefix: &str) {
        self.inner.props.lock().unwrap().retain(|k, _| !k.starts_with(prefix));
    }

    /// Writes the session now if it changed since the last save.
    pub fn save(&self) {
        if let Some((n, snapshot)) = self.changed_snapshot() {
            write(&self.inner, &self.file, n, &snapshot);
        }
    }

    /// Writes the session on a background thread if it changed since the last save. Returns the
    /// pending write, or `None` when nothing changed.
    pub fn save_in_background(&self) -> Option<JoinHandle<()>> {
        let (n, snapshot) = self.changed_snapshot()?;
        let (inner, file) = (self.inner.clone(), self.file.clone());
        Some(
            std::thread::Builder::new()
                .name("fileem-session-writer".into())
                .spawn(move || write(&inner, &file, n, &snapshot))
                .expect("spawn the session writer"),
        )
    }

    fn changed_snapshot(&self) -> Option<(u64, Props)> {
        let snapshot = self.inner.props.lock().unwrap().clone();
        let mut last = self.inner.last.lock().unwrap();
        if last.saved.as_ref() == Some(&snapshot) {
            return None;
        }
        last.n += 1;
        last.saved = Some(snapshot.clone());
        Some((last.n, snapshot))
    }
}

fn write(inner: &Inner, file: &Path, n: u64, snapshot: &Props) {
    let _writing = inner.writing.lock().unwrap();
    if inner.last.lock().unwrap().n != n {
        return; // a newer snapshot was taken (and is or was written): don't overwrite it with this one
    }
    if let Err(e) = write_atomically(file, &store(snapshot)) {
        // try again next time, even if nothing changes
        let mut last = inner.last.lock().unwrap();
        if last.n == n {
            last.saved = None;
        }
        eprintln!("Could not save session: {e}");
    }
}

fn write_atomically(file: &Path, text: &str) -> io::Result<()> {
    let dir = file.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".session-{}.tmp", std::process::id()));
    let result = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, file));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

// ---- Java .properties format ----

/// Parses the `java.util.Properties` text format: `key=value`, `key: value` or `key value`
/// lines, `#`/`!` comments, `\` line continuations and escapes (`\t \n \r \f \uXXXX`).
pub fn parse(text: &str) -> Result<Props, String> {
    let mut props = Props::new();
    let mut lines = text.lines();
    while let Some(first) = lines.next() {
        let trimmed = first.trim_start_matches([' ', '\t', '\u{c}']);
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
            continue;
        }
        // join continuation lines: an odd number of trailing backslashes continues the line
        let mut logical = String::from(trimmed);
        while ends_with_odd_backslashes(&logical) {
            logical.pop();
            match lines.next() {
                Some(next) => logical.push_str(next.trim_start_matches([' ', '\t', '\u{c}'])),
                None => break,
            }
        }
        let (key, value) = split_key_value(&logical);
        props.insert(unescape(key)?, unescape(value)?);
    }
    Ok(props)
}

fn ends_with_odd_backslashes(s: &str) -> bool {
    s.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1
}

/// The key ends at the first unescaped `=`, `:` or whitespace; then whitespace, one optional
/// `=`/`:`, and more whitespace precede the value.
fn split_key_value(line: &str) -> (&str, &str) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'=' | b':' | b' ' | b'\t' | b'\x0c' => break,
            _ => i += 1,
        }
    }
    let i = i.min(bytes.len());
    let key = &line[..i];
    let mut rest = line[i..].trim_start_matches([' ', '\t', '\u{c}']);
    if let Some(r) = rest.strip_prefix(['=', ':']) {
        rest = r.trim_start_matches([' ', '\t', '\u{c}']);
    }
    (key, rest)
}

fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                let code = u32::from_str_radix(&hex, 16)
                    .ok()
                    .filter(|_| hex.len() == 4)
                    .ok_or_else(|| format!("Malformed \\uxxxx encoding: \\u{hex}"))?;
                // a surrogate pair arrives as two \u escapes
                if (0xD800..0xDC00).contains(&code) {
                    let rest: String = chars.clone().take(6).collect();
                    if let Some(low) = rest.strip_prefix("\\u").and_then(|h| u32::from_str_radix(h, 16).ok()) {
                        if (0xDC00..0xE000).contains(&low) {
                            chars.nth(5);
                            let combined = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                            out.extend(char::from_u32(combined));
                            continue;
                        }
                    }
                }
                out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    Ok(out)
}

/// Writes properties like Java's `Properties.store(Writer, comment)`: escaped keys and values,
/// non-ASCII kept as UTF-8.
pub fn store(props: &Props) -> String {
    let mut out = String::from("#fileEm session\n");
    for (k, v) in props {
        escape_into(&mut out, k, true);
        out.push('=');
        escape_into(&mut out, v, false);
        out.push('\n');
    }
    out
}

fn escape_into(out: &mut String, s: &str, is_key: bool) {
    for (i, c) in s.chars().enumerate() {
        match c {
            ' ' if is_key || i == 0 => out.push_str("\\ "),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '=' | ':' | '#' | '!' => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files_in(dir: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        v.sort();
        v
    }

    #[test]
    fn round_trips_values_through_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("nested").join("session.properties");
        let s = Session::load(&file);
        s.put("text", "a b=c:ü");
        s.put("int", 42);
        s.put("double", 0.25);
        s.put("bool", true);
        s.put(" odd key:=", " leading space\tand\nnewline\\");
        s.save();

        assert!(file.is_file(), "save creates missing parent folders");
        let loaded = Session::load(&file);
        assert_eq!(loaded.get("text").as_deref(), Some("a b=c:ü"));
        assert_eq!(loaded.get_int("int", 0), 42);
        assert_eq!(loaded.get_f64("double", 0.0), 0.25);
        assert!(loaded.get_bool("bool", false));
        assert_eq!(loaded.get(" odd key:=").as_deref(), Some(" leading space\tand\nnewline\\"));
    }

    #[test]
    fn reads_what_java_writes() {
        // as written by Properties.store(Writer): escapes, \u sequences, continuation lines
        let text = "#fileEm session\n#Mon Oct 19 09:00:00 CEST 2026\nleft.tab.0=/home/u/My\\ Files\n\
                    window.w=1280.0\ntheme.name : aurora\nsearch.history.0=*.txt\\=x \\u00FC\\uD83D\\uDE00\n\
                    multi=one \\\n    two\n! comment\nempty=\nkey\\ with\\ space value\n";
        let p = parse(text).unwrap();
        assert_eq!(p["left.tab.0"], "/home/u/My Files");
        assert_eq!(p["window.w"], "1280.0");
        assert_eq!(p["theme.name"], "aurora");
        assert_eq!(p["search.history.0"], "*.txt=x ü😀");
        assert_eq!(p["multi"], "one two");
        assert_eq!(p["empty"], "");
        assert_eq!(p["key with space"], "value");
        assert_eq!(p.len(), 7);
    }

    #[test]
    fn returns_defaults_for_missing_or_malformed_values() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Session::load(&tmp.path().join("none.properties"));
        s.put("notANumber", "abc");
        assert_eq!(s.get_int("missing", 7), 7);
        assert_eq!(s.get_int("notANumber", 7), 7);
        assert_eq!(s.get_f64("notANumber", 1.5), 1.5);
        assert!(!s.get_bool("missing", false));
        assert_eq!(s.get("missing"), None);
        // Java writes doubles as "1280.0" and ints as "2": both read either way where it makes sense
        s.put("w", "1280.0");
        assert_eq!(s.get_f64("w", 0.0), 1280.0);
    }

    #[test]
    fn remove_prefix_only_removes_matching_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Session::load(&tmp.path().join("s.properties"));
        s.put("left.tab.0", "/a");
        s.put("left.tab.1", "/b");
        s.put("left.tabs", 2);
        s.remove_prefix("left.tab.");
        assert_eq!(s.get("left.tab.0"), None);
        assert_eq!(s.get("left.tab.1"), None);
        assert_eq!(s.get_int("left.tabs", 0), 2);
    }

    #[test]
    fn corrupt_file_starts_empty_instead_of_failing() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("bad.properties");
        fs::write(&file, b"k=\\uZZ").unwrap();
        assert_eq!(Session::load(&file).get_or("k", "d"), "d");
        fs::write(&file, [0xff, 0xfe, b'=', 0x80]).unwrap();
        assert_eq!(Session::load(&file).get_or("k", "d"), "d", "not UTF-8 either");
    }

    #[test]
    fn save_writes_only_when_something_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("session.properties");
        let s = Session::load(&file);
        s.put("a", 1);
        s.save();
        assert!(file.is_file());

        fs::remove_file(&file).unwrap();
        s.put("a", 1); // same value
        s.save();
        assert!(!file.exists(), "unchanged contents are not written again");
        assert!(s.save_in_background().is_none(), "nothing to write in the background either");

        s.put("a", 2);
        s.save();
        assert_eq!(Session::load(&file).get_int("a", 0), 2);
    }

    #[test]
    fn background_save_writes_the_file_and_leaves_no_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("session.properties");
        let s = Session::load(&file);
        s.put("theme.name", "aurora");
        s.save_in_background().unwrap().join().unwrap();
        assert_eq!(Session::load(&file).get("theme.name").as_deref(), Some("aurora"));
        assert_eq!(files_in(tmp.path()), [file], "the temp file was renamed into place");
    }

    #[test]
    fn an_older_background_write_never_replaces_a_newer_save() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("session.properties");
        let s = Session::load(&file);
        s.put("v", "old");
        // hold the writer so the background write surely runs after the newer save
        let blocker = s.inner.writing.lock().unwrap();
        let older = s.save_in_background().unwrap();
        s.put("v", "new");
        drop(blocker);
        s.save();
        older.join().unwrap();
        assert_eq!(Session::load(&file).get("v").as_deref(), Some("new"));
    }

    #[test]
    fn a_failed_save_is_retried_on_the_next_save() {
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("blocker");
        fs::write(&blocker, "not a folder").unwrap();
        let broken = Session::load(&blocker.join("session.properties"));
        broken.put("a", 1);
        broken.save(); // parent is a file: fails, reports, does not panic
        fs::remove_file(&blocker).unwrap();
        broken.save(); // nothing changed, but the last write failed
        assert_eq!(Session::load(&blocker.join("session.properties")).get_int("a", 0), 1);
    }
}
