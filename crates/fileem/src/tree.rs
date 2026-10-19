//! The folder tree's state, free of the UI toolkit: sections, lazily loaded folders, expansion,
//! the current folder and the flattened rows the list view draws.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use fileem_core::fs::places::Place;
use fileem_core::fs::FileEntry;
use fileem_core::util::natural_order;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Header,
    Quick,
    Volume,
    Folder,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub name: String,
    pub path: Option<PathBuf>,
    pub hidden: bool,
    pub depth: usize,
    /// `None` until loaded.
    pub children: Option<Vec<usize>>,
    pub expanded: bool,
}

/// What the tree needs from the caller to continue revealing a folder.
#[derive(Debug, PartialEq, Eq)]
pub enum Reveal {
    /// Load this node's children, then call `reveal` again.
    Load(usize, PathBuf),
    /// The folder is shown (and current) at this row.
    Done(usize),
    /// The folder isn't under any volume (or a part of its path doesn't exist).
    NotFound,
}

pub struct Tree {
    nodes: Vec<Node>,
    roots: Vec<usize>,
    /// Visible node ids, top to bottom.
    rows: Vec<usize>,
    current: Option<usize>,
    show_hidden: bool,
    /// Branches `reveal` expanded: folded again once the current folder leaves them.
    auto_expanded: HashSet<usize>,
}

impl Tree {
    pub fn new(quick: &[Place], volumes: &[Place]) -> Tree {
        let mut t = Tree {
            nodes: Vec::new(),
            roots: Vec::new(),
            rows: Vec::new(),
            current: None,
            show_hidden: false,
            auto_expanded: HashSet::new(),
        };
        for (title, kind, places) in
            [("Quick access", NodeKind::Quick, quick), ("This computer", NodeKind::Volume, volumes)]
        {
            if places.is_empty() {
                continue;
            }
            let header = t.add(Node {
                kind: NodeKind::Header,
                name: title.into(),
                path: None,
                hidden: false,
                depth: 0,
                children: Some(Vec::new()),
                expanded: true,
            });
            t.roots.push(header);
            for p in places {
                let id = t.add(Node {
                    kind,
                    name: p.name.clone(),
                    path: Some(p.path.clone()),
                    hidden: false,
                    depth: 0,
                    children: None,
                    expanded: false,
                });
                t.nodes[header].children.as_mut().unwrap().push(id);
            }
        }
        t.flatten();
        t
    }

    fn add(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    pub fn node(&self, id: usize) -> &Node {
        &self.nodes[id]
    }

    pub fn rows(&self) -> &[usize] {
        &self.rows
    }

    pub fn row_of(&self, id: usize) -> Option<usize> {
        self.rows.iter().position(|&r| r == id)
    }

    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// Whether `id` is the current folder or one of its parents (drawn bold).
    pub fn on_path(&self, id: usize) -> bool {
        let Some(cur) = self.current.and_then(|c| self.nodes[c].path.clone()) else {
            return false;
        };
        let node = &self.nodes[id];
        node.kind != NodeKind::Header && node.path.as_deref().is_some_and(|p| cur.starts_with(p))
    }

    /// Whether the row can be expanded (folders whose children aren't known count as expandable).
    pub fn expandable(&self, id: usize) -> bool {
        let n = &self.nodes[id];
        n.kind != NodeKind::Header
            && match &n.children {
                None => true,
                Some(c) => c.iter().any(|&k| self.show_hidden || !self.nodes[k].hidden),
            }
    }

    pub fn set_show_hidden(&mut self, show: bool) {
        self.show_hidden = show;
        self.flatten();
    }

    /// Expands or collapses the node at `row`. Returns the node to load when its children aren't
    /// known yet. A branch the user toggles is theirs: reveal won't fold it away.
    pub fn toggle(&mut self, row: usize) -> Option<(usize, PathBuf)> {
        let id = *self.rows.get(row)?;
        if self.nodes[id].kind == NodeKind::Header {
            return None;
        }
        self.auto_expanded.remove(&id);
        let n = &mut self.nodes[id];
        n.expanded = !n.expanded;
        let load = if n.expanded && n.children.is_none() { n.path.clone().map(|p| (id, p)) } else { None };
        self.flatten();
        load
    }

    /// Replaces a node's children with `folders`, keeping existing child nodes (and their
    /// expansion) by path, so a reload doesn't collapse anything.
    pub fn set_children(&mut self, id: usize, folders: Vec<FileEntry>) {
        let depth = self.nodes[id].depth + 1;
        let old: Vec<usize> = self.nodes[id].children.clone().unwrap_or_default();
        let mut children = Vec::with_capacity(folders.len());
        for f in folders {
            let existing = old.iter().copied().find(|&k| self.nodes[k].path.as_deref() == Some(f.path()));
            let child = match existing {
                Some(k) => k,
                None => self.add(Node {
                    kind: NodeKind::Folder,
                    name: f.name().to_string(),
                    path: Some(f.path().to_path_buf()),
                    hidden: f.is_hidden(),
                    depth,
                    children: None,
                    expanded: false,
                }),
            };
            children.push(child);
        }
        children.sort_by(|&a, &b| natural_order::compare(&self.nodes[a].name, &self.nodes[b].name));
        self.nodes[id].children = Some(children);
        self.flatten();
    }

    /// Shows `dir` in the volumes section: expands its parents, loading what isn't loaded yet
    /// (one [`Reveal::Load`] per step), and makes it the current folder. Branches opened only for
    /// this are folded again when the current folder later leaves them.
    pub fn reveal(&mut self, dir: &Path) -> Reveal {
        let Some(mut id) = self.volume_for(dir) else {
            self.set_current(None);
            return Reveal::NotFound;
        };
        let mut opened = Vec::new();
        loop {
            let node_path = self.nodes[id].path.clone().unwrap_or_default();
            if node_path == dir {
                break;
            }
            let Some(children) = self.nodes[id].children.clone() else {
                return Reveal::Load(id, node_path);
            };
            if !self.nodes[id].expanded {
                self.nodes[id].expanded = true;
                opened.push(id);
            }
            match children.into_iter().find(|&k| self.nodes[k].path.as_deref().is_some_and(|p| dir.starts_with(p))) {
                Some(k) => id = k,
                None => {
                    self.set_current(None);
                    self.flatten();
                    return Reveal::NotFound;
                }
            }
        }
        self.auto_expanded.extend(opened);
        self.set_current(Some(id));
        self.flatten();
        Reveal::Done(self.row_of(id).unwrap_or(0))
    }

    fn set_current(&mut self, id: Option<usize>) {
        self.current = id;
        // fold the branches reveal opened that no longer lead to the current folder
        let cur = id.and_then(|c| self.nodes[c].path.clone());
        let leave: Vec<usize> = self
            .auto_expanded
            .iter()
            .copied()
            .filter(|&k| match (&cur, &self.nodes[k].path) {
                (Some(c), Some(p)) => !c.starts_with(p),
                _ => true,
            })
            .collect();
        for k in leave {
            self.nodes[k].expanded = false;
            self.auto_expanded.remove(&k);
        }
    }

    /// The volume with the longest path that contains `dir`.
    fn volume_for(&self, dir: &Path) -> Option<usize> {
        self.roots
            .iter()
            .flat_map(|&h| self.nodes[h].children.clone().unwrap_or_default())
            .filter(|&k| self.nodes[k].kind == NodeKind::Volume)
            .filter(|&k| self.nodes[k].path.as_deref().is_some_and(|p| dir.starts_with(p)))
            .max_by_key(|&k| self.nodes[k].path.as_ref().map(|p| p.components().count()))
    }

    fn flatten(&mut self) {
        let mut rows = Vec::new();
        for &r in &self.roots {
            self.push_rows(r, &mut rows);
        }
        self.rows = rows;
    }

    fn push_rows(&self, id: usize, rows: &mut Vec<usize>) {
        let n = &self.nodes[id];
        // a hidden folder still shows while you are in it (or below it)
        if n.hidden && !self.show_hidden && !self.on_path(id) {
            return;
        }
        rows.push(id);
        if n.expanded {
            for &c in n.children.as_deref().unwrap_or_default() {
                self.push_rows(c, rows);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fileem_core::fs::listing::subfolders;
    use std::fs;

    fn names(t: &Tree) -> Vec<String> {
        t.rows()
            .iter()
            .map(|&id| {
                let n = t.node(id);
                format!("{}{}", "  ".repeat(n.depth), n.name)
            })
            .collect()
    }

    /// Drives `reveal` to the end, loading children from disk like the app does in the background.
    fn reveal(t: &mut Tree, dir: &Path) -> Reveal {
        loop {
            match t.reveal(dir) {
                Reveal::Load(id, path) => t.set_children(id, subfolders(&path, true).unwrap()),
                other => return other,
            }
        }
    }

    fn setup() -> (tempfile::TempDir, Tree) {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path().join("vol");
        fs::create_dir_all(v.join("a/deep/er")).unwrap();
        fs::create_dir_all(v.join("b")).unwrap();
        fs::create_dir_all(v.join(".git")).unwrap();
        let quick = [Place { name: "Home".into(), path: tmp.path().to_path_buf() }];
        let volumes = [Place { name: "Vol".into(), path: v }];
        (tmp, Tree::new(&quick, &volumes))
    }

    #[test]
    fn sections_list_places_until_expanded() {
        let (_tmp, t) = setup();
        assert_eq!(names(&t), ["Quick access", "Home", "This computer", "Vol"]);
        assert!(t.expandable(t.rows()[1]), "unknown children count as expandable");
        assert!(!t.expandable(t.rows()[0]), "headers are not folders");
    }

    #[test]
    fn toggling_asks_to_load_once_and_hidden_folders_are_filtered() {
        let (tmp, mut t) = setup();
        let (id, path) = t.toggle(3).expect("first expand loads");
        assert_eq!(path, tmp.path().join("vol"));
        t.set_children(id, subfolders(&path, true).unwrap());
        assert_eq!(names(&t), ["Quick access", "Home", "This computer", "Vol", "  a", "  b"]);
        t.set_show_hidden(true);
        assert_eq!(names(&t)[4..], ["  .git", "  a", "  b"]);
        assert_eq!(t.toggle(3), None, "collapsing loads nothing");
        assert_eq!(names(&t).len(), 4);
        assert_eq!(t.toggle(3), None, "children are known now");
        assert_eq!(names(&t).len(), 7);
    }

    #[test]
    fn reveal_expands_the_path_marks_it_and_folds_it_again_when_leaving() {
        let (tmp, mut t) = setup();
        let deep = tmp.path().join("vol/a/deep");
        let Reveal::Done(row) = reveal(&mut t, &deep) else { panic!("revealed") };
        assert_eq!(names(&t), ["Quick access", "Home", "This computer", "Vol", "  a", "    deep", "  b"]);
        assert_eq!(t.node(t.rows()[row]).name, "deep");
        assert_eq!(t.current(), Some(t.rows()[row]));
        assert!(t.on_path(t.rows()[4]), "parents are on the path");
        assert!(!t.on_path(t.rows()[6]), "b is not");

        // a jump elsewhere folds the branches reveal opened
        let b = tmp.path().join("vol/b");
        reveal(&mut t, &b);
        assert_eq!(names(&t), ["Quick access", "Home", "This computer", "Vol", "  a", "  b"]);
    }

    #[test]
    fn a_branch_the_user_opened_stays_open() {
        let (tmp, mut t) = setup();
        reveal(&mut t, &tmp.path().join("vol/a/deep"));
        let a_row = t.row_of(t.rows().iter().copied().find(|&i| t.node(i).name == "a").unwrap()).unwrap();
        t.toggle(a_row); // the user collapses ...
        t.toggle(a_row); // ... and opens it again: now it's theirs
        reveal(&mut t, &tmp.path().join("vol/b"));
        assert!(names(&t).contains(&"    deep".to_string()));
    }

    #[test]
    fn a_hidden_current_folder_and_its_hidden_parents_still_show() {
        let (tmp, mut t) = setup();
        fs::create_dir_all(tmp.path().join("vol/.git/objects")).unwrap();
        let Reveal::Done(row) = reveal(&mut t, &tmp.path().join("vol/.git/objects")) else { panic!("revealed") };
        assert_eq!(t.node(t.rows()[row]).name, "objects");
        assert!(names(&t).contains(&"  .git".to_string()));
        reveal(&mut t, &tmp.path().join("vol/b"));
        assert!(!names(&t).contains(&"  .git".to_string()), "hidden again once we left it");
    }

    #[test]
    fn folders_outside_every_volume_are_not_found() {
        let (_tmp, mut t) = setup();
        assert_eq!(t.reveal(Path::new("/definitely/elsewhere")), Reveal::NotFound);
        assert_eq!(t.current(), None);
    }
}
