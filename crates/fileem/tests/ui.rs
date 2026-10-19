//! End-to-end UI tests on Slint's headless testing backend: the real window, real keys and
//! clicks, real background listings. The backend can only be initialized once per process, so
//! this file has a single `#[test]` that runs each scenario with a fresh window and folder.
//! Nothing is ever launched (`FILEEM_NO_LAUNCH`): opened files are only recorded.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use std::rc::Rc;

use fileem::{app, shell, AppWindow, FileRow, ProgressWindow, PropertiesWindow, SideView, TreeRow};
use fileem_core::ops::{Decision, FileOperation, Kind, Resolution};
use fileem_core::session::Session;
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, SharedString};

struct Fixture {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    ui: AppWindow,
    app: app::Shared,
}

/// A folder with two subfolders, files of different sizes, a hidden file, and a window on it.
fn open_window() -> Fixture {
    open_window_with(|_, _| {}, true)
}

/// Like [`open_window`], with a session (in the temp folder: never the user's) prepared by
/// `prepare`; `start_there` opens the test folder, else the window starts where the session says.
fn open_window_with(prepare: impl FnOnce(&Session, &Path), start_there: bool) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("files");
    fs::create_dir(&dir).unwrap();
    fs::create_dir(dir.join("alpha")).unwrap();
    fs::create_dir(dir.join("beta")).unwrap();
    fs::write(dir.join("alpha/inside.txt"), "i").unwrap();
    fs::write(dir.join("notes.txt"), "1234567890").unwrap();
    fs::write(dir.join("big.bin"), vec![0u8; 5000]).unwrap();
    fs::write(dir.join("a.png"), "png").unwrap();
    fs::write(dir.join(".hidden"), "h").unwrap();
    let session = Rc::new(Session::load(&tmp.path().join("session.properties")));
    prepare(&session, &dir);
    let ui = AppWindow::new().unwrap();
    let app = app::install(&ui, session, start_there.then(|| dir.clone()));
    // never the user's clipboard
    app::set_clipboard(&app, Box::new(fileem::clipboard::Memory::default()));
    // a real size: the list views only create the rows that fit
    ui.window().set_size(slint::LogicalSize::new(1100.0, 700.0));
    ui.show().unwrap();
    let f = Fixture { _tmp: tmp, dir, ui, app };
    if start_there {
        wait_until(&f, "the folder is listed", |f| f.names().len() == 5);
    }
    f
}

impl Fixture {
    /// The active side's view.
    fn view(&self) -> SideView {
        if app::active_side(&self.app) == 1 {
            self.ui.get_right()
        } else {
            self.ui.get_left()
        }
    }

    fn rows(&self) -> Vec<FileRow> {
        self.view().files.iter().collect()
    }

    fn names(&self) -> Vec<String> {
        self.rows().iter().map(|r| r.name.to_string()).collect()
    }

    fn selected(&self) -> Vec<String> {
        self.rows().iter().filter(|r| r.selected).map(|r| r.name.to_string()).collect()
    }

    fn current(&self) -> Option<String> {
        self.rows().iter().find(|r| r.current).map(|r| r.name.to_string())
    }

    fn tree(&self) -> Vec<TreeRow> {
        self.view().tree.iter().collect()
    }

    fn address(&self) -> PathBuf {
        PathBuf::from(self.view().address.as_str())
    }

    fn key(&self, text: impl Into<SharedString>) {
        let text = text.into();
        self.ui.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.ui.window().dispatch_event(WindowEvent::KeyReleased { text });
    }

    /// A key with modifiers held (Control, Alt or Shift), as a user presses them.
    fn chord(&self, modifiers: &[Key], key: impl Into<SharedString>) {
        for m in modifiers {
            self.ui.window().dispatch_event(WindowEvent::KeyPressed { text: (*m).into() });
        }
        self.key(key);
        for m in modifiers.iter().rev() {
            self.ui.window().dispatch_event(WindowEvent::KeyReleased { text: (*m).into() });
        }
    }

    fn element(&self, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(&self.ui, label)
            .next()
            .unwrap_or_else(|| panic!("no element labelled {label:?}"))
    }

    /// A button by its text (a dialog's title may name the same thing).
    fn button(&self, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(&self.ui, label)
            .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
            .unwrap_or_else(|| panic!("no button labelled {label:?}"))
    }

    /// A click near the element's left end (rows are as wide as all columns, which can reach
    /// past the side they are in).
    fn click_at(&self, e: &ElementHandle) {
        let (pos, size) = (e.absolute_position(), e.size());
        let position = LogicalPosition::new(pos.x + (size.width / 2.0).min(30.0), pos.y + size.height / 2.0);
        let w = self.ui.window();
        w.dispatch_event(WindowEvent::PointerMoved { position });
        w.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
        w.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
    }

    fn click(&self, label: &str) {
        self.click_at(&self.element(label));
    }

    fn right_click_at(&self, position: LogicalPosition) {
        let w = self.ui.window();
        w.dispatch_event(WindowEvent::PointerMoved { position });
        w.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Right });
        w.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Right });
        pump();
    }

    fn right_click(&self, e: &ElementHandle) {
        let (pos, size) = (e.absolute_position(), e.size());
        self.right_click_at(LogicalPosition::new(pos.x + (size.width / 2.0).min(30.0), pos.y + size.height / 2.0));
    }

    /// A row of the file list (the tree may show the same name).
    fn list_row(&self, name: &str) -> ElementHandle {
        let tree_width = self.ui.get_left().tree_width;
        ElementHandle::find_by_accessible_label(&self.ui, name)
            .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem) && e.absolute_position().x > tree_width)
            .unwrap_or_else(|| panic!("no list row {name:?}"))
    }

    fn tree_row(&self, name: &str) -> ElementHandle {
        let tree_width = self.ui.get_left().tree_width;
        ElementHandle::find_by_accessible_label(&self.ui, name)
            .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem) && e.absolute_position().x < tree_width)
            .unwrap_or_else(|| panic!("no tree row {name:?}"))
    }

    /// Scrolls the left tree to its top with the mouse wheel.
    fn scroll_tree_to_top(&self) {
        let position = LogicalPosition::new(50.0, 300.0);
        let w = self.ui.window();
        w.dispatch_event(WindowEvent::PointerMoved { position });
        w.dispatch_event(WindowEvent::PointerScrolled { position, delta_x: 0.0, delta_y: 100_000.0 });
        pump();
    }

    /// An item of the open menu, by its full title.
    fn menu_item(&self, title: &str) -> Option<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.ui, title).find(|e| e.size().width > 0.0)
    }

    /// Clicks a menu item (a submenu opens; an item runs and closes the menu).
    fn pick(&self, title: &str) {
        let item = self.menu_item(title).unwrap_or_else(|| panic!("no menu item {title:?}"));
        // positions inside a popup are relative to it; the newest popup is the innermost menu
        let offset = {
            let inner = i_slint_core::window::WindowInner::from_pub(self.ui.window());
            let popups = inner.active_popups();
            match popups.last().map(|p| &p.location) {
                Some(i_slint_core::window::PopupWindowLocation::ChildWindow(at)) => (at.x, at.y),
                _ => (0.0, 0.0),
            }
        };
        let (pos, size) = (item.absolute_position(), item.size());
        let position = LogicalPosition::new(offset.0 + pos.x + size.width / 2.0, offset.1 + pos.y + size.height / 2.0);
        let w = self.ui.window();
        w.dispatch_event(WindowEvent::PointerMoved { position });
        w.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
        w.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        pump();
    }

    /// A tab of the tab bars by its title.
    fn tab_button(&self, title: &str, side: usize) -> ElementHandle {
        let split = if app::is_dual(&self.app) { self.ui.window().size().width as f32 / self.ui.window().scale_factor() / 2.0 } else { f32::MAX };
        ElementHandle::find_by_accessible_label(&self.ui, title)
            .filter(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Tab))
            .find(|e| (e.absolute_position().x < split) == (side == 0))
            .unwrap_or_else(|| panic!("no tab {title:?} on side {side}"))
    }

    fn type_text(&self, text: &str) {
        for c in text.chars() {
            self.key(c.to_string());
        }
    }

    fn property(&self, w: &PropertiesWindow, label: &str) -> String {
        w.get_lines().iter().find(|l| l.label == label).map(|l| l.value.to_string()).unwrap_or_default()
    }

    /// A point on a list row's name (where a press drags files).
    fn on_name(&self, name: &str) -> LogicalPosition {
        let e = self.list_row(name);
        LogicalPosition::new(e.absolute_position().x + 30.0, e.absolute_position().y + e.size().height / 2.0)
    }

    /// A point on a list row past its name, in the Size column (where a press starts a box).
    fn on_size(&self, name: &str) -> LogicalPosition {
        let e = self.list_row(name);
        let x = e.absolute_position().x + self.ui.get_name_width() + 20.0;
        LogicalPosition::new(x, e.absolute_position().y + e.size().height / 2.0)
    }

    fn center(&self, e: &ElementHandle) -> LogicalPosition {
        let (pos, size) = (e.absolute_position(), e.size());
        LogicalPosition::new(pos.x + (size.width / 2.0).min(30.0), pos.y + size.height / 2.0)
    }

    /// Presses at `from` and moves to `to` in steps, the button still down; `modifiers` are held
    /// during the press.
    fn drag_to(&self, from: LogicalPosition, to: LogicalPosition, modifiers: &[Key]) {
        let w = self.ui.window();
        w.dispatch_event(WindowEvent::PointerMoved { position: from });
        for m in modifiers {
            w.dispatch_event(WindowEvent::KeyPressed { text: (*m).into() });
        }
        w.dispatch_event(WindowEvent::PointerPressed { position: from, button: PointerEventButton::Left });
        for m in modifiers.iter().rev() {
            w.dispatch_event(WindowEvent::KeyReleased { text: (*m).into() });
        }
        // a list holds a press back for 100 ms (it might become a scroll); people hold that long
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(120) {
            pump();
        }
        for step in 1..=10 {
            let t = step as f32 / 10.0;
            let position = LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            w.dispatch_event(WindowEvent::PointerMoved { position });
        }
    }

    fn release(&self, at: LogicalPosition) {
        self.ui.window().dispatch_event(WindowEvent::PointerReleased { position: at, button: PointerEventButton::Left });
        pump();
    }

    /// A whole drag and drop.
    fn drag_drop(&self, from: LogicalPosition, to: LogicalPosition) {
        self.drag_to(from, to, &[]);
        self.release(to);
    }

    fn double_click(&self, label: &str) {
        let e = self.element(label);
        self.click_at(&e);
        self.click_at(&e);
    }
}

/// Runs the event loop for a moment, so background results (delivered through it) arrive.
fn pump() {
    slint::Timer::single_shot(Duration::from_millis(5), || {
        let _ = slint::quit_event_loop();
    });
    slint::run_event_loop_until_quit().unwrap();
}

/// Pumps events until `cond` holds; fails after 10 s. Never sleeps blindly.
fn wait_until(f: &Fixture, what: &str, cond: impl Fn(&Fixture) -> bool) {
    let start = Instant::now();
    while !cond(f) {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting until {what}");
        pump();
    }
}

/// Lets queued events (background results) settle when nothing visible should change.
fn settle(f: &Fixture) {
    let start = Instant::now();
    wait_until(f, "settled", move |_| start.elapsed() > Duration::from_millis(150));
}

#[test]
fn ui() {
    std::env::set_var("FILEEM_NO_LAUNCH", "1");
    i_slint_backend_testing::init_integration_test_with_system_time();

    // each scenario gets a fresh window and folder
    lists_folders_first_with_status_and_no_hidden_files();
    double_click_opens_folders_and_backspace_goes_up_selecting_the_child();
    alt_arrows_go_back_and_forward();
    keyboard_selection_and_enter_opens_files_without_launching();
    ctrl_h_and_the_toolbar_toggle_hidden_files();
    clicking_a_column_header_sorts_and_again_reverses();
    typing_jumps_to_a_name();
    click_ctrl_click_and_shift_click_select();
    the_address_bar_navigates_and_reports_bad_paths();
    the_tree_reveals_the_current_folder_and_navigates_on_click();
    the_session_restores_hidden_files_sort_and_the_last_folder();
    a_vanished_last_folder_opens_its_nearest_parent();
    saving_the_session_keeps_what_only_the_java_app_uses();
    changes_made_elsewhere_show_up_and_keep_the_selection();
    a_new_subfolder_shows_up_in_the_tree();
    when_the_shown_folder_is_deleted_its_parent_opens();
    delete_and_f8_ask_first_and_cancel_keeps_the_files();
    shift_delete_deletes_permanently_after_confirming();
    f2_renames_and_refuses_taken_names();
    f7_creates_a_folder_under_a_free_name();
    a_conflict_is_asked_in_the_progress_row();
    a_slow_operation_gets_a_row_that_pauses_and_cancels();
    operation_errors_are_listed_afterwards();
    closing_with_operations_running_asks_first();
    copy_and_paste_into_a_subfolder();
    cut_and_paste_moves_once();
    pasting_into_the_same_folder_makes_a_copy();
    right_click_selects_the_row_and_opens_its_menu();
    right_click_on_a_selected_row_keeps_the_selection();
    right_click_on_empty_space_offers_the_folder_menu();
    right_click_in_the_tree_opens_a_menu_without_navigating();
    alt_enter_and_the_menu_show_properties();
    new_text_file_from_the_menu();
    duplicate_compress_and_extract_from_the_menu();
    paste_into_a_folder_from_its_menu();
    ctrl_t_opens_and_ctrl_w_closes_tabs_but_never_the_last();
    the_tab_bar_switches_adds_and_closes_tabs();
    f9_opens_the_second_side_and_tab_switches();
    f5_copies_and_f6_moves_to_the_other_side();
    copy_and_move_to_the_other_side_need_the_second_side();
    the_filter_box_narrows_the_list_and_escape_clears_it();
    ctrl_l_edits_the_address_and_a_file_path_selects_the_file();
    crumbs_show_names_as_they_are_and_navigate();
    the_header_menu_shows_more_columns_and_they_sort();
    the_session_restores_tabs_and_the_second_side();
    renaming_a_folder_moves_the_tabs_inside_it();
    both_lists_stay_usable_in_a_narrow_dual_window();
    dragging_a_file_onto_a_folder_row_asks_then_moves();
    choosing_copy_after_a_drop_onto_the_tree_keeps_the_original();
    cancelling_the_drop_question_leaves_everything_alone();
    drops_into_the_dragged_items_or_where_they_are_are_refused();
    dragging_to_the_other_side_drops_into_its_folder();
    a_press_on_a_selection_drags_all_of_it_and_a_click_selects_one();
    a_tree_folder_can_be_dragged_into_the_list();
    dragging_from_the_size_column_draws_a_box_that_selects_rows();
    a_box_from_empty_space_selects_upwards_and_shift_adds();
    ctrl_box_toggles_and_a_box_below_the_list_scrolls_it();
    showing_the_second_side_keeps_the_left_scroll_position();
}

fn lists_folders_first_with_status_and_no_hidden_files() {
    let f = open_window();
    assert_eq!(f.names(), ["alpha", "beta", "a.png", "big.bin", "notes.txt"]);
    assert_eq!(f.view().status, "5 items (1 hidden)");
    assert_eq!(f.address(), f.dir);
    assert_eq!(f.current().as_deref(), Some("alpha"), "the first row has the focus");
    let png = &f.rows()[2];
    assert_eq!((png.kind.as_str(), png.icon), ("PNG Image", 2));
    assert_eq!(f.rows()[3].size, "4.88 KB");
}

fn double_click_opens_folders_and_backspace_goes_up_selecting_the_child() {
    let f = open_window();
    f.double_click("alpha");
    wait_until(&f, "alpha is open", |f| f.names() == ["inside.txt"]);
    assert_eq!(f.address(), f.dir.join("alpha"));
    assert!(f.ui.get_can_back());

    f.key(Key::Backspace);
    wait_until(&f, "back up", |f| f.names().len() == 5);
    assert_eq!(f.address(), f.dir);
    assert_eq!(f.selected(), ["alpha"], "the folder we came from is selected");
}

fn alt_arrows_go_back_and_forward() {
    let f = open_window();
    f.double_click("beta");
    wait_until(&f, "beta is open", |f| f.address() == f.dir.join("beta"));
    assert_eq!(f.view().message, "This folder is empty.");
    f.chord(&[Key::Alt], Key::LeftArrow);
    wait_until(&f, "back", |f| f.address() == f.dir);
    assert!(f.ui.get_can_forward());
    f.chord(&[Key::Alt], Key::RightArrow);
    wait_until(&f, "forward", |f| f.address() == f.dir.join("beta"));
    f.chord(&[Key::Alt], Key::UpArrow);
    wait_until(&f, "up", |f| f.address() == f.dir);
}

fn keyboard_selection_and_enter_opens_files_without_launching() {
    let f = open_window();
    f.key(Key::End);
    assert_eq!(f.selected(), ["notes.txt"]);
    f.chord(&[Key::Shift], Key::UpArrow);
    assert_eq!(f.selected(), ["big.bin", "notes.txt"]);
    assert_eq!(f.view().status, "5 items (1 hidden), 2 selected (4.89 KB)");
    f.key(Key::Home);
    assert_eq!(f.selected(), ["alpha"]);
    f.chord(&[Key::Control], "a");
    assert_eq!(f.selected().len(), 5);

    f.key(Key::End);
    f.key(Key::Return);
    settle(&f);
    assert!(shell::launched().contains(&format!("open {}", f.dir.join("notes.txt").display())), "{:?}", shell::launched());
    assert_eq!(f.address(), f.dir, "opening a file doesn't navigate");
}

fn ctrl_h_and_the_toolbar_toggle_hidden_files() {
    let f = open_window();
    f.chord(&[Key::Control], "h");
    assert_eq!(f.names().len(), 6);
    assert!(f.rows().iter().any(|r| r.name == ".hidden" && r.hidden), "hidden rows are dimmed");
    assert!(f.ui.get_show_hidden());
    f.element("Show hidden files (Ctrl+H)").invoke_accessible_default_action();
    assert_eq!(f.names().len(), 5);
}

fn clicking_a_column_header_sorts_and_again_reverses() {
    let f = open_window();
    f.click("Size");
    assert_eq!(f.names(), ["alpha", "beta", "a.png", "notes.txt", "big.bin"]);
    f.click("Size");
    assert_eq!(f.names(), ["beta", "alpha", "big.bin", "notes.txt", "a.png"], "folders stay first");
    assert!(!f.view().sort_ascending);
}

fn typing_jumps_to_a_name() {
    let f = open_window();
    f.key("n");
    assert_eq!(f.selected(), ["notes.txt"]);
    assert_eq!(f.current().as_deref(), Some("notes.txt"));
}

fn click_ctrl_click_and_shift_click_select() {
    let f = open_window();
    f.click("beta");
    assert_eq!(f.selected(), ["beta"]);
    f.ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    f.click("big.bin");
    f.ui.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    assert_eq!(f.selected(), ["beta", "big.bin"]);
    f.ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Shift.into() });
    f.click("notes.txt");
    f.ui.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Shift.into() });
    assert_eq!(f.selected(), ["big.bin", "notes.txt"], "shift selects from the last clicked row");
    settle(&f);
    assert_eq!(f.address(), f.dir, "single clicks don't open anything");
}

fn the_address_bar_navigates_and_reports_bad_paths() {
    let f = open_window();
    f.ui.invoke_address_entered(0, f.dir.join("alpha").display().to_string().into());
    wait_until(&f, "alpha", |f| f.names() == ["inside.txt"]);
    let missing = f.dir.join("missing");
    f.ui.invoke_address_entered(0, missing.display().to_string().into());
    wait_until(&f, "the error", |f| f.view().status.contains("Not found"));
    assert_eq!(f.address(), f.dir.join("alpha"), "the address bar returns to the folder still shown");
    assert_eq!(f.names(), ["inside.txt"]);
}

fn the_tree_reveals_the_current_folder_and_navigates_on_click() {
    let f = open_window();
    let name = f.dir.file_name().unwrap().to_string_lossy().into_owned();
    wait_until(&f, "the tree shows the folder", |f| f.tree().iter().any(|r| r.current));
    let current: Vec<TreeRow> = f.tree().into_iter().filter(|r| r.current).collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].name.as_str(), name);
    assert!(current[0].on_path);
    // its parents are expanded and bold; its subfolders aren't loaded until asked
    let parent = f.dir.parent().and_then(Path::file_name).unwrap().to_string_lossy().into_owned();
    assert!(f.tree().iter().any(|r| r.name.as_str() == parent && r.expanded && r.on_path));

    // expanding it shows its subfolders, a click on one navigates
    let row = f.tree().iter().position(|r| r.current).unwrap();
    f.ui.invoke_tree_toggled(0, row as i32);
    wait_until(&f, "subfolders", |f| f.tree().iter().any(|r| r.name == "alpha"));
    let alpha = f.tree().iter().position(|r| r.name == "alpha").unwrap();
    f.ui.invoke_tree_clicked(0, alpha as i32);
    wait_until(&f, "alpha", |f| f.address() == f.dir.join("alpha"));
    wait_until(&f, "alpha is current in the tree", |f| f.tree().iter().any(|r| r.current && r.name == "alpha"));
}

fn the_session_restores_hidden_files_sort_and_the_last_folder() {
    let f = open_window_with(
        |s, dir| {
            s.put("showHidden", true);
            s.put("sort.column", "size");
            s.put("sort.ascending", false);
            s.put("left.treeWidth", 300);
            s.put("left.tabs", 2);
            s.put("left.selected", 1);
            s.put("left.tab.0", "/elsewhere");
            s.put("left.tab.1", dir.display());
        },
        false,
    );
    wait_until(&f, "the last folder with hidden files", |f| f.names().len() == 6);
    assert_eq!(f.address(), f.dir, "the selected tab's folder");
    assert!(f.ui.get_show_hidden());
    assert_eq!(f.view().sort_column, 1);
    assert!(!f.view().sort_ascending);
    assert_eq!(f.names()[2], "big.bin", "largest file first");
    assert_eq!(f.view().tree_width, 300.0);
}

fn a_vanished_last_folder_opens_its_nearest_parent() {
    let f = open_window_with(|s, dir| s.put("left.tab.0", dir.join("alpha/was-here/deeper").display()), false);
    wait_until(&f, "the nearest existing parent", |f| f.address() == f.dir.join("alpha"));
    assert_eq!(f.names(), ["inside.txt"]);
}

fn saving_the_session_keeps_what_only_the_java_app_uses() {
    let f = open_window_with(
        |s, dir| {
            s.put("left.tabs", 3);
            s.put("left.selected", 1);
            s.put("left.tab.0", dir.join("beta").display());
            s.put("left.tab.0.view", "large-icons");
            s.put("left.tab.0.size", "large-icons:2");
            s.put("left.tab.1", dir.display());
            s.put("left.tab.2", dir.display());
            s.put("right.tabs", 1);
            s.put("right.tab.0", dir.join("alpha").display());
            s.put("columns.size.width", 120);
            s.put("theme.name", "aurora");
            s.put("preview.enabled", true);
        },
        false,
    );
    wait_until(&f, "the selected tab", |f| f.names().len() == 5);
    f.double_click("alpha");
    wait_until(&f, "alpha", |f| f.address() == f.dir.join("alpha"));
    f.chord(&[Key::Control], "h");
    app::collect_session(&f.app);
    app::session(&f.app).save();

    let saved = Session::load(app::session(&f.app).file());
    assert_eq!(saved.get("left.tab.1").map(PathBuf::from), Some(f.dir.join("alpha")), "the shown tab's folder");
    assert!(saved.get_bool("showHidden", false));
    let dir = |p: &Path| p.display().to_string();
    for (k, v) in [
        ("left.tabs", "3".to_string()),
        ("left.selected", "1".to_string()),
        ("left.tab.0", dir(&f.dir.join("beta"))),
        ("left.tab.0.view", "large-icons".to_string()),
        ("left.tab.0.size", "large-icons:2".to_string()),
        ("left.tab.2", dir(&f.dir)),
        ("right.tab.0", dir(&f.dir.join("alpha"))),
        ("columns.size.width", "120".to_string()),
        ("theme.name", "aurora".to_string()),
        ("preview.enabled", "true".to_string()),
    ] {
        assert_eq!(saved.get(k).as_deref(), Some(v.as_str()), "{k} kept");
    }
    assert!(saved.get_f64("window.w", 0.0) > 0.0);
}

fn changes_made_elsewhere_show_up_and_keep_the_selection() {
    let f = open_window();
    f.click("notes.txt");
    fs::write(f.dir.join("added.txt"), "new").unwrap();
    fs::remove_file(f.dir.join("a.png")).unwrap();
    wait_until(&f, "the watcher refreshed", |f| f.names().contains(&"added.txt".to_string()));
    assert!(!f.names().contains(&"a.png".to_string()));
    assert_eq!(f.selected(), ["notes.txt"], "the selection survives a refresh");
    assert_eq!(f.view().status, "5 items (1 hidden), 1 selected (10 bytes)");
}

fn a_new_subfolder_shows_up_in_the_tree() {
    let f = open_window();
    wait_until(&f, "the tree shows the folder", |f| f.tree().iter().any(|r| r.current));
    let row = f.tree().iter().position(|r| r.current).unwrap();
    f.ui.invoke_tree_toggled(0, row as i32);
    wait_until(&f, "subfolders", |f| f.tree().iter().any(|r| r.name == "alpha"));
    fs::create_dir(f.dir.join("gamma")).unwrap();
    wait_until(&f, "the new folder in the tree", |f| f.tree().iter().any(|r| r.name == "gamma"));
    wait_until(&f, "and in the list", |f| f.names().contains(&"gamma".to_string()));
}

fn when_the_shown_folder_is_deleted_its_parent_opens() {
    let f = open_window();
    f.double_click("alpha");
    wait_until(&f, "alpha", |f| f.address() == f.dir.join("alpha"));
    fs::remove_dir_all(f.dir.join("alpha")).unwrap();
    wait_until(&f, "the parent", |f| f.address() == f.dir);
    assert!(!f.names().contains(&"alpha".to_string()));
}

// ---- operations and dialogs ----

fn dialog_open(f: &Fixture) -> bool {
    f.ui.get_dialog_open()
}

fn progress(f: &Fixture) -> Option<ProgressWindow> {
    app::ops(&f.app).window()
}

fn progress_button(w: &ProgressWindow, label: &str, nth: usize) -> ElementHandle {
    ElementHandle::find_by_accessible_label(w, label)
        .filter(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
        .nth(nth)
        .unwrap_or_else(|| panic!("no {label:?} button"))
}

fn no_temp_files(dir: &Path) -> bool {
    fs::read_dir(dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".fileem-"))
}

fn delete_and_f8_ask_first_and_cancel_keeps_the_files() {
    let f = open_window();
    f.click("notes.txt");
    for key in [Key::Delete, Key::F8] {
        f.key(key);
        assert!(dialog_open(&f));
        assert_eq!(f.ui.get_dialog_title(), "Move to Trash");
        assert_eq!(f.ui.get_dialog_message(), "Move \u{201c}notes.txt\u{201d} to the Trash?");
        f.key(Key::Escape);
        assert!(!dialog_open(&f), "Escape cancels");
    }
    settle(&f);
    assert!(f.dir.join("notes.txt").exists(), "nothing was trashed");
}

fn shift_delete_deletes_permanently_after_confirming() {
    let f = open_window();
    f.click("big.bin");
    f.ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    f.click("notes.txt");
    f.ui.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    f.chord(&[Key::Shift], Key::Delete);
    assert_eq!(f.ui.get_dialog_title(), "Delete permanently");
    assert!(f.ui.get_dialog_message().starts_with("Permanently delete 2 items?"));
    f.key(Key::Return);
    wait_until(&f, "the files are gone", |f| !f.names().contains(&"notes.txt".to_string()));
    assert!(!f.dir.join("big.bin").exists() && !f.dir.join("notes.txt").exists());
    assert!(!dialog_open(&f), "no problems to report");
}

fn f2_renames_and_refuses_taken_names() {
    let f = open_window();
    f.click("a.png");
    f.key(Key::F2);
    assert_eq!(f.ui.get_dialog_title(), "Rename");
    assert_eq!(f.ui.get_dialog_text(), "a.png");
    assert_eq!(f.ui.get_dialog_select_end(), 1, "the name without its extension is selected");

    f.ui.set_dialog_text("notes.txt".into());
    f.button("Rename").invoke_accessible_default_action();
    assert!(dialog_open(&f), "a taken name keeps the dialog open");
    assert!(f.ui.get_dialog_error().contains("already exists"));
    assert_eq!(fs::read_to_string(f.dir.join("notes.txt")).unwrap(), "1234567890", "nothing replaced");

    f.ui.set_dialog_text("b.png".into());
    f.button("Rename").invoke_accessible_default_action();
    assert!(!dialog_open(&f));
    wait_until(&f, "the renamed file", |f| f.names().contains(&"b.png".to_string()));
    assert_eq!(f.selected(), ["b.png"], "the renamed file is selected");
    assert!(!f.dir.join("a.png").exists());
}

fn f7_creates_a_folder_under_a_free_name() {
    let f = open_window();
    f.key(Key::F7);
    assert_eq!(f.ui.get_dialog_title(), "New folder");
    assert_eq!(f.ui.get_dialog_text(), "New folder");
    f.button("Create").invoke_accessible_default_action();
    wait_until(&f, "the new folder", |f| f.names().contains(&"New folder".to_string()));
    assert_eq!(f.selected(), ["New folder"]);
    assert!(f.dir.join("New folder").is_dir());

    f.chord(&[Key::Control, Key::Shift], "n");
    assert_eq!(f.ui.get_dialog_text(), "New folder (2)", "a free name is offered");
    f.ui.set_dialog_text("bad/name".into());
    f.button("Create").invoke_accessible_default_action();
    assert!(f.ui.get_dialog_error().contains("/"), "{}", f.ui.get_dialog_error());
    f.button("Cancel").invoke_accessible_default_action();
    assert!(!dialog_open(&f));
}

fn a_conflict_is_asked_in_the_progress_row() {
    let f = open_window();
    fs::write(f.dir.join("alpha/notes.txt"), "old").unwrap();
    app::run_operation(&f.app, Kind::Copy, vec![f.dir.join("notes.txt")], Some(f.dir.join("alpha")));
    wait_until(&f, "the question", |f| app::ops(&f.app).rows().iter().any(|r| r.asking));
    let row = &app::ops(&f.app).rows()[0];
    assert!(row.question.contains("already has a file named"), "{}", row.question);
    assert!(row.title.starts_with("Copying \u{201c}notes.txt\u{201d} to "));
    assert!(!dialog_open(&f), "never a modal dialog");

    let w = progress(&f).expect("the progress window is open");
    progress_button(&w, "Replace", 0).invoke_accessible_default_action();
    wait_until(&f, "replaced", |f| fs::read_to_string(f.dir.join("alpha/notes.txt")).unwrap() == "1234567890");
    wait_until(&f, "the window closes with its last row", |f| progress(f).is_none());
    assert!(no_temp_files(&f.dir.join("alpha")));
}

/// A copy that stays running: its conflict handler waits until the operation is cancelled.
fn blocking_copy(f: &Fixture) -> fileem_core::ops::Handle {
    fs::write(f.dir.join("alpha/notes.txt"), "old").unwrap();
    let mut op = FileOperation::new(Kind::Copy, vec![f.dir.join("notes.txt")], Some(f.dir.join("alpha")));
    let h = op.handle();
    op.on_conflict(move |_, _| {
        while !h.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        Decision::once(Resolution::Cancel)
    });
    app::ops(&f.app).start(op, |_| {})
}

fn a_slow_operation_gets_a_row_that_pauses_and_cancels() {
    let f = open_window();
    let h = blocking_copy(&f);
    assert!(progress(&f).is_none(), "quick operations never show up");
    wait_until(&f, "a row after 350 ms", |f| progress(f).is_some());
    let w = progress(&f).unwrap();
    assert_eq!(w.get_window_title(), app::ops(&f.app).rows()[0].title);

    progress_button(&w, "Pause", 0).invoke_accessible_default_action();
    assert!(h.is_paused());
    wait_until(&f, "the row says paused", |f| app::ops(&f.app).rows()[0].detail.starts_with("Paused"));
    progress_button(&w, "Resume", 0).invoke_accessible_default_action();
    assert!(!h.is_paused());

    progress_button(&w, "Cancel", 0).invoke_accessible_default_action();
    assert!(h.is_cancelled());
    wait_until(&f, "the row leaves", |f| progress(f).is_none());
    assert_eq!(fs::read_to_string(f.dir.join("alpha/notes.txt")).unwrap(), "old", "the existing file is intact");
    assert!(no_temp_files(&f.dir.join("alpha")));
}

fn operation_errors_are_listed_afterwards() {
    let f = open_window();
    app::run_operation(&f.app, Kind::Delete, vec![f.dir.join("missing.txt"), f.dir.join("a.png")], None);
    wait_until(&f, "the problems are listed", dialog_open);
    assert_eq!(f.ui.get_dialog_message(), "1 item could not be processed.");
    let lines: Vec<String> = f.ui.get_dialog_lines().iter().map(|l| l.to_string()).collect();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].ends_with("missing.txt: Not found"), "{lines:?}");
    assert!(!f.ui.get_dialog_can_cancel());
    f.key(Key::Return);
    assert!(!dialog_open(&f));
    wait_until(&f, "the other item was deleted", |f| !f.names().contains(&"a.png".to_string()));
}

fn closing_with_operations_running_asks_first() {
    let f = open_window();
    let h = blocking_copy(&f);
    assert!(!app::close_requested(&f.app), "the window stays open");
    assert_eq!(f.ui.get_dialog_title(), "Operations running");
    f.key(Key::Escape);
    assert!(!dialog_open(&f));
    assert!(!h.is_cancelled(), "saying no keeps the operation running");
    h.cancel();
    wait_until(&f, "stopped", |_| h.is_done());
    settle(&f);
    assert!(app::close_requested(&f.app), "nothing running: closes right away");
}

// ---- clipboard ----

fn copy_and_paste_into_a_subfolder() {
    let f = open_window();
    f.click("notes.txt");
    f.chord(&[Key::Control], "c");
    f.double_click("alpha");
    wait_until(&f, "alpha", |f| f.address() == f.dir.join("alpha"));
    f.chord(&[Key::Control], "v");
    wait_until(&f, "the pasted copy", |f| f.names() == ["inside.txt", "notes.txt"]);
    assert_eq!(fs::read_to_string(f.dir.join("alpha/notes.txt")).unwrap(), "1234567890");
    assert!(f.dir.join("notes.txt").exists(), "a copy keeps the original");
}

fn cut_and_paste_moves_once() {
    let f = open_window();
    f.click("a.png");
    f.chord(&[Key::Control], "x");
    assert!(f.dir.join("a.png").exists(), "cutting alone moves nothing");
    f.double_click("beta");
    wait_until(&f, "beta", |f| f.address() == f.dir.join("beta"));
    f.chord(&[Key::Control], "v");
    wait_until(&f, "the moved file", |f| f.names() == ["a.png"]);
    assert!(!f.dir.join("a.png").exists());
    f.chord(&[Key::Control], "v");
    settle(&f);
    assert_eq!(f.names(), ["a.png"], "the clipboard is empty after a move");
    assert!(!dialog_open(&f));
}

fn pasting_into_the_same_folder_makes_a_copy() {
    let f = open_window();
    f.click("notes.txt");
    f.chord(&[Key::Control], "c");
    f.chord(&[Key::Control], "v");
    wait_until(&f, "the copy", |f| f.names().contains(&"notes - Copy.txt".to_string()));
}

// ---- right-click menus and properties ----

fn right_click_selects_the_row_and_opens_its_menu() {
    let f = open_window();
    f.click("notes.txt");
    f.right_click(&f.list_row("a.png"));
    assert_eq!(f.selected(), ["a.png"], "an unselected row is selected alone");
    assert!(f.menu_item("Rename  (F2)").is_some(), "the item's menu");
    assert!(f.menu_item("Sort by").is_none(), "not the folder's menu");
    f.pick("Copy path");
    wait_until(&f, "the menu closed", |f| f.menu_item("Copy path").is_none());
    assert_eq!(app::clipboard_text(&f.app), Some(f.dir.join("a.png").display().to_string()));
}

fn right_click_on_a_selected_row_keeps_the_selection() {
    let f = open_window();
    f.click("notes.txt");
    f.ui.invoke_row_pressed(0, 3, true, false); // Ctrl+click big.bin
    f.right_click(&f.list_row("notes.txt"));
    assert_eq!(f.selected(), ["big.bin", "notes.txt"]);
    f.pick("Copy");
    wait_until(&f, "the menu closed", |f| f.menu_item("Copy path").is_none());
    let text = app::clipboard_text(&f.app).unwrap();
    assert_eq!(text.lines().count(), 2, "both items were copied: {text}");
}

fn right_click_on_empty_space_offers_the_folder_menu() {
    let f = open_window();
    f.click("notes.txt");
    let last = f.list_row("notes.txt");
    let below = LogicalPosition::new(last.absolute_position().x + 50.0, last.absolute_position().y + 150.0);
    f.right_click_at(below);
    assert!(f.selected().is_empty(), "the menu is about the folder");
    assert!(f.menu_item("Show hidden files").is_some() && f.menu_item("Refresh  (Ctrl+R)").is_some());
    f.pick("Sort by");
    wait_until(&f, "the sort submenu", |f| f.menu_item("Descending").is_some());
    f.pick("Descending");
    wait_until(&f, "sorted descending", |f| f.names() == ["beta", "alpha", "notes.txt", "big.bin", "a.png"]);
    assert!(!f.view().sort_ascending);
}

fn right_click_in_the_tree_opens_a_menu_without_navigating() {
    let f = open_window();
    let home = fileem_core::util::os::home();
    // the tree has revealed (and scrolled to) the current folder; Home is above it
    wait_until(&f, "the tree marks the current folder", |f| f.tree().iter().any(|r| r.current));
    settle(&f);
    f.scroll_tree_to_top();
    f.right_click(&f.tree_row("Home"));
    settle(&f);
    assert_eq!(f.address(), f.dir, "a right-click doesn't navigate");
    assert!(f.menu_item("Open in terminal").is_some());
    assert!(f.menu_item("Cut").is_none() && f.menu_item("Delete permanently").is_none(), "Home is a shortcut");
    f.pick("Open in terminal");
    let wanted = format!("terminal {}", home.display());
    wait_until(&f, "the terminal request", |_| shell::launched().contains(&wanted));
    assert_eq!(f.address(), f.dir);
}

fn alt_enter_and_the_menu_show_properties() {
    let f = open_window();
    f.click("notes.txt");
    f.chord(&[Key::Alt], Key::Return);
    wait_until(&f, "the properties window", |f| app::properties_windows(&f.app).len() == 1);
    let w = app::properties_windows(&f.app).remove(0);
    assert_eq!(w.get_window_title(), "notes.txt Properties");
    assert_eq!(f.property(&w, "Size:"), "10 bytes");
    assert_eq!(f.property(&w, "Location:"), f.dir.display().to_string());
    w.invoke_close_clicked();
    assert!(app::properties_windows(&f.app).is_empty());

    f.right_click(&f.list_row("alpha"));
    f.pick("Properties  (Alt+Enter)");
    wait_until(&f, "the folder counted", |f| {
        app::properties_windows(&f.app).first().is_some_and(|w| f.property(w, "Contains:") == "1 files, 0 folders")
    });
    let w = app::properties_windows(&f.app).remove(0);
    assert_eq!(f.property(&w, "Size:"), "1 byte");
    app::close_windows(&f.app);
    assert!(app::properties_windows(&f.app).is_empty());
}

fn new_text_file_from_the_menu() {
    let f = open_window();
    let last = f.list_row("notes.txt");
    f.right_click_at(LogicalPosition::new(last.absolute_position().x + 50.0, last.absolute_position().y + 150.0));
    f.pick("New");
    wait_until(&f, "the new submenu", |f| f.menu_item("Text file").is_some());
    f.pick("Text file");
    wait_until(&f, "the name dialog", dialog_open);
    assert_eq!(f.ui.get_dialog_text(), "New text file.txt");
    assert_eq!(f.ui.get_dialog_select_end(), 13, "the name before .txt is selected");
    f.ui.set_dialog_text("todo.txt".into());
    f.button("Create").invoke_accessible_default_action();
    wait_until(&f, "the new file", |f| f.selected() == ["todo.txt"]);
    assert_eq!(fs::read(f.dir.join("todo.txt")).unwrap(), b"", "an empty file");
}

fn duplicate_compress_and_extract_from_the_menu() {
    let f = open_window();
    f.right_click(&f.list_row("notes.txt"));
    f.pick("Duplicate");
    wait_until(&f, "the duplicate", |f| f.names().contains(&"notes - Copy.txt".to_string()));
    assert_eq!(fs::read_to_string(f.dir.join("notes - Copy.txt")).unwrap(), "1234567890");

    f.click("notes.txt");
    let big = f.names().iter().position(|n| n == "big.bin").unwrap();
    f.ui.invoke_row_pressed(0, big as i32, true, false);
    f.right_click(&f.list_row("notes.txt"));
    f.pick("Compress to ZIP");
    wait_until(&f, "the archive selected", |f| f.selected() == ["Archive.zip"]);

    f.right_click(&f.list_row("Archive.zip"));
    f.pick("Extract here");
    wait_until(&f, "the extracted folder selected", |f| f.selected() == ["Archive"]);
    assert_eq!(fs::read_to_string(f.dir.join("Archive/notes.txt")).unwrap(), "1234567890");
    assert_eq!(fs::read(f.dir.join("Archive/big.bin")).unwrap().len(), 5000);
}

fn paste_into_a_folder_from_its_menu() {
    let f = open_window();
    f.click("notes.txt");
    f.chord(&[Key::Control], "c");
    f.right_click(&f.list_row("beta"));
    f.pick("Paste into folder");
    wait_until(&f, "pasted into beta", |f| f.dir.join("beta/notes.txt").exists());
    assert_eq!(f.address(), f.dir, "the folder stays shown");
}

// ---- tabs, the second side, address bar, filter, columns ----

fn tab_dirs(f: &Fixture, side: usize) -> Vec<PathBuf> {
    app::tabs(&f.app, side).0.into_iter().map(|d| d.unwrap_or_default()).collect()
}

fn ctrl_t_opens_and_ctrl_w_closes_tabs_but_never_the_last() {
    let f = open_window();
    f.chord(&[Key::Control], "t");
    wait_until(&f, "a second tab, listed", |f| app::tabs(&f.app, 0).0.len() == 2 && f.names().len() == 5);
    assert_eq!(app::tabs(&f.app, 0).1, 1, "the new tab is selected");
    assert_eq!(tab_dirs(&f, 0), [f.dir.clone(), f.dir.clone()], "it opens the folder you were in");
    f.chord(&[Key::Control], "w");
    assert_eq!(app::tabs(&f.app, 0).0.len(), 1);
    f.chord(&[Key::Control], "w");
    assert_eq!(app::tabs(&f.app, 0).0.len(), 1, "the last tab stays");
}

fn the_tab_bar_switches_adds_and_closes_tabs() {
    let f = open_window();
    f.click_at(&f.button("New tab (Ctrl+T)"));
    wait_until(&f, "a second tab", |f| app::tabs(&f.app, 0).0.len() == 2 && f.names().len() == 5);
    f.double_click("alpha");
    wait_until(&f, "alpha in the second tab", |f| f.address() == f.dir.join("alpha"));
    f.click_at(&f.tab_button("files", 0));
    wait_until(&f, "the first tab again", |f| f.address() == f.dir && f.names().len() == 5);
    assert_eq!(f.view().tabs.iter().map(|t| t.title.to_string()).collect::<Vec<_>>(), ["files", "alpha"]);
    f.right_click(&f.tab_button("files", 0));
    f.pick("Close other tabs");
    wait_until(&f, "one tab", |f| app::tabs(&f.app, 0).0.len() == 1);
    assert_eq!(f.address(), f.dir);
}

fn f9_opens_the_second_side_and_tab_switches() {
    let f = open_window();
    f.key(Key::F9);
    assert!(app::is_dual(&f.app));
    assert_eq!(app::active_side(&f.app), 0);
    f.key(Key::Tab);
    assert_eq!(app::active_side(&f.app), 1);
    f.key(Key::Tab);
    assert_eq!(app::active_side(&f.app), 0);
    f.key(Key::Tab);
    f.key(Key::F9);
    assert!(!app::is_dual(&f.app));
    assert_eq!(app::active_side(&f.app), 0, "the left side takes over");
}

fn f5_copies_and_f6_moves_to_the_other_side() {
    let f = open_window();
    f.key(Key::F9);
    f.ui.invoke_address_entered(1, f.dir.join("alpha").display().to_string().into());
    wait_until(&f, "the right side in alpha", |f| f.ui.get_right().address == f.dir.join("alpha").display().to_string());
    f.ui.invoke_activate(0);
    f.click("notes.txt");
    f.key(Key::F5);
    assert_eq!(f.ui.get_dialog_title(), "Copy");
    assert_eq!(f.ui.get_dialog_text(), f.dir.join("alpha").display().to_string(), "the other side's folder");
    f.button("Copy").invoke_accessible_default_action();
    wait_until(&f, "copied", |f| f.dir.join("alpha/notes.txt").exists());
    assert!(f.dir.join("notes.txt").exists());

    f.click("big.bin");
    f.key(Key::F6);
    f.button("Move").invoke_accessible_default_action();
    wait_until(&f, "moved, and both sides refreshed", |f| {
        let right: Vec<String> = f.ui.get_right().files.iter().map(|r| r.name.to_string()).collect();
        right == ["big.bin", "inside.txt", "notes.txt"] && !f.names().contains(&"big.bin".to_string())
    });

    f.click("a.png");
    f.key(Key::F5);
    f.ui.set_dialog_text("no/such/folder".into());
    f.button("Copy").invoke_accessible_default_action();
    wait_until(&f, "the error", |f| f.ui.get_dialog_title() == "Copy" && f.ui.get_dialog_message().contains("does not exist"));
    f.button("OK").invoke_accessible_default_action();
}

fn copy_and_move_to_the_other_side_need_the_second_side() {
    let f = open_window();
    f.click("notes.txt");
    f.key(Key::F5);
    f.key(Key::F6);
    assert!(!dialog_open(&f), "no target dialog without the second side");
    f.right_click(&f.list_row("notes.txt"));
    assert!(f.menu_item("Copy to other side…  (F5)").is_some());
    f.key(Key::Escape);
    pump();
}

fn the_filter_box_narrows_the_list_and_escape_clears_it() {
    let f = open_window();
    f.chord(&[Key::Control], "f");
    pump();
    f.type_text("b*");
    wait_until(&f, "filtered", |f| f.names() == ["beta", "big.bin"]);
    assert_eq!(f.view().status, "2 items (4 filtered out)");
    f.key(Key::Escape);
    wait_until(&f, "cleared", |f| f.names().len() == 5);
    assert_eq!(f.view().filter, "");
    f.key(Key::DownArrow);
    assert_eq!(f.current().as_deref(), Some("a.png"), "the list has the keys again (focus was on beta)");

    f.chord(&[Key::Control], "f");
    pump();
    f.type_text("NOTES");
    wait_until(&f, "contains, any case", |f| f.names() == ["notes.txt"]);
    f.key(Key::Return);
    f.ui.invoke_address_entered(0, f.dir.join("alpha").display().to_string().into());
    wait_until(&f, "another folder, unfiltered", |f| f.names() == ["inside.txt"]);
    assert_eq!(f.view().filter, "");
}

fn ctrl_l_edits_the_address_and_a_file_path_selects_the_file() {
    let f = open_window();
    f.chord(&[Key::Control], "l");
    pump();
    let field = f.element("Address field");
    assert_eq!(field.accessible_value().as_deref(), Some(f.dir.display().to_string().as_str()));
    f.type_text(&f.dir.join("alpha/inside.txt").display().to_string()); // replaces the selected text
    f.key(Key::Return);
    wait_until(&f, "the file's folder", |f| f.address() == f.dir.join("alpha"));
    wait_until(&f, "the file selected", |f| f.selected() == ["inside.txt"]);

    f.chord(&[Key::Control], "l");
    pump();
    f.type_text("..");
    f.key(Key::Return);
    wait_until(&f, "relative to the folder shown", |f| f.address() == f.dir);
}

fn crumbs_show_names_as_they_are_and_navigate() {
    let f = open_window();
    fs::create_dir(f.dir.join("2019_09_08")).unwrap();
    f.ui.invoke_address_entered(0, f.dir.join("2019_09_08").display().to_string().into());
    wait_until(&f, "the folder", |f| f.address() == f.dir.join("2019_09_08"));
    let crumbs: Vec<String> = f.view().crumbs.iter().map(|c| c.label.to_string()).collect();
    assert_eq!(crumbs.last().map(String::as_str), Some("2019_09_08"));
    assert!(f.view().crumbs.iter().next().unwrap().drive, "the root is a drive");
    f.click_at(&f.button("files"));
    wait_until(&f, "a click on a crumb opens it", |f| f.address() == f.dir);
}

fn the_header_menu_shows_more_columns_and_they_sort() {
    let f = open_window();
    assert!(ElementHandle::find_by_accessible_label(&f.ui, "Extension").next().is_none());
    let header = f.button("Name");
    f.right_click(&header);
    f.pick("Extension");
    wait_until(&f, "the Extension column", |f| f.ui.get_columns().iter().any(|c| c.title == "Extension"));
    assert_eq!(f.rows().iter().find(|r| r.name == "notes.txt").unwrap().extension, "txt");
    f.click_at(&f.button("Extension"));
    wait_until(&f, "sorted by extension", |f| f.names() == ["alpha", "beta", "big.bin", "a.png", "notes.txt"]);
    app::collect_session(&f.app);
    assert!(app::session(&f.app).get_bool("columns.extension.visible", false));
    assert_eq!(app::session(&f.app).get("sort.column").as_deref(), Some("extension"));
}

fn the_session_restores_tabs_and_the_second_side() {
    let f = open_window();
    f.chord(&[Key::Control], "t");
    wait_until(&f, "a second tab", |f| app::tabs(&f.app, 0).0.len() == 2 && f.names().len() == 5);
    f.double_click("alpha");
    wait_until(&f, "alpha", |f| f.address() == f.dir.join("alpha"));
    f.key(Key::F9);
    app::collect_session(&f.app);

    let ui = AppWindow::new().unwrap();
    let again = app::install(&ui, app::session(&f.app), None);
    app::set_clipboard(&again, Box::new(fileem::clipboard::Memory::default()));
    let start = Instant::now();
    while app::tabs(&again, 0).0 != [Some(f.dir.clone()), Some(f.dir.join("alpha"))] {
        assert!(start.elapsed() < Duration::from_secs(10), "tabs restored: {:?}", app::tabs(&again, 0));
        pump();
    }
    assert_eq!(app::tabs(&again, 0).1, 1, "the selected tab");
    assert!(app::is_dual(&again));
}

fn renaming_a_folder_moves_the_tabs_inside_it() {
    let f = open_window();
    fs::create_dir(f.dir.join("alpha/inner")).unwrap();
    f.ui.invoke_address_entered(0, f.dir.join("alpha/inner").display().to_string().into());
    wait_until(&f, "inner", |f| f.address() == f.dir.join("alpha/inner"));
    app::run_command(&f.app, fileem::menu::Command::Rename(f.dir.join("alpha")));
    f.ui.set_dialog_text("renamed".into());
    f.button("Rename").invoke_accessible_default_action();
    wait_until(&f, "the tab followed", |f| f.address() == f.dir.join("renamed/inner"));
    assert!(f.dir.join("renamed/inner").is_dir());
}

fn both_lists_stay_usable_in_a_narrow_dual_window() {
    let f = open_window();
    f.ui.window().set_size(slint::LogicalSize::new(700.0, 500.0));
    f.key(Key::F9);
    pump();
    let lists: Vec<f32> = ElementHandle::find_by_accessible_label(&f.ui, "Status").map(|e| e.size().width).collect();
    assert_eq!(lists.len(), 2);
    for w in lists {
        assert!(w >= 200.0, "a file list keeps a usable width: {w}");
    }
}

// ---- drag & drop, selection box ----

fn box_shown(f: &Fixture) -> bool {
    ElementHandle::find_by_accessible_label(&f.ui, "Selection box").any(|e| e.size().height > 0.0)
}

fn answer_drop(f: &Fixture, button: &str) {
    wait_until(f, "the move or copy question", |f| dialog_open(f) && f.ui.get_dialog_title() == "Move or copy");
    f.button(button).invoke_accessible_default_action();
    pump();
}

fn dragging_a_file_onto_a_folder_row_asks_then_moves() {
    let f = open_window();
    f.click("notes.txt");
    f.drag_drop(f.on_name("notes.txt"), f.center(&f.list_row("alpha")));
    wait_until(&f, "the question", dialog_open);
    assert!(f.ui.get_dialog_message().contains("Move \u{201c}notes.txt\u{201d} to \u{201c}alpha\u{201d}?"), "{}", f.ui.get_dialog_message());
    answer_drop(&f, "Move");
    wait_until(&f, "moved into the folder", |f| f.dir.join("alpha/notes.txt").exists() && !f.dir.join("notes.txt").exists());
    wait_until(&f, "the list follows", |f| !f.names().contains(&"notes.txt".to_string()));
}

fn choosing_copy_after_a_drop_onto_the_tree_keeps_the_original() {
    let f = open_window();
    // inside alpha the tree shows its siblings
    f.double_click("alpha");
    wait_until(&f, "alpha is open, the tree shows beta", |f| {
        f.names() == ["inside.txt"]
            && ElementHandle::find_by_accessible_label(&f.ui, "beta").any(|e| e.absolute_position().x < f.ui.get_left().tree_width)
    });
    f.click("inside.txt");
    f.drag_drop(f.on_name("inside.txt"), f.center(&f.tree_row("beta")));
    answer_drop(&f, "Copy");
    wait_until(&f, "copied", |f| f.dir.join("beta/inside.txt").exists());
    assert!(f.dir.join("alpha/inside.txt").exists(), "the original stays");
    assert_eq!(fs::read_to_string(f.dir.join("beta/inside.txt")).unwrap(), "i");
}
fn cancelling_the_drop_question_leaves_everything_alone() {
    let f = open_window();
    f.click("a.png");
    f.drag_drop(f.on_name("a.png"), f.center(&f.list_row("beta")));
    answer_drop(&f, "Cancel");
    assert!(!dialog_open(&f));
    settle(&f);
    assert!(app::ops(&f.app).running().is_empty(), "no operation started");
    assert!(f.dir.join("a.png").exists() && !f.dir.join("beta/a.png").exists());
}

fn drops_into_the_dragged_items_or_where_they_are_are_refused() {
    let f = open_window();
    f.click("alpha");
    // onto itself: refused as a row, and the folder it is in already holds it
    f.drag_drop(f.on_name("alpha"), f.on_size("alpha"));
    settle(&f);
    assert!(!dialog_open(&f), "no question for a drop that can't happen");
    // onto a file: the shown folder, where it already is
    f.drag_drop(f.on_name("alpha"), f.on_name("notes.txt"));
    settle(&f);
    assert!(!dialog_open(&f));
    assert!(f.dir.join("alpha/inside.txt").exists());
}

fn dragging_to_the_other_side_drops_into_its_folder() {
    let f = open_window();
    f.key(Key::F9);
    f.ui.invoke_address_entered(1, f.dir.join("beta").display().to_string().into());
    wait_until(&f, "the right side in beta", |f| f.ui.get_right().address == f.dir.join("beta").display().to_string());
    f.ui.invoke_activate(0);
    f.click("notes.txt");
    // the right list's empty space
    let status = ElementHandle::find_by_accessible_label(&f.ui, "Status").max_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x)).unwrap();
    let target = LogicalPosition::new(status.absolute_position().x + 100.0, status.absolute_position().y - 100.0);
    f.drag_drop(f.on_name("notes.txt"), target);
    assert!(f.ui.get_dialog_message().contains("\u{201c}beta\u{201d}"), "{}", f.ui.get_dialog_message());
    answer_drop(&f, "Move");
    wait_until(&f, "moved to the other side", |f| {
        f.ui.get_right().files.iter().any(|r| r.name == "notes.txt") && !f.dir.join("notes.txt").exists()
    });
}

fn ctrl_click(f: &Fixture, name: &str) {
    f.ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    f.click_at(&f.list_row(name));
    f.ui.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
}

fn a_press_on_a_selection_drags_all_of_it_and_a_click_selects_one() {
    let f = open_window();
    f.click("a.png");
    ctrl_click(&f, "big.bin");
    assert_eq!(f.selected(), ["a.png", "big.bin"]);
    f.click_at(&f.list_row("big.bin"));
    assert_eq!(f.selected(), ["big.bin"], "a click without a drag selects the row alone, on release");

    ctrl_click(&f, "a.png");
    f.drag_to(f.on_name("big.bin"), f.center(&f.list_row("alpha")), &[]);
    assert_eq!(f.selected(), ["a.png", "big.bin"], "pressing a selected row keeps the selection");
    f.release(f.center(&f.list_row("alpha")));
    assert!(f.ui.get_dialog_message().contains("Move 2 items"), "{}", f.ui.get_dialog_message());
    answer_drop(&f, "Move");
    wait_until(&f, "both moved", |f| f.dir.join("alpha/a.png").exists() && f.dir.join("alpha/big.bin").exists());
}

fn a_tree_folder_can_be_dragged_into_the_list() {
    let f = open_window();
    f.double_click("alpha");
    wait_until(&f, "alpha is open, the tree shows beta", |f| {
        f.names() == ["inside.txt"]
            && ElementHandle::find_by_accessible_label(&f.ui, "beta").any(|e| e.absolute_position().x < f.ui.get_left().tree_width)
    });
    let empty = {
        let row = f.list_row("inside.txt");
        LogicalPosition::new(row.absolute_position().x + 200.0, row.absolute_position().y + 100.0)
    };
    f.drag_drop(f.center(&f.tree_row("beta")), empty);
    answer_drop(&f, "Copy");
    wait_until(&f, "copied into the shown folder", |f| f.dir.join("alpha/beta").is_dir());
    assert!(f.dir.join("beta").is_dir());
    assert_eq!(f.address(), f.dir.join("alpha"), "the press in the tree did not navigate");
}
fn dragging_from_the_size_column_draws_a_box_that_selects_rows() {
    let f = open_window();
    f.click("notes.txt");
    f.drag_to(f.on_size("alpha"), f.on_size("a.png"), &[]);
    assert!(box_shown(&f), "the box is drawn while dragging");
    assert_eq!(f.selected(), ["alpha", "beta", "a.png"]);
    f.release(f.on_size("a.png"));
    assert!(!box_shown(&f), "gone after the release");
    assert_eq!(f.selected(), ["alpha", "beta", "a.png"], "a release in a row doesn't reselect it");
    assert_eq!(f.current().as_deref(), Some("a.png"));
    settle(&f);
    assert!(!dialog_open(&f), "nothing was dropped");
    assert!(f.dir.join("alpha").exists() && f.dir.join("notes.txt").exists());
}

fn a_box_from_empty_space_selects_upwards_and_shift_adds() {
    let f = open_window();
    f.click("alpha");
    let last = f.list_row("notes.txt");
    let below = LogicalPosition::new(last.absolute_position().x + 400.0, last.absolute_position().y + 80.0);
    f.drag_to(below, f.on_size("big.bin"), &[Key::Shift]);
    f.release(f.on_size("big.bin"));
    assert_eq!(f.selected(), ["alpha", "big.bin", "notes.txt"]);

    // a plain box replaces the selection; a click in empty space clears it
    f.drag_to(below, f.on_size("notes.txt"), &[]);
    f.release(f.on_size("notes.txt"));
    assert_eq!(f.selected(), ["notes.txt"]);
    f.click_at(&f.list_row("notes.txt"));
    let w = f.ui.window();
    w.dispatch_event(WindowEvent::PointerPressed { position: below, button: PointerEventButton::Left });
    w.dispatch_event(WindowEvent::PointerReleased { position: below, button: PointerEventButton::Left });
    assert!(f.selected().is_empty());
}

fn ctrl_box_toggles_and_a_box_below_the_list_scrolls_it() {
    let f = open_window();
    f.click("alpha");
    f.drag_to(f.on_size("alpha"), f.on_size("beta"), &[Key::Control]);
    f.release(f.on_size("beta"));
    assert_eq!(f.selected(), ["beta"], "alpha toggled off, beta on");

    // a long folder in a short window: dragging below the list scrolls it and selects on
    let many = f.dir.join("many");
    fs::create_dir(&many).unwrap();
    for i in 0..60 {
        fs::write(many.join(format!("f{i:02}.txt")), "x").unwrap();
    }
    f.ui.invoke_address_entered(0, many.display().to_string().into());
    wait_until(&f, "many listed", |f| f.names().len() == 60);
    let status = f.element("Status");
    let below = LogicalPosition::new(status.absolute_position().x + 200.0, status.absolute_position().y + 10.0);
    f.drag_to(f.on_size("f00.txt"), below, &[]);
    wait_until(&f, "scrolled to the end", |f| f.selected().len() == 60);
    f.release(below);
    assert_eq!(f.current().as_deref(), Some("f59.txt"));
}

fn showing_the_second_side_keeps_the_left_scroll_position() {
    let f = open_window();
    let many = f.dir.join("many");
    fs::create_dir(&many).unwrap();
    for i in 0..60 {
        fs::write(many.join(format!("f{i:02}.txt")), "x").unwrap();
    }
    f.ui.invoke_address_entered(0, many.display().to_string().into());
    wait_until(&f, "many listed", |f| f.names().len() == 60);
    f.key(Key::End);
    pump();
    let y = f.list_row("f59.txt").absolute_position().y;
    f.key(Key::F9);
    pump();
    f.key(Key::F9);
    pump();
    // regression: toggling rebuilt the left side, back at the top with f59 out of view
    assert_eq!(f.list_row("f59.txt").absolute_position().y, y);
}
