//! Layout of the grid views (every [`ViewMode`] but Details; the Java `FileGrid`): items sit in
//! fixed slots of `cell_w × cell_h`; List fills columns top to bottom and scrolls sideways, the
//! other views fill rows. The controller computes it from the grid's size (reported by Slint)
//! and uses it for the keys and the selection box; Slint places the visible slots with the same
//! numbers. Coordinates are content coordinates (0,0: the content's top left, scrolling
//! included).

use crate::view_mode::ViewMode;

/// Space around the items.
pub const PAD: f32 = 6.0;
/// Room left for the scroll bar when the items overflow (fluent draws it over the content).
pub const BAR: f32 = 14.0;
/// Average width of a character of the 13 px UI font, to pick the names to measure (and to size
/// List's columns until they are measured).
const CHAR_WIDTH: f32 = 7.0;
/// List measures this many names, the widest by [`rough_width`]; not every name of a big folder.
pub const MEASURED_NAMES: usize = 32;
/// List's room around a name: icon (6 + 16), gap (8), right inset (6), and a little slack.
const LIST_EXTRA: f32 = 38.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub mode: ViewMode,
    pub icon: u32,
    pub cell_w: f32,
    pub cell_h: f32,
    /// Slots across the non-scrolling axis: columns (rows flow) or rows (List).
    pub lanes: usize,
    /// The part of the grid items are placed in (the scroll bar's room left out).
    pub view_w: f32,
    pub view_h: f32,
    pub count: usize,
}

/// A key that moves the focus in the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
}

/// A name's width in the UI font, roughly: narrow and wide letters, capitals and East Asian
/// characters weigh differently, which is enough to find the names worth measuring.
pub fn rough_width(name: &str) -> f32 {
    let em: f32 = name
        .chars()
        .map(|c| match c {
            'i' | 'l' | 'j' | 't' | 'f' | 'r' | 'I' | '!' | '.' | ',' | '\'' | '|' | ':' | ';' | ' ' => 0.5,
            'm' | 'w' | 'M' | 'W' | '@' => 1.5,
            c if c.is_uppercase() => 1.2,
            c if u32::from(c) >= 0x2e80 => 2.0,
            _ => 1.0,
        })
        .sum();
    em * CHAR_WIDTH
}

/// The names List measures to size its columns: the [`MEASURED_NAMES`] roughly widest ones.
pub fn names_to_measure<'a>(names: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut all: Vec<(f32, &str)> = names.map(|n| (rough_width(n), n)).collect();
    let widest_first = |a: &(f32, &str), b: &(f32, &str)| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1));
    if all.len() > MEASURED_NAMES {
        all.select_nth_unstable_by(MEASURED_NAMES - 1, widest_first);
        all.truncate(MEASURED_NAMES);
    }
    all.sort_by(widest_first);
    all.into_iter().map(|(_, n)| n).collect()
}

/// List's column width for names whose widest one is `text_width` wide: plus icon and padding,
/// within limits.
pub fn list_width(text_width: f32) -> f32 {
    (text_width + LIST_EXTRA).clamp(140.0, 420.0)
}

impl Grid {
    /// Lays out `count` items with icons of `icon` px in a grid of `width × height`;
    /// `list_width` is List's column width (see [`list_width`]).
    pub fn layout(mode: ViewMode, icon: u32, width: f32, height: f32, count: usize, list_width: f32) -> Grid {
        let mut g = Grid { mode, icon, cell_w: 0.0, cell_h: 0.0, lanes: 1, view_w: width.max(0.0), view_h: height.max(0.0), count };
        g.measure(width, list_width);
        g.update_lanes();
        // leave room for the scroll bar only when the content overflows
        if g.content_len() > g.view_along() {
            if g.columns() {
                g.view_h = (height - BAR).max(0.0);
            } else {
                g.view_w = (width - BAR).max(0.0);
                g.measure(g.view_w, list_width);
            }
            g.update_lanes();
        }
        g
    }

    pub fn columns(&self) -> bool {
        self.mode.flows_in_columns()
    }

    fn measure(&mut self, width: f32, list_width: f32) {
        let (w, h) = match self.mode {
            ViewMode::ExtraLargeIcons => (228.0, 244.0),
            ViewMode::LargeIcons => (124.0, 146.0),
            ViewMode::MediumIcons => (92.0, 98.0),
            ViewMode::SmallIcons => (220.0, 24.0),
            ViewMode::List => (list_width, 24.0),
            ViewMode::Tiles => (264.0, 64.0),
            ViewMode::Content => ((width - 2.0 * PAD).max(200.0), 56.0),
            ViewMode::Details => (0.0, 24.0),
        };
        (self.cell_w, self.cell_h) = (w, h);
        if self.mode.resizable() {
            // the text area keeps its size
            let delta = self.icon as f32 - self.mode.base_icon() as f32;
            self.cell_w += delta;
            self.cell_h += delta;
            if self.mode == ViewMode::Tiles {
                self.cell_h = self.cell_h.max(56.0); // three lines of text
            } else if self.mode == ViewMode::SmallIcons {
                self.cell_h = self.cell_h.max(24.0);
            }
        }
    }

    fn update_lanes(&mut self) {
        let (across, slot) = if self.columns() { (self.view_h, self.cell_h) } else { (self.view_w, self.cell_w) };
        self.lanes = if slot > 0.0 { (((across - 2.0 * PAD) / slot).floor() as usize).max(1) } else { 1 };
    }

    /// The size of one line of slots along the scroll axis.
    pub fn along(&self) -> f32 {
        if self.columns() {
            self.cell_w
        } else {
            self.cell_h
        }
    }

    fn view_along(&self) -> f32 {
        if self.columns() {
            self.view_w
        } else {
            self.view_h
        }
    }

    /// The content's length along the scroll axis.
    pub fn content_len(&self) -> f32 {
        let lines = self.count.div_ceil(self.lanes);
        2.0 * PAD + lines as f32 * self.along()
    }

    /// The top left of item `i`'s slot.
    pub fn slot(&self, i: usize) -> (f32, f32) {
        let (line, lane) = ((i / self.lanes) as f32, (i % self.lanes) as f32);
        if self.columns() {
            (PAD + line * self.cell_w, PAD + lane * self.cell_h)
        } else {
            (PAD + lane * self.cell_w, PAD + line * self.cell_h)
        }
    }

    /// The item at a point, or `None` for empty space.
    pub fn index_at(&self, x: f32, y: f32) -> Option<usize> {
        let (cx, cy) = (x - PAD, y - PAD);
        if cx < 0.0 || cy < 0.0 {
            return None;
        }
        let (col, row) = ((cx / self.cell_w) as usize, (cy / self.cell_h) as usize);
        let i = if self.columns() {
            (row < self.lanes).then(|| col * self.lanes + row)?
        } else {
            (col < self.lanes).then(|| row * self.lanes + col)?
        };
        (i < self.count).then_some(i)
    }

    /// The items a selection box from `a` to `b` touches.
    pub fn box_hits(&self, a: (f32, f32), b: (f32, f32)) -> Vec<usize> {
        let (min_x, max_x) = (a.0.min(b.0) - PAD, a.0.max(b.0) - PAD);
        let (min_y, max_y) = (a.1.min(b.1) - PAD, a.1.max(b.1) - PAD);
        if max_x < 0.0 || max_y < 0.0 || self.count == 0 {
            return Vec::new();
        }
        let (c1, c2) = ((min_x.max(0.0) / self.cell_w) as usize, (max_x / self.cell_w) as usize);
        let (r1, r2) = ((min_y.max(0.0) / self.cell_h) as usize, (max_y / self.cell_h) as usize);
        let mut hits = Vec::new();
        for r in r1..=r2 {
            for c in c1..=c2 {
                let i = match self.columns() {
                    true if r < self.lanes => c * self.lanes + r,
                    false if c < self.lanes => r * self.lanes + c,
                    _ => continue,
                };
                if i < self.count {
                    hits.push(i);
                }
            }
        }
        hits.sort_unstable();
        hits
    }

    /// How many slots the view needs: the lines that fit, plus one partly shown at each end.
    pub fn slot_count(&self) -> usize {
        if self.along() <= 0.0 {
            return 0;
        }
        ((self.view_along() / self.along()).ceil() as usize + 1) * self.lanes
    }

    /// How far a key moves the focus: Up/Down jump a row in row flow, Left/Right a column in
    /// List; a page is the lines in view.
    pub fn delta(&self, m: Move) -> isize {
        let lanes = self.lanes as isize;
        let (along, side) = if self.columns() { (1, lanes) } else { (lanes, 1) };
        let page = (((self.view_along() / self.along()).floor() as isize).max(1)) * lanes;
        match m {
            Move::Left => -side,
            Move::Right => side,
            Move::Up => -along,
            Move::Down => along,
            Move::PageUp => -page,
            Move::PageDown => page,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(mode: ViewMode, count: usize) -> Grid {
        Grid::layout(mode, mode.base_icon(), 600.0, 400.0, count, 200.0)
    }

    #[test]
    fn icons_fill_rows_and_leave_room_for_the_bar_only_when_overflowing() {
        let few = grid(ViewMode::LargeIcons, 3);
        assert_eq!((few.cell_w, few.cell_h, few.lanes), (124.0, 146.0, 4), "(600 - 12) / 124");
        assert_eq!(few.view_w, 600.0);
        let many = grid(ViewMode::LargeIcons, 40);
        assert_eq!(many.view_w, 600.0 - BAR);
        assert_eq!(many.lanes, 4);
        assert_eq!(many.content_len(), 2.0 * PAD + 10.0 * 146.0);
        assert_eq!(many.slot(5), (PAD + 124.0, PAD + 146.0));
        // a bigger size grows the cell by the icon's change
        let big = Grid::layout(ViewMode::LargeIcons, 120, 600.0, 400.0, 3, 200.0);
        assert_eq!((big.cell_w, big.cell_h), (148.0, 170.0));
        let small = Grid::layout(ViewMode::SmallIcons, 16, 600.0, 400.0, 3, 200.0);
        assert_eq!(small.cell_h, 24.0);
        let tiles = Grid::layout(ViewMode::Tiles, 30, 600.0, 400.0, 3, 200.0);
        assert_eq!(tiles.cell_h, 56.0, "three lines of text");
    }

    #[test]
    fn list_fills_columns_and_content_spans_the_width() {
        let list = grid(ViewMode::List, 40);
        assert!(list.columns());
        assert_eq!(list.lanes, ((400.0 - BAR - 2.0 * PAD) / 24.0) as usize, "a horizontal bar");
        assert_eq!(list.slot(list.lanes + 1), (PAD + 200.0, PAD + 24.0));
        assert_eq!(list.index_at(PAD + 210.0, PAD + 30.0), Some(list.lanes + 1));
        assert_eq!(list.index_at(PAD + 10.0, PAD + list.lanes as f32 * 24.0 + 2.0), None, "below the last lane");
        let content = grid(ViewMode::Content, 3);
        assert_eq!((content.cell_w, content.lanes), (600.0 - 2.0 * PAD, 1));
        assert_eq!(list_width(7.0), 140.0);
        assert_eq!(list_width(200.0), 238.0);
        assert_eq!(list_width(1400.0), 420.0);
    }

    #[test]
    fn list_measures_the_roughly_widest_names_only() {
        assert!(rough_width("WWWW") > rough_width("iiiiiiii"), "wide letters count more than narrow ones");
        assert!(rough_width("写真") > rough_width("ab"));
        let mut names: Vec<String> = (0..100).map(|i| format!("document {i:03}")).collect();
        names.push("iiiiiiiiiiiiiiiiiiii".into());
        names.push("MMMMMMMMMMMMMMMM".into());
        let picked = names_to_measure(names.iter().map(String::as_str));
        assert_eq!(picked.len(), MEASURED_NAMES);
        assert_eq!(picked[0], "MMMMMMMMMMMMMMMM", "widest first");
        assert!(!picked.contains(&"iiiiiiiiiiiiiiiiiiii"), "more letters, but narrow ones");
        assert_eq!(names_to_measure(["b", "a"].into_iter()), ["a", "b"], "a short list: all of it, in a stable order");
    }

    #[test]
    fn points_find_items_and_empty_space() {
        let g = grid(ViewMode::LargeIcons, 6);
        assert_eq!(g.index_at(PAD + 1.0, PAD + 1.0), Some(0));
        assert_eq!(g.index_at(PAD + 130.0, PAD + 150.0), Some(5));
        assert_eq!(g.index_at(PAD + 250.0, PAD + 150.0), None, "past the last item");
        assert_eq!(g.index_at(PAD + 4.0 * 124.0 + 2.0, PAD + 1.0), None, "right of the last lane");
        assert_eq!(g.index_at(2.0, 20.0), None, "in the padding");
    }

    #[test]
    fn a_box_touches_the_slots_it_overlaps() {
        let g = grid(ViewMode::LargeIcons, 6);
        // from empty space right of item 5 into item 2 (row 0, col 2)
        assert_eq!(g.box_hits((PAD + 380.0, PAD + 200.0), (PAD + 250.0, PAD + 100.0)), [2, 3]);
        assert_eq!(g.box_hits((PAD + 1.0, PAD + 1.0), (PAD + 130.0, PAD + 150.0)), [0, 1, 4, 5]);
        assert!(g.box_hits((0.0, 0.0), (2.0, 2.0)).is_empty(), "only the padding");
        let list = grid(ViewMode::List, 40);
        assert_eq!(list.box_hits((PAD + 1.0, PAD + 1.0), (PAD + 201.0, PAD + 25.0)), [0, 1, list.lanes, list.lanes + 1]);
    }

    #[test]
    fn keys_follow_the_layout() {
        let g = grid(ViewMode::MediumIcons, 40);
        assert_eq!(g.lanes, 6, "(586 - 12) / 92");
        assert_eq!((g.delta(Move::Right), g.delta(Move::Down), g.delta(Move::Up)), (1, 6, -6));
        assert_eq!(g.delta(Move::PageDown), 4 * 6, "four rows of 98 in view");
        assert_eq!(g.slot_count(), (5 + 1) * 6, "400 / 98 rounded up, plus one");
        let list = grid(ViewMode::List, 40);
        assert_eq!((list.delta(Move::Down), list.delta(Move::Right)), (1, list.lanes as isize));
    }
}
