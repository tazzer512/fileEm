//! Cut / copy / paste of files through the OS clipboard (the Java `FileClipboard`).
//!
//! Formats: Linux file managers exchange `x-special/gnome-copied-files` ("copy" or "cut", then
//! one `file://` URI per line) and `text/uri-list`; plain text gets the paths. Windows uses a
//! file list (CF_HDROP) plus `Preferred DropEffect` for cut; macOS file URLs. On Linux the
//! clipboard goes through X11 (XWayland on Wayland desktops, whose clipboard GNOME and KDE bridge
//! with all formats). Without any system clipboard, copy and paste still work inside fileEm.

use std::path::{Path, PathBuf};

/// Files on the clipboard, and whether they were cut (a paste moves them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipFiles {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

pub trait FileClipboard {
    fn put(&mut self, paths: &[PathBuf], cut: bool);
    /// The files on the clipboard, if any.
    fn get(&mut self) -> Option<ClipFiles>;
    /// After a cut was pasted (moved): the files are gone from where they were.
    fn clear(&mut self);
    /// Plain text ("Copy path"); replaces the files.
    fn put_text(&mut self, text: &str);
    fn text(&mut self) -> Option<String>;
}

/// The paths as text, one per line.
fn paths_text(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n")
}

/// A clipboard inside the app only (tests; desktops without a system clipboard).
#[derive(Default)]
pub struct Memory {
    files: Option<ClipFiles>,
    text: Option<String>,
}

impl FileClipboard for Memory {
    fn put(&mut self, paths: &[PathBuf], cut: bool) {
        self.files = Some(ClipFiles { paths: paths.to_vec(), cut });
        self.text = Some(paths_text(paths));
    }

    fn get(&mut self) -> Option<ClipFiles> {
        self.files.clone()
    }

    fn clear(&mut self) {
        self.files = None;
        self.text = None;
    }

    fn put_text(&mut self, text: &str) {
        self.files = None;
        self.text = Some(text.to_string());
    }

    fn text(&mut self) -> Option<String> {
        self.text.clone()
    }
}

/// The OS clipboard, connected on first use; falls back to [`Memory`] when there is none.
#[derive(Default)]
pub struct System {
    ctx: Option<Option<clipboard_rs::ClipboardContext>>,
    fallback: Memory,
}

const GNOME: &str = "x-special/gnome-copied-files";
const URI_LIST: &str = "text/uri-list";
#[cfg(windows)]
const DROP_EFFECT: &str = "Preferred DropEffect";

impl System {
    fn ctx(&mut self) -> Option<&clipboard_rs::ClipboardContext> {
        self.ctx.get_or_insert_with(|| clipboard_rs::ClipboardContext::new().ok()).as_ref()
    }
}

impl FileClipboard for System {
    fn put(&mut self, paths: &[PathBuf], cut: bool) {
        use clipboard_rs::{Clipboard, ClipboardContent};
        self.fallback.put(paths, cut);
        let mut contents = vec![ClipboardContent::Text(paths_text(paths))];
        if cfg!(any(windows, target_os = "macos")) {
            contents.push(ClipboardContent::Files(paths.iter().map(|p| p.display().to_string()).collect()));
            #[cfg(windows)]
            contents.push(ClipboardContent::Other(DROP_EFFECT.into(), (if cut { 2u32 } else { 5u32 }).to_le_bytes().to_vec()));
        } else {
            contents.push(ClipboardContent::Other(GNOME.into(), gnome_payload(paths, cut).into_bytes()));
            contents.push(ClipboardContent::Other(URI_LIST.into(), uri_list(paths).into_bytes()));
        }
        if let Some(ctx) = self.ctx() {
            let _ = ctx.set(contents);
        }
    }

    fn get(&mut self) -> Option<ClipFiles> {
        use clipboard_rs::Clipboard;
        let Some(ctx) = self.ctx() else { return self.fallback.get() };
        if cfg!(any(windows, target_os = "macos")) {
            let paths: Vec<PathBuf> = ctx.get_files().ok()?.iter().map(|f| from_uri(f).unwrap_or_else(|| f.into())).collect();
            #[cfg(windows)]
            let cut = ctx.get_buffer(DROP_EFFECT).ok().and_then(|b| b.get(..4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))).is_some_and(|e| e & 2 != 0);
            #[cfg(not(windows))]
            let cut = false;
            return (!paths.is_empty()).then_some(ClipFiles { paths, cut });
        }
        if let Some(files) = ctx.get_buffer(GNOME).ok().and_then(|b| parse_gnome(&String::from_utf8_lossy(&b))) {
            return Some(files);
        }
        let paths = parse_uri_list(&String::from_utf8_lossy(&ctx.get_buffer(URI_LIST).ok()?));
        (!paths.is_empty()).then_some(ClipFiles { paths, cut: false })
    }

    fn clear(&mut self) {
        use clipboard_rs::Clipboard;
        self.fallback.clear();
        if let Some(ctx) = self.ctx() {
            let _ = ctx.clear();
        }
    }

    fn put_text(&mut self, text: &str) {
        use clipboard_rs::Clipboard;
        self.fallback.put_text(text);
        if let Some(ctx) = self.ctx() {
            let _ = ctx.set_text(text.to_string());
        }
    }

    fn text(&mut self) -> Option<String> {
        use clipboard_rs::Clipboard;
        match self.ctx() {
            Some(ctx) => ctx.get_text().ok(),
            None => self.fallback.text(),
        }
    }
}

// ---- formats ----

/// `file:///home/u/My%20Files` (RFC 3986: unreserved characters and `/` stay as they are).
pub fn to_uri(p: &Path) -> String {
    let mut out = String::from("file://");
    let text = p.to_string_lossy();
    #[cfg(windows)]
    let text = format!("/{}", text.replace('\\', "/"));
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) || (cfg!(windows) && b == b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The path of a `file://` URI (with or without a host part), or `None` for other URIs.
pub fn from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.trim().strip_prefix("file://")?;
    // "file:///path" or "file://localhost/path"
    let path = if rest.starts_with('/') { rest } else { &rest[rest.find('/')?..] };
    let bytes = path.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let decoded = String::from_utf8(out).ok()?;
    #[cfg(windows)]
    let decoded = decoded.trim_start_matches('/').replace('/', "\\");
    Some(PathBuf::from(decoded))
}

/// "copy\nfile:///a\nfile:///b" or with "cut".
pub fn gnome_payload(paths: &[PathBuf], cut: bool) -> String {
    let mut s = String::from(if cut { "cut" } else { "copy" });
    for p in paths {
        s.push('\n');
        s.push_str(&to_uri(p));
    }
    s
}

pub fn parse_gnome(text: &str) -> Option<ClipFiles> {
    let mut lines = text.lines();
    let cut = match lines.next()?.trim() {
        "cut" => true,
        "copy" => false,
        _ => return None,
    };
    let paths: Vec<PathBuf> = lines.filter_map(from_uri).collect();
    (!paths.is_empty()).then_some(ClipFiles { paths, cut })
}

/// `text/uri-list`: one URI per line, CRLF separated.
pub fn uri_list(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| to_uri(p)).collect::<Vec<_>>().join("\r\n")
}

/// The file paths of a `text/uri-list` (comments and other URIs are skipped).
pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(from_uri).collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn uris_round_trip_paths_with_special_characters() {
        for p in ["/home/u/My Files/ü #1%.txt", "/a/b", "/x/[brackets] & 'quotes'"] {
            let uri = to_uri(Path::new(p));
            assert!(uri.starts_with("file:///"), "{uri}");
            assert!(!uri.contains(' ') && !uri.contains('#'), "{uri}");
            assert_eq!(from_uri(&uri).unwrap(), Path::new(p));
        }
        assert_eq!(to_uri(Path::new("/a b")), "file:///a%20b");
        assert_eq!(from_uri("file://localhost/etc/hosts").unwrap(), Path::new("/etc/hosts"));
        assert_eq!(from_uri("https://example.com/x"), None);
        assert_eq!(from_uri("file:///bad%2"), Some(PathBuf::from("/bad%2")), "a cut-off escape stays literal");
    }

    #[test]
    fn gnome_payloads_carry_cut_or_copy() {
        let paths = vec![PathBuf::from("/a/one.txt"), PathBuf::from("/a/two words")];
        let cut = gnome_payload(&paths, true);
        assert_eq!(cut, "cut\nfile:///a/one.txt\nfile:///a/two%20words");
        assert_eq!(parse_gnome(&cut), Some(ClipFiles { paths: paths.clone(), cut: true }));
        assert!(!parse_gnome(&gnome_payload(&paths, false)).unwrap().cut);
        assert_eq!(parse_gnome("something else\nfile:///a"), None);
        assert_eq!(parse_gnome("copy\n"), None, "no files");
    }

    #[test]
    fn uri_lists_skip_comments_and_other_uris() {
        let paths = vec![PathBuf::from("/a"), PathBuf::from("/b c")];
        assert_eq!(uri_list(&paths), "file:///a\r\nfile:///b%20c");
        assert_eq!(parse_uri_list("# comment\r\nfile:///a\r\nhttp://x/y\r\n\r\nfile:///b%20c\r\n"), paths);
    }

    #[test]
    fn the_memory_clipboard_keeps_what_was_put() {
        let mut m = Memory::default();
        assert_eq!(m.get(), None);
        m.put(&[PathBuf::from("/a")], true);
        assert_eq!(m.get(), Some(ClipFiles { paths: vec![PathBuf::from("/a")], cut: true }));
        assert_eq!(m.text().as_deref(), Some("/a"));
        m.put_text("/x\n/y");
        assert_eq!((m.get(), m.text().as_deref()), (None, Some("/x\n/y")), "text replaces the files");
        m.clear();
        assert_eq!(m.get(), None);
    }
}
