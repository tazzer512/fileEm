//! What the preview can show for a file, and the reading it needs (the Java `Preview`). No UI
//! here; everything that touches the disk is meant for a background thread. Images are decoded
//! by [`crate::images`].

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::images::Format;

/// What a file is previewed as, judged from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Image(Format),
    Text,
    /// Not known by name: it may still turn out to be text ([`read_text`]).
    Other,
}

/// Extensions that are text, so no need to look inside first.
const TEXT: [&str; 75] = [
    "txt", "text", "md", "markdown", "log", "csv", "tsv", "json", "xml", "yaml", "yml", "toml", "ini", "cfg", "conf",
    "properties", "env", "html", "htm", "css", "js", "mjs", "ts", "tsx", "jsx", "java", "kt", "kts", "groovy", "gradle",
    "scala", "py", "rb", "go", "rs", "c", "h", "cpp", "hpp", "cc", "cs", "swift", "m", "php", "pl", "lua", "sh", "bash",
    "zsh", "fish", "bat", "cmd", "ps1", "sql", "r", "tex", "srt", "vtt", "diff", "patch", "gitignore", "dockerfile",
    "makefile", "svg", "lock", "service", "desktop", "nfo", "rst", "adoc", "cmake", "mk", "vue", "dart", "proto",
];

/// Text is shown up to this much; the preview says when a file is longer.
pub const MAX_TEXT_BYTES: usize = 256 * 1024;

/// Images are decoded at most this big: enough to zoom in, bounded in memory. Photoshop,
/// JPEG 2000 and Netpbm stop at half that (their decoding costs more; Java did the same).
pub fn image_side(format: Format) -> u32 {
    match format {
        Format::Common | Format::Raw => 4096,
        Format::Psd | Format::Jpeg2000 | Format::Netpbm => 2048,
    }
}

/// The kind judged from the name alone (`extension` in any case).
pub fn kind_of(extension: &str, directory: bool) -> Kind {
    if directory {
        return Kind::Folder;
    }
    if let Some(f) = Format::of(extension) {
        return Kind::Image(f);
    }
    if TEXT.contains(&extension.to_ascii_lowercase().as_str()) { Kind::Text } else { Kind::Other }
}

/// Text read for the preview, and whether the file is longer than what was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    pub content: String,
    pub truncated: bool,
}

/// Reads the start of a regular file (links followed) as text: UTF-8, else Latin-1. `None` when
/// it looks binary (NUL bytes or many control characters) or isn't a regular file. Reads at most
/// `max_bytes`.
pub fn read_text(path: &Path, max_bytes: usize) -> io::Result<Option<Text>> {
    if !std::fs::metadata(path)?.is_file() {
        return Ok(None);
    }
    let mut data = Vec::new();
    let mut f = File::open(path)?.take(max_bytes as u64 + 1);
    f.read_to_end(&mut data)?;
    let truncated = data.len() > max_bytes;
    data.truncate(max_bytes);
    if looks_binary(&data) {
        return Ok(None);
    }
    Ok(Some(Text { content: decode(&data, truncated), truncated }))
}

/// NUL bytes, or more than 5 % control characters (other than tab, newline, CR, form feed,
/// escape) in the first 8 KB.
fn looks_binary(data: &[u8]) -> bool {
    let head = &data[..data.len().min(8192)];
    if head.contains(&0) {
        return true;
    }
    let control = head.iter().filter(|&&b| b < 32 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c | 27)).count();
    !head.is_empty() && control * 20 > head.len()
}

/// UTF-8 (tolerating a character cut off by the read limit), else Latin-1; a byte order mark
/// is dropped.
fn decode(data: &[u8], truncated: bool) -> String {
    let data = data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(data);
    let mut end = data.len();
    if truncated {
        // a multi-byte character split by the limit mustn't spoil the whole decode
        let back = data.iter().rev().take(3).take_while(|&&b| b & 0xc0 == 0x80).count();
        if end > back && data[end - back - 1] & 0xc0 == 0xc0 {
            end -= back + 1;
        }
    }
    match std::str::from_utf8(&data[..end]) {
        Ok(s) => s.to_string(),
        Err(_) => data.iter().map(|&b| char::from(b)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn kinds_by_name() {
        assert_eq!(kind_of("", true), Kind::Folder);
        assert_eq!(kind_of("png", false), Kind::Image(Format::Common));
        assert_eq!(kind_of("JPEG", false), Kind::Image(Format::Common));
        assert_eq!(kind_of("psd", false), Kind::Image(Format::Psd));
        assert_eq!(kind_of("cr2", false), Kind::Image(Format::Raw));
        assert_eq!(kind_of("jp2", false), Kind::Image(Format::Jpeg2000));
        assert_eq!(kind_of("java", false), Kind::Text);
        assert_eq!(kind_of("MD", false), Kind::Text);
        assert_eq!(kind_of("zip", false), Kind::Other);
        assert_eq!(kind_of("", false), Kind::Other);
        assert_eq!(image_side(Format::Common), 4096);
        assert_eq!(image_side(Format::Psd), 2048);
    }

    #[test]
    fn reads_text_and_refuses_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        fs::write(d.join("a.txt"), "hello\nwörld\n").unwrap();
        let t = read_text(&d.join("a.txt"), 1000).unwrap().unwrap();
        assert_eq!(t, Text { content: "hello\nwörld\n".into(), truncated: false });

        fs::write(d.join("b.bin"), [1, 2, 0, 4, 5]).unwrap();
        assert_eq!(read_text(&d.join("b.bin"), 1000).unwrap(), None, "NUL bytes: binary");
        fs::write(d.join("c.bin"), [3u8; 100]).unwrap();
        assert_eq!(read_text(&d.join("c.bin"), 1000).unwrap(), None, "control characters: binary");
        assert_eq!(read_text(d, 1000).unwrap(), None, "folders are not text");
        assert!(read_text(&d.join("missing.txt"), 1000).is_err());
    }

    #[test]
    fn long_files_are_cut_at_the_limit_without_breaking_a_character() {
        let tmp = tempfile::tempdir().unwrap();
        // "é" is two bytes in UTF-8; a limit of 5 bytes falls in the middle of the third one
        fs::write(tmp.path().join("long.txt"), "ééé").unwrap();
        let t = read_text(&tmp.path().join("long.txt"), 5).unwrap().unwrap();
        assert!(t.truncated);
        assert_eq!(t.content, "éé");
        fs::write(tmp.path().join("exact.txt"), "abcde").unwrap();
        assert!(!read_text(&tmp.path().join("exact.txt"), 5).unwrap().unwrap().truncated, "exactly the limit");
    }

    #[test]
    fn non_utf8_text_falls_back_to_latin1_and_the_bom_is_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        fs::write(d.join("latin.txt"), [b'c', b'a', b'f', 0xe9]).unwrap();
        assert_eq!(read_text(&d.join("latin.txt"), 100).unwrap().unwrap().content, "café");
        fs::write(d.join("bom.txt"), [0xef, 0xbb, 0xbf, b'h', b'i']).unwrap();
        assert_eq!(read_text(&d.join("bom.txt"), 100).unwrap().unwrap().content, "hi");
        fs::write(d.join("empty.txt"), "").unwrap();
        assert!(read_text(&d.join("empty.txt"), 100).unwrap().is_some(), "empty is text");
    }
}
