//! The progress window's state, free of the UI toolkit (the Java `ProgressWindow` rows): one row
//! per running operation with its title, progress line, speed/time-left estimate and, while the
//! operation waits for one, a conflict question.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use fileem_core::ops::{Decision, Handle, Kind, Resolution};
use fileem_core::util::eta::{self, Eta};
use fileem_core::util::{formats, fs_util};

/// What a conflict question shows; built on the operation's thread (it reads file attributes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub question: String,
    pub source: String,
    pub existing: String,
}

impl Conflict {
    pub fn describe(source: &Path, target: &Path) -> Conflict {
        let dir = target.symlink_metadata().is_ok_and(|m| m.is_dir());
        Conflict {
            question: format!(
                "The destination already has {} named \u{201c}{}\u{201d}.",
                if dir { "a folder" } else { "a file" },
                fs_util::display_name(target)
            ),
            source: describe(source),
            existing: describe(target),
        }
    }
}

/// "12.0 KB, modified Jan 5, 2024, 3:04 PM\n/path".
fn describe(p: &Path) -> String {
    match p.symlink_metadata() {
        Ok(m) => {
            let size = if m.is_dir() { "Folder".to_string() } else { formats::size(Some(m.len())) };
            format!("{size}, modified {}\n{}", formats::date(m.modified().ok()), p.display())
        }
        Err(_) => p.display().to_string(),
    }
}

/// "Copying “a.txt” to /dst", "Moving 3 items to Trash".
pub fn title(kind: Kind, sources: &[PathBuf], target: Option<&Path>) -> String {
    let what = match sources {
        [one] => format!("\u{201c}{}\u{201d}", fs_util::display_name(one)),
        many => format!("{} items", many.len()),
    };
    let to = target.map(|t| t.display().to_string()).unwrap_or_default();
    match kind {
        Kind::Copy => format!("Copying {what} to {to}"),
        Kind::Move => format!("Moving {what} to {to}"),
        Kind::Delete => format!("Deleting {what}"),
        Kind::Trash => format!("Moving {what} to Trash"),
        Kind::Compress => format!("Compressing {what}"),
        Kind::Extract => format!("Extracting {what}"),
    }
}

/// The progress line: percentage, amount, and (once known) speed and time left. Paused
/// operations say so instead of guessing a time.
pub fn detail_text(done: u64, total: u64, bytes: bool, paused: bool, estimate: &str) -> String {
    let percent = if total == 0 { 0.0 } else { done as f64 * 100.0 / total as f64 };
    let amount = if bytes {
        format!("{} of {}", formats::size(Some(done)), formats::size(Some(total)))
    } else {
        format!("{} of {} items", formats::count(done), formats::count(total))
    };
    let text = format!("{percent:.0}%   \u{b7}   {amount}");
    if paused {
        format!("Paused   \u{b7}   {text}")
    } else if estimate.is_empty() {
        text
    } else {
        format!("{text}   \u{b7}   {estimate}")
    }
}

/// An open conflict question; dropping the sender answers Cancel.
pub struct Question {
    pub conflict: Conflict,
    pub answer: Sender<Decision>,
}

pub struct Row {
    pub id: u64,
    pub handle: Handle,
    pub title: String,
    /// Progress counts bytes (copy, move, archives) or items (delete, trash).
    pub bytes: bool,
    pub question: Option<Question>,
    /// Cancel was pressed: the buttons are off while the operation cleans up.
    pub cancelling: bool,
    eta: Eta,
    /// "40 MB/s   ·   About 1 minute left", or empty while unknown.
    pub estimate: String,
}

impl Row {
    pub fn new(id: u64, handle: Handle, title: String) -> Row {
        let bytes = !matches!(handle.kind(), Kind::Delete | Kind::Trash);
        Row { id, handle, title, bytes, question: None, cancelling: false, eta: Eta::new(), estimate: String::new() }
    }

    /// Samples the progress for the speed and time-left estimate (once a second).
    pub fn tick(&mut self, now: Duration) {
        let (done, total) = self.handle.progress();
        // waiting is not slow progress: measure afresh once the operation moves again
        if self.handle.is_paused() || self.question.is_some() || self.handle.is_done() {
            self.eta.reset();
            self.estimate.clear();
            return;
        }
        self.eta.sample(now, done);
        let left = Eta::format(self.eta.seconds_left(total));
        let speed = match self.eta.rate() {
            Some(r) if self.bytes && r > 0.0 => eta::speed(r),
            _ => String::new(),
        };
        self.estimate = if speed.is_empty() || left.is_empty() { left } else { format!("{speed}   \u{b7}   {left}") };
    }

    pub fn detail(&self) -> String {
        let (done, total) = self.handle.progress();
        detail_text(done, total, self.bytes, self.handle.is_paused(), &self.estimate)
    }

    /// 0…1, for the progress bar.
    pub fn fraction(&self) -> f32 {
        let (done, total) = self.handle.progress();
        if total == 0 {
            0.0
        } else {
            (done as f64 / total as f64) as f32
        }
    }

    pub fn toggle_pause(&mut self) {
        if self.handle.is_paused() {
            self.handle.resume();
        } else {
            self.handle.pause();
        }
    }

    pub fn cancel(&mut self) {
        self.cancelling = true;
        self.question = None; // an unanswered question counts as Cancel
        self.handle.cancel();
    }

    pub fn answer(&mut self, resolution: Resolution, apply_to_all: bool) {
        if let Some(q) = self.question.take() {
            let _ = q.answer.send(Decision { resolution, apply_to_all });
        }
    }
}

/// The window title: the single row's title, or how many operations run.
pub fn window_title(rows: &[Row]) -> String {
    match rows {
        [one] => one.title.clone(),
        many => format!("{} operations in progress", many.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fileem_core::ops::FileOperation;
    use std::fs;
    use std::sync::mpsc;

    #[test]
    fn the_detail_line_shows_amounts_and_the_estimate() {
        assert_eq!(detail_text(512, 1024, true, false, ""), "50%   ·   512 bytes of 1.00 KB");
        assert_eq!(
            detail_text(3, 4, false, false, "About 10 seconds left"),
            "75%   ·   3 of 4 items   ·   About 10 seconds left"
        );
        assert_eq!(detail_text(1, 4, false, true, "ignored"), "Paused   ·   25%   ·   1 of 4 items");
        assert_eq!(detail_text(0, 0, true, false, ""), "0%   ·   0 bytes of 0 bytes");
    }

    #[test]
    fn titles_name_one_item_or_count_several() {
        let one = [PathBuf::from("/src/a.txt")];
        let two = [PathBuf::from("/a"), PathBuf::from("/b")];
        assert_eq!(title(Kind::Copy, &one, Some(Path::new("/dst"))), "Copying \u{201c}a.txt\u{201d} to /dst");
        assert_eq!(title(Kind::Trash, &two, None), "Moving 2 items to Trash");
        assert_eq!(title(Kind::Delete, &two, None), "Deleting 2 items");
    }

    #[test]
    fn conflicts_describe_both_items() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a.txt"), tmp.path().join("b"));
        fs::write(&a, "12345").unwrap();
        fs::create_dir(&b).unwrap();
        let c = Conflict::describe(&a, &b);
        assert_eq!(c.question, "The destination already has a folder named \u{201c}b\u{201d}.");
        assert!(c.source.starts_with("5 bytes, modified ") && c.source.ends_with(&a.display().to_string()));
        assert!(c.existing.starts_with("Folder, modified "));
    }

    #[test]
    fn rows_pause_answer_and_cancel_their_operation() {
        let tmp = tempfile::tempdir().unwrap();
        let op = FileOperation::new(Kind::Delete, vec![tmp.path().join("x")], None);
        let mut row = Row::new(1, op.handle(), "Deleting".into());
        assert!(!row.bytes, "delete counts items");
        row.toggle_pause();
        assert!(row.handle.is_paused());
        row.tick(Duration::from_secs(1));
        assert_eq!(row.estimate, "", "no estimate while paused");
        row.toggle_pause();
        assert!(!row.handle.is_paused());

        let (tx, rx) = mpsc::channel();
        row.question = Some(Question { conflict: Conflict::describe(tmp.path(), tmp.path()), answer: tx });
        row.answer(Resolution::Skip, true);
        assert_eq!(rx.recv().unwrap(), Decision::for_all(Resolution::Skip));
        assert!(row.question.is_none());

        let (tx, rx) = mpsc::channel();
        row.question = Some(Question { conflict: Conflict::describe(tmp.path(), tmp.path()), answer: tx });
        row.cancel();
        assert!(rx.recv().is_err(), "an open question is dropped: the worker reads Cancel");
        assert!(row.handle.is_cancelled() && row.cancelling);
    }

    #[test]
    fn the_window_title_follows_the_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = |id| Row::new(id, FileOperation::new(Kind::Trash, vec![tmp.path().into()], None).handle(), format!("op {id}"));
        let mut rows = vec![mk(1)];
        assert_eq!(window_title(&rows), "op 1");
        rows.push(mk(2));
        assert_eq!(window_title(&rows), "2 operations in progress");
    }
}
