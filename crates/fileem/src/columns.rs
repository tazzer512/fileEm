//! The details view's columns (the Java `FileTable` columns): which are shown, their widths and
//! order, saved in the session as `columns.<id>.visible`, `columns.<id>.width` and
//! `columns.order` (same format as Java). Name always comes first here; the others are dragged
//! into any order.

use fileem_core::fs::listing::SortKey;
use fileem_core::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnId {
    Name,
    Size,
    Type,
    Modified,
    Created,
    Accessed,
    Extension,
    Attributes,
}

impl ColumnId {
    pub const ALL: [ColumnId; 8] = [
        ColumnId::Name,
        ColumnId::Size,
        ColumnId::Type,
        ColumnId::Modified,
        ColumnId::Created,
        ColumnId::Accessed,
        ColumnId::Extension,
        ColumnId::Attributes,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ColumnId::Name => "name",
            ColumnId::Size => "size",
            ColumnId::Type => "type",
            ColumnId::Modified => "modified",
            ColumnId::Created => "created",
            ColumnId::Accessed => "accessed",
            ColumnId::Extension => "extension",
            ColumnId::Attributes => "attributes",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ColumnId::Name => "Name",
            ColumnId::Size => "Size",
            ColumnId::Type => "Type",
            ColumnId::Modified => "Date modified",
            ColumnId::Created => "Date created",
            ColumnId::Accessed => "Date accessed",
            ColumnId::Extension => "Extension",
            ColumnId::Attributes => "Attributes",
        }
    }

    fn default_width(self) -> f32 {
        match self {
            ColumnId::Name => 300.0,
            ColumnId::Size => 95.0,
            ColumnId::Type => 140.0,
            ColumnId::Modified | ColumnId::Created | ColumnId::Accessed => 165.0,
            ColumnId::Extension => 80.0,
            ColumnId::Attributes => 160.0,
        }
    }

    fn visible_by_default(self) -> bool {
        matches!(self, ColumnId::Name | ColumnId::Size | ColumnId::Type | ColumnId::Modified)
    }

    /// The sort key of this column; Date accessed and Attributes don't sort.
    pub fn sort_key(self) -> Option<SortKey> {
        SortKey::from_id(self.id())
    }

    pub fn from_index(i: usize) -> Option<ColumnId> {
        ColumnId::ALL.get(i).copied()
    }

    pub fn index(self) -> usize {
        ColumnId::ALL.iter().position(|&c| c == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Column {
    pub id: ColumnId,
    pub width: f32,
    pub visible: bool,
}

pub const MIN_WIDTH: f32 = 40.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Columns {
    /// All columns in display order.
    cols: Vec<Column>,
}

impl Default for Columns {
    fn default() -> Self {
        Columns {
            cols: ColumnId::ALL
                .iter()
                .map(|&id| Column { id, width: id.default_width(), visible: id.visible_by_default() })
                .collect(),
        }
    }
}

impl Columns {
    pub fn restore(s: &Session) -> Columns {
        let mut c = Columns::default();
        if let Some(order) = s.get("columns.order") {
            let mut ordered: Vec<Column> = Vec::new();
            for id in order.split(',') {
                if let Some(at) = c.cols.iter().position(|col| col.id.id() == id.trim()) {
                    ordered.push(c.cols.remove(at));
                }
            }
            ordered.append(&mut c.cols);
            c.cols = ordered;
        }
        for col in &mut c.cols {
            let id = col.id.id();
            col.width = (s.get_f64(&format!("columns.{id}.width"), col.width as f64) as f32).max(MIN_WIDTH);
            col.visible = s.get_bool(&format!("columns.{id}.visible"), col.visible);
        }
        c.name_always_visible();
        c
    }

    pub fn save(&self, s: &Session) {
        s.put("columns.order", self.cols.iter().map(|c| c.id.id()).collect::<Vec<_>>().join(","));
        for col in &self.cols {
            let id = col.id.id();
            s.put(&format!("columns.{id}.width"), col.width.round() as i64);
            s.put(&format!("columns.{id}.visible"), col.visible);
        }
    }

    fn name_always_visible(&mut self) {
        for col in &mut self.cols {
            if col.id == ColumnId::Name {
                col.visible = true;
            }
        }
    }

    /// The visible columns in order.
    pub fn visible(&self) -> impl Iterator<Item = &Column> {
        self.cols.iter().filter(|c| c.visible)
    }

    pub fn all(&self) -> &[Column] {
        &self.cols
    }

    pub fn is_visible(&self, id: ColumnId) -> bool {
        self.cols.iter().any(|c| c.id == id && c.visible)
    }

    /// Shows or hides a column; Name always stays.
    pub fn toggle(&mut self, id: ColumnId) {
        if id == ColumnId::Name {
            return;
        }
        if let Some(c) = self.cols.iter_mut().find(|c| c.id == id) {
            c.visible = !c.visible;
        }
    }

    pub fn show(&mut self, id: ColumnId) {
        if let Some(c) = self.cols.iter_mut().find(|c| c.id == id) {
            c.visible = true;
        }
    }

    /// The visible columns after Name, as the header shows them.
    pub fn shown_after_name(&self) -> Vec<ColumnId> {
        self.visible().map(|c| c.id).filter(|&id| id != ColumnId::Name).collect()
    }

    /// Moves `id` into `gap` among [`Columns::shown_after_name`] (0: right after Name); hidden
    /// columns keep their places. Returns whether anything moved.
    pub fn move_to_gap(&mut self, id: ColumnId, gap: usize) -> bool {
        let mut shown = self.shown_after_name();
        let Some(from) = shown.iter().position(|&c| c == id) else { return false };
        let Some(to) = crate::tab::reordered(from, gap.min(shown.len())) else { return false };
        shown.remove(from);
        shown.insert(to, id);
        let at = self.cols.iter().position(|c| c.id == id).unwrap();
        let col = self.cols.remove(at);
        let index_of = |cols: &[Column], id: ColumnId| cols.iter().position(|c| c.id == id).unwrap();
        let at = match (shown.get(to + 1), to.checked_sub(1).map(|i| shown[i])) {
            (Some(&next), _) => index_of(&self.cols, next),
            (None, Some(prev)) => index_of(&self.cols, prev) + 1,
            (None, None) => self.cols.len(),
        };
        self.cols.insert(at, col);
        true
    }

    pub fn set_width(&mut self, id: ColumnId, width: f32) {
        if let Some(c) = self.cols.iter_mut().find(|c| c.id == id) {
            c.width = width.max(MIN_WIDTH);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(c: &Columns) -> Vec<&'static str> {
        c.visible().map(|c| c.id.id()).collect()
    }

    #[test]
    fn defaults_show_the_four_explorer_columns() {
        assert_eq!(ids(&Columns::default()), ["name", "size", "type", "modified"]);
    }

    #[test]
    fn the_session_keeps_visibility_widths_and_javas_order() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Session::load(&tmp.path().join("s.properties"));
        s.put("columns.order", "name,modified,size,type,created,accessed,extension,attributes");
        let mut c = Columns::restore(&s);
        assert_eq!(ids(&c), ["name", "modified", "size", "type"]);
        c.toggle(ColumnId::Attributes);
        c.toggle(ColumnId::Size);
        c.toggle(ColumnId::Name);
        c.set_width(ColumnId::Type, 10.0);
        c.save(&s);
        assert_eq!(s.get("columns.order").as_deref(), Some("name,modified,size,type,created,accessed,extension,attributes"), "order kept");
        let back = Columns::restore(&s);
        assert_eq!(ids(&back), ["name", "modified", "type", "attributes"], "name can't be hidden");
        assert_eq!(back.all().iter().find(|c| c.id == ColumnId::Type).unwrap().width, MIN_WIDTH);
        assert_eq!(back, c);
    }

    #[test]
    fn dragged_columns_move_among_the_shown_ones_and_the_order_is_saved() {
        let mut c = Columns::default(); // name size type modified; hidden: created accessed extension attributes
        assert!(c.move_to_gap(ColumnId::Modified, 0));
        assert_eq!(ids(&c), ["name", "modified", "size", "type"]);
        assert!(c.move_to_gap(ColumnId::Modified, 3));
        assert_eq!(ids(&c), ["name", "size", "type", "modified"], "back to the end");
        assert!(!c.move_to_gap(ColumnId::Size, 1), "dropped where it is");
        assert!(!c.move_to_gap(ColumnId::Name, 2), "Name stays first");
        assert!(!c.move_to_gap(ColumnId::Created, 0), "hidden columns aren't in the header");
        c.toggle(ColumnId::Extension);
        assert!(c.move_to_gap(ColumnId::Extension, 1));
        assert_eq!(ids(&c), ["name", "size", "extension", "type", "modified"]);
        c.toggle(ColumnId::Extension);
        c.toggle(ColumnId::Extension);
        assert_eq!(ids(&c), ["name", "size", "extension", "type", "modified"], "hidden and shown again: same place");

        let tmp = tempfile::tempdir().unwrap();
        let s = Session::load(&tmp.path().join("s.properties"));
        c.save(&s);
        assert_eq!(Columns::restore(&s), c);
        assert!(s.get("columns.order").unwrap().starts_with("name,size,extension,type,"));
    }

    #[test]
    fn sortable_columns() {
        assert_eq!(ColumnId::Created.sort_key(), Some(SortKey::Created));
        assert_eq!(ColumnId::Attributes.sort_key(), None);
        assert_eq!(ColumnId::Accessed.sort_key(), None);
        for c in ColumnId::ALL {
            assert_eq!(ColumnId::from_index(c.index()), Some(c));
        }
    }
}
