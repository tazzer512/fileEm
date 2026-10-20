//! The window's controller (the Java `MainWindow` + `ExplorerSide`): two sides, each with a folder
//! tree and tabs; the second side shows in dual mode (F9). Commands and keys act on the active
//! side's active tab. Slint draws what [`push_view`] hands it; all state lives here.
//!
//! - `nav`: navigation, listings, watching, the tree, tabs and sides
//! - `commands`: operations, clipboard, dialogs, menus, properties
//! - `keys`: the keyboard
//! - `dnd`: mouse selection (box) and drag & drop
//! - `views`: view modes, item sizes, the grid and its thumbnails
//! - `preview`: the preview of the selected file, docked or in its own window
//! - `search`: search windows
//! - `bookmarks`: bookmarks, their tree section and the Manage bookmarks window
//! - `volumes`: the drives (polling) and ejecting them

mod bookmarks;
mod commands;
mod dnd;
mod keys;
mod nav;
mod preview;
mod search;
mod views;
mod volumes;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use fileem_core::bookmarks::Bookmarks;
use fileem_core::fs::listing::SortKey;
use fileem_core::fs::volumes::Volume;
use fileem_core::fs::places;
use fileem_core::fs::watcher::WatchHandle;
use fileem_core::fs::{Category, FileEntry};
use fileem_core::search_history::SearchHistory;
use fileem_core::session::Session;
use fileem_core::util::{formats, fs_util};

use crate::address;
use crate::bg::Job;
use crate::clipboard::{self, FileClipboard};
use crate::columns::{ColumnId, Columns};
use crate::menu::Command;
use crate::ops_ui::OpsUi;
use crate::tab::{Nav, Tab};
use crate::thumbs::Thumbnails;
use crate::tree::{NodeKind, Tree};
use crate::view_mode::{ViewMode, ViewState};
use crate::{AppWindow, ColumnRow, CrumbRow, FileRow, PropertiesWindow, SideView, TabRow, TreeRow};

pub use volumes::{Ejector, SystemEjector};
pub use commands::{
    clipboard_text, close_requested, close_windows, open_dialog, properties_windows, run_command, run_operation,
    set_clipboard, Dialog, DialogSpec,
};

/// One tab: its folder state plus the bookkeeping of its listing and watch.
pub struct TabState {
    /// Async results find their tab by this id (tabs move when others close).
    id: u64,
    tab: Tab,
    load: Option<Job>,
    /// Drops listings that finished after a newer navigation started.
    generation: u64,
    /// A navigation (not a refresh) is loading: watcher refreshes wait, it lists fresh anyway.
    navigating: bool,
    /// A refresh is loading; further requests collapse into one follow-up.
    refreshing: bool,
    refresh_pending: bool,
    /// How long the last listing took: follow-up refreshes wait twice that.
    last_list: Duration,
    /// Selected once the next listing is shown (a renamed or created item, a typed file path).
    select_after: Option<PathBuf>,
    /// The watch on the tab's folder; dropping it stops watching.
    watch: Option<WatchHandle>,
    /// Drops watcher reports for a folder no longer shown.
    watch_token: u64,
    /// View mode and item sizes.
    view: ViewState,
}

/// One explorer side: a tree and tabs, and the Slint models that show them.
pub struct Side {
    tree: Tree,
    tabs: Vec<TabState>,
    active: usize,
    /// Drops tree loads of an older reveal.
    reveal_token: u64,
    tree_width: f32,
    files: Rc<VecModel<FileRow>>,
    /// The Details list's model in the grid views.
    no_files: Rc<VecModel<FileRow>>,
    /// The names Slint measures for List's column width (`grid::names_to_measure`, a line each),
    /// the widest one's measured width (0 until measured) and its rough width (used until then).
    measure_names: SharedString,
    measured_width: f32,
    rough_width: f32,
    /// The grid's slots (0, 1, …): only grows or shrinks, so Slint keeps the slots it built.
    slots: Rc<VecModel<i32>>,
    tree_rows: Rc<VecModel<TreeRow>>,
    tab_rows: Rc<VecModel<TabRow>>,
    crumbs: Rc<VecModel<CrumbRow>>,
    /// Requests to the view (scroll, edit the address, focus the filter): counters that change.
    ensure_row: i32,
    ensure_seq: i32,
    top_seq: i32,
    edit_seq: i32,
    filter_seq: i32,
    tree_row: i32,
    tree_seq: i32,
    /// The tree row whose menu is open (outlined), -1 for none.
    tree_menu_row: i32,
    /// The grid's size, as Slint laid it out, and the slots it shows (first, count).
    grid_size: (f32, f32),
    grid_range: (usize, usize),
    /// Ctrl+wheel turned since the last view change.
    wheel: f32,
    /// When the size slider was last let go (two quick ones are a double-click).
    slider_released: Option<std::time::Instant>,
    /// A message shown in the status line instead of the counts (`set_status`).
    status_message: Option<String>,
}

impl Side {
    fn new(quick: &[places::Place], drives: &[crate::tree::Drive]) -> Side {
        Side {
            tree: Tree::new(quick, drives),
            tabs: Vec::new(),
            active: 0,
            reveal_token: 0,
            tree_width: 240.0,
            files: Rc::new(VecModel::default()),
            no_files: Rc::new(VecModel::default()),
            measure_names: SharedString::new(),
            measured_width: 0.0,
            rough_width: 0.0,
            slots: Rc::new(VecModel::default()),
            tree_rows: Rc::new(VecModel::default()),
            tab_rows: Rc::new(VecModel::default()),
            crumbs: Rc::new(VecModel::default()),
            ensure_row: -1,
            ensure_seq: 0,
            top_seq: 0,
            edit_seq: 0,
            filter_seq: 0,
            tree_row: -1,
            tree_seq: 0,
            tree_menu_row: -1,
            grid_size: (0.0, 0.0),
            grid_range: (0, 0),
            wheel: 0.0,
            slider_released: None,
            status_message: None,
        }
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.active].tab
    }

    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active].tab
    }
}

pub struct App {
    ui: slint::Weak<AppWindow>,
    session: Rc<Session>,
    sides: [Side; 2],
    /// The side commands and keys act on.
    active: usize,
    /// The second side is shown.
    dual: bool,
    show_hidden: bool,
    columns: Columns,
    /// Tab ids and watch tokens.
    next_id: u64,
    type_ahead: String,
    type_ahead_at: Option<Instant>,
    ops: OpsUi,
    dialog: Dialog,
    clipboard: Box<dyn FileClipboard>,
    /// The commands of the menu shown last, by `MenuRow.id`.
    menu: Vec<Command>,
    /// Open Properties windows by id; the flag stops their folder walk.
    properties: Vec<(u64, PropertiesWindow, Arc<AtomicBool>)>,
    /// A press on a row of the selection that waits for its release (side, row): see `Tab::press`.
    late_press: Option<(usize, usize)>,
    drag: dnd::Drag,
    thumbs: Thumbnails,
    preview: preview::Preview,
    searches: Vec<search::Search>,
    /// The last searches, shared by every search window.
    history: SearchHistory,
    /// Shared by both trees.
    bookmarks: Bookmarks,
    manage: Option<bookmarks::Manage>,
    /// The drives as last listed, and the ones being ejected.
    volumes: Vec<Volume>,
    ejecting: HashSet<PathBuf>,
    ejector: Arc<dyn Ejector>,
    /// Lists the drives every few seconds; `None` once a test shows its own.
    poller: Option<volumes::Poller>,
}

pub type Shared = Rc<RefCell<App>>;

const SIDE_IDS: [&str; 2] = ["left", "right"];

impl App {
    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn side(&self) -> &Side {
        &self.sides[self.active]
    }

    fn side_mut(&mut self) -> &mut Side {
        &mut self.sides[self.active]
    }

    /// The active side's active tab.
    fn tab(&self) -> &Tab {
        self.side().tab()
    }

    fn tab_mut(&mut self) -> &mut Tab {
        self.side_mut().tab_mut()
    }

    fn active_tab_id(&self) -> u64 {
        self.side().tabs[self.side().active].id
    }

    /// Side and index of the tab with `id`, if it is still open.
    fn find(&self, id: u64) -> Option<(usize, usize)> {
        (0..2).find_map(|s| self.sides[s].tabs.iter().position(|t| t.id == id).map(|i| (s, i)))
    }

    fn tab_state(&mut self, id: u64) -> Option<&mut TabState> {
        let (s, i) = self.find(id)?;
        Some(&mut self.sides[s].tabs[i])
    }

    /// Whether the tab with `id` is the one its side shows.
    fn is_shown(&self, id: u64) -> bool {
        self.find(id).is_some_and(|(s, i)| self.sides[s].active == i && (s == 0 || self.dual))
    }

    fn new_tab_state(&mut self, sort: (SortKey, bool), mode: ViewMode) -> TabState {
        let mut tab = Tab::new();
        tab.set_show_hidden(self.show_hidden);
        tab.set_sort(sort.0, sort.1);
        TabState {
            id: self.next_id(),
            tab,
            load: None,
            generation: 0,
            navigating: false,
            refreshing: false,
            refresh_pending: false,
            last_list: Duration::ZERO,
            select_after: None,
            watch: None,
            watch_token: 0,
            view: ViewState::new(mode),
        }
    }
}

/// Creates the controller for `ui`, restores the state saved in `session` (tabs of both sides,
/// dual mode, hidden files, sort, columns, tree widths), wires the callbacks and opens the
/// folders. `start` replaces the left side's selected tab's folder.
pub fn install(ui: &AppWindow, session: Rc<Session>, start: Option<PathBuf>) -> Shared {
    let show_hidden = session.get_bool("showHidden", false);
    let sort = (
        SortKey::from_id(&session.get_or("sort.column", "name")).unwrap_or(SortKey::Name),
        session.get_bool("sort.ascending", true),
    );
    // the mount table only: the free space comes with the first poll
    let drives_now = fileem_core::fs::volumes::mounts();
    let (quick, drive_rows) = (places::quick_access(), volumes::drives(&drives_now, &HashSet::new()));
    let app = Rc::new(RefCell::new(App {
        ui: ui.as_weak(),
        columns: Columns::restore(&session),
        dual: session.get_bool("dual", false),
        session: session.clone(),
        sides: [Side::new(&quick, &drive_rows), Side::new(&quick, &drive_rows)],
        active: 0,
        show_hidden,
        next_id: 0,
        type_ahead: String::new(),
        type_ahead_at: None,
        ops: OpsUi::new(),
        dialog: Dialog::None,
        clipboard: Box::new(clipboard::System::default()),
        menu: Vec::new(),
        properties: Vec::new(),
        late_press: None,
        drag: dnd::Drag::default(),
        preview: preview::Preview::default(),
        searches: Vec::new(),
        history: SearchHistory::load(&session),
        bookmarks: Bookmarks::load(&session),
        manage: None,
        volumes: drives_now.clone(),
        ejecting: HashSet::new(),
        ejector: Arc::new(SystemEjector),
        poller: None,
        thumbs: Thumbnails::new({
            let ui = ui.as_weak();
            move || {
                let _ = ui.upgrade_in_event_loop(|ui| ui.invoke_thumbs_ready());
            }
        }),
    }));
    ui.set_side_split(session.get_f64("split.sides", 0.5).clamp(0.2, 0.8) as f32);
    let mut opening: Vec<(u64, PathBuf)> = Vec::new();
    for (s, id) in SIDE_IDS.iter().enumerate() {
        let mut a = app.borrow_mut();
        a.sides[s].tree.set_show_hidden(show_hidden);
        a.sides[s].tree_width = session.get_f64(&format!("{id}.treeWidth"), 240.0).clamp(120.0, 800.0) as f32;
        // without a count (hand-edited files), the consecutive tab keys
        let count = match session.get_int(&format!("{id}.tabs"), -1) {
            n if n >= 0 => n.min(50),
            _ => (0..50).take_while(|i| session.get(&format!("{id}.tab.{i}")).is_some()).count() as i64,
        };
        for i in 0..count {
            let Some(dir) = session.get(&format!("{id}.tab.{i}")).filter(|p| !p.trim().is_empty()) else { continue };
            let mode = ViewMode::by_id(&session.get_or(&format!("{id}.tab.{i}.view"), "details"));
            let mut t = a.new_tab_state(sort, mode);
            t.view.restore_sizes(&session.get_or(&format!("{id}.tab.{i}.size"), ""));
            opening.push((t.id, PathBuf::from(dir)));
            a.sides[s].tabs.push(t);
        }
        if a.sides[s].tabs.is_empty() {
            let t = a.new_tab_state(sort, ViewMode::Details);
            opening.push((t.id, fileem_core::util::os::home()));
            a.sides[s].tabs.push(t);
        }
        let n = a.sides[s].tabs.len();
        a.sides[s].active = (session.get_int(&format!("{id}.selected"), 0).max(0) as usize).min(n - 1);
    }
    if let Some(dir) = start {
        let id = app.borrow().sides[0].tabs[app.borrow().sides[0].active].id;
        if let Some(entry) = opening.iter_mut().find(|(t, _)| *t == id) {
            entry.1 = dir;
        }
    }
    wire(ui, &app);
    // a drag needs something to carry; what is dragged is known here (dnd::drag_started)
    let mut drag = slint::DataTransfer::default();
    drag.set_plain_text("fileem".into());
    ui.set_drag_data(drag);
    push_columns(&app);
    for s in 0..2 {
        sync_tabs(&app, s);
        sync_tree(&app, s);
        sync_files(&app, s);
    }
    bookmarks::sync(&app);
    volumes::start(&app, drives_now);
    sync_window(&app);
    preview::restore(&app);
    for (id, dir) in opening {
        // a folder from last time may be gone: its nearest existing parent then
        nav::go_with(&app, id, dir, Nav::Go, true);
    }
    app
}

/// Copies the window's state into the session (autosave, close). Only keys this version knows
/// are written; the rest (theme, preview, …) stays as the Java app left it, so both versions can
/// share the file during the migration.
pub fn collect_session(app: &Shared) {
    let a = app.borrow();
    let s = &a.session;
    s.put("showHidden", a.show_hidden);
    let (key, asc) = a.tab().sort();
    s.put("sort.column", key.id());
    s.put("sort.ascending", asc);
    s.put("dual", a.dual);
    a.columns.save(s);
    for (side, id) in a.sides.iter().zip(SIDE_IDS) {
        s.remove_prefix(&format!("{id}.tab."));
        s.put(&format!("{id}.tabs"), side.tabs.len());
        for (i, t) in side.tabs.iter().enumerate() {
            let dir = t.tab.dir().map(Path::to_path_buf).unwrap_or_else(fileem_core::util::os::home);
            s.put(&format!("{id}.tab.{i}"), dir.display());
            s.put(&format!("{id}.tab.{i}.view"), t.view.mode.id());
            let sizes = t.view.sizes_state();
            if !sizes.is_empty() {
                s.put(&format!("{id}.tab.{i}.size"), sizes);
            }
        }
        s.put(&format!("{id}.selected"), side.active);
        s.put(&format!("{id}.treeWidth"), side.tree_width.round() as i64);
    }
    if let Some(ui) = a.ui.upgrade() {
        if a.dual {
            s.put("split.sides", ui.get_side_split());
        }
        let w = ui.window();
        s.put("window.maximized", w.is_maximized());
        if !w.is_maximized() && !w.is_minimized() {
            let size = w.size().to_logical(w.scale_factor());
            if size.width > 0.0 && size.height > 0.0 {
                // Java writes doubles; keep the same shape so it reads them back unchanged
                s.put("window.w", format!("{:.1}", size.width));
                s.put("window.h", format!("{:.1}", size.height));
            }
        }
    }
    drop(a);
    preview::save(app);
}

/// The preview for tests: (on, docked, shown file's name, zoom, fitting) and its own window.
pub fn preview_state(app: &Shared) -> (bool, bool, Option<String>, f64, bool) {
    preview::state(app)
}

pub fn preview_window(app: &Shared) -> Option<crate::PreviewWindow> {
    preview::window(app)
}

/// Opens a search window for `dir` (the menus' "Search in folder…", Ctrl+Shift+F).
pub fn open_search(app: &Shared, dir: PathBuf) -> Option<crate::SearchWindow> {
    search::open(app, dir)
}

/// The open search windows, oldest first.
pub fn search_windows(app: &Shared) -> Vec<crate::SearchWindow> {
    search::windows(app)
}

/// The `index`th search window's results as shown, and its selection.
pub fn search_results(app: &Shared, index: usize) -> (Vec<PathBuf>, Vec<PathBuf>) {
    search::results(app, index)
}

/// The bookmarks (name, folder) in order.
pub fn bookmarks(app: &Shared) -> Vec<(String, PathBuf)> {
    app.borrow().bookmarks.items().iter().map(|b| (b.name.clone(), b.path.clone())).collect()
}

/// The Manage bookmarks window, when open.
pub fn bookmarks_window(app: &Shared) -> Option<crate::BookmarksWindow> {
    bookmarks::window(app)
}

/// Shows these drives instead of the system's and stops following the system's (tests).
pub fn show_volumes(app: &Shared, list: Vec<Volume>) {
    volumes::show(app, list);
}

/// Replaces the real eject (tests: never eject for real).
pub fn set_ejector(app: &Shared, ejector: Arc<dyn Ejector>) {
    app.borrow_mut().ejector = ejector;
}

/// Whether the drive at `root` is being ejected.
pub fn is_ejecting(app: &Shared, root: &Path) -> bool {
    volumes::is_ejecting(app, root)
}

pub fn session(app: &Shared) -> Rc<Session> {
    app.borrow().session.clone()
}

// ---- read access for tests ----

/// The active side (0 left, 1 right).
pub fn active_side(app: &Shared) -> usize {
    app.borrow().active
}

pub fn is_dual(app: &Shared) -> bool {
    app.borrow().dual
}

/// The folders of a side's tabs, in order, and which one is selected.
pub fn tabs(app: &Shared, side: usize) -> (Vec<Option<PathBuf>>, usize) {
    let a = app.borrow();
    let s = &a.sides[side];
    (s.tabs.iter().map(|t| t.tab.dir().map(Path::to_path_buf)).collect(), s.active)
}

/// The selected paths of a side's active tab.
pub fn selected_paths(app: &Shared, side: usize) -> Vec<PathBuf> {
    app.borrow().sides[side].tab().selected_paths()
}

/// The view mode and icon size of a side's shown tab.
pub fn view_of(app: &Shared, side: usize) -> (ViewMode, u32) {
    let a = app.borrow();
    let v = &a.sides[side].tabs[a.sides[side].active].view;
    (v.mode, v.icon_px())
}

/// The view modes of a side's tabs, in order.
pub fn tab_views(app: &Shared, side: usize) -> Vec<ViewMode> {
    app.borrow().sides[side].tabs.iter().map(|t| t.view.mode).collect()
}

/// The grid layout of a side's shown tab (`None` in Details).
pub fn grid(app: &Shared, side: usize) -> Option<crate::grid::Grid> {
    views::grid_of(&app.borrow().sides[side])
}

/// The operations of this window (tests start their own through it).
pub fn ops(app: &Shared) -> OpsUi {
    app.borrow().ops.clone()
}

fn wire(ui: &AppWindow, app: &Shared) {
    macro_rules! on {
        ($setter:ident, $app:ident => |$($arg:ident),*| $body:expr) => {{
            let a = app.clone();
            ui.$setter(move |$($arg),*| {
                let $app = &a;
                $body
            });
        }};
    }
    on!(on_back, app => | | nav::back(app));
    on!(on_forward, app => | | nav::forward(app));
    on!(on_up, app => | | nav::up(app));
    on!(on_refresh, app => | | nav::refresh_active(app));
    on!(on_toggle_hidden, app => | | nav::toggle_hidden(app));
    on!(on_toggle_dual, app => | | nav::toggle_dual(app));
    on!(on_toggle_bookmark, app => | | bookmarks::toggle(app));
    on!(on_tool, app => |name| commands::tool(app, &name));
    on!(on_activate, app => |side| nav::set_active_side(app, side as usize));
    on!(on_tree_resized, app => |side, width| {
        app.borrow_mut().sides[side as usize].tree_width = width;
        push_view(app, side as usize);
    });
    on!(on_row_pressed, app => |side, row, ctrl, shift| dnd::row_pressed(app, side as usize, row as usize, ctrl, shift));
    on!(on_row_released, app => |side, row| dnd::row_released(app, side as usize, row as usize));
    on!(on_box_start, app => |side, ctrl, shift| dnd::box_start(app, side as usize, ctrl, shift));
    on!(on_box_moved, app => |side, ax, ay, mx, my| dnd::box_moved(app, side as usize, (ax, ay), (mx, my)));
    on!(on_box_ended, app => |side, drawn| dnd::box_ended(app, side as usize, drawn));
    on!(on_drag_started, app => |side, area, row, x, y, w, h| {
        dnd::drag_started(app, side as usize, area, row as usize);
        dnd::show_drag_image(app, (x, y, w, h));
    });
    on!(on_drop_check, app => |side, area, row| dnd::drop_check(app, side as usize, area, row));
    on!(on_dropped, app => |side, area, row| dnd::dropped(app, side as usize, area, row));
    on!(on_row_activated, app => |side, row| {
        nav::set_active_side(app, side as usize);
        commands::activate(app, row as usize);
    });
    on!(on_row_menu, app => |side, row| {
        nav::set_active_side(app, side as usize);
        commands::row_menu(app, row);
    });
    on!(on_empty_pressed, app => |side| {
        nav::set_active_side(app, side as usize);
        app.borrow_mut().tab_mut().context_click(None);
        sync_selection(app, side as usize, false);
    });
    on!(on_tree_clicked, app => |side, row| nav::tree_clicked(app, side as usize, row as usize));
    on!(on_tree_toggled, app => |side, row| nav::tree_toggled(app, side as usize, row as usize));
    on!(on_tree_menu, app => |side, row| {
        nav::set_active_side(app, side as usize);
        commands::tree_menu(app, side as usize, row as usize)
    });
    on!(on_menu_closed, app => | | commands::menu_closed(app));
    on!(on_terminate, app => | | commands::terminate(app));
    on!(on_toggle_preview, app => | | preview::toggle(app));
    on!(on_preview_zoom, app => |direction| preview::zoom(app, direction));
    on!(on_preview_fit, app => | | preview::fit(app));
    on!(on_preview_actual, app => | | preview::actual(app));
    on!(on_preview_dock, app => | | preview::toggle_dock(app));
    on!(on_preview_resized, app => |width, height| preview::resized(app, width, height));
    on!(on_sort_by, app => |side, col| {
        nav::set_active_side(app, side as usize);
        if let Some(key) = ColumnId::from_index(col as usize).and_then(ColumnId::sort_key) {
            app.borrow_mut().tab_mut().sort_by(key);
            sync_files(app, side as usize);
        }
    });
    on!(on_header_menu, app => |side| {
        nav::set_active_side(app, side as usize);
        commands::header_menu(app);
    });
    on!(on_column_moved, app => |col, gap| {
        let moved = ColumnId::from_index(col as usize).is_some_and(|c| app.borrow_mut().columns.move_to_gap(c, gap.max(0) as usize));
        if moved {
            push_columns(app);
        }
    });
    on!(on_tab_moved, app => |side, from, gap| nav::move_tab(app, side as usize, from.max(0) as usize, gap.max(0) as usize));
    on!(on_column_resized, app => |col, width| {
        if let Some(c) = ColumnId::from_index(col as usize) {
            app.borrow_mut().columns.set_width(c, width);
            push_columns(app);
        }
    });
    on!(on_tab_clicked, app => |side, i| {
        nav::set_active_side(app, side as usize);
        nav::select_tab(app, side as usize, i as usize);
    });
    on!(on_tab_closed, app => |side, i| nav::close_tab(app, side as usize, i as usize));
    on!(on_tab_new, app => |side| {
        nav::set_active_side(app, side as usize);
        let dir = app.borrow().sides[side as usize].tab().dir().map(Path::to_path_buf);
        nav::new_tab(app, side as usize, dir.unwrap_or_else(fileem_core::util::os::home), true);
    });
    on!(on_tab_menu, app => |side, i| commands::tab_menu(app, side as usize, i as usize));
    on!(on_crumb_clicked, app => |side, i| {
        nav::set_active_side(app, side as usize);
        let target = app.borrow().tab().dir().and_then(|d| address::crumbs(d).get(i as usize).map(|c| c.path.clone()));
        if let Some(dir) = target {
            let id = app.borrow().active_tab_id();
            nav::go(app, id, dir, Nav::Go);
        }
    });
    on!(on_address_entered, app => |side, text| {
        nav::set_active_side(app, side as usize);
        nav::navigate_address(app, &text);
    });
    on!(on_filter_edited, app => |side, text| {
        nav::set_active_side(app, side as usize);
        app.borrow_mut().tab_mut().set_filter(&text);
        sync_files(app, side as usize);
    });
    on!(on_dir_changed, app => |token| nav::dir_changed(app, token as u64));
    on!(on_keyboard_menu, app => | | commands::keyboard_menu(app));
    on!(on_menu_activated, app => |id| commands::menu_activated(app, id));
    on!(on_dialog_accepted, app => |text| commands::dialog_accepted(app, &text));
    on!(on_dialog_cancelled, app => | | commands::close_dialog(app));
    on!(on_dialog_alternative, app => | | dnd::copy_instead(app));
    on!(on_key, app => |text, ctrl, alt, shift| keys::key(app, &text, ctrl, alt, shift));
    on!(on_view_menu, app => |side| {
        nav::set_active_side(app, side as usize);
        commands::view_menu(app);
    });
    on!(on_sort_menu, app => |side| {
        nav::set_active_side(app, side as usize);
        commands::sort_menu(app);
    });
    on!(on_size_step, app => |side, direction| {
        nav::set_active_side(app, side as usize);
        views::resize(app, side as usize, direction);
    });
    on!(on_size_slider_moved, app => |side, position| {
        nav::set_active_side(app, side as usize);
        views::slider_moved(app, side as usize, position);
    });
    on!(on_size_slider_released, app => |side| views::slider_released(app, side as usize));
    on!(on_list_measured, app => |side, width| views::list_measured(app, side as usize, width));
    on!(on_view_wheel, app => |side, delta| {
        nav::set_active_side(app, side as usize);
        views::wheel(app, side as usize, delta);
    });
    on!(on_grid_resized, app => |side, width, height| views::grid_resized(app, side as usize, width, height));
    on!(on_grid_range, app => |side, first, count| {
        views::grid_range(app, side as usize, first.max(0) as usize, count.max(0) as usize);
    });
    on!(on_thumbs_ready, app => | | views::thumbs_ready(app));
    on!(on_volumes_changed, app => | | volumes::changed(app));
    on!(on_tree_eject, app => |side, row| {
        nav::set_active_side(app, side as usize);
        let drive = {
            let a = app.borrow();
            let t = &a.sides[side as usize].tree;
            t.rows().get(row as usize).and_then(|&id| t.node(id).drive.clone()).filter(|d| d.ejectable)
        };
        if let Some(d) = drive {
            volumes::eject(app, d.path, d.name);
        }
    });
}

// ---- models ----

/// A message in the active side's status line, until the list or selection changes.
fn set_status(app: &Shared, text: String) {
    let side = app.borrow().active;
    app.borrow_mut().sides[side].status_message = Some(text);
    push_view(app, side);
}

/// Category to the icon numbers `FileIcon` in icons.slint draws.
fn icon_of(e: &FileEntry) -> i32 {
    match e.category() {
        Category::Folder => 0,
        Category::File => 1,
        Category::Image => 2,
        Category::Audio => 3,
        Category::Video => 4,
        Category::Archive => 5,
        Category::Pdf => 6,
        Category::Word => 7,
        Category::Spreadsheet => 8,
        Category::Presentation => 9,
        Category::Code => 10,
        Category::Text => 11,
        Category::Executable => 12,
        Category::Link => 13,
    }
}

fn file_row(tab: &Tab, row: usize, cols: &Columns, thumb: Option<slint::Image>) -> FileRow {
    let e = tab.row(row);
    // the extra columns cost per row (attributes look up the owner): only when shown
    let shown = |c: ColumnId| cols.is_visible(c);
    let text = |c: ColumnId, f: &dyn Fn() -> String| if shown(c) { SharedString::from(f()) } else { SharedString::new() };
    FileRow {
        name: e.name().into(),
        size: formats::size(e.size()).into(),
        kind: e.type_name().into(),
        modified: formats::date(e.modified()).into(),
        created: text(ColumnId::Created, &|| formats::date(e.created())),
        accessed: text(ColumnId::Accessed, &|| formats::date(e.accessed())),
        extension: text(ColumnId::Extension, &|| e.extension().to_string()),
        attributes: text(ColumnId::Attributes, &|| e.attributes().to_string()),
        icon: icon_of(e),
        selected: tab.is_selected(row),
        current: tab.current_row() == Some(row),
        hidden: e.is_hidden(),
        thumb: thumb.unwrap_or_default(),
    }
}

/// Rebuilds a side's file rows and crumbs (after a listing, sort, filter or tab change).
fn sync_files(app: &Shared, side: usize) {
    {
        let mut a = app.borrow_mut();
        let a = &mut *a;
        a.sides[side].status_message = None;
        let s = &a.sides[side];
        let t = &s.tabs[s.active];
        let (tab, mode, px) = (&t.tab, t.view.mode, t.view.icon_px());
        let thumbs = &mut a.thumbs;
        let rows: Vec<FileRow> = (0..tab.len())
            .map(|r| {
                let thumb = views::thumb_key(mode, px, tab.row(r)).and_then(|k| thumbs.get(&k));
                file_row(tab, r, &a.columns, thumb)
            })
            .collect();
        s.files.set_vec(rows);
        let names = crate::grid::names_to_measure((0..tab.len()).map(|r| tab.row(r).name()));
        let rough = names.first().map_or(0.0, |n| crate::grid::rough_width(n));
        let names: SharedString = names.iter().map(|n| n.replace(['\n', '\r'], " ")).collect::<Vec<_>>().join("\n").into();
        let crumbs: Vec<CrumbRow> = tab
            .dir()
            .map(address::crumbs)
            .unwrap_or_default()
            .into_iter()
            .map(|c| CrumbRow { label: c.label.into(), drive: c.drive })
            .collect();
        s.crumbs.set_vec(crumbs);
        a.sides[side].rough_width = rough;
        a.sides[side].measure_names = names;
    }
    push_view(app, side);
    sync_window(app);
    views::request_thumbs(app);
}

/// Updates only the selection flags of rows that changed; with `scroll`, the focused row is
/// scrolled into view.
fn sync_selection(app: &Shared, side: usize, scroll: bool) {
    {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        s.status_message = None;
        let current = s.tab().current_row();
        for r in 0..s.files.row_count() {
            let Some(mut row) = s.files.row_data(r) else { continue };
            let (sel, cur) = (s.tab().is_selected(r), current == Some(r));
            if row.selected != sel || row.current != cur {
                row.selected = sel;
                row.current = cur;
                s.files.set_row_data(r, row);
            }
        }
        if let (true, Some(r)) = (scroll, current) {
            s.ensure_row = r as i32;
            s.ensure_seq += 1;
        }
    }
    push_view(app, side);
}

/// Scrolls the focused row of `side` into view.
fn ensure_visible(app: &Shared, side: usize) {
    {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        if let Some(r) = s.tab().current_row() {
            s.ensure_row = r as i32;
            s.ensure_seq += 1;
        }
    }
    push_view(app, side);
}

fn sync_tree(app: &Shared, side: usize) {
    let a = app.borrow();
    let t = &a.sides[side].tree;
    let rows: Vec<TreeRow> = t
        .rows()
        .iter()
        .map(|&id| {
            let n = t.node(id);
            TreeRow {
                name: n.name.clone().into(),
                depth: n.depth as i32,
                header: n.kind.is_header(),
                expandable: t.expandable(id),
                expanded: n.expanded,
                current: t.current() == Some(id),
                on_path: t.on_path(id),
                draggable: n.path.is_some() && matches!(n.kind, NodeKind::Folder | NodeKind::Quick),
                extra: n.drive.as_ref().and_then(|d| d.extra.clone()).unwrap_or_default().into(),
                ejectable: n.drive.as_ref().is_some_and(|d| d.ejectable),
                ejecting: n.path.as_ref().is_some_and(|p| a.ejecting.contains(p)),
                icon: match n.kind {
                    NodeKind::Quick if n.depth == 0 && n.name == "Home" => 20,
                    NodeKind::Volume => match n.drive.as_ref().map(|d| d.kind) {
                        Some(fileem_core::fs::volumes::Kind::Removable) => 24,
                        Some(fileem_core::fs::volumes::Kind::Network) => 25,
                        _ => 21,
                    },
                    NodeKind::Bookmark if n.depth == 0 => 22,
                    _ => 0,
                },
            }
        })
        .collect();
    a.sides[side].tree_rows.set_vec(rows);
}

fn sync_tabs(app: &Shared, side: usize) {
    {
        let a = app.borrow();
        let s = &a.sides[side];
        let rows: Vec<TabRow> = s
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabRow {
                title: t.tab.dir().map(fs_util::display_name).unwrap_or_else(|| "…".into()).into(),
                active: i == s.active,
            })
            .collect();
        s.tab_rows.set_vec(rows);
    }
    push_view(app, side);
}

/// Hands a side's state to its `SideView`.
fn push_view(app: &Shared, side: usize) {
    let a = app.borrow();
    let Some(ui) = a.ui.upgrade() else { return };
    let s = &a.sides[side];
    let tab = s.tab();
    let (key, asc) = tab.sort();
    let sort_col = ColumnId::ALL.iter().find(|c| c.sort_key() == Some(key)).map_or(0, |c| c.index());
    let message = match (tab.is_empty() && tab.dir().is_some(), tab.filter().trim().is_empty()) {
        (false, _) => "",
        (true, true) => "This folder is empty.",
        (true, false) => "No items match the filter.",
    };
    let v = &s.tabs[s.active].view;
    let g = views::grid_of(s);
    let wanted = g.map_or(0, |g| g.slot_count());
    while s.slots.row_count() < wanted {
        s.slots.push(s.slots.row_count() as i32);
    }
    while s.slots.row_count() > wanted {
        s.slots.remove(s.slots.row_count() - 1);
    }
    let view = SideView {
        view_mode: v.mode.index() as i32,
        icon_size: v.icon_px() as f32,
        cell_w: g.map_or(0.0, |g| g.cell_w),
        cell_h: g.map_or(0.0, |g| g.cell_h),
        lanes: g.map_or(1, |g| g.lanes as i32),
        content_len: g.map_or(0.0, |g| g.content_len()),
        flows_columns: v.mode.flows_in_columns(),
        text_below: v.mode.text_below_icon(),
        resizable: v.mode.resizable(),
        size_slider: v.slider() as f32,
        can_smaller: v.can_resize(-1),
        can_bigger: v.can_resize(1),
        sort_tooltip: format!("Sort by {}, {}", key.title(), if asc { "ascending" } else { "descending" }).into(),
        files: ModelRc::from(s.files.clone()),
        slots: ModelRc::from(s.slots.clone()),
        measure_names: s.measure_names.clone(),
        list_files: ModelRc::from(if g.is_none() { s.files.clone() } else { s.no_files.clone() }),
        tree: ModelRc::from(s.tree_rows.clone()),
        tabs: ModelRc::from(s.tab_rows.clone()),
        crumbs: ModelRc::from(s.crumbs.clone()),
        address: tab.dir().map(|d| d.display().to_string()).unwrap_or_default().into(),
        status: s.status_message.clone().unwrap_or_else(|| tab.status()).into(),
        message: message.into(),
        filter: tab.filter().into(),
        sort_column: sort_col as i32,
        sort_ascending: asc,
        active: side == a.active,
        tree_width: s.tree_width,
        ensure_row: s.ensure_row,
        ensure_seq: s.ensure_seq,
        top_seq: s.top_seq,
        edit_seq: s.edit_seq,
        filter_seq: s.filter_seq,
        tree_row: s.tree_row,
        tree_seq: s.tree_seq,
        tree_menu_row: s.tree_menu_row,
    };
    if side == 0 {
        ui.set_left(view);
    } else {
        ui.set_right(view);
    }
    // the preview follows the active tab's selection
    let active = side == a.active;
    drop(a);
    if active {
        preview::selection_changed(app);
    }
}

/// Toolbar state and title, from the active tab.
fn sync_window(app: &Shared) {
    let a = app.borrow();
    let Some(ui) = a.ui.upgrade() else { return };
    let tab = a.tab();
    ui.set_dual(a.dual);
    ui.set_active_side(a.active as i32);
    ui.set_show_hidden(a.show_hidden);
    ui.set_can_back(tab.back_target().is_some());
    ui.set_can_forward(tab.forward_target().is_some());
    ui.set_can_up(tab.up_target().is_some());
    ui.set_bookmarked(tab.dir().is_some_and(|d| a.bookmarks.contains(d)));
    let title = tab.dir().map(|d| format!("{} - fileEm", fs_util::display_name(d))).unwrap_or_else(|| "fileEm".into());
    ui.set_window_title(title.into());
}

/// The visible columns (shared by both sides).
fn push_columns(app: &Shared) {
    let a = app.borrow();
    let Some(ui) = a.ui.upgrade() else { return };
    let mut name_width = 300.0;
    let mut rows = Vec::new();
    for c in a.columns.visible() {
        if c.id == ColumnId::Name {
            name_width = c.width;
        } else {
            rows.push(ColumnRow { col: c.id.index() as i32, title: c.id.title().into(), width: c.width, align_right: c.id == ColumnId::Size });
        }
    }
    let total = name_width + rows.iter().map(|r| r.width).sum::<f32>();
    ui.set_name_width(name_width);
    ui.set_columns_width(total);
    ui.set_columns(ModelRc::from(Rc::new(VecModel::from(rows))));
}
