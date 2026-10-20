//! The last [`MAX`] search patterns, newest first, shared by every search window (the Java
//! `SearchHistory`). Each change goes into the session (`search.history.<i>`); the autosave
//! writes it to disk.

use crate::session::Session;

pub const MAX: usize = 15;
const PREFIX: &str = "search.history.";

#[derive(Debug, Clone, Default)]
pub struct SearchHistory {
    items: Vec<String>,
}

impl SearchHistory {
    /// The patterns saved in `session`; blanks and duplicates are dropped.
    pub fn load(session: &Session) -> SearchHistory {
        let mut items: Vec<String> = Vec::new();
        for i in 0..MAX {
            if let Some(p) = session.get(&format!("{PREFIX}{i}")) {
                let p = p.trim();
                if !p.is_empty() && !items.iter().any(|x| x == p) {
                    items.push(p.to_string());
                }
            }
        }
        SearchHistory { items }
    }

    /// Newest first.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// Puts `pattern` first (moving it up when it was used before), keeping the newest [`MAX`].
    pub fn add(&mut self, pattern: &str, session: &Session) {
        let p = pattern.trim();
        if p.is_empty() || self.items.first().is_some_and(|f| f == p) {
            return;
        }
        self.items.retain(|x| x != p);
        self.items.insert(0, p.to_string());
        self.items.truncate(MAX);
        session.remove_prefix(PREFIX);
        for (i, item) in self.items.iter().enumerate() {
            session.put(&format!("{PREFIX}{i}"), item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn newest_first_and_a_pattern_used_again_moves_up() {
        let t = tempfile::tempdir().unwrap();
        let s = Session::load(&t.path().join("s.properties"));
        let mut h = SearchHistory::load(&s);
        assert!(h.items().is_empty());
        h.add("*.jpg", &s);
        h.add("  a*.* !*.mp3 ", &s);
        h.add("", &s);
        h.add("   ", &s);
        assert_eq!(h.items(), ["a*.* !*.mp3", "*.jpg"], "trimmed, blanks ignored");
        h.add("*.jpg", &s);
        assert_eq!(h.items(), ["*.jpg", "a*.* !*.mp3"], "no duplicates");
        h.add("*.JPG", &s);
        assert_eq!(h.items(), ["*.JPG", "*.jpg", "a*.* !*.mp3"], "as typed: case is kept apart");
    }

    #[test]
    fn keeps_the_last_15() {
        let t = tempfile::tempdir().unwrap();
        let s = Session::load(&t.path().join("s.properties"));
        let mut h = SearchHistory::load(&s);
        for i in 1..=20 {
            h.add(&format!("p{i}"), &s);
        }
        assert_eq!(h.items().len(), 15);
        assert_eq!(h.items()[0], "p20");
        assert_eq!(h.items()[14], "p6");
    }

    #[test]
    fn survives_a_restart_through_the_session() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("s.properties");
        let s = Session::load(&file);
        let mut h = SearchHistory::load(&s);
        for i in 1..=17 {
            h.add(&format!("p{i}"), &s);
        }
        h.add("p3 \"with space\"", &s);
        s.save();
        assert_eq!(h.items(), SearchHistory::load(&Session::load(&file)).items());

        // a hand-edited or damaged file: duplicates and blanks are dropped, the rest kept in order
        fs::write(&file, "search.history.0=b\nsearch.history.1=\nsearch.history.2=b\nsearch.history.3=a\n").unwrap();
        assert_eq!(SearchHistory::load(&Session::load(&file)).items(), ["b", "a"]);
    }
}
