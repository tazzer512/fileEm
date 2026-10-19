//! The details view's columns (the Java `FileTable` columns): which are shown, their widths and
//! order, saved in the session as `columns.<id>.visible`, `columns.<id>.width` and
//! `columns.order` (the order is only read: there is no reordering yet, and Java's stays as it is).

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
        assert_eq!(s.get("columns.order").as_deref(), Some("name,modified,size,type,created,accessed,extension,attributes"), "order untouched");
        let back = Columns::restore(&s);
        assert_eq!(ids(&back), ["name", "modified", "type", "attributes"], "name can't be hidden");
        assert_eq!(back.all().iter().find(|c| c.id == ColumnId::Type).unwrap().width, MIN_WIDTH);
        assert_eq!(back, c);
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
