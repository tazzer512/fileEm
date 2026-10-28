//! End-to-end UI tests on Slint's headless testing backend: the real window, real keys and
//! clicks, real background listings. The backend can only be initialized once per process, so
//! this file has a single `#[test]` that runs each scenario with a fresh window and folder.
//! Nothing is ever launched (`FILEEM_NO_LAUNCH`): opened files are only recorded.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use std::rc::Rc;

use fileem::view_mode::ViewMode;
use fileem::{app, shell, AppWindow, FileRow, ProgressWindow, PropertiesWindow, SearchWindow, SideView, TreeRow};
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
    fileem_core::util::fs_util::hide(&dir.join(".hidden"));
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

    fn click_point(&self, position: LogicalPosition) {
        let w = self.ui.window();
        w.dispatch_event(WindowEvent::PointerMoved { position });
        w.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
        w.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        pump();
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
        // the open menu's popup comes after the window's elements (a column header may have the
        // same text)
        ElementHandle::find_by_accessible_label(&self.ui, title)
            .filter(|e| e.size().width > 0.0 && e.accessible_role() != Some(i_slint_backend_testing::AccessibleRole::Button))
            .last()
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

    /// The middle of a tree row's name, where people press to drag it (a deep row's left part is
    /// indentation).
    fn on_tree_name(&self, name: &str) -> LogicalPosition {
        let tree_width = self.ui.get_left().tree_width;
        let e = ElementHandle::find_by_accessible_label(&self.ui, name)
            .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Text) && e.absolute_position().x < tree_width)
            .unwrap_or_else(|| panic!("no tree name {name:?}"));
        self.center(&e)
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

    /// Lets go of the mouse. Like a person, it lingers a moment first: hover state is updated
    /// between event loop runs, which a slow machine needs time for.
    fn release(&self, at: LogicalPosition) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(50) {
            pump();
        }
        self.ui.window().dispatch_event(WindowEvent::PointerReleased { position: at, button: PointerEventButton::Left });
        pump();
    }

    /// A whole drag and drop.
    fn drag_drop(&self, from: LogicalPosition, to: LogicalPosition) {
        self.drag_to(from, to, &[]);
        self.release(to);
    }

    /// A button of the main toolbar (a tab's bar may have one with the same tooltip).
    fn tool(&self, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(&self.ui, label)
            .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button) && e.absolute_position().y < 42.0)
            .unwrap_or_else(|| panic!("no toolbar button {label:?}"))
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

/// Waits until the tree has revealed and scrolled to the current folder (in the background, a
/// moment after the list): rows found before that may still move.
fn tree_settled(f: &Fixture) {
    wait_until(f, "the tree shows the folder", |f| f.tree().iter().any(|r| r.current));
    // then until the tree stops moving: its current row in the same place for 150 ms
    let place = |f: &Fixture| {
        let name = f.tree().into_iter().find(|r| r.current).map(|r| r.name).unwrap_or_default();
        let name = name.trim().to_string();
        f.tree_row(&name).absolute_position()
    };
    let seen = std::cell::Cell::new((place(f), Instant::now()));
    wait_until(f, "the tree settled", |f| {
        let (last, since) = seen.get();
        let now = place(f);
        if now != last {
            seen.set((now, Instant::now()));
            return false;
        }
        since.elapsed() > Duration::from_millis(150)
    });
}

/// Waits until the tree has revealed (and scrolled to) the current folder, which it does in the
/// background, then scrolls it to the top: scrolled earlier, the reveal would scroll it away again.
fn tree_to_top(f: &Fixture) {
    wait_until(f, "the tree shows the folder", |f| f.tree().iter().any(|r| r.current));
    settle(f);
    f.scroll_tree_to_top();
}

#[test]
fn ui() {
    std::env::set_var("FILEEM_NO_LAUNCH", "1");
    // the testing backend with a real rasterizer: drags take a snapshot for their picture
    slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(i_slint_backend_testing::TestingBackendOptions {
        mock_time: false,
        threading: true,
        renderer_name: Some("software".into()),
    })))
    .unwrap();

    // each scenario gets a fresh window and folder; FILEEM_UI_ONLY=<part of a name> runs only
    // the matching ones while working on them
    let only = std::env::var("FILEEM_UI_ONLY").ok();
    macro_rules! scenarios {
        ($($name:ident;)*) => {
            // the name first, so a failure says which scenario it was in
            $(if only.as_deref().is_none_or(|o| stringify!($name).contains(o)) {
                eprintln!("scenario {}", stringify!($name));
                $name();
            })*
        };
    }
    scenarios! {
        lists_folders_first_with_status_and_no_hidden_files;
        double_click_opens_folders_and_backspace_goes_up_selecting_the_child;
        alt_arrows_go_back_and_forward;
        keyboard_selection_and_enter_opens_files_without_launching;
        ctrl_h_and_the_toolbar_toggle_hidden_files;
        clicking_a_column_header_sorts_and_again_reverses;
        typing_jumps_to_a_name;
        space_toggles_the_focused_row_keeping_the_others;
        click_ctrl_click_and_shift_click_select;
        the_address_bar_navigates_and_reports_bad_paths;
        the_tree_reveals_the_current_folder_and_navigates_on_click;
        the_session_restores_hidden_files_sort_and_the_last_folder;
        a_vanished_last_folder_opens_its_nearest_parent;
        saving_the_session_keeps_what_only_the_java_app_uses;
        changes_made_elsewhere_show_up_and_keep_the_selection;
        a_new_subfolder_shows_up_in_the_tree;
        an_open_tree_folder_follows_the_disk_while_the_tab_shows_another;
        a_change_right_after_the_listing_is_not_lost;
        when_the_shown_folder_is_deleted_its_parent_opens;
        delete_and_f8_ask_first_and_cancel_keeps_the_files;
        shift_delete_deletes_permanently_after_confirming;
        f2_renames_and_refuses_taken_names;
        f7_creates_a_folder_under_a_free_name;
        a_conflict_is_asked_in_the_progress_row;
        a_slow_operation_gets_a_row_that_pauses_and_cancels;
        operation_errors_are_listed_afterwards;
        closing_with_operations_running_asks_first;
        a_termination_signal_saves_and_stops_operations;
        copy_and_paste_into_a_subfolder;
        cut_and_paste_moves_once;
        pasting_into_the_same_folder_makes_a_copy;
        right_click_selects_the_row_and_opens_its_menu;
        right_click_on_a_selected_row_keeps_the_selection;
        right_click_on_empty_space_offers_the_folder_menu;
        right_click_in_the_tree_opens_a_menu_without_navigating;
        alt_enter_and_the_menu_show_properties;
        new_text_file_from_the_menu;
        duplicate_compress_and_extract_from_the_menu;
        paste_into_a_folder_from_its_menu;
        ctrl_t_opens_and_ctrl_w_closes_tabs_but_never_the_last;
        the_tab_bar_switches_adds_and_closes_tabs;
        tabs_and_columns_are_dragged_into_another_order;
        f9_opens_the_second_side_and_tab_switches;
        f5_copies_and_f6_moves_to_the_other_side;
        copy_and_move_to_the_other_side_need_the_second_side;
        the_f_key_bar_buttons_do_what_their_keys_do;
        the_filter_box_narrows_the_list_and_escape_clears_it;
        ctrl_l_edits_the_address_and_a_file_path_selects_the_file;
        crumbs_show_names_as_they_are_and_navigate;
        the_header_menu_shows_more_columns_and_they_sort;
        the_session_restores_tabs_and_the_second_side;
        renaming_a_folder_moves_the_tabs_and_bookmarks_inside_it;
        both_lists_stay_usable_in_a_narrow_dual_window;
        dragging_a_file_onto_a_folder_row_asks_then_moves;
        a_refresh_during_the_press_still_drags;
        choosing_copy_after_a_drop_onto_the_tree_keeps_the_original;
        cancelling_the_drop_question_leaves_everything_alone;
        drops_into_the_dragged_items_or_where_they_are_are_refused;
        dragging_to_the_other_side_drops_into_its_folder;
        a_press_on_a_selection_drags_all_of_it_and_a_click_selects_one;
        a_tree_folder_can_be_dragged_into_the_list;
        dragging_from_the_size_column_draws_a_box_that_selects_rows;
        a_box_from_empty_space_selects_upwards_and_shift_adds;
        ctrl_box_toggles_and_a_box_below_the_list_scrolls_it;
        showing_the_second_side_keeps_the_left_scroll_position;
        // view modes (stage 5)
        the_view_button_and_menus_switch_views_and_keep_the_selection;
        the_toolbar_buttons_do_what_their_keys_do;
        the_toolbar_view_menu_switches_the_active_sides_view;
        the_sort_button_sorts_and_shows_the_direction;
        ctrl_shift_digits_and_ctrl_wheel_switch_the_view;
        size_controls_resize_within_limits_and_each_view_keeps_its_size;
        list_columns_fit_the_widest_name_in_the_real_font;
        sizes_and_views_survive_the_session_and_new_tabs_inherit_the_view;
        clicking_in_the_grid_selects_like_explorer_and_double_click_opens;
        arrow_keys_follow_the_grid_layout;
        a_box_in_the_grid_selects_the_items_it_touches;
        a_file_dragged_onto_a_folder_in_the_grid_asks_then_moves;
        grid_menus_target_the_item_or_the_folder_and_type_ahead_works;
        images_show_as_thumbnails_in_the_big_icon_views;
        photoshop_raw_jpeg2000_and_netpbm_files_show_thumbnails;
        // the preview (stage 6)
        the_preview_is_switched_on_and_off_with_f3_the_toolbar_or_by_closing_it;
        an_image_preview_zooms_fits_and_pans;
        text_files_show_their_text_and_zoom_changes_the_font;
        quick_selection_changes_only_load_the_last_one;
        photoshop_raw_jpeg2000_and_netpbm_files_are_previewed;
        the_preview_docks_into_the_main_window_and_floats_again;
        the_preview_is_remembered_in_the_session;
        // search (stage 7)
        search_from_the_right_click_menu_finds_files_in_all_subfolders;
        the_last_search_patterns_are_offered_in_every_window_and_remembered;
        search_results_can_be_renamed_and_deleted_in_place;
        ctrl_shift_f_searches_the_current_folder_and_hidden_files_follow_the_toggle;
        // bookmarks (stage 7)
        ctrl_d_and_the_toolbar_toggle_a_bookmark_shown_in_both_trees;
        folder_menus_add_bookmarks_and_a_bookmark_opens_from_the_tree;
        the_bookmark_menu_renames_moves_and_removes_and_bookmarks_survive_a_restart;
        the_manage_window_lists_reorders_renames_removes_and_opens_bookmarks;
        // drives and eject (stage 7)
        drives_show_their_space_and_only_plugged_in_ones_offer_eject;
        eject_moves_tabs_off_the_drive_first_then_ejects_and_confirms;
        the_tree_stops_watching_a_drive_while_it_is_ejected;
        eject_is_refused_while_an_operation_uses_the_drive;
        eject_removes_the_whole_drive_and_moves_tabs_off_every_partition;
        a_failed_eject_shows_the_reason_and_can_be_retried;
        // themes and About (stage 8)
        the_theme_menu_switches_family_and_mode_in_every_open_window;
        the_light_dark_button_toggles_and_the_theme_survives_a_restart;
        the_about_button_shows_version_build_date_and_system;
        windows_carry_the_app_icon;
    }
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
    f.key(Key::DownArrow);
    assert_eq!(f.selected(), ["beta"], "a move after select all clears the other rows");
    assert_eq!(f.rows().iter().filter(|r| r.current).count(), 1);

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

fn space_toggles_the_focused_row_keeping_the_others() {
    let f = open_window();
    assert!(f.selected().is_empty());
    assert_eq!(f.current().as_deref(), Some("alpha"));
    f.key(" ");
    assert_eq!(f.selected(), ["alpha"]);
    // once rows are marked, plain arrows only move the focus
    f.key(Key::DownArrow);
    f.key(Key::DownArrow);
    assert_eq!(f.selected(), ["alpha"]);
    f.key(" ");
    assert_eq!(f.selected(), ["alpha", "a.png"]);
    f.key(Key::DownArrow);
    f.key(" ");
    assert_eq!(f.selected(), ["alpha", "a.png", "big.bin"]);
    f.key(" ");
    assert_eq!(f.selected(), ["alpha", "a.png"], "a second Space deselects only that row");
    assert_eq!(f.current().as_deref(), Some("big.bin"), "the focus stays");
    f.key(Key::UpArrow);
    assert_eq!(f.selected(), ["alpha", "a.png"]);
    // a typed name moves the focus too, keeping the marks
    f.key("n");
    assert_eq!(f.current().as_deref(), Some("notes.txt"));
    f.key(" ");
    assert_eq!(f.selected(), ["alpha", "a.png", "notes.txt"]);

    // a click ends marking: arrows select again
    f.click("beta");
    f.key(Key::DownArrow);
    assert_eq!(f.selected(), ["a.png"]);

    // within a typed name a space is part of it
    fs::write(f.dir.join("my file.txt"), "x").unwrap();
    wait_until(&f, "the new file", |f| f.names().contains(&"my file.txt".to_string()));
    for k in ["m", "y", " ", "f"] {
        f.key(k);
    }
    assert_eq!(f.selected(), ["my file.txt"]);
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

fn an_open_tree_folder_follows_the_disk_while_the_tab_shows_another() {
    let f = open_window();
    fs::create_dir(f.dir.join("alpha/old")).unwrap();
    wait_until(&f, "the tree shows the folder", |f| f.tree().iter().any(|r| r.current));
    let row = |f: &Fixture, name: &str| f.tree().iter().position(|r| r.name == name);
    let current = f.tree().iter().position(|r| r.current).unwrap();
    f.ui.invoke_tree_toggled(0, current as i32);
    wait_until(&f, "subfolders", |f| row(f, "alpha").is_some());
    f.ui.invoke_tree_toggled(0, row(&f, "alpha").unwrap() as i32);
    wait_until(&f, "alpha's subfolder", |f| row(f, "old").is_some());

    // regression: only the tab's folder was watched, so the tree kept showing old folders
    fs::create_dir(f.dir.join("alpha/new")).unwrap();
    fs::remove_dir(f.dir.join("alpha/old")).unwrap();
    wait_until(&f, "the tree follows the disk", |f| row(f, "new").is_some() && row(f, "old").is_none());

    // closed it isn't watched; opened again it shows what changed meanwhile
    f.ui.invoke_tree_toggled(0, row(&f, "alpha").unwrap() as i32);
    wait_until(&f, "alpha closed", |f| row(f, "new").is_none());
    fs::create_dir(f.dir.join("alpha/later")).unwrap();
    f.ui.invoke_tree_toggled(0, row(&f, "alpha").unwrap() as i32);
    wait_until(&f, "caught up after opening", |f| row(f, "later").is_some() && row(f, "new").is_some());
}

fn a_change_right_after_the_listing_is_not_lost() {
    let f = open_window();
    fs::create_dir(f.dir.join("gap")).unwrap();
    fs::write(f.dir.join("gap/first.txt"), "x").unwrap();
    wait_until(&f, "gap listed", |f| f.names().contains(&"gap".to_string()));
    // regression: the watch started only once the listing was shown, so this was never seen
    app::set_after_listing(Some(|dir: &Path| {
        let late = dir.join("late.txt");
        if dir.file_name().is_some_and(|n| n == "gap") && !late.exists() {
            fs::write(late, "x").unwrap();
        }
    }));
    f.double_click("gap");
    wait_until(&f, "gap", |f| f.address() == f.dir.join("gap"));
    app::set_after_listing(None);
    wait_until(&f, "the late file shows up", |f| f.names().contains(&"late.txt".to_string()));
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

/// What logging out or `kill` does (main.rs runs it for SIGTERM, SIGHUP and SIGINT, then quits).
fn a_termination_signal_saves_and_stops_operations() {
    let f = open_window();
    let h = blocking_copy(&f);
    f.chord(&[Key::Control], "h");
    let file = app::session(&f.app).file().to_path_buf();
    assert!(!Session::load(&file).get_bool("showHidden", false), "not saved yet");
    f.ui.invoke_terminate();
    assert!(h.is_cancelled() && h.is_done(), "cancelled and waited for");
    assert!(Session::load(&file).get_bool("showHidden", false), "saved");
    assert!(!dialog_open(&f), "no question: there is nobody to answer it");
    assert_eq!(fs::read_to_string(f.dir.join("alpha/notes.txt")).unwrap(), "old", "the existing file is intact");
    assert!(no_temp_files(&f.dir.join("alpha")));
    settle(&f);
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
    // once the tree has revealed (and scrolled to) the current folder; Home is above it
    tree_to_top(&f);
    // the right-clicked row is outlined while its menu is open, however the menu closes
    let home_row = f.tree().iter().position(|r| r.name == "Home").unwrap() as i32;
    f.right_click(&f.tree_row("Home"));
    settle(&f);
    assert_eq!(f.view().tree_menu_row, home_row, "outlined");
    f.key(Key::Escape);
    wait_until(&f, "Escape: no outline", |f| f.view().tree_menu_row == -1);
    f.right_click(&f.tree_row("Home"));
    settle(&f);
    assert_eq!(f.view().tree_menu_row, home_row);
    f.click_point(LogicalPosition::new(700.0, 650.0));
    wait_until(&f, "a click elsewhere: no outline", |f| f.view().tree_menu_row == -1);
    // the address field had the keys: they come to the list for the menu, and so does the outline's end
    f.chord(&[Key::Control], "l");
    pump();
    f.right_click(&f.tree_row("Home"));
    settle(&f);
    assert_eq!(f.view().tree_menu_row, home_row);
    f.key(Key::Escape);
    wait_until(&f, "no outline", |f| f.view().tree_menu_row == -1);
    f.right_click(&f.tree_row("Home"));
    settle(&f);
    assert_eq!(f.address(), f.dir, "a right-click doesn't navigate");
    assert!(f.menu_item("Open in terminal").is_some());
    assert!(f.menu_item("Cut").is_none() && f.menu_item("Delete permanently").is_none(), "Home is a shortcut");
    f.pick("Open in terminal");
    let wanted = format!("terminal {}", home.display());
    wait_until(&f, "the terminal request", |_| shell::launched().contains(&wanted));
    assert_eq!(f.address(), f.dir);
    wait_until(&f, "picked: no outline", |f| f.view().tree_menu_row == -1);
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
    // step by step, so a failure says which part went wrong
    wait_until(&f, "todo.txt created in the folder shown", |f| f.dir.join("todo.txt").exists());
    wait_until(&f, "todo.txt listed", |f| f.names().contains(&"todo.txt".to_string()));
    wait_until(&f, "todo.txt selected", |f| f.selected() == ["todo.txt"]);
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

    // a single item goes into a folder named after the archive
    f.click("notes.txt");
    f.right_click(&f.list_row("notes.txt"));
    f.pick("Compress to ZIP");
    wait_until(&f, "the single-item archive selected", |f| f.selected() == ["notes.zip"]);
    f.right_click(&f.list_row("notes.zip"));
    f.pick("Extract to folder");
    wait_until(&f, "the new folder selected", |f| f.selected() == ["notes"]);
    assert_eq!(fs::read_to_string(f.dir.join("notes/notes.txt")).unwrap(), "1234567890");
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

fn tabs_and_columns_are_dragged_into_another_order() {
    let f = open_window();
    for (i, name) in ["alpha", "beta"].into_iter().enumerate() {
        f.chord(&[Key::Control], "t");
        wait_until(&f, "a new tab", |f| app::tabs(&f.app, 0).0.len() == i + 2);
        f.ui.invoke_address_entered(0, f.dir.join(name).display().to_string().into());
        wait_until(&f, name, |f| f.address() == f.dir.join(name));
    }
    let titles = |f: &Fixture| f.view().tabs.iter().map(|t| t.title.to_string()).collect::<Vec<_>>();
    assert_eq!(titles(&f), ["files", "alpha", "beta"]);
    let at = |e: &ElementHandle, fx: f32| {
        let (p, s) = (e.absolute_position(), e.size());
        LogicalPosition::new(p.x + s.width * fx, p.y + s.height / 2.0)
    };

    // beta onto the left half of files: the front; the shown tab stays shown
    f.drag_drop(at(&f.tab_button("beta", 0), 0.5), at(&f.tab_button("files", 0), 0.25));
    assert_eq!(titles(&f), ["beta", "files", "alpha"]);
    assert_eq!(app::tabs(&f.app, 0).1, 0, "beta is still the shown tab");
    assert_eq!(f.address(), f.dir.join("beta"));
    // files far past the last tab: the end
    let end = at(&f.tab_button("alpha", 0), 0.5);
    f.drag_drop(at(&f.tab_button("files", 0), 0.5), LogicalPosition::new(end.x + 400.0, end.y));
    assert_eq!(titles(&f), ["beta", "alpha", "files"]);
    assert_eq!(tab_dirs(&f, 0), [f.dir.join("beta"), f.dir.join("alpha"), f.dir.clone()]);
    // a press that hardly moves is a click: it selects, nothing moves
    let alpha = at(&f.tab_button("alpha", 0), 0.5);
    f.drag_drop(alpha, LogicalPosition::new(alpha.x + 3.0, alpha.y));
    assert_eq!(titles(&f), ["beta", "alpha", "files"]);
    wait_until(&f, "alpha shown", |f| f.address() == f.dir.join("alpha"));

    // columns: Date modified onto the left half of Size goes right after Name; the order is saved
    let header = |f: &Fixture, title: &str| {
        ElementHandle::find_by_accessible_label(&f.ui, title)
            .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button) && e.size().width > 0.0)
            .unwrap_or_else(|| panic!("no header {title:?}"))
    };
    let columns = |f: &Fixture| f.ui.get_columns().iter().map(|c| c.title.to_string()).collect::<Vec<_>>();
    assert_eq!(columns(&f), ["Size", "Type", "Date modified"]);
    let sort = f.view().sort_column;
    f.drag_drop(at(&header(&f, "Date modified"), 0.5), at(&header(&f, "Size"), 0.2));
    assert_eq!(columns(&f), ["Date modified", "Size", "Type"]);
    assert_eq!(f.view().sort_column, sort, "a drag doesn't sort");
    f.drag_drop(at(&header(&f, "Size"), 0.5), at(&header(&f, "Name"), 0.5));
    assert_eq!(columns(&f), ["Size", "Date modified", "Type"], "dropped on Name: right after it");
    f.drag_drop(at(&header(&f, "Size"), 0.5), LogicalPosition::new(1090.0, at(&header(&f, "Size"), 0.5).y));
    assert_eq!(columns(&f), ["Date modified", "Type", "Size"], "past the last: the end");
    app::collect_session(&f.app);
    assert!(app::session(&f.app).get("columns.order").unwrap().starts_with("name,modified,type,size,"));
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

    // the F-key bar's Copy and Move are greyed out too, until the second side shows
    let enabled = |f: &Fixture, label: &str| f.element(label).accessible_enabled() == Some(true);
    assert!(!enabled(&f, "F5 Copy") && !enabled(&f, "F6 Move"), "F-key bar buttons");
    f.click("F5 Copy");
    pump();
    assert!(!dialog_open(&f), "a greyed out button does nothing");
    f.key(Key::F9);
    assert!(enabled(&f, "F5 Copy") && enabled(&f, "F6 Move"));
    f.key(Key::F9);
    assert!(!enabled(&f, "F5 Copy") && !enabled(&f, "F6 Move"), "greyed out again");
}

fn the_f_key_bar_buttons_do_what_their_keys_do() {
    let f = open_window();
    f.click("notes.txt");
    let press = |label: &str| {
        f.click(label);
        pump();
    };
    press("F2 Rename");
    assert_eq!(f.ui.get_dialog_title(), "Rename");
    assert_eq!(f.ui.get_dialog_text(), "notes.txt");
    f.key(Key::Escape);
    press("F7 New folder");
    assert_eq!(f.ui.get_dialog_title(), "New folder");
    f.key(Key::Escape);
    press("F8 Delete");
    assert_eq!(f.ui.get_dialog_title(), "Move to Trash");
    f.key(Key::Escape);
    assert!(!dialog_open(&f) && f.dir.join("notes.txt").exists(), "cancelled, nothing trashed");

    press("F9 Dual pane");
    assert!(app::is_dual(&f.app));
    assert_eq!(app::active_side(&f.app), 0);
    press("Tab Switch side");
    assert_eq!(app::active_side(&f.app), 1);
    press("Tab Switch side");
    assert_eq!(app::active_side(&f.app), 0);
    f.click("notes.txt");
    press("F5 Copy");
    assert!(dialog_open(&f), "the target question");
    assert!(f.ui.get_dialog_title().starts_with("Copy"), "{}", f.ui.get_dialog_title());
    f.key(Key::Escape);
    press("F6 Move");
    assert!(f.ui.get_dialog_title().starts_with("Move"), "{}", f.ui.get_dialog_title());
    f.key(Key::Escape);
    press("F9 Dual pane");
    assert!(!app::is_dual(&f.app));
    f.key(Key::UpArrow);
    assert_eq!(f.current().as_deref(), Some("big.bin"), "the list has the keys again: {:?}", f.current());
    settle(&f);
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

fn renaming_a_folder_moves_the_tabs_and_bookmarks_inside_it() {
    let f = open_window_with(with_bookmarks(&[("alpha", "alpha"), ("Custom", "alpha/inner"), ("beta", "beta")]), true);
    fs::create_dir(f.dir.join("alpha/inner")).unwrap();
    f.ui.invoke_address_entered(0, f.dir.join("alpha/inner").display().to_string().into());
    wait_until(&f, "inner", |f| f.address() == f.dir.join("alpha/inner"));
    app::run_command(&f.app, fileem::menu::Command::Rename(f.dir.join("alpha")));
    f.ui.set_dialog_text("renamed".into());
    f.button("Rename").invoke_accessible_default_action();
    wait_until(&f, "renamed or refused", |f| f.dir.join("renamed").exists() || !f.ui.get_dialog_error().is_empty());
    assert!(f.dir.join("renamed").exists(), "the rename failed: {}", f.ui.get_dialog_error());
    wait_until(&f, "the tab followed", |f| f.address() == f.dir.join("renamed/inner"));
    assert!(f.dir.join("renamed/inner").is_dir());
    assert_eq!(
        app::bookmarks(&f.app),
        [
            ("renamed".to_string(), f.dir.join("renamed")),
            ("Custom".to_string(), f.dir.join("renamed/inner")),
            ("beta".to_string(), f.dir.join("beta"))
        ],
        "bookmarks follow; a default name takes the new name, a custom one stays"
    );
    assert_eq!(tree_bookmarks(&f.ui.get_left()), ["renamed", "Custom", "beta"]);
    wait_until(&f, "the toolbar knows renamed/inner is bookmarked", |f| f.ui.get_bookmarked());
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
    ElementHandle::find_by_accessible_label(&f.ui, "Selection box").any(|e| e.size().height > 0.0 || e.size().width > 0.0)
}

fn answer_drop(f: &Fixture, button: &str) {
    wait_until(f, "a question", dialog_open);
    let title = f.ui.get_dialog_title();
    assert_eq!(title, "Move or copy", "another dialog: {}", f.ui.get_dialog_message());
    f.button(button).invoke_accessible_default_action();
    pump();
}

fn a_refresh_during_the_press_still_drags() {
    let f = open_window();
    f.click("notes.txt");
    settle(&f);
    let (from, to) = (f.on_name("notes.txt"), f.center(&f.list_row("alpha")));
    let w = f.ui.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: from });
    w.dispatch_event(WindowEvent::PointerPressed { position: from, button: PointerEventButton::Left });
    settle(&f);
    // regression: a refresh rebuilt every row, dropping the press (on macOS every watch change
    // refreshes, so drags failed there)
    fs::write(f.dir.join("new.txt"), "n").unwrap();
    wait_until(&f, "the watcher's refresh", |f| f.names().len() == 6);
    for step in 1..=10 {
        let t = step as f32 / 10.0;
        let position = LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        w.dispatch_event(WindowEvent::PointerMoved { position });
    }
    wait_until(&f, "the drag started", |f| f.ui.get_drag_image().size().width > 0);
    f.release(to);
    answer_drop(&f, "Cancel");
    assert!(f.dir.join("notes.txt").exists());
}

fn dragging_a_file_onto_a_folder_row_asks_then_moves() {
    let f = open_window();
    f.click("notes.txt");
    // while dragging, the row's icon and name follow the mouse
    f.drag_to(f.on_name("notes.txt"), f.center(&f.list_row("alpha")), &[]);
    wait_until(&f, "the drag picture", |f| f.ui.get_drag_image().size().width > 0);
    let picture = f.ui.get_drag_image();
    let size = picture.size();
    let shot = f.ui.window().take_snapshot().map(|s| (s.width(), s.height()));
    let name = f.on_name("notes.txt");
    assert!(
        size.height == 24 && size.width > 60 && size.width < 200,
        "the row's icon and name: {size:?} (window snapshot {shot:?}, scale {}, name at {name:?})",
        f.ui.window().scale_factor()
    );
    let px = picture.to_rgba8().unwrap();
    assert!(px.as_slice().iter().any(|p| p.a > 0 && p.a < 255), "faded");
    f.release(f.center(&f.list_row("alpha")));
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
    tree_settled(&f);
    f.click("inside.txt");
    let beta = f.center(&f.tree_row("beta"));
    f.drag_to(f.on_name("inside.txt"), beta, &[]);
    wait_until(&f, "the drag started", |f| f.ui.get_drag_image().size().width > 0);
    // a drop finds its tree row by position: let go where beta is now (a drop that lands on
    // alpha, which holds the file already, is refused without a question)
    let now = f.center(&f.tree_row("beta"));
    if now != beta {
        eprintln!("the tree moved during the drag: beta from {beta:?} to {now:?}");
    }
    f.ui.window().dispatch_event(WindowEvent::PointerMoved { position: now });
    f.release(now);
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
    // regression: pressing after the tree's background reload (it rebuilt every row, dropping
    // the press) never started a drag
    tree_settled(&f);
    let empty = {
        let row = f.list_row("inside.txt");
        LogicalPosition::new(row.absolute_position().x + 200.0, row.absolute_position().y + 100.0)
    };
    f.drag_to(f.on_tree_name("beta"), empty, &[]);
    wait_until(&f, "the drag started", |f| f.ui.get_drag_image().size().width > 0);
    f.release(empty);
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
    // ListView may nudge the scroll position while it re-measures: wait for the end itself
    wait_until(&f, "scrolled to the end", |f| f.selected().len() == 60 && f.current().as_deref() == Some("f59.txt"));
    f.release(below);
    assert_eq!(f.current().as_deref(), Some("f59.txt"));
    // regression: the row the box started on had scrolled away and took the release with it;
    // the box stayed armed, kept scrolling and blocked the next drag
    settle(&f);
    assert!(!box_shown(&f), "the box ended with the release");
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

// ---- view modes ----

/// A fresh folder `name` inside the test folder with these files ("x/" for a folder), shown.
fn open_folder(f: &Fixture, name: &str, files: &[&str]) -> PathBuf {
    let dir = f.dir.join(name);
    fs::create_dir(&dir).unwrap();
    for n in files {
        match n.strip_suffix('/') {
            Some(d) => fs::create_dir(dir.join(d)).unwrap(),
            None => fs::write(dir.join(n), "x").unwrap(),
        }
    }
    f.ui.invoke_address_entered(0, dir.display().to_string().into());
    wait_until(f, "the folder is shown", |f| f.address() == dir && f.names().len() == files.len());
    dir
}

fn set_view(f: &Fixture, mode: ViewMode) {
    app::run_command(&f.app, fileem::menu::Command::SetView(mode));
    pump();
}

fn view(f: &Fixture) -> ViewMode {
    app::view_of(&f.app, 0).0
}

fn icon_px(f: &Fixture) -> u32 {
    app::view_of(&f.app, 0).1
}

/// The names of the items the grid shows, in order (only built slots exist).
fn grid_names(f: &Fixture) -> Vec<String> {
    let tree_width = f.ui.get_left().tree_width;
    let mut items: Vec<(f32, f32, String)> = f
        .names()
        .iter()
        .flat_map(|n| ElementHandle::find_by_accessible_label(&f.ui, n).collect::<Vec<_>>())
        .filter(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem) && e.absolute_position().x > tree_width)
        .map(|e| (e.absolute_position().y, e.absolute_position().x, e.accessible_label().unwrap().to_string()))
        .collect();
    items.sort_by(|a, b| a.partial_cmp(b).unwrap());
    items.dedup();
    items.into_iter().map(|i| i.2).collect()
}

fn the_view_button_and_menus_switch_views_and_keep_the_selection() {
    let f = open_window();
    f.click("notes.txt");
    let tab_button = ElementHandle::find_by_accessible_label(&f.ui, "View (Ctrl+Shift+1…8, Ctrl+wheel)")
        .find(|e| e.absolute_position().y > 42.0)
        .expect("the tab's view button");
    f.click_at(&tab_button);
    pump();
    for title in ["Extra large icons  (Ctrl+Shift+1)", "Details  (Ctrl+Shift+6)", "Content  (Ctrl+Shift+8)"] {
        assert!(f.menu_item(title).is_some(), "{title}");
    }
    f.pick("Tiles  (Ctrl+Shift+7)");
    assert_eq!(view(&f), ViewMode::Tiles);
    assert_eq!(f.view().view_mode, ViewMode::Tiles.index() as i32, "the button shows the view");
    for m in ViewMode::ALL {
        set_view(&f, m);
        assert_eq!(view(&f), m);
        if m == ViewMode::Details {
            assert_eq!(f.view().list_files.row_count(), 5, "the list shows the rows");
        } else {
            assert_eq!(grid_names(&f), ["alpha", "beta", "a.png", "big.bin", "notes.txt"], "{m:?} shows the items in order");
            assert_eq!(f.view().list_files.row_count(), 0, "no hidden list rows");
        }
        assert_eq!(f.selected(), ["notes.txt"], "the selection carries over to {m:?}");
    }
    // the folder's menu has the views too, the current one checked
    f.right_click_at(LogicalPosition::new(700.0, 500.0));
    f.pick("View");
    assert!(f.menu_item("Content  (Ctrl+Shift+8)").is_some());
    f.pick("Small icons  (Ctrl+Shift+4)");
    assert_eq!(view(&f), ViewMode::SmallIcons);
}

fn the_toolbar_buttons_do_what_their_keys_do() {
    let f = open_window();
    let press = |label: &str| {
        f.click_at(&f.tool(label));
        pump();
    };
    // new folder, rename and delete ask first (cancelled here)
    press("New folder (F7)");
    assert_eq!(f.ui.get_dialog_title(), "New folder");
    f.key(Key::Escape);
    f.click("notes.txt");
    press("Rename (F2)");
    assert_eq!((f.ui.get_dialog_title().as_str(), f.ui.get_dialog_text().as_str()), ("Rename", "notes.txt"));
    f.key(Key::Escape);
    press("Delete (Del)");
    assert_eq!(f.ui.get_dialog_title(), "Move to Trash");
    f.key(Key::Escape);
    assert!(!dialog_open(&f));

    // copy and paste, cut and paste
    press("Copy (Ctrl+C)");
    f.double_click("alpha");
    wait_until(&f, "alpha", |f| f.address() == f.dir.join("alpha"));
    press("Paste (Ctrl+V)");
    wait_until(&f, "the pasted copy", |f| f.names() == ["inside.txt", "notes.txt"]);
    assert!(f.dir.join("notes.txt").exists(), "a copy keeps the original");
    f.click("inside.txt");
    press("Cut (Ctrl+X)");
    assert!(f.dir.join("alpha/inside.txt").exists(), "cutting alone moves nothing");
    f.key(Key::Backspace);
    wait_until(&f, "back up", |f| f.address() == f.dir);
    f.double_click("beta");
    wait_until(&f, "beta", |f| f.address() == f.dir.join("beta"));
    press("Paste (Ctrl+V)");
    wait_until(&f, "the moved file", |f| f.names() == ["inside.txt"]);
    assert!(!f.dir.join("alpha/inside.txt").exists());

    // properties of the selection
    f.click("inside.txt");
    press("Properties (Alt+Enter)");
    wait_until(&f, "the properties window", |f| app::properties_windows(&f.app).len() == 1);
    assert_eq!(app::properties_windows(&f.app)[0].get_window_title(), "inside.txt Properties");
    app::close_windows(&f.app);

    // a new tab on the folder shown
    press("New tab (Ctrl+T)");
    wait_until(&f, "a second tab", |f| app::tabs(&f.app, 0).0.len() == 2);
    assert_eq!(app::tabs(&f.app, 0).1, 1, "the new tab is selected");
    // a tab has its folder once the listing is in (on a slow machine only a moment later)
    wait_until(&f, "the new tab's folder", |f| tab_dirs(f, 0)[1] != PathBuf::new());
    assert_eq!(tab_dirs(&f, 0), [f.dir.join("beta"), f.dir.join("beta")]);
    settle(&f);
}

fn the_toolbar_view_menu_switches_the_active_sides_view() {
    let f = open_window();
    f.key(Key::F9);
    f.ui.invoke_address_entered(1, f.dir.join("alpha").display().to_string().into());
    wait_until(&f, "the right side in alpha", |f| f.ui.get_right().address == f.dir.join("alpha").display().to_string());
    assert_eq!(app::active_side(&f.app), 1);
    f.click_at(&f.tool("View (Ctrl+Shift+1…8, Ctrl+wheel)"));
    pump();
    assert!(f.menu_item("Content  (Ctrl+Shift+8)").is_some(), "the view menu is open");
    f.pick("Tiles  (Ctrl+Shift+7)");
    assert_eq!(app::view_of(&f.app, 1).0, ViewMode::Tiles, "the active side changes");
    assert_eq!(view(&f), ViewMode::Details, "the other side keeps its view");
    assert_eq!(f.ui.get_right().view_mode, ViewMode::Tiles.index() as i32, "the tab's button shows it");
}

fn the_sort_button_sorts_and_shows_the_direction() {
    let f = open_window();
    let sort_button = |f: &Fixture| f.view().sort_tooltip.to_string();
    assert_eq!(sort_button(&f), "Sort by Name, ascending");
    f.click(&sort_button(&f));
    f.pick("Size");
    assert_eq!(f.names(), ["alpha", "beta", "a.png", "notes.txt", "big.bin"]);
    f.click(&sort_button(&f));
    f.pick("Descending");
    assert_eq!(f.names(), ["beta", "alpha", "big.bin", "notes.txt", "a.png"]);
    assert_eq!(sort_button(&f), "Sort by Size, descending");
    assert!(!f.view().sort_ascending);
}

fn ctrl_shift_digits_and_ctrl_wheel_switch_the_view() {
    let f = open_window();
    f.chord(&[Key::Control, Key::Shift], "!");
    assert_eq!(view(&f), ViewMode::ExtraLargeIcons, "US layout: Shift+1 is !");
    f.chord(&[Key::Control, Key::Shift], "7");
    assert_eq!(view(&f), ViewMode::Tiles);
    f.chord(&[Key::Control, Key::Shift], "^");
    assert_eq!(view(&f), ViewMode::Details);

    // Ctrl+wheel over the files: up toward bigger icons, down toward Content
    let wheel = |f: &Fixture, at: LogicalPosition, delta: f32| {
        let w = f.ui.window();
        w.dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
        w.dispatch_event(WindowEvent::PointerMoved { position: at });
        w.dispatch_event(WindowEvent::PointerScrolled { position: at, delta_x: 0.0, delta_y: delta });
        w.dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
        pump();
    };
    let empty = LogicalPosition::new(700.0, 500.0);
    wheel(&f, empty, 40.0);
    assert_eq!(view(&f), ViewMode::List, "below the rows");
    wheel(&f, empty, -40.0);
    assert_eq!(view(&f), ViewMode::Details, "on the grid's empty space");
    wheel(&f, f.on_name("notes.txt"), -20.0);
    assert_eq!(view(&f), ViewMode::Details, "less than a notch");
    wheel(&f, f.on_name("notes.txt"), -20.0);
    assert_eq!(view(&f), ViewMode::Tiles);

    // a Details list that scrolls takes the wheel itself, but not while Ctrl is held: anywhere
    // over its rows (here the Size column, past the names) Ctrl+wheel still changes the view
    set_view(&f, ViewMode::Details);
    let many = f.dir.join("alpha");
    for i in 0..80 {
        fs::write(many.join(format!("file-{i:02}.txt")), "x").unwrap();
    }
    f.ui.invoke_address_entered(0, many.display().to_string().into());
    wait_until(&f, "the long folder is listed", |f| f.names().len() == 81);
    let row = f.list_row("file-03.txt");
    let size_column = LogicalPosition::new(row.absolute_position().x + f.ui.get_name_width() + 30.0, row.absolute_position().y + 5.0);
    wheel(&f, size_column, 40.0);
    assert_eq!(view(&f), ViewMode::List, "Ctrl+wheel over the Size column of a list that scrolls");
    set_view(&f, ViewMode::Details);
    let w = f.ui.window();
    let before = f.list_row("file-20.txt").absolute_position().y;
    w.dispatch_event(WindowEvent::PointerMoved { position: size_column });
    w.dispatch_event(WindowEvent::PointerScrolled { position: size_column, delta_x: 0.0, delta_y: -200.0 });
    pump();
    assert_eq!(view(&f), ViewMode::Details, "without Ctrl the wheel scrolls");
    wait_until(&f, "the list scrolled", |f| f.list_row("file-20.txt").absolute_position().y < before);
    // with Ctrl held, a click there still selects the row under it (the Ctrl layer passes it on)
    let row = f.list_row("file-20.txt");
    let at = LogicalPosition::new(row.absolute_position().x + f.ui.get_name_width() + 30.0, row.absolute_position().y + 5.0);
    w.dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    f.click_point(at);
    w.dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    pump();
    assert!(f.selected().contains(&"file-20.txt".to_string()), "Ctrl+click toggles the row in: {:?}", f.selected());
}

fn list_columns_fit_the_widest_name_in_the_real_font() {
    let f = open_window();
    set_view(&f, ViewMode::List);
    // the same letters to a rough estimate, different widths in the font: only measuring tells
    let column = |f: &Fixture, name: &str| {
        let dir = f.dir.join(format!("only-{}", name.len()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join(name), "x").unwrap();
        fs::write(dir.join("a.txt"), "x").unwrap();
        f.ui.invoke_address_entered(0, dir.display().to_string().into());
        wait_until(f, "the folder is listed", |f| f.names().iter().any(|n| n == name));
        pump();
        f.view().cell_w
    };
    let wide = column(&f, &"W".repeat(24));
    let narrow = column(&f, &"w".repeat(23));
    assert!(wide > narrow + 30.0, "measured: {wide} for W, {narrow} for w");
    assert!(narrow > 140.0 && wide < 420.0, "within the limits: {narrow}, {wide}");
    let short = column(&f, "b.txt");
    assert_eq!(short, 140.0, "short names: the narrowest column");
}

fn size_controls_resize_within_limits_and_each_view_keeps_its_size() {
    let f = open_window();
    for m in ViewMode::ALL {
        set_view(&f, m);
        assert_eq!(f.view().resizable, m.resizable(), "{m:?}");
        assert_eq!(ElementHandle::find_by_accessible_label(&f.ui, "Bigger items (Ctrl++)").next().is_some(), m.resizable(), "{m:?} +");
    }
    set_view(&f, ViewMode::LargeIcons);
    let width = f.list_row("notes.txt").size().width;
    assert_eq!(icon_px(&f), 96);
    f.click("Bigger items (Ctrl++)");
    assert_eq!(icon_px(&f), 120);
    pump();
    assert!(f.list_row("notes.txt").size().width > width, "the items grew");
    f.click("Smaller items (Ctrl+−)");
    f.click("Smaller items (Ctrl+−)");
    assert_eq!(icon_px(&f), 77);

    // the slider: the middle is the view's own size, the ends the limits
    f.ui.invoke_size_slider_moved(0, 1.0);
    assert_eq!(icon_px(&f), 320);
    assert!(!f.view().can_bigger);
    f.ui.invoke_size_slider_moved(0, -1.0);
    assert_eq!(icon_px(&f), 16);
    f.ui.invoke_size_slider_moved(0, 0.02);
    assert_eq!(icon_px(&f), 96, "near the middle snaps to the default");
    assert_eq!(f.view().size_slider, 0.0);

    // a double-click on the slider goes back to the default; single clicks (and slow ones) move it
    let slider = f.element("Item size");
    f.click_at(&slider);
    pump();
    let moved = icon_px(&f);
    assert_ne!(moved, 96, "a click on the track moves the slider there");
    std::thread::sleep(std::time::Duration::from_millis(600));
    // the release handler runs within the click: no pump in between, a slow runner's pump
    // (a redraw) once made the quick pair slower than the double-click interval
    f.click_at(&slider);
    assert_eq!(icon_px(&f), moved, "two slow clicks are no double-click");
    f.click_at(&slider);
    pump();
    assert_eq!(icon_px(&f), 96, "double-click: the default size");
    assert_eq!(f.view().size_slider, 0.0);

    // keys, and each view keeps its own size
    set_view(&f, ViewMode::Tiles);
    f.chord(&[Key::Control], "=");
    assert_eq!(icon_px(&f), 60);
    set_view(&f, ViewMode::MediumIcons);
    assert_eq!(icon_px(&f), 48, "another view keeps its own size");
    f.chord(&[Key::Control], "-");
    assert_eq!(icon_px(&f), 38);
    set_view(&f, ViewMode::SmallIcons);
    assert!(!f.view().can_smaller, "Small icons are the smallest already");
    set_view(&f, ViewMode::Details);
    f.chord(&[Key::Control], "=");
    set_view(&f, ViewMode::Tiles);
    assert_eq!(icon_px(&f), 60, "switching back restores the size; Ctrl+= did nothing in Details");
}

fn sizes_and_views_survive_the_session_and_new_tabs_inherit_the_view() {
    let f = open_window();
    set_view(&f, ViewMode::MediumIcons);
    f.ui.invoke_size_slider_moved(0, 0.37);
    let px = icon_px(&f);
    set_view(&f, ViewMode::Tiles);
    f.chord(&[Key::Control], "t");
    wait_until(&f, "a second tab", |f| app::tabs(&f.app, 0).0.len() == 2 && f.names().len() == 5);
    assert_eq!(view(&f), ViewMode::Tiles, "a new tab opens in the view you're using");
    set_view(&f, ViewMode::List);
    app::collect_session(&f.app);
    let session = app::session(&f.app);
    assert_eq!(session.get("left.tab.0.view").as_deref(), Some("tiles"));
    assert_eq!(session.get("left.tab.1.view").as_deref(), Some("list"));
    assert!(session.get("left.tab.0.size").is_some_and(|s| s.starts_with("medium-icons:") && s.contains('.')), "fractional steps: {:?}", session.get("left.tab.0.size"));

    let ui = AppWindow::new().unwrap();
    let again = app::install(&ui, session, None);
    assert_eq!(app::tab_views(&again, 0), [ViewMode::Tiles, ViewMode::List]);
    assert_eq!(app::view_of(&again, 0).0, ViewMode::List, "the selected tab");
    ui.invoke_tab_clicked(0, 0);
    app::run_command(&again, fileem::menu::Command::SetView(ViewMode::MediumIcons));
    assert_eq!(app::view_of(&again, 0), (ViewMode::MediumIcons, px), "the fractional size came back");
}

fn clicking_in_the_grid_selects_like_explorer_and_double_click_opens() {
    let f = open_window();
    let dir = open_folder(&f, "gridclick", &["sub/", "a.txt", "b.txt", "c.txt"]);
    set_view(&f, ViewMode::LargeIcons);
    f.click_at(&f.list_row("a.txt"));
    assert_eq!(f.selected(), ["a.txt"]);
    ctrl_click(&f, "c.txt");
    assert_eq!(f.selected(), ["a.txt", "c.txt"]);
    let w = f.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed { text: Key::Shift.into() });
    f.click_at(&f.list_row("b.txt"));
    w.dispatch_event(WindowEvent::KeyReleased { text: Key::Shift.into() });
    assert_eq!(f.selected(), ["b.txt", "c.txt"], "shift selects from the last clicked item");
    let sub = f.list_row("sub");
    f.click_at(&sub);
    f.click_at(&sub);
    wait_until(&f, "sub opened", |f| f.address() == dir.join("sub"));
    assert_eq!(view(&f), ViewMode::LargeIcons, "the tab keeps its view");
}

fn arrow_keys_follow_the_grid_layout() {
    let f = open_window();
    let names: Vec<String> = (0..40).map(|i| format!("f{i:02}.txt")).collect();
    open_folder(&f, "gridkeys", &names.iter().map(String::as_str).collect::<Vec<_>>());
    set_view(&f, ViewMode::MediumIcons);
    f.key(Key::Home);
    let lanes = app::grid(&f.app, 0).unwrap().lanes;
    assert!(lanes > 1, "several icons per row");
    f.key(Key::RightArrow);
    assert_eq!(f.selected(), ["f01.txt"]);
    f.key(Key::DownArrow);
    assert_eq!(f.selected(), [names[1 + lanes].clone()], "Down moves one row");
    f.chord(&[Key::Shift], Key::LeftArrow);
    assert_eq!(f.selected(), [names[lanes].clone(), names[1 + lanes].clone()], "Shift extends");
    f.key(Key::End);
    assert_eq!(f.selected(), ["f39.txt"]);
    pump();
    assert!(f.list_row("f39.txt").absolute_position().y < 700.0, "scrolled into view");

    set_view(&f, ViewMode::List);
    f.key(Key::Home);
    let rows = app::grid(&f.app, 0).unwrap().lanes;
    f.key(Key::DownArrow);
    assert_eq!(f.selected(), ["f01.txt"], "List runs down the column");
    f.key(Key::RightArrow);
    assert_eq!(f.selected(), [names[(1 + rows).min(39)].clone()], "Right moves one column");
    set_view(&f, ViewMode::Details);
    f.key(Key::Home);
    f.key(Key::RightArrow);
    assert_eq!(f.selected(), ["f00.txt"], "Details doesn't move sideways");
}

fn a_box_in_the_grid_selects_the_items_it_touches() {
    let f = open_window();
    open_folder(&f, "gridbox", &["a.txt", "b.txt", "c.txt"]);
    set_view(&f, ViewMode::LargeIcons);
    let c = f.center(&f.list_row("c.txt"));
    let b = f.center(&f.list_row("b.txt"));
    // from the empty space right of the last item, left into b
    let from = LogicalPosition::new(c.x + 300.0, c.y);
    f.drag_to(from, b, &[]);
    assert!(box_shown(&f), "the box shows while dragging");
    f.release(b);
    assert_eq!(f.selected(), ["b.txt", "c.txt"]);
    assert!(!box_shown(&f));
    let w = f.ui.window();
    w.dispatch_event(WindowEvent::PointerPressed { position: from, button: PointerEventButton::Left });
    w.dispatch_event(WindowEvent::PointerReleased { position: from, button: PointerEventButton::Left });
    assert!(f.selected().is_empty(), "a click on empty space clears the selection");
}

fn a_file_dragged_onto_a_folder_in_the_grid_asks_then_moves() {
    let f = open_window();
    let dir = open_folder(&f, "griddnd", &["into/", "drag.txt"]);
    set_view(&f, ViewMode::Tiles);
    let launched = shell::launched().len();
    f.drag_drop(f.center(&f.list_row("drag.txt")), f.center(&f.list_row("into")));
    answer_drop(&f, "Move");
    wait_until(&f, "moved into the folder", |_| dir.join("into/drag.txt").exists() && !dir.join("drag.txt").exists());
    assert_eq!(shell::launched().len(), launched, "dragging never opens the file");
}

fn grid_menus_target_the_item_or_the_folder_and_type_ahead_works() {
    let f = open_window();
    open_folder(&f, "gridmenu", &["apple.txt", "notes.txt", "zebra.txt"]);
    set_view(&f, ViewMode::Content);
    f.right_click(&f.list_row("notes.txt"));
    assert_eq!(f.selected(), ["notes.txt"], "right-click selects the item");
    assert!(f.menu_item("Open").is_some());
    f.key(Key::Escape);
    pump();
    f.right_click_at(LogicalPosition::new(700.0, 500.0));
    assert!(f.selected().is_empty(), "empty space: the folder's menu");
    assert!(f.menu_item("View").is_some(), "the folder menu offers the views");
    f.key(Key::Escape);
    pump();
    set_view(&f, ViewMode::SmallIcons);
    f.type_text("ze");
    assert_eq!(f.selected(), ["zebra.txt"]);
}

fn images_show_as_thumbnails_in_the_big_icon_views() {
    let f = open_window();
    let dir = f.dir.join("thumbs");
    fs::create_dir(&dir).unwrap();
    image::RgbImage::from_pixel(64, 40, image::Rgb([20, 120, 220])).save(dir.join("pic.png")).unwrap();
    fs::write(dir.join("broken.png"), "not an image").unwrap();
    fs::write(dir.join("plain.txt"), "x").unwrap();
    f.ui.invoke_address_entered(0, dir.display().to_string().into());
    wait_until(&f, "listed", |f| f.names() == ["broken.png", "pic.png", "plain.txt"]);
    let thumb = |f: &Fixture, i: usize| f.rows()[i].thumb.size().width > 0;
    assert!(!thumb(&f, 1), "no thumbnails in Details");
    set_view(&f, ViewMode::LargeIcons);
    wait_until(&f, "the thumbnail", |f| thumb(f, 1));
    assert!(ElementHandle::find_by_accessible_label(&f.ui, "Thumbnail").next().is_some(), "drawn");
    settle(&f);
    assert!(!thumb(&f, 0), "a broken file keeps its icon");
    assert!(!thumb(&f, 2), "other files keep their type icon");
    set_view(&f, ViewMode::SmallIcons);
    assert!(!thumb(&f, 1), "small views use icons");
    // regression: reading the images for thumbnails refreshed the folder, over and over
    let listed = f.names();
    settle(&f);
    assert_eq!(f.names(), listed);
}

/// The Java tests `photoshopFilesShowAPreviewInTheIconViews`, `cameraRawFilesShowAPreviewInTheIconViews`
/// and the thumbnail half of `jpeg2000AndPpmImagesArePreviewedAndShowAsThumbnails`.
fn photoshop_raw_jpeg2000_and_netpbm_files_show_thumbnails() {
    use fileem_core::images::fixtures;
    let f = open_window();
    let dir = f.dir.join("own");
    fs::create_dir(&dir).unwrap();
    // a 2 × 2 raw RGB Photoshop file: red
    let planes = [vec![255; 4], vec![0; 4], vec![0; 4]];
    fs::write(dir.join("art.psd"), fixtures::psd(&fixtures::Psd { width: 2, height: 2, planes: &planes, ..fixtures::Psd::default() })).unwrap();
    fs::write(dir.join("IMG_2.CR2"), fixtures::cr2(&fixtures::jpeg(200, 150), &fixtures::jpeg(40, 30), 1)).unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fileem-core/testdata/jpeg2000");
    fs::copy(core.join("red-blue.j2k"), dir.join("red-blue.j2k")).unwrap();
    // a 2 × 1 PPM: green then white
    fs::write(dir.join("pixels.ppm"), b"P6 2 1 255\n\x00\xff\x00\xff\xff\xff").unwrap();
    fs::write(dir.join("broken.jp2"), "not an image").unwrap();
    fs::write(dir.join("plain.txt"), "x").unwrap();
    f.ui.invoke_address_entered(0, dir.display().to_string().into());
    wait_until(&f, "listed", |f| f.names().len() == 6);
    set_view(&f, ViewMode::LargeIcons);
    let thumb = |f: &Fixture, name: &str| {
        let row = f.rows().into_iter().find(|r| r.name == name).unwrap();
        row.thumb.to_rgba8().filter(|b| b.width() > 0)
    };
    let pixel = |b: &slint::SharedPixelBuffer<slint::Rgba8Pixel>, x: usize, y: usize| {
        let p = b.as_slice()[y * b.width() as usize + x];
        [p.r, p.g, p.b, p.a]
    };
    for name in ["art.psd", "IMG_2.CR2", "red-blue.j2k", "pixels.ppm"] {
        wait_until(&f, name, |f| thumb(f, name).is_some());
    }
    assert_eq!(pixel(&thumb(&f, "art.psd").unwrap(), 1, 1), [255, 0, 0, 255], "the Photoshop file is red");
    let raw = thumb(&f, "IMG_2.CR2").unwrap();
    assert_eq!((raw.width(), raw.height()), (96, 72), "the raw's preview (not its tiny thumbnail), loaded at 96 px");
    let j2k = thumb(&f, "red-blue.j2k").unwrap();
    assert_eq!((j2k.width(), j2k.height()), (96, 64), "120 × 80, shrunk");
    assert_eq!(pixel(&j2k, 10, 32), [255, 0, 0, 255], "left is red");
    assert_eq!(pixel(&j2k, 86, 32), [0, 0, 255, 255], "right is blue");
    let ppm = thumb(&f, "pixels.ppm").unwrap();
    assert_eq!((pixel(&ppm, 0, 0), pixel(&ppm, 1, 0)), ([0, 255, 0, 255], [255, 255, 255, 255]));
    settle(&f);
    assert!(thumb(&f, "broken.jp2").is_none(), "a broken file keeps its icon");
    assert!(thumb(&f, "plain.txt").is_none());
}

// ---- the preview (stage 6) ----

fn preview_on(f: &Fixture) {
    if !app::preview_state(&f.app).0 {
        f.key(Key::F3);
    }
    pump();
}

fn preview_floating(f: &Fixture) -> bool {
    app::preview_window(&f.app).is_some_and(|w| w.window().is_visible())
}

/// A button of the preview (the floating window's, or the docked pane's).
fn preview_button(f: &Fixture, label: &str) -> ElementHandle {
    let found = match app::preview_window(&f.app).filter(|w| w.window().is_visible()) {
        Some(w) => ElementHandle::find_by_accessible_label(&w, label).find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button)),
        None => ElementHandle::find_by_accessible_label(&f.ui, label).find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button)),
    };
    found.unwrap_or_else(|| panic!("no preview button {label:?}"))
}

fn press_preview(f: &Fixture, label: &str) {
    preview_button(f, label).invoke_accessible_default_action();
    pump();
}

fn zoom_of(f: &Fixture) -> f64 {
    app::preview_state(&f.app).3
}

/// Selects `name` alone in the list (the preview follows the selection).
fn select_only(f: &Fixture, name: &str) {
    f.click_at(&f.list_row(name));
    pump();
}

/// A test folder with these files in it, opened.
fn open_with_files(f: &Fixture, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let dir = f.dir.join(name);
    fs::create_dir(&dir).unwrap();
    for (n, data) in files {
        fs::write(dir.join(n), data).unwrap();
    }
    f.ui.invoke_address_entered(0, dir.display().to_string().into());
    wait_until(f, "the folder", |f| f.names().len() == files.len());
    dir
}

fn the_preview_is_switched_on_and_off_with_f3_the_toolbar_or_by_closing_it() {
    let f = open_window();
    assert!(!app::preview_state(&f.app).0 && !preview_floating(&f), "off by default");
    f.key(Key::F3);
    assert!(preview_floating(&f), "F3 shows it");
    assert!(f.ui.get_preview_on(), "the toolbar button follows");
    f.key(Key::F3);
    assert!(!preview_floating(&f) && !f.ui.get_preview_on(), "F3 hides it");
    f.click("Preview (F3)");
    pump();
    assert!(preview_floating(&f), "the toolbar button shows it");
    // Escape in its window, and closing the window, switch it off
    let w = app::preview_window(&f.app).unwrap();
    w.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Escape.into() });
    w.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Escape.into() });
    pump();
    assert!(!app::preview_state(&f.app).0 && !preview_floating(&f), "Escape");
    f.click("F3 Preview");
    pump();
    assert!(preview_floating(&f), "the F-key bar");
    w.window().dispatch_event(WindowEvent::CloseRequested);
    pump();
    assert!(!app::preview_state(&f.app).0 && !f.ui.get_preview_on(), "closing switches it off");
    // keys the preview window doesn't use go to the files
    f.key(Key::F3);
    w.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Home.into() });
    pump();
    assert_eq!(f.current().as_deref(), Some("alpha"), "Home in the preview window moves in the list");
    f.key(Key::F3);
}

fn an_image_preview_zooms_fits_and_pans() {
    let f = open_window();
    let mut png = Vec::new();
    image::RgbImage::from_pixel(2000, 1500, image::Rgb([30, 120, 200]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    open_with_files(&f, "previmg", &[("big.png", &png), ("a.txt", b"x")]);
    preview_on(&f);
    select_only(&f, "big.png");
    wait_until(&f, "the image", |f| f.ui.get_preview().mode == 1);
    let v = f.ui.get_preview();
    assert_eq!((v.image.size().width, v.image.size().height), (2000, 1500), "decoded at its size");
    assert_eq!(v.title, "big.png");
    let (_, _, shown, zoom, fitting) = app::preview_state(&f.app);
    assert_eq!(shown.as_deref(), Some("big.png"));
    assert!(fitting && zoom < 1.0, "starts fitted to the window: {zoom}");
    assert!(v.image_w <= 560.0 && (v.image_w / v.image_h - 4.0 / 3.0).abs() < 0.01, "the whole image, same shape: {} × {}", v.image_w, v.image_h);

    press_preview(&f, "Actual size (100%)");
    assert_eq!(zoom_of(&f), 1.0);
    assert_eq!(f.ui.get_preview().image_w, 2000.0);
    press_preview(&f, "Zoom in");
    assert_eq!(zoom_of(&f), 1.25);
    press_preview(&f, "Zoom out");
    press_preview(&f, "Zoom out");
    assert!((zoom_of(&f) - 0.8).abs() < 1e-9);

    press_preview(&f, "Actual size (100%)");
    let w = app::preview_window(&f.app).unwrap();
    let image_at = || ElementHandle::find_by_accessible_label(&w, "Previewed image").next().unwrap().absolute_position();
    let start = image_at();
    press_preview(&f, "Pan right");
    press_preview(&f, "Pan down");
    let moved = image_at();
    assert!(moved.x < start.x && moved.y < start.y, "panned: {start:?} → {moved:?}");
    press_preview(&f, "Pan left");
    assert!(image_at().x > moved.x, "panned back left");
    press_preview(&f, "Fit to window");
    let (_, _, _, zoom, fitting) = app::preview_state(&f.app);
    assert!(fitting && zoom < 1.0);

    // Ctrl+wheel zooms
    let at = LogicalPosition::new(image_at().x + 50.0, image_at().y + 50.0);
    w.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    w.window().dispatch_event(WindowEvent::PointerMoved { position: at });
    w.window().dispatch_event(WindowEvent::PointerScrolled { position: at, delta_x: 0.0, delta_y: 40.0 });
    w.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    pump();
    assert!((zoom_of(&f) - zoom * 1.25).abs() < 1e-9, "Ctrl+wheel up zooms in");
    f.key(Key::F3);
}

fn text_files_show_their_text_and_zoom_changes_the_font() {
    let f = open_window();
    let dir = open_with_files(&f, "prevtxt", &[("notes.md", b"# Title\nsome notes"), ("data.bin", &[0, 1, 2, 3]), ("README", b"plain text without an extension")]);
    fs::create_dir(dir.join("sub")).unwrap();
    wait_until(&f, "sub listed", |f| f.names().len() == 4);
    preview_on(&f);
    select_only(&f, "notes.md");
    wait_until(&f, "the text", |f| f.ui.get_preview().text == "# Title\nsome notes");
    assert_eq!(f.ui.get_preview().font_size, 13.0);
    press_preview(&f, "Zoom in");
    assert_eq!(zoom_of(&f), 1.25);
    assert_eq!(f.ui.get_preview().font_size, 16.25, "zoom scales the font");
    press_preview(&f, "Fit to window");
    assert_eq!(zoom_of(&f), 1.0, "fit: the normal size for text");

    select_only(&f, "README");
    wait_until(&f, "unknown files are checked for text", |f| f.ui.get_preview().text == "plain text without an extension");
    select_only(&f, "data.bin");
    wait_until(&f, "binary: no preview", |f| f.ui.get_preview().mode == 0 && f.ui.get_preview().message.contains("No preview available"));
    assert!(f.ui.get_preview().message.starts_with("data.bin\nBIN Application\n4 bytes"), "{}", f.ui.get_preview().message);
    assert!(!preview_button(&f, "Zoom in").accessible_enabled().unwrap_or(true), "no zooming a message");
    select_only(&f, "sub");
    wait_until(&f, "folder", |f| f.ui.get_preview().message == "sub\nFolder");

    // long files: the start, and a note
    fs::write(dir.join("long.txt"), "x".repeat(300 * 1024)).unwrap();
    wait_until(&f, "listed", |f| f.names().len() == 5);
    select_only(&f, "long.txt");
    wait_until(&f, "the start of it", |f| f.ui.get_preview().mode == 2);
    assert!(f.ui.get_preview().text.ends_with("… (only the first 256 KB is shown)"), "a note");
    f.ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    f.key("a");
    f.ui.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    pump();
    settle(&f);
    assert_eq!(app::preview_state(&f.app).2.as_deref(), Some("long.txt"), "the focused one of several selected");
    f.key(Key::F3);
}

fn quick_selection_changes_only_load_the_last_one() {
    let f = open_window();
    open_with_files(&f, "prevquick", &[("one.txt", b"one"), ("two.txt", b"two"), ("three.txt", b"three")]);
    preview_on(&f);
    // arrow keys through the folder, faster than the settle time
    f.key(Key::Home);
    f.key(Key::DownArrow);
    f.key(Key::DownArrow);
    wait_until(&f, "the last one", |f| f.ui.get_preview().text == "two");
    assert_eq!(f.current().as_deref(), Some("two.txt"));
    settle(&f);
    assert_eq!(f.ui.get_preview().title, "two.txt");
    f.key(Key::F3);
}

fn photoshop_raw_jpeg2000_and_netpbm_files_are_previewed() {
    use fileem_core::images::fixtures;
    let f = open_window();
    let planes = [vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
    let psd = fixtures::psd(&fixtures::Psd { width: 3, height: 1, planes: &planes, ..fixtures::Psd::default() });
    let cr2 = fixtures::cr2(&fixtures::jpeg(60, 40), &fixtures::jpeg(16, 12), 6);
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fileem-core/testdata/jpeg2000");
    let j2k = fs::read(core.join("red-blue.j2k")).unwrap();
    let dng = fixtures::tiff(false, 42, &[fixtures::Ifd::new().short(0x103, 1)]);
    open_with_files(
        &f,
        "previmages",
        &[
            ("art.psd", &psd),
            ("IMG_1.CR2", &cr2),
            ("hdr.dng", &dng),
            ("red-blue.j2k", &j2k),
            ("broken.jp2", b"not an image"),
            ("pixels.ppm", b"P6 2 1 255\n\x00\xff\x00\xff\xff\xff"),
        ],
    );
    preview_on(&f);
    let pixel = |f: &Fixture, x: usize, y: usize| {
        let b = f.ui.get_preview().image.to_rgba8().unwrap();
        let p = b.as_slice()[y * b.width() as usize + x];
        [p.r, p.g, p.b, p.a]
    };
    let image = |f: &Fixture, name: &str| {
        select_only(f, name);
        wait_until(f, name, |f| f.ui.get_preview().title == name && f.ui.get_preview().mode != 0);
        let size = f.ui.get_preview().image.size();
        (size.width, size.height)
    };
    assert_eq!(image(&f, "art.psd"), (3, 1));
    // the camera's JPEG turned upright (orientation 6: 90° clockwise): its red left half is on top
    assert_eq!(image(&f, "IMG_1.CR2"), (40, 60));
    assert!(pixel(&f, 20, 5)[0] > 200 && pixel(&f, 20, 55)[0] < 60, "top red, bottom black");
    assert_eq!(image(&f, "red-blue.j2k"), (120, 80));
    assert_eq!(pixel(&f, 10, 40), [255, 0, 0, 255], "left is red");
    assert_eq!(pixel(&f, 110, 40), [0, 0, 255, 255], "right is blue");
    assert_eq!(image(&f, "pixels.ppm"), (2, 1));
    assert_eq!(pixel(&f, 0, 0), [0, 255, 0, 255]);
    assert_eq!(pixel(&f, 1, 0), [255, 255, 255, 255]);

    let message = |f: &Fixture, name: &str, says: &str| {
        select_only(f, name);
        wait_until(f, says, |f| f.ui.get_preview().title == name && f.ui.get_preview().message.contains(says));
    };
    message(&f, "hdr.dng", "no embedded preview");
    message(&f, "broken.jp2", "Cannot show this image.");
    f.key(Key::F3);
}

fn the_preview_docks_into_the_main_window_and_floats_again() {
    let f = open_window();
    open_with_files(&f, "prevdock", &[("notes.txt", b"docked text")]);
    preview_on(&f);
    let w = app::preview_window(&f.app).unwrap();
    let close = |e: &ElementHandle| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button);
    assert!(ElementHandle::find_by_accessible_label(&w, "Close the preview (F3)").find(close).is_none(), "a floating window closes like a window");

    press_preview(&f, "Dock in the main window");
    assert!(f.ui.get_preview_docked() && !preview_floating(&f), "docked");
    assert!(app::preview_state(&f.app).1);
    select_only(&f, "notes.txt");
    wait_until(&f, "previews while docked", |f| f.ui.get_preview().text == "docked text");
    assert!(f.list_row("notes.txt").absolute_position().x < 1100.0 * 0.7, "the files make room");

    f.key(Key::F3);
    assert!(!f.ui.get_preview_docked() && !app::preview_state(&f.app).0, "F3 hides the docked pane");
    f.key(Key::F3);
    assert!(f.ui.get_preview_docked() && !preview_floating(&f), "and shows it docked again");
    wait_until(&f, "the text again", |f| f.ui.get_preview().text == "docked text");

    press_preview(&f, "Float in its own window");
    assert!(preview_floating(&f) && !f.ui.get_preview_docked(), "floating again");
    press_preview(&f, "Dock in the main window");
    assert!(f.ui.get_preview_docked());
    press_preview(&f, "Close the preview (F3)");
    assert!(!f.ui.get_preview_docked() && !app::preview_state(&f.app).0, "the close button switches it off");
}

fn the_preview_is_remembered_in_the_session() {
    let f = open_window();
    preview_on(&f);
    press_preview(&f, "Dock in the main window");
    f.ui.set_preview_split(0.6);
    app::collect_session(&f.app);
    let s = app::session(&f.app);
    assert!(s.get_bool("preview.enabled", false) && s.get_bool("preview.docked", false));
    assert!((s.get_f64("split.preview", 0.0) - 0.6).abs() < 0.01);
    let saved: Vec<(&str, String)> = ["preview.enabled", "preview.docked", "split.preview"].into_iter().map(|k| (k, s.get(k).unwrap())).collect();
    drop(f);

    let f = open_window_with(
        |s, _| {
            for (k, v) in &saved {
                s.put(k, v);
            }
        },
        true,
    );
    assert!(f.ui.get_preview_docked() && app::preview_state(&f.app).0, "restored docked");
    assert!((f.ui.get_preview_split() - 0.6).abs() < 0.01);
    assert!(!preview_floating(&f));
    press_preview(&f, "Float in its own window");
    assert!(preview_floating(&f));
    let w = app::preview_window(&f.app).unwrap();
    w.window().set_size(slint::LogicalSize::new(700.0, 500.0));
    app::collect_session(&f.app);
    assert_eq!(app::session(&f.app).get_int("preview.width", 0), 700);
    f.key(Key::F3);
}

// ---- search (stage 7) ----

fn sw_key(w: &SearchWindow, text: impl Into<SharedString>) {
    let text = text.into();
    w.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.window().dispatch_event(WindowEvent::KeyReleased { text });
    pump();
}

fn sw_chord(w: &SearchWindow, modifiers: &[Key], key: impl Into<SharedString>) {
    for m in modifiers {
        w.window().dispatch_event(WindowEvent::KeyPressed { text: (*m).into() });
    }
    sw_key(w, key);
    for m in modifiers.iter().rev() {
        w.window().dispatch_event(WindowEvent::KeyReleased { text: (*m).into() });
    }
}

fn sw_type(w: &SearchWindow, text: &str) {
    for c in text.chars() {
        sw_key(w, c.to_string());
    }
}

/// A click on an element of a search window (`offset`: a popup's position in it).
fn sw_click(w: &SearchWindow, e: &ElementHandle, offset: (f32, f32), modifiers: &[Key]) {
    let (pos, size) = (e.absolute_position(), e.size());
    let position = LogicalPosition::new(offset.0 + pos.x + (size.width / 2.0).min(30.0), offset.1 + pos.y + size.height / 2.0);
    let win = w.window();
    for m in modifiers {
        win.dispatch_event(WindowEvent::KeyPressed { text: (*m).into() });
    }
    win.dispatch_event(WindowEvent::PointerMoved { position });
    win.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
    win.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
    for m in modifiers.iter().rev() {
        win.dispatch_event(WindowEvent::KeyReleased { text: (*m).into() });
    }
    pump();
}

/// Clicks the result row `name` (Ctrl held: adds to the selection).
fn sw_select(w: &SearchWindow, name: &str, ctrl: bool) {
    let row = ElementHandle::find_by_accessible_label(w, name)
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem))
        .unwrap_or_else(|| panic!("no result {name:?}"));
    sw_click(w, &row, (0.0, 0.0), if ctrl { &[Key::Control] } else { &[] });
}

/// Types `pattern` into the search window's field and runs it to the end.
fn run_search(f: &Fixture, w: &SearchWindow, pattern: &str) {
    w.invoke_focus_pattern();
    w.set_pattern("".into());
    sw_type(w, pattern);
    sw_key(w, Key::Return);
    wait_until(f, "the search finished", |_| !w.get_running());
}

/// The `index`th search window's results, relative to `root`, sorted.
fn found(f: &Fixture, index: usize, root: &Path) -> Vec<String> {
    let mut v: Vec<String> = app::search_results(&f.app, index)
        .0
        .iter()
        .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    v.sort();
    v
}

/// A test folder with these files (a path ending in / is a folder), opened in the tab.
fn open_tree(f: &Fixture, name: &str, paths: &[&str]) -> PathBuf {
    let dir = f.dir.join(name);
    for p in paths {
        if let Some(folder) = p.strip_suffix('/') {
            fs::create_dir_all(dir.join(folder)).unwrap();
        } else {
            fs::create_dir_all(dir.join(p).parent().unwrap()).unwrap();
            fs::write(dir.join(p), p).unwrap();
        }
        // dot names are hidden (on Windows by attribute)
        let mut at = dir.clone();
        for part in Path::new(p.trim_end_matches('/')).components() {
            at.push(part);
            if part.as_os_str().to_string_lossy().starts_with('.') && !fileem_core::util::fs_util::is_hidden(&at) {
                fileem_core::util::fs_util::hide(&at);
            }
        }
    }
    f.ui.invoke_address_entered(0, dir.display().to_string().into());
    wait_until(f, "the folder", |f| f.address() == dir);
    dir
}

/// Runs a command of the search window's menu by its title, as a click would.
fn sw_menu(w: &SearchWindow, title: &str) {
    let rows: Vec<fileem::MenuRow> =
        [w.get_menu_s0(), w.get_menu_s1(), w.get_menu_s2(), w.get_menu_s3()].iter().flat_map(|m| m.iter().collect::<Vec<_>>()).collect();
    let item = rows.iter().find(|r| r.title.starts_with(title)).unwrap_or_else(|| panic!("no {title:?} in the menu"));
    assert!(item.enabled, "{title} is enabled");
    w.invoke_menu_activated(item.id);
    pump();
}

fn search_from_the_right_click_menu_finds_files_in_all_subfolders() {
    let f = open_window();
    let dir = open_tree(&f, "searchhere", &["top.jpg", "notes.txt", "a/b/Deep.JPG", "a/b/2019_09_08.txt", "photos.jpg/"]);
    let last = f.list_row("top.jpg");
    f.right_click_at(LogicalPosition::new(last.absolute_position().x + 50.0, last.absolute_position().y + 150.0));
    f.pick("Search in folder…  (Ctrl+Shift+F)");
    wait_until(&f, "a search window", |f| app::search_windows(&f.app).len() == 1);
    let w = app::search_windows(&f.app).remove(0);
    assert_eq!(w.get_root_path(), dir.display().to_string());
    assert_eq!(w.get_window_title(), "Search in searchhere");
    assert!(w.window().is_visible());

    run_search(&f, &w, "*.jpg");
    assert_eq!(found(&f, 0, &dir), ["a/b/Deep.JPG", "photos.jpg", "top.jpg"], "every subfolder, any case, folders too");
    assert!(w.get_status().starts_with("3 items found"), "{}", w.get_status());
    for title in ["Name", "Folder", "Size", "Type", "Date modified"] {
        assert!(ElementHandle::find_by_accessible_label(&w, title).next().is_some(), "column {title}");
    }
    let rows: Vec<fileem::SearchRow> = w.get_rows().iter().collect();
    let row = |n: &str| rows.iter().find(|r| r.name == n).unwrap().clone();
    assert_eq!(row("photos.jpg").kind, "Folder");
    assert_eq!(row("top.jpg").kind, fileem_core::fs::FileEntry::read(&dir.join("top.jpg")).type_name(), "the file list's types");
    assert_eq!(row("Deep.JPG").folder, Path::new("a").join("b").display().to_string());
    assert_eq!(row("top.jpg").folder, ".");

    run_search(&f, &w, "*.jpg !top*");
    assert_eq!(found(&f, 0, &dir), ["a/b/Deep.JPG", "photos.jpg"], "! excludes");
    run_search(&f, &w, "_09_");
    assert_eq!(found(&f, 0, &dir), ["a/b/2019_09_08.txt"], "plain text matches inside names");

    // the folder holding a result opens in the tab, with the result selected
    sw_select(&w, "2019_09_08.txt", false);
    w.invoke_row_menu(0);
    sw_menu(&w, "Open containing folder");
    wait_until(&f, "the result's folder", |f| f.address() == dir.join("a/b"));
    wait_until(&f, "the result selected", |f| f.selected() == ["2019_09_08.txt"]);

    // a file opens with its program, a folder in the tab
    run_search(&f, &w, "*.jpg");
    sw_select(&w, "top.jpg", false);
    sw_key(&w, Key::Return);
    let wanted = format!("open {}", dir.join("top.jpg").display());
    assert!(shell::launched().contains(&wanted), "{:?}", shell::launched());
    let photos = ElementHandle::find_by_accessible_label(&w, "photos.jpg").next().unwrap();
    sw_click(&w, &photos, (0.0, 0.0), &[]);
    sw_click(&w, &photos, (0.0, 0.0), &[]);
    wait_until(&f, "the folder result opened", |f| f.address() == dir.join("photos.jpg"));

    sw_key(&w, Key::Escape);
    assert!(app::search_windows(&f.app).is_empty(), "Escape closes it when nothing runs");
    assert!(!w.window().is_visible());
}

fn the_last_search_patterns_are_offered_in_every_window_and_remembered() {
    let f = open_window();
    let dir = open_tree(&f, "searchhist", &["a.jpg", "b.png", "c.txt"]);
    let first = app::open_search(&f.app, dir.clone()).unwrap();
    run_search(&f, &first, "*.jpg");
    run_search(&f, &first, "*.png");
    run_search(&f, &first, "*.jpg"); // used again: moves to the top, not listed twice
    run_search(&f, &first, "   "); // nothing to look for: not remembered
    assert_eq!(first.get_status(), "Type a name or a pattern to search for.");
    first.window().dispatch_event(WindowEvent::CloseRequested);
    pump();
    assert!(app::search_windows(&f.app).is_empty());

    let second = app::open_search(&f.app, dir.clone()).unwrap();
    assert_eq!(second.get_history().iter().collect::<Vec<_>>(), ["*.jpg", "*.png"], "newest first");
    assert_eq!(second.get_pattern(), "", "a new window starts empty");

    // Alt+Down lists them; picking one searches it
    second.invoke_focus_pattern();
    sw_chord(&second, &[Key::Alt], Key::DownArrow);
    let item = ElementHandle::find_by_accessible_label(&second, "*.png")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem))
        .expect("the history list is open");
    let offset = {
        let inner = i_slint_core::window::WindowInner::from_pub(second.window());
        match inner.active_popups().last().map(|p| &p.location) {
            Some(i_slint_core::window::PopupWindowLocation::ChildWindow(at)) => (at.x, at.y),
            _ => (0.0, 0.0),
        }
    };
    sw_click(&second, &item, offset, &[]);
    wait_until(&f, "the picked pattern searched", |f| !second.get_running() && found(f, 0, &dir) == ["b.png"]);
    assert_eq!(second.get_pattern(), "*.png");
    assert!(i_slint_core::window::WindowInner::from_pub(second.window()).active_popups().is_empty(), "the list closed");
    sw_key(&second, Key::Escape);

    // a restart offers them again
    app::collect_session(&f.app);
    let ui = AppWindow::new().unwrap();
    let again = app::install(&ui, app::session(&f.app), Some(dir.clone()));
    let third = app::open_search(&again, dir).unwrap();
    assert_eq!(third.get_history().iter().collect::<Vec<_>>(), ["*.png", "*.jpg"]);
    app::close_windows(&again);
}

fn search_results_can_be_renamed_and_deleted_in_place() {
    let f = open_window();
    let dir = open_tree(&f, "searchact", &["keep.txt", "old.txt", "gone.txt", "box/inner.txt"]);
    let w = app::open_search(&f.app, dir.clone()).unwrap();
    run_search(&f, &w, "*.txt box");
    assert_eq!(found(&f, 0, &dir), ["box", "box/inner.txt", "gone.txt", "keep.txt", "old.txt"]);

    // the right-click menu has the file list's commands
    sw_select(&w, "old.txt", false);
    w.invoke_row_menu(app::search_results(&f.app, 0).0.iter().position(|p| p.ends_with("old.txt")).unwrap() as i32);
    let titles: Vec<String> = [w.get_menu_s0(), w.get_menu_s1(), w.get_menu_s2(), w.get_menu_s3()]
        .iter()
        .flat_map(|m| m.iter().map(|r| r.title.to_string()).collect::<Vec<_>>())
        .collect();
    for expected in ["Open", "Cut", "Copy", "Rename", "Delete permanently", "Copy path", "Properties"] {
        assert!(titles.iter().any(|t| t.starts_with(expected)), "{expected} in {titles:?}");
    }
    assert_eq!(titles.iter().any(|t| t.starts_with("Move to Trash")), fileem_core::fs::trash::is_supported());

    // F2 renames in place, over the search window: the result follows, still selected
    sw_key(&w, Key::F2);
    assert!(w.get_dialog_open() && !f.ui.get_dialog_open(), "asked over the search window");
    assert_eq!(w.get_dialog_text(), "old.txt");
    sw_type(&w, "new"); // replaces the selected "old"
    sw_key(&w, Key::Return);
    wait_until(&f, "renamed", |_| dir.join("new.txt").exists() && !dir.join("old.txt").exists());
    wait_until(&f, "the result renamed", |f| found(f, 0, &dir).contains(&"new.txt".to_string()));
    assert!(!found(&f, 0, &dir).contains(&"old.txt".to_string()));
    assert_eq!(app::search_results(&f.app, 0).1, [dir.join("new.txt")], "still selected");

    // renaming a folder moves the results found inside it along
    sw_select(&w, "box", false);
    sw_key(&w, Key::F2);
    w.set_dialog_text("crate".into());
    w.invoke_dialog_accepted("crate".into());
    wait_until(&f, "the folder's results follow", |f| {
        let r = found(f, 0, &dir);
        r.contains(&"crate".to_string()) && r.contains(&"crate/inner.txt".to_string())
    });
    assert!(app::search_results(&f.app, 0).0.iter().all(|p| p.exists()), "{:?}", app::search_results(&f.app, 0));

    // Shift+Del asks, then deletes; deleted results (and what was inside a deleted folder) leave
    sw_select(&w, "gone.txt", false);
    sw_chord(&w, &[Key::Shift], Key::Delete);
    assert!(w.get_dialog_open());
    sw_key(&w, Key::Escape);
    assert!(!w.get_dialog_open() && dir.join("gone.txt").exists(), "cancelled: nothing deleted");
    assert!(!app::search_windows(&f.app).is_empty(), "Escape closed only the question");

    sw_select(&w, "crate", true);
    sw_chord(&w, &[Key::Shift], Key::Delete);
    assert!(w.get_dialog_message().contains("2 items"), "{}", w.get_dialog_message());
    sw_key(&w, Key::Return);
    wait_until(&f, "deleted", |_| !dir.join("gone.txt").exists() && !dir.join("crate").exists());
    wait_until(&f, "the results updated", |f| found(f, 0, &dir) == ["keep.txt", "new.txt"]);
    assert!(w.get_status().contains("3 items removed"), "{}", w.get_status());

    // Ctrl+C puts the results on the file clipboard
    sw_select(&w, "keep.txt", false);
    sw_chord(&w, &[Key::Control], "c");
    let into = dir.join("into");
    fs::create_dir(&into).unwrap();
    f.ui.invoke_address_entered(0, into.display().to_string().into());
    wait_until(&f, "the paste target", |f| f.address() == into);
    f.chord(&[Key::Control], "v");
    wait_until(&f, "pasted", |_| into.join("keep.txt").exists());
    assert!(dir.join("keep.txt").exists(), "copied, not moved");
    sw_key(&w, Key::Escape);
}

fn ctrl_shift_f_searches_the_current_folder_and_hidden_files_follow_the_toggle() {
    let f = open_window();
    let dir = open_tree(&f, "searchkey", &["x.log", ".hidden/y.log", "sub/.z.log"]);
    f.chord(&[Key::Control, Key::Shift], "F");
    wait_until(&f, "a search window", |f| app::search_windows(&f.app).len() == 1);
    let w = app::search_windows(&f.app).remove(0);
    assert_eq!(w.get_root_path(), dir.display().to_string());
    f.type_text("zz");
    assert_eq!(f.view().filter, "", "Ctrl+Shift+F is not the filter");
    assert!(!w.get_include_hidden(), "hidden files are off like in the list");

    run_search(&f, &w, "*.log");
    assert_eq!(found(&f, 0, &dir), ["x.log"]);
    let hidden = ElementHandle::find_by_accessible_label(&w, "Hidden files").next().unwrap();
    sw_click(&w, &hidden, (0.0, 0.0), &[]);
    assert!(w.get_include_hidden());
    run_search(&f, &w, "*.log");
    assert_eq!(found(&f, 0, &dir), [".hidden/y.log", "sub/.z.log", "x.log"]);

    run_search(&f, &w, "   ");
    assert!(found(&f, 0, &dir).is_empty(), "nothing to look for");

    // a search can be stopped: the results so far stay
    for i in 0..300 {
        fs::create_dir_all(dir.join(format!("many/d{i}"))).unwrap();
    }
    w.invoke_focus_pattern();
    w.set_pattern("*".into());
    w.invoke_search("*".into());
    assert!(w.get_running());
    sw_key(&w, Key::Escape);
    assert!(!w.get_running(), "Escape stops a running search");
    assert!(w.get_status().starts_with("Stopped."), "{}", w.get_status());
    assert!(!app::search_windows(&f.app).is_empty(), "and leaves the window open");
    sw_key(&w, Key::Escape);
    assert!(app::search_windows(&f.app).is_empty());
}

// ---- bookmarks ----

/// The names in a tree's Bookmarks section.
fn tree_bookmarks(view: &SideView) -> Vec<String> {
    let rows: Vec<TreeRow> = view.tree.iter().collect();
    let Some(start) = rows.iter().position(|r| r.header && r.name == "Bookmarks") else { return Vec::new() };
    rows[start + 1..].iter().take_while(|r| !r.header).filter(|r| r.depth == 0).map(|r| r.name.to_string()).collect()
}

/// Bookmarks written into the session before the window opens (`(name, folder)`).
fn with_bookmarks(marks: &'static [(&'static str, &'static str)]) -> impl FnOnce(&Session, &Path) {
    move |s, dir| {
        s.put("bookmarks.count", marks.len());
        for (i, (name, path)) in marks.iter().enumerate() {
            s.put(&format!("bookmarks.{i}.name"), name);
            s.put(&format!("bookmarks.{i}.path"), dir.join(path).display());
        }
    }
}

/// The entries of the menu shown last (with their enabled state, which the popup doesn't expose).
fn menu_rows(f: &Fixture) -> Vec<fileem::MenuRow> {
    let ui = &f.ui;
    [ui.get_menu_s0(), ui.get_menu_s1(), ui.get_menu_s2(), ui.get_menu_s3(), ui.get_menu_s4(), ui.get_menu_s5()]
        .iter()
        .flat_map(|m| m.iter().collect::<Vec<_>>())
        .collect()
}

fn menu_row(f: &Fixture, title: &str) -> fileem::MenuRow {
    menu_rows(f).into_iter().find(|r| r.title == title).unwrap_or_else(|| panic!("no menu entry {title:?}"))
}

/// Exactly this entry ("Copy", not "Copy path").
fn menu_has(f: &Fixture, title: &str) -> bool {
    menu_rows(f).iter().any(|r| r.title == title)
}

fn bookmark_names(f: &Fixture) -> Vec<String> {
    app::bookmarks(&f.app).into_iter().map(|(n, _)| n).collect()
}

fn ctrl_d_and_the_toolbar_toggle_a_bookmark_shown_in_both_trees() {
    let f = open_window();
    assert!(tree_bookmarks(&f.ui.get_left()).is_empty(), "no section without bookmarks");
    assert!(!f.tree().iter().any(|r| r.name == "Bookmarks"));
    assert!(!f.ui.get_bookmarked());
    f.button("Bookmark this folder (Ctrl+D)");

    f.chord(&[Key::Control], "d");
    wait_until(&f, "bookmarked", |f| app::bookmarks(&f.app) == [("files".to_string(), f.dir.clone())]);
    assert_eq!(tree_bookmarks(&f.ui.get_left()), ["files"]);
    assert_eq!(tree_bookmarks(&f.ui.get_right()), ["files"], "both trees");
    let rows = f.tree();
    let header = rows.iter().position(|r| r.name == "Bookmarks").unwrap();
    assert_eq!(rows[header - 1].depth, 0, "after Quick access' places");
    assert!(rows[header..].iter().any(|r| r.header && r.name == "This computer"), "before the drives");
    assert!(f.ui.get_bookmarked());

    // the toolbar button follows the folder shown
    f.double_click("alpha");
    wait_until(&f, "alpha", |f| f.address() == f.dir.join("alpha"));
    assert!(!f.ui.get_bookmarked());
    f.key(Key::Backspace);
    wait_until(&f, "back up", |f| f.address() == f.dir && f.ui.get_bookmarked());

    f.button("Remove bookmark (Ctrl+D)").invoke_accessible_default_action();
    wait_until(&f, "removed", |f| app::bookmarks(&f.app).is_empty());
    assert!(tree_bookmarks(&f.ui.get_left()).is_empty() && tree_bookmarks(&f.ui.get_right()).is_empty());
    assert!(!f.tree().iter().any(|r| r.name == "Bookmarks"), "the section is gone");
    f.button("Bookmark this folder (Ctrl+D)").invoke_accessible_default_action();
    wait_until(&f, "bookmarked from the toolbar", |f| app::bookmarks(&f.app).len() == 1);
    f.chord(&[Key::Control], "d");
    wait_until(&f, "Ctrl+D again removes it", |f| app::bookmarks(&f.app).is_empty());
}

fn folder_menus_add_bookmarks_and_a_bookmark_opens_from_the_tree() {
    let f = open_window();
    f.right_click(&f.list_row("alpha"));
    assert!(f.menu_item("Remove from bookmarks").is_none());
    f.pick("Add to bookmarks  (Ctrl+D)");
    wait_until(&f, "alpha bookmarked", |f| app::bookmarks(&f.app) == [("alpha".to_string(), f.dir.join("alpha"))]);
    f.right_click(&f.list_row("alpha"));
    assert!(f.menu_item("Remove from bookmarks").is_some(), "the menu now offers removal");
    assert!(f.menu_item("Add to bookmarks  (Ctrl+D)").is_none());
    f.key(Key::Escape);
    pump();
    // a file has no bookmark item; empty space bookmarks the folder shown
    f.right_click(&f.list_row("notes.txt"));
    assert!(f.menu_item("Add to bookmarks  (Ctrl+D)").is_none() && f.menu_item("Remove from bookmarks").is_none());
    f.key(Key::Escape);
    pump();
    let last = f.list_row("a.png");
    f.right_click_at(LogicalPosition::new(last.absolute_position().x + 50.0, last.absolute_position().y + 150.0));
    f.pick("Add to bookmarks  (Ctrl+D)");
    wait_until(&f, "the folder bookmarked", |f| bookmark_names(f) == ["alpha", "files"]);

    // the tree's bookmark (renamed, so its name is its own) opens in the active tab
    app::run_command(&f.app, fileem::menu::Command::RenameBookmark(f.dir.join("alpha")));
    wait_until(&f, "the rename question", dialog_open);
    assert_eq!(f.ui.get_dialog_title(), "Rename bookmark");
    assert_eq!(f.ui.get_dialog_text(), "alpha");
    f.ui.set_dialog_text("  Inbox  ".into());
    f.button("Rename").invoke_accessible_default_action();
    wait_until(&f, "renamed", |f| tree_bookmarks(&f.ui.get_left()) == ["Inbox", "files"]);
    assert!(f.dir.join("alpha").is_dir(), "the folder keeps its name");
    tree_to_top(&f);
    f.click_at(&f.tree_row("Inbox"));
    wait_until(&f, "the bookmark opened", |f| f.address() == f.dir.join("alpha") && f.names() == ["inside.txt"]);

    // a folder in the tree offers it too
    tree_to_top(&f);
    f.right_click(&f.tree_row("Inbox"));
    assert!(f.menu_item("Remove bookmark").is_some());
    f.key(Key::Escape);
    pump();
    f.right_click(&f.tree_row("Home"));
    assert!(f.menu_item("Add to bookmarks  (Ctrl+D)").is_some(), "{:?}", app::bookmarks(&f.app));
    f.key(Key::Escape);
    pump();
}

fn the_bookmark_menu_renames_moves_and_removes_and_bookmarks_survive_a_restart() {
    let f = open_window_with(with_bookmarks(&[("Mark A", "alpha"), ("Mark B", "beta")]), true);
    assert_eq!(tree_bookmarks(&f.ui.get_left()), ["Mark A", "Mark B"], "from the session");
    tree_to_top(&f);

    f.right_click(&f.tree_row("Mark B"));
    for forbidden in ["Cut", "Rename…", "Move to Trash", "Delete permanently", "Duplicate", "Add to bookmarks  (Ctrl+D)"] {
        assert!(f.menu_item(forbidden).is_none(), "{forbidden} must not be offered for a bookmark");
    }
    assert!(!menu_row(&f, "Move down").enabled, "the last one can't go down");
    assert!(menu_row(&f, "Move up").enabled);
    f.pick("Move up");
    wait_until(&f, "B first", |f| tree_bookmarks(&f.ui.get_left()) == ["Mark B", "Mark A"]);
    assert_eq!(tree_bookmarks(&f.ui.get_right()), ["Mark B", "Mark A"]);

    f.right_click(&f.tree_row("Mark A"));
    f.pick("Rename bookmark…");
    wait_until(&f, "the rename question", dialog_open);
    f.ui.set_dialog_text("Alpha's".into());
    f.button("Rename").invoke_accessible_default_action();
    wait_until(&f, "renamed", |f| bookmark_names(f) == ["Mark B", "Alpha's"]);
    f.right_click(&f.tree_row("Mark B"));
    f.pick("Remove bookmark");
    wait_until(&f, "removed", |f| tree_bookmarks(&f.ui.get_left()) == ["Alpha's"]);
    assert!(f.dir.join("beta").is_dir(), "removing a bookmark never touches the folder");

    // the section title offers the Manage window
    f.right_click(&f.tree_row("Bookmarks"));
    f.pick("Manage bookmarks…  (Ctrl+Shift+B)");
    wait_until(&f, "the Manage window", |f| app::bookmarks_window(&f.app).is_some());
    app::bookmarks_window(&f.app).unwrap().invoke_close_clicked();
    assert!(app::bookmarks_window(&f.app).is_none());

    // saved at once: a fresh read of the file, as after a crash
    let session = Rc::new(Session::load(&f._tmp.path().join("session.properties")));
    let ui = AppWindow::new().unwrap();
    let again = app::install(&ui, session, None);
    app::set_clipboard(&again, Box::new(fileem::clipboard::Memory::default()));
    assert_eq!(app::bookmarks(&again), [("Alpha's".to_string(), f.dir.join("alpha"))]);
    assert_eq!(tree_bookmarks(&ui.get_left()), ["Alpha's"]);
}

fn bw_key(w: &fileem::BookmarksWindow, modifiers: &[Key], text: impl Into<SharedString>) {
    let win = w.window();
    for m in modifiers {
        win.dispatch_event(WindowEvent::KeyPressed { text: (*m).into() });
    }
    let text = text.into();
    win.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    win.dispatch_event(WindowEvent::KeyReleased { text });
    for m in modifiers.iter().rev() {
        win.dispatch_event(WindowEvent::KeyReleased { text: (*m).into() });
    }
    pump();
}

fn bw_button(w: &fileem::BookmarksWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(w, label)
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no button {label:?}"))
}

fn bw_rows(w: &fileem::BookmarksWindow) -> Vec<fileem::BookmarkRow> {
    w.get_rows().iter().collect()
}

fn the_manage_window_lists_reorders_renames_removes_and_opens_bookmarks() {
    let f = open_window_with(with_bookmarks(&[("m1", "alpha"), ("gone", "not-there"), ("m3", "beta")]), true);
    f.chord(&[Key::Control, Key::Shift], "b");
    wait_until(&f, "the Manage window", |f| app::bookmarks_window(&f.app).is_some());
    let w = app::bookmarks_window(&f.app).unwrap();
    assert!(w.window().is_visible());
    let names = |w: &fileem::BookmarksWindow| bw_rows(w).iter().map(|r| r.name.to_string()).collect::<Vec<_>>();
    assert_eq!(names(&w), ["m1", "gone", "m3"]);
    assert_eq!(bw_rows(&w)[0].path, f.dir.join("alpha").display().to_string());
    assert_eq!(w.get_selected(), 0, "the first one is selected");
    wait_until(&f, "the missing folder is marked", |_| bw_rows(&w)[1].missing);
    assert!(!bw_rows(&w)[0].missing && !bw_rows(&w)[2].missing);
    assert_eq!(bw_button(&w, "Move up").accessible_enabled(), Some(false), "nothing above the first");

    bw_button(&w, "Move down").invoke_accessible_default_action();
    wait_until(&f, "m1 moved down", |f| bookmark_names(f) == ["gone", "m1", "m3"]);
    assert_eq!(names(&w), ["gone", "m1", "m3"]);
    assert_eq!(w.get_selected(), 1, "the moved one stays selected");
    bw_key(&w, &[Key::Alt], Key::DownArrow);
    wait_until(&f, "Alt+Down moves it again", |f| bookmark_names(f) == ["gone", "m3", "m1"]);
    assert_eq!(tree_bookmarks(&f.ui.get_left()), ["gone", "m3", "m1"], "the trees follow");

    bw_key(&w, &[], Key::UpArrow);
    bw_key(&w, &[], Key::UpArrow);
    assert_eq!(w.get_selected(), 0);
    bw_button(&w, "Remove").invoke_accessible_default_action();
    wait_until(&f, "removed", |f| bookmark_names(f) == ["m3", "m1"]);
    assert_eq!(names(&w), ["m3", "m1"], "the list is live");
    assert_eq!(w.get_selected(), 0, "the next one is selected");

    // F2 asks over the window itself
    bw_key(&w, &[], Key::F2);
    assert!(w.get_dialog_open() && !f.ui.get_dialog_open());
    assert_eq!(w.get_dialog_text(), "m3");
    w.set_dialog_text("Third".into());
    bw_button(&w, "Rename").invoke_accessible_default_action();
    wait_until(&f, "renamed", |f| bookmark_names(f) == ["Third", "m1"]);
    assert!(!w.get_dialog_open());

    // changes from the main window show up too
    f.chord(&[Key::Control], "d");
    wait_until(&f, "a bookmark added elsewhere", |_| names(&w) == ["Third", "m1", "files"]);
    bw_key(&w, &[], Key::Delete);
    wait_until(&f, "Del removes", |f| bookmark_names(f) == ["m1", "files"]);

    // Enter opens the selected one in the active tab and closes the window
    bw_key(&w, &[], Key::Return);
    wait_until(&f, "m1 opened", |f| f.address() == f.dir.join("alpha"));
    assert!(app::bookmarks_window(&f.app).is_none(), "closed");
    assert!(!w.window().is_visible());

    // empty: a hint instead of rows, and nothing to act on
    app::run_command(&f.app, fileem::menu::Command::ManageBookmarks);
    let w = app::bookmarks_window(&f.app).unwrap();
    bw_key(&w, &[], Key::Delete);
    bw_key(&w, &[], Key::Delete);
    wait_until(&f, "all removed", |f| app::bookmarks(&f.app).is_empty());
    assert_eq!(w.get_rows().row_count(), 0);
    assert!(ElementHandle::find_by_accessible_label(&w, "No bookmarks yet.\nOpen a folder and press Ctrl+D to add one.").next().is_some(), "the hint");
    assert_eq!(bw_button(&w, "Open").accessible_enabled(), Some(false));
    bw_key(&w, &[], Key::Escape);
    assert!(app::bookmarks_window(&f.app).is_none(), "Escape closes it");
}

// ---- drives and eject ----

/// A test's stand-in for the system's eject: never ejects anything, records what it was asked.
struct FakeEjector {
    /// The other partitions `drive` reports (besides the asked root).
    partitions: Vec<PathBuf>,
    /// Each eject's roots, sent when it starts.
    started: std::sync::Mutex<std::sync::mpsc::Sender<Vec<PathBuf>>>,
    /// Each eject waits for an answer here.
    answer: std::sync::Mutex<std::sync::mpsc::Receiver<Result<(), String>>>,
}

impl app::Ejector for FakeEjector {
    fn drive(&self, root: &Path) -> Vec<PathBuf> {
        let mut v = vec![root.to_path_buf()];
        v.extend(self.partitions.iter().cloned());
        v
    }

    fn eject(&self, roots: &[PathBuf]) -> Result<(), String> {
        let _ = self.started.lock().unwrap().send(roots.to_vec());
        self.answer.lock().unwrap().recv_timeout(Duration::from_secs(20)).unwrap_or(Err("no answer".into()))
    }
}

/// Puts in a fake ejector; returns where its ejects start and where to answer them.
fn fake_ejector(
    f: &Fixture,
    partitions: Vec<PathBuf>,
) -> (std::sync::mpsc::Receiver<Vec<PathBuf>>, std::sync::mpsc::Sender<Result<(), String>>) {
    let (started_tx, started) = std::sync::mpsc::channel();
    let (answer, answer_rx) = std::sync::mpsc::channel();
    app::set_ejector(
        &f.app,
        std::sync::Arc::new(FakeEjector {
            partitions,
            started: std::sync::Mutex::new(started_tx),
            answer: std::sync::Mutex::new(answer_rx),
        }),
    );
    (started, answer)
}

/// Shows a removable drive "STICK" at `root`, then the real root (so the rest still shows, below
/// the stick: its row stays in view).
fn fake_drive(f: &Fixture, root: &Path) {
    use fileem_core::fs::volumes::{Kind as VKind, Volume};
    let mut stick = Volume::new(root, "STICK", VKind::Removable);
    stick.space = Some((1 << 30, 4 << 30));
    // the root holding the test folder (`/` on Windows contains no folder: nothing to reveal)
    let mut disk = Volume::new(f.dir.ancestors().last().unwrap(), "File System", VKind::System);
    disk.space = Some((5 << 30, 100 << 30));
    app::show_volumes(&f.app, vec![stick, disk]);
}

/// Waits for the next eject to start; fails after 10 s.
fn eject_started(f: &Fixture, started: &std::sync::mpsc::Receiver<Vec<PathBuf>>) -> Vec<PathBuf> {
    let start = Instant::now();
    loop {
        if let Ok(roots) = started.try_recv() {
            return roots;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for the eject");
        pump();
        let _ = f;
    }
}

fn drives_show_their_space_and_only_plugged_in_ones_offer_eject() {
    let f = open_window();
    let stick = f.dir.join("alpha");
    fake_drive(&f, &stick);
    let rows = f.tree();
    let row = |name: &str| rows.iter().find(|r| r.name == name && r.depth == 0 && !r.header).unwrap().clone();
    let (s, root) = (row("STICK"), row("File System"));
    assert!(s.ejectable && !root.ejectable);
    assert_eq!(s.extra, "1.00 GB free of 4.00 GB");
    assert_eq!(root.extra, "5.00 GB free of 100 GB");
    assert_eq!(s.icon, 24, "a plugged-in drive's icon");
    let after = rows.iter().position(|r| r.name == "This computer").unwrap();
    assert_eq!(rows[after + 1].name, "STICK", "in the order given");

    tree_to_top(&f);
    f.right_click(&f.tree_row("STICK"));
    assert!(f.menu_item("Eject").is_some(), "a stick can be ejected");
    assert!(!menu_has(&f, "Cut") && !menu_has(&f, "Copy"), "a drive is a shortcut");
    f.key(Key::Escape);
    pump();
    f.right_click(&f.tree_row("File System"));
    assert!(f.menu_item("Eject").is_none(), "the system drive can't");
    f.key(Key::Escape);
    pump();
    // the drive's row has an eject button
    let button = ElementHandle::find_by_accessible_label(&f.ui, "Eject (safely remove)").collect::<Vec<_>>();
    assert_eq!(button.len(), 1, "one button, on the stick's row");
    assert!((button[0].absolute_position().y - f.tree_row("STICK").absolute_position().y).abs() < 24.0);
}

fn eject_moves_tabs_off_the_drive_first_then_ejects_and_confirms() {
    let f = open_window();
    let drive = f._tmp.path().join("stick2");
    fs::create_dir_all(drive.join("photos")).unwrap();
    fake_drive(&f, &drive);
    f.ui.invoke_address_entered(0, drive.join("photos").display().to_string().into());
    wait_until(&f, "on the stick", |f| f.address() == drive.join("photos"));
    let (started, answer) = fake_ejector(&f, Vec::new());

    // the row's eject button
    tree_to_top(&f);
    ElementHandle::find_by_accessible_label(&f.ui, "Eject (safely remove)").next().unwrap().invoke_accessible_default_action();
    assert!(app::is_ejecting(&f.app, &drive));
    assert_eq!(eject_started(&f, &started), std::slice::from_ref(&drive));
    assert_eq!(f.address(), fileem_core::util::os::home(), "the tab left the drive before ejecting");
    let stick = f.tree().into_iter().find(|r| r.name == "STICK").unwrap();
    assert!(stick.ejecting && stick.extra == "Ejecting…");
    f.right_click(&f.tree_row("STICK"));
    assert!(!menu_row(&f, "Eject").enabled, "no second eject meanwhile");
    f.key(Key::Escape);
    pump();

    answer.send(Ok(())).unwrap();
    wait_until(&f, "the safe to remove notice", |f| dialog_open(f) && f.ui.get_dialog_message().contains("can now be safely removed"));
    assert_eq!(f.ui.get_dialog_message(), "\u{201c}STICK\u{201d} can now be safely removed.");
    assert!(!app::is_ejecting(&f.app, &drive));
    f.key(Key::Return);
    assert!(drive.join("photos").is_dir(), "a fake eject: nothing touched");
}

fn the_tree_stops_watching_a_drive_while_it_is_ejected() {
    let f = open_window();
    let drive = f._tmp.path().join("stick3");
    fs::create_dir_all(drive.join("photos")).unwrap();
    fake_drive(&f, &drive);
    tree_to_top(&f);
    let row = f.tree().iter().position(|r| r.name == "STICK").unwrap();
    f.ui.invoke_tree_toggled(0, row as i32);
    wait_until(&f, "the open drive is watched", |f| app::tree_watched(&f.app, 0).contains(&drive));
    let (started, answer) = fake_ejector(&f, Vec::new());

    ElementHandle::find_by_accessible_label(&f.ui, "Eject (safely remove)").next().unwrap().invoke_accessible_default_action();
    eject_started(&f, &started);
    // a watch can keep a drive busy (Windows holds the folder open)
    assert!(!app::tree_watched(&f.app, 0).iter().any(|p| p.starts_with(&drive)), "not watched while ejecting");
    answer.send(Ok(())).unwrap();
    wait_until(&f, "the safe to remove notice", dialog_open);
    f.key(Key::Return);
}

fn eject_is_refused_while_an_operation_uses_the_drive() {
    let f = open_window();
    let h = blocking_copy(&f);
    let drive = f.dir.join("alpha"); // the copy's target
    fake_drive(&f, &drive);
    let (started, _answer) = fake_ejector(&f, Vec::new());
    app::run_command(&f.app, fileem::menu::Command::Eject(drive.clone(), "STICK".into()));
    wait_until(&f, "the refusal", |f| dialog_open(f) && f.ui.get_dialog_message().contains("in use by a running file operation"));
    assert!(started.try_recv().is_err(), "nothing was ejected");
    assert!(!app::is_ejecting(&f.app, &drive));
    f.key(Key::Return);
    h.cancel();
    wait_until(&f, "the copy stopped", |f| app::ops(&f.app).running().is_empty());
}

fn eject_removes_the_whole_drive_and_moves_tabs_off_every_partition() {
    let f = open_window();
    let photos = f._tmp.path().join("photos");
    let storage = f._tmp.path().join("storage");
    fs::create_dir_all(&photos).unwrap();
    fs::create_dir_all(storage.join("sub")).unwrap();
    fake_drive(&f, &photos);
    // a tab on the other partition of the same drive, in the second side
    f.key(Key::F9);
    wait_until(&f, "the second side", |f| app::active_side(&f.app) == 1 || app::is_dual(&f.app));
    let right = app::tabs(&f.app, 1).0;
    assert_eq!(right.len(), 1);
    app::run_command(&f.app, fileem::menu::Command::OpenInOtherSide(storage.join("sub")));
    wait_until(&f, "the other partition shown", |f| app::tabs(&f.app, 1).0.contains(&Some(storage.join("sub"))));
    let (started, answer) = fake_ejector(&f, vec![storage.clone()]);
    app::run_command(&f.app, fileem::menu::Command::Eject(photos.clone(), "photos".into()));
    assert_eq!(eject_started(&f, &started), [photos.clone(), storage.clone()]);
    let dirs = app::tabs(&f.app, 1).0;
    assert!(dirs.iter().all(|d| !d.as_ref().is_some_and(|d| d.starts_with(&storage))), "the tab left the other partition first: {dirs:?}");
    answer.send(Ok(())).unwrap();
    wait_until(&f, "the notice", dialog_open);
    assert_eq!(f.ui.get_dialog_message(), "The drive with \u{201c}photos\u{201d} and \u{201c}storage\u{201d} can now be safely removed.");
    f.key(Key::Return);
}

fn a_failed_eject_shows_the_reason_and_can_be_retried() {
    let f = open_window();
    let drive = f._tmp.path().join("stick3");
    fs::create_dir_all(&drive).unwrap();
    fake_drive(&f, &drive);
    let (started, answer) = fake_ejector(&f, Vec::new());
    app::run_command(&f.app, fileem::menu::Command::Eject(drive.clone(), "STICK".into()));
    eject_started(&f, &started);
    answer.send(Err("The drive is in use.".into())).unwrap();
    wait_until(&f, "the reason", |f| dialog_open(f) && f.ui.get_dialog_message().contains("The drive is in use."));
    assert_eq!(f.ui.get_dialog_message(), "Cannot eject \u{201c}STICK\u{201d}:\nThe drive is in use.");
    assert!(!app::is_ejecting(&f.app, &drive), "can be tried again");
    f.key(Key::Return);
    app::run_command(&f.app, fileem::menu::Command::Eject(drive.clone(), "STICK".into()));
    eject_started(&f, &started);
    answer.send(Ok(())).unwrap();
    wait_until(&f, "ejected the second time", |f| dialog_open(f) && f.ui.get_dialog_message().contains("safely removed"));
    f.key(Key::Return);
}

// ---- themes and About (stage 8) ----

/// A window's theme: its `Theme` data and whether the std widgets are dark.
macro_rules! theme_of {
    ($w:expr) => {
        (
            $w.global::<fileem::Theme>().get_data(),
            $w.global::<fileem::Palette>().get_color_scheme() == slint::language::ColorScheme::Dark,
        )
    };
}

fn the_theme_menu_switches_family_and_mode_in_every_open_window() {
    use fileem::theme::Family;
    let f = open_window();
    assert_eq!(app::theme(&f.app), (Family::Classic, false), "Classic light without a saved theme");
    assert_eq!(theme_of!(f.ui), (app::theme_data(Family::Classic, false), false));
    // every kind of window open: preview, search, bookmarks, properties, progress
    f.click("notes.txt");
    f.key(Key::F3);
    let preview = app::preview_window(&f.app).expect("the preview floats");
    let search = app::open_search(&f.app, f.dir.clone()).unwrap();
    app::run_command(&f.app, fileem::menu::Command::ManageBookmarks);
    let manage = app::bookmarks_window(&f.app).unwrap();
    app::run_command(&f.app, fileem::menu::Command::Properties(vec![f.dir.clone()]));
    wait_until(&f, "the properties", |f| !app::properties_windows(&f.app).is_empty());
    let props = app::properties_windows(&f.app).remove(0);
    let copy = blocking_copy(&f);
    wait_until(&f, "the progress window", |f| progress(f).is_some());
    let ops = progress(&f).unwrap();
    let all = |f: &Fixture| -> Vec<(fileem::ThemeData, bool)> {
        vec![theme_of!(f.ui), theme_of!(preview), theme_of!(search), theme_of!(manage), theme_of!(props), theme_of!(ops)]
    };

    f.click_at(&f.tool("Theme"));
    assert!(f.menu_item("Adwaita (GNOME)").is_some(), "the theme menu is open");
    assert!(menu_row(&f, "Classic").checked && menu_row(&f, "Light").checked);
    assert!(!menu_row(&f, "Aurora").checked && !menu_row(&f, "Dark").checked);
    f.pick("Aurora");
    assert_eq!(app::theme(&f.app), (Family::Aurora, false));
    let aurora_light = (app::theme_data(Family::Aurora, false), false);
    assert!(all(&f).iter().all(|t| *t == aurora_light), "every window switched");

    f.click_at(&f.tool("Theme"));
    assert!(menu_row(&f, "Aurora").checked && menu_row(&f, "Light").checked, "the marks follow");
    f.pick("Dark");
    assert_eq!(app::theme(&f.app), (Family::Aurora, true), "the family stays");
    let aurora_dark = (app::theme_data(Family::Aurora, true), true);
    assert!(all(&f).iter().all(|t| *t == aurora_dark), "every window is dark, std widgets too");
    assert_ne!(aurora_light.0, aurora_dark.0);

    // windows opened later start in the theme
    let later = app::open_search(&f.app, f.dir.join("alpha")).unwrap();
    assert_eq!(theme_of!(later), aurora_dark);
    copy.cancel();
    wait_until(&f, "the copy stopped", |f| progress(f).is_none());
}

fn the_light_dark_button_toggles_and_the_theme_survives_a_restart() {
    use fileem::theme::Family;
    let f = open_window_with(
        |s, _| {
            s.put("theme.name", "macos");
            s.put("theme.dark", true);
        },
        true,
    );
    assert_eq!(app::theme(&f.app), (Family::MacOs, true), "the saved theme");
    assert_eq!(theme_of!(f.ui), (app::theme_data(Family::MacOs, true), true));
    f.click_at(&f.tool("Light / dark mode"));
    assert_eq!(app::theme(&f.app), (Family::MacOs, false));
    assert_eq!(theme_of!(f.ui), (app::theme_data(Family::MacOs, false), false));
    app::collect_session(&f.app);
    app::session(&f.app).save();
    let saved = Session::load(app::session(&f.app).file());
    assert_eq!(saved.get("theme.name").as_deref(), Some("macos"), "the Java app's ids");
    assert_eq!(saved.get("theme.dark").as_deref(), Some("false"));
    f.click_at(&f.tool("Light / dark mode"));
    assert_eq!(app::theme(&f.app), (Family::MacOs, true), "and back");

    // an unknown theme (a newer version's, a typo) is Classic
    let g = open_window_with(|s, _| s.put("theme.name", "nope"), true);
    assert_eq!(app::theme(&g.app), (Family::Classic, false));
}

fn windows_carry_the_app_icon() {
    // the icon every window binds (tests/packaging.rs checks they all do), and that it is the
    // 256 px one, not a missing image
    let f = open_window();
    let icon = f.ui.global::<fileem::Theme>().get_app_icon();
    assert_eq!((icon.size().width, icon.size().height), (256, 256));
    app::run_command(&f.app, fileem::menu::Command::Properties(vec![f.dir.clone()]));
    wait_until(&f, "the properties window", |f| app::properties_windows(&f.app).len() == 1);
    let p = app::properties_windows(&f.app).remove(0);
    assert_eq!(p.global::<fileem::Theme>().get_app_icon().size().width, 256, "floating windows too");
    p.invoke_close_clicked();
}

fn the_about_button_shows_version_build_date_and_system() {
    let f = open_window();
    let version = fileem::about::VERSION;
    f.click_at(&f.tool(&format!("About fileEm {version}")));
    assert!(dialog_open(&f));
    assert_eq!(f.ui.get_dialog_title(), format!("fileEm {version}"));
    let text = f.ui.get_dialog_message().to_string();
    assert!(text.starts_with(&format!("Version {version}\nBuilt 20")), "{text}");
    assert!(text.contains("Rust 1.") && text.contains("Slint 1."), "{text}");
    assert!(text.ends_with(&format!("({})", std::env::consts::ARCH)), "{text}");
    assert!(!f.ui.get_dialog_can_cancel(), "only OK");
    f.key(Key::Escape);
    assert!(!dialog_open(&f), "Escape is OK");

    // at the window's smallest width every toolbar button still fits, none on top of another
    let min = f.ui.get_toolbar_min_width();
    assert!(min > 600.0, "the toolbar sets the smallest width: {min}");
    f.ui.window().set_size(slint::LogicalSize::new(min, 500.0));
    pump();
    let right_edge = |label: &str| {
        let e = f.tool(label);
        e.absolute_position().x + e.size().width
    };
    assert!(right_edge(&format!("About fileEm {version}")) <= min);
    assert!(right_edge("Bookmark this folder (Ctrl+D)") <= f.tool("View (Ctrl+Shift+1…8, Ctrl+wheel)").absolute_position().x);
}
