//! Ordered, named folder bookmarks shared by both sides (the Java `Bookmarks`). Every change is
//! written to the session file at once (`bookmarks.count`, `bookmarks.<i>.name/.path`), so
//! bookmarks survive a crash.

use std::path::{Path, PathBuf};

use crate::session::Session;
use crate::util::fs_util;

const PREFIX: &str = "bookmarks.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Default)]
pub struct Bookmarks {
    items: Vec<Bookmark>,
}

impl Bookmarks {
    /// The bookmarks saved in `session`; entries without a path, invalid paths and duplicates are
    /// skipped.
    pub fn load(session: &Session) -> Bookmarks {
        let mut b = Bookmarks::default();
        let count = session.get_int(&format!("{PREFIX}count"), 0).clamp(0, 10_000);
        for i in 0..count {
            let Some(path) = session.get(&format!("{PREFIX}{i}.path")).filter(|p| !p.trim().is_empty()) else { continue };
            if path.contains('\0') {
                continue; // not a path on any OS
            }
            let p = fs_util::normalize(Path::new(&path));
            if !b.contains(&p) {
                let name = clean_name(session.get(&format!("{PREFIX}{i}.name")).as_deref(), &p);
                b.items.push(Bookmark { name, path: p });
            }
        }
        b
    }

    /// In display order.
    pub fn items(&self) -> &[Bookmark] {
        &self.items
    }

    pub fn contains(&self, dir: &Path) -> bool {
        self.index_of(dir).is_some()
    }

    pub fn find(&self, dir: &Path) -> Option<&Bookmark> {
        self.index_of(dir).map(|i| &self.items[i])
    }

    pub fn index_of(&self, dir: &Path) -> Option<usize> {
        let p = fs_util::normalize(dir);
        self.items.iter().position(|b| b.path == p)
    }

    /// Adds `dir`, named `name` or after the folder (`None` or blank). False when it is already
    /// bookmarked.
    pub fn add(&mut self, dir: &Path, name: Option<&str>, session: &Session) -> bool {
        let p = fs_util::normalize(dir);
        if self.contains(&p) {
            return false;
        }
        self.items.push(Bookmark { name: clean_name(name, &p), path: p });
        self.save(session);
        true
    }

    pub fn remove(&mut self, dir: &Path, session: &Session) {
        if let Some(i) = self.index_of(dir) {
            self.items.remove(i);
            self.save(session);
        }
    }

    /// Renames a bookmark; a blank name falls back to the folder name.
    pub fn rename(&mut self, dir: &Path, name: &str, session: &Session) {
        if let Some(i) = self.index_of(dir) {
            self.items[i].name = clean_name(Some(name), &self.items[i].path);
            self.save(session);
        }
    }

    /// After a folder was renamed or moved: bookmarks of it, or of folders inside it, follow
    /// along. A bookmark named after the renamed folder takes the new name; custom names stay.
    pub fn relocate(&mut self, from: &Path, to: &Path, session: &Session) {
        let (f, t) = (fs_util::normalize(from), fs_util::normalize(to));
        let mut changed = false;
        for b in &mut self.items {
            let Ok(rest) = b.path.strip_prefix(&f) else { continue };
            let moved = if rest.as_os_str().is_empty() { t.clone() } else { t.join(rest) };
            if b.path == f && b.name == fs_util::display_name(&f) {
                b.name = fs_util::display_name(&moved);
            }
            b.path = moved;
            changed = true;
        }
        if changed {
            self.save(session);
        }
    }

    /// Moves a bookmark `delta` places up (negative) or down, clamped to the list.
    pub fn move_by(&mut self, dir: &Path, delta: isize, session: &Session) {
        let Some(from) = self.index_of(dir) else { return };
        let to = (from as isize + delta).clamp(0, self.items.len() as isize - 1) as usize;
        if to != from {
            let b = self.items.remove(from);
            self.items.insert(to, b);
            self.save(session);
        }
    }

    fn save(&self, session: &Session) {
        session.remove_prefix(PREFIX);
        session.put(&format!("{PREFIX}count"), self.items.len());
        for (i, b) in self.items.iter().enumerate() {
            session.put(&format!("{PREFIX}{i}.name"), &b.name);
            session.put(&format!("{PREFIX}{i}.path"), b.path.display());
        }
        session.save();
    }
}

fn clean_name(name: Option<&str>, path: &Path) -> String {
    match name.map(str::trim) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => fs_util::display_name(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(b: &Bookmarks) -> Vec<&str> {
        b.items().iter().map(|b| b.name.as_str()).collect()
    }

    fn setup() -> (tempfile::TempDir, Session) {
        let t = tempfile::tempdir().unwrap();
        let s = Session::load(&t.path().join("session.properties"));
        (t, s)
    }

    #[test]
    fn add_uses_the_folder_name_and_ignores_duplicates() {
        let (t, s) = setup();
        let dir = t.path();
        let mut b = Bookmarks::load(&s);
        assert!(b.add(&dir.join("Projects"), None, &s));
        assert!(!b.add(&dir.join("Projects"), None, &s), "same folder twice");
        assert!(!b.add(&dir.join("x/../Projects"), None, &s), "same folder, different spelling");
        assert_eq!(names(&b), ["Projects"]);
        assert!(b.contains(&dir.join("./Projects")));
    }

    #[test]
    fn custom_names_are_trimmed_and_blank_falls_back_to_the_folder_name() {
        let (t, s) = setup();
        let dir = t.path();
        let mut b = Bookmarks::load(&s);
        b.add(&dir.join("a"), Some("  Work  "), &s);
        b.add(&dir.join("b"), Some("   "), &s);
        assert_eq!(names(&b), ["Work", "b"]);
        b.rename(&dir.join("a"), "Renamed", &s);
        b.rename(&dir.join("b"), "", &s);
        assert_eq!(names(&b), ["Renamed", "b"]);
        b.rename(&dir.join("unknown"), "ignored", &s);
        assert_eq!(b.items().len(), 2);
    }

    #[test]
    fn remove_and_move() {
        let (t, s) = setup();
        let dir = t.path();
        let mut b = Bookmarks::load(&s);
        for n in ["a", "b", "c", "d"] {
            b.add(&dir.join(n), None, &s);
        }
        b.move_by(&dir.join("c"), -1, &s);
        assert_eq!(names(&b), ["a", "c", "b", "d"]);
        b.move_by(&dir.join("a"), 10, &s);
        assert_eq!(names(&b), ["c", "b", "d", "a"], "moves are clamped to the end");
        b.move_by(&dir.join("b"), -10, &s);
        assert_eq!(names(&b), ["b", "c", "d", "a"], "moves are clamped to the start");
        b.remove(&dir.join("d"), &s);
        b.remove(&dir.join("not-there"), &s);
        assert_eq!(names(&b), ["b", "c", "a"]);
    }

    #[test]
    fn every_change_is_saved_immediately_and_restored_in_order() {
        let (t, s) = setup();
        let dir = t.path();
        let file = dir.join("session.properties");
        let mut b = Bookmarks::load(&s);
        b.add(&dir.join("one"), Some("First"), &s);
        b.add(&dir.join("two"), None, &s);
        b.move_by(&dir.join("two"), -1, &s);
        assert!(file.is_file(), "written without an explicit save");

        let s2 = Session::load(&file);
        let mut restored = Bookmarks::load(&s2);
        assert_eq!(names(&restored), ["two", "First"]);
        assert_eq!(restored.items()[0].path, dir.join("two"));

        restored.remove(&dir.join("two"), &s2);
        assert_eq!(names(&Bookmarks::load(&Session::load(&file))), ["First"]);
    }

    #[test]
    fn broken_entries_in_the_session_are_skipped() {
        let (t, s) = setup();
        let dir = t.path();
        s.put("bookmarks.count", 4);
        s.put("bookmarks.0.path", dir.join("ok").display());
        s.put("bookmarks.1.name", "no path");
        s.put("bookmarks.2.path", dir.join("ok").display()); // duplicate
        s.put("bookmarks.3.path", "\0invalid");
        assert_eq!(names(&Bookmarks::load(&s)), ["ok"]);
    }

    #[test]
    fn relocate_follows_renamed_folders_and_keeps_custom_names() {
        let (t, s) = setup();
        let dir = t.path();
        let mut b = Bookmarks::load(&s);
        b.add(&dir.join("old"), None, &s);
        b.add(&dir.join("old/inner"), Some("Custom"), &s);
        b.add(&dir.join("other"), None, &s);
        b.add(&dir.join("older"), None, &s);
        b.relocate(&dir.join("old"), &dir.join("new"), &s);
        assert_eq!(names(&b), ["new", "Custom", "other", "older"], "default names follow, custom names stay");
        assert_eq!(b.items()[1].path, dir.join("new/inner"));
        assert_eq!(b.items()[3].path, dir.join("older"), "a name that only starts the same isn't inside");
        assert!(!b.contains(&dir.join("old")));
        assert_eq!(names(&Bookmarks::load(&Session::load(&dir.join("session.properties")))), names(&b), "saved");
    }
}
