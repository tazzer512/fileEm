//! How a tab shows its files (the Java `ViewMode`), after Windows Explorer's View menu: same
//! order, same Ctrl+Shift+1…8 shortcuts. Details is the column list; the others are the grid
//! (`grid.rs`). Each tab keeps its own item size per resizable view ([`ViewState`]).

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ViewMode {
    ExtraLargeIcons,
    LargeIcons,
    MediumIcons,
    SmallIcons,
    List,
    #[default]
    Details,
    Tiles,
    Content,
}

/// Icons at least this big show image thumbnails instead of a file type icon.
pub const THUMBNAIL_MIN: u32 = 48;
/// Item sizes are bounded so the smallest icons stay legible and the biggest fit a window.
pub const MIN_ICON: u32 = 16;
pub const MAX_ICON: u32 = 320;
/// Each + / - step scales the icons by this factor.
pub const SIZE_STEP: f64 = 1.25;

impl ViewMode {
    pub const ALL: [ViewMode; 8] = [
        ViewMode::ExtraLargeIcons,
        ViewMode::LargeIcons,
        ViewMode::MediumIcons,
        ViewMode::SmallIcons,
        ViewMode::List,
        ViewMode::Details,
        ViewMode::Tiles,
        ViewMode::Content,
    ];

    /// The position in [`ViewMode::ALL`] (the shortcut digit minus one; the icon number in
    /// side.slint).
    pub fn index(self) -> usize {
        ViewMode::ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Option<ViewMode> {
        ViewMode::ALL.get(i).copied()
    }

    /// The session id (shared with the Java app).
    pub fn id(self) -> &'static str {
        match self {
            ViewMode::ExtraLargeIcons => "xl-icons",
            ViewMode::LargeIcons => "large-icons",
            ViewMode::MediumIcons => "medium-icons",
            ViewMode::SmallIcons => "small-icons",
            ViewMode::List => "list",
            ViewMode::Details => "details",
            ViewMode::Tiles => "tiles",
            ViewMode::Content => "content",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::ExtraLargeIcons => "Extra large icons",
            ViewMode::LargeIcons => "Large icons",
            ViewMode::MediumIcons => "Medium icons",
            ViewMode::SmallIcons => "Small icons",
            ViewMode::List => "List",
            ViewMode::Details => "Details",
            ViewMode::Tiles => "Tiles",
            ViewMode::Content => "Content",
        }
    }

    /// The icon size at the view's own size.
    pub fn base_icon(self) -> u32 {
        match self {
            ViewMode::ExtraLargeIcons => 192,
            ViewMode::LargeIcons => 96,
            ViewMode::MediumIcons | ViewMode::Tiles => 48,
            ViewMode::Content => 32,
            ViewMode::SmallIcons | ViewMode::List | ViewMode::Details => 16,
        }
    }

    /// The mode with `id`, or Details for unknown or missing ids.
    pub fn by_id(id: &str) -> ViewMode {
        ViewMode::ALL.into_iter().find(|m| m.id() == id).unwrap_or(ViewMode::Details)
    }

    /// The next mode in Explorer's order, toward Content (+1) or Extra large icons (-1).
    pub fn step(self, direction: i32) -> ViewMode {
        let i = (self.index() as i32 + direction.signum()).clamp(0, ViewMode::ALL.len() as i32 - 1);
        ViewMode::ALL[i as usize]
    }

    /// List fills columns top to bottom and scrolls sideways; the other grid views fill rows.
    pub fn flows_in_columns(self) -> bool {
        self == ViewMode::List
    }

    /// Big icon views put the name under the icon; the others put text beside it.
    pub fn text_below_icon(self) -> bool {
        matches!(self, ViewMode::ExtraLargeIcons | ViewMode::LargeIcons | ViewMode::MediumIcons)
    }

    /// Views whose items can be made bigger or smaller with + / -: the icon views and Tiles.
    pub fn resizable(self) -> bool {
        matches!(
            self,
            ViewMode::ExtraLargeIcons | ViewMode::LargeIcons | ViewMode::MediumIcons | ViewMode::SmallIcons | ViewMode::Tiles
        )
    }

    /// The icon size after `steps` size steps (0: the view's own size). Steps may be fractional
    /// (the slider); each whole step scales by [`SIZE_STEP`]. Clamped to the limits.
    pub fn icon_size(self, steps: f64) -> u32 {
        if !self.resizable() {
            return self.base_icon();
        }
        (self.base_icon() as f64 * SIZE_STEP.powf(self.clamp_steps(steps))).round() as u32
    }

    /// The fewest steps allowed (negative, or 0 when the view is already at the smallest size).
    pub fn min_steps(self) -> f64 {
        if !self.resizable() {
            return 0.0;
        }
        ((MIN_ICON as f64 / self.base_icon() as f64).ln() / SIZE_STEP.ln()).min(0.0)
    }

    pub fn max_steps(self) -> f64 {
        if !self.resizable() {
            return 0.0;
        }
        ((MAX_ICON as f64 / self.base_icon() as f64).ln() / SIZE_STEP.ln()).max(0.0)
    }

    pub fn clamp_steps(self, steps: f64) -> f64 {
        steps.clamp(self.min_steps(), self.max_steps())
    }

    /// Whether one more step in `direction` (+1 bigger, -1 smaller) changes anything.
    pub fn can_resize(self, steps: f64, direction: i32) -> bool {
        self.resizable() && if direction > 0 { steps < self.max_steps() - 1e-6 } else { steps > self.min_steps() + 1e-6 }
    }

    /// Slider position (-1 … 1, 0 in the middle is the view's own size) to size steps: each half
    /// covers the range to its limit, so the middle always means the default size.
    pub fn steps_for_slider(self, position: f64) -> f64 {
        let p = position.clamp(-1.0, 1.0);
        if p < 0.0 {
            -p * self.min_steps()
        } else {
            p * self.max_steps()
        }
    }

    /// The slider position that shows `steps`.
    pub fn slider_for_steps(self, steps: f64) -> f64 {
        let s = self.clamp_steps(steps);
        if s < 0.0 {
            if self.min_steps() == 0.0 {
                0.0
            } else {
                -s / self.min_steps()
            }
        } else if self.max_steps() == 0.0 {
            0.0
        } else {
            s / self.max_steps()
        }
    }
}

/// A tab's view: the mode, and the size steps of each resizable view (missing: its own size).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewState {
    pub mode: ViewMode,
    sizes: HashMap<ViewMode, f64>,
}

impl ViewState {
    /// `mode` at every view's own size.
    pub fn new(mode: ViewMode) -> ViewState {
        ViewState { mode, sizes: HashMap::new() }
    }

    pub fn steps(&self) -> f64 {
        self.sizes.get(&self.mode).copied().unwrap_or(0.0)
    }

    /// The icon size of the current view and size.
    pub fn icon_px(&self) -> u32 {
        self.mode.icon_size(self.steps())
    }

    pub fn can_resize(&self, direction: i32) -> bool {
        self.mode.can_resize(self.steps(), direction)
    }

    /// One size step bigger (+1) or smaller (-1), for the current view only; false when nothing
    /// changed (Details, or at a limit).
    pub fn resize(&mut self, direction: i32) -> bool {
        if !self.can_resize(direction) {
            return false;
        }
        self.set_steps(self.mode.clamp_steps(self.steps() + direction.signum() as f64));
        true
    }

    /// Sets the current view's size in (possibly fractional) steps, 0 being its own size.
    pub fn set_steps(&mut self, steps: f64) {
        if !self.mode.resizable() || !steps.is_finite() {
            return;
        }
        let s = self.mode.clamp_steps(steps);
        if s.abs() < 1e-6 {
            self.sizes.remove(&self.mode);
        } else {
            self.sizes.insert(self.mode, s);
        }
    }

    /// The slider moved to `position`: a position near the middle snaps to the default size.
    pub fn set_slider(&mut self, position: f64) {
        let p = if position.abs() < 0.04 { 0.0 } else { position };
        self.set_steps(self.mode.steps_for_slider(p));
    }

    pub fn slider(&self) -> f64 {
        self.mode.slider_for_steps(self.steps())
    }

    /// The sizes for the session, as the Java app writes them: "large-icons:2,tiles:-0.5"
    /// (views at their own size are left out), in Explorer's order.
    pub fn sizes_state(&self) -> String {
        ViewMode::ALL
            .into_iter()
            .filter_map(|m| self.sizes.get(&m).map(|s| (m, s)))
            .map(|(m, s)| {
                let n = format!("{s:.3}");
                let n = n.trim_end_matches('0').trim_end_matches('.');
                format!("{}:{n}", m.id())
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Restores [`ViewState::sizes_state`]; unknown views are ignored, sizes beyond the limits
    /// are clamped.
    pub fn restore_sizes(&mut self, state: &str) {
        self.sizes.clear();
        for part in state.split(',') {
            let Some((id, value)) = part.split_once(':') else { continue };
            let m = ViewMode::by_id(id.trim());
            let Ok(steps) = value.trim().parse::<f64>() else { continue };
            if m.resizable() && m.id() == id.trim() && steps.is_finite() {
                let s = m.clamp_steps(steps);
                if s.abs() > 1e-6 {
                    self.sizes.insert(m, s);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_have_explorers_order_ids_and_neighbours() {
        let ids: Vec<&str> = ViewMode::ALL.iter().map(|m| m.id()).collect();
        assert_eq!(ids, ["xl-icons", "large-icons", "medium-icons", "small-icons", "list", "details", "tiles", "content"]);
        assert_eq!(ViewMode::by_id("tiles"), ViewMode::Tiles);
        assert_eq!(ViewMode::by_id("nonsense"), ViewMode::Details);
        assert_eq!(ViewMode::Details.step(-1), ViewMode::List, "Ctrl+wheel up: toward bigger icons");
        assert_eq!(ViewMode::Details.step(1), ViewMode::Tiles);
        assert_eq!(ViewMode::ExtraLargeIcons.step(-1), ViewMode::ExtraLargeIcons, "stops at the ends");
        assert_eq!(ViewMode::Content.step(1), ViewMode::Content);
        let resizable: Vec<bool> = ViewMode::ALL.iter().map(|m| m.resizable()).collect();
        assert_eq!(resizable, [true, true, true, true, false, false, true, false]);
    }

    #[test]
    fn plus_and_minus_resize_within_limits() {
        let mut v = ViewState { mode: ViewMode::LargeIcons, ..ViewState::default() };
        assert_eq!(v.icon_px(), 96);
        assert!(v.resize(1));
        assert_eq!(v.icon_px(), 120);
        v.resize(-1);
        v.resize(-1);
        assert_eq!(v.icon_px(), 77);

        v.mode = ViewMode::SmallIcons;
        assert!(!v.can_resize(-1), "Small icons are already the smallest");
        v.mode = ViewMode::ExtraLargeIcons;
        v.resize(1);
        v.resize(1);
        assert_eq!(v.icon_px(), 300);
        assert!(v.can_resize(1));
        v.resize(1);
        assert_eq!(v.icon_px(), MAX_ICON, "the last step stops at the limit");
        assert!(!v.can_resize(1));

        v.mode = ViewMode::Details;
        assert!(!v.resize(1), "no sizes in Details");
        assert_eq!(v.icon_px(), 16);
    }

    #[test]
    fn the_slider_has_the_default_in_the_middle_and_each_view_its_own_size() {
        let mut v = ViewState { mode: ViewMode::LargeIcons, ..ViewState::default() };
        assert_eq!(v.slider(), 0.0);
        v.set_slider(1.0);
        assert_eq!(v.icon_px(), MAX_ICON);
        v.set_slider(-1.0);
        assert_eq!(v.icon_px(), MIN_ICON);
        v.set_slider(0.5);
        assert!(v.icon_px() > 96 && v.icon_px() < MAX_ICON, "{}", v.icon_px());
        v.set_slider(0.02);
        assert_eq!(v.icon_px(), 96, "near the middle snaps to the default");
        v.resize(1);
        assert!(v.slider() > 0.1);
        v.mode = ViewMode::Tiles;
        assert_eq!((v.slider(), v.icon_px()), (0.0, 48));
        v.resize(1);
        assert_eq!(v.icon_px(), 60);
        v.mode = ViewMode::LargeIcons;
        assert_eq!(v.icon_px(), 120);
    }

    #[test]
    fn sizes_round_trip_in_the_java_format() {
        let mut v = ViewState { mode: ViewMode::LargeIcons, ..ViewState::default() };
        v.resize(1);
        v.resize(1);
        v.mode = ViewMode::Tiles;
        v.set_steps(-0.5);
        v.mode = ViewMode::MediumIcons;
        v.set_slider(0.37);
        let px = v.icon_px();
        let state = v.sizes_state();
        assert!(state.starts_with("large-icons:2,medium-icons:"), "{state}");
        assert!(state.ends_with(",tiles:-0.5"), "{state}");

        let mut again = ViewState { mode: ViewMode::MediumIcons, ..ViewState::default() };
        again.restore_sizes(&state);
        assert_eq!(again.icon_px(), px, "fractional sizes survive");
        again.restore_sizes("tiles:99,details:3,bogus:1,list:x,small-icons:0");
        again.mode = ViewMode::Tiles;
        assert_eq!(again.icon_px(), MAX_ICON, "clamped");
        assert_eq!(again.sizes_state(), format!("tiles:{}", (ViewMode::Tiles.max_steps() * 1000.0).round() / 1000.0));
    }
}
