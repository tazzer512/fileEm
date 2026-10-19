//! End-to-end UI tests on Slint's headless testing backend: the real window, real keys and
//! clicks, real background listings. The backend can only be initialized once per process, so
//! this file has a single `#[test]` that runs each scenario with a fresh window and folder.
//! Nothing is ever launched (`FILEEM_NO_LAUNCH`): opened files are only recorded.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use std::rc::Rc;

use fileem::{app, shell, AppWindow, FileRow, TreeRow};
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
    fn rows(&self) -> Vec<FileRow> {
        self.ui.get_files().iter().collect()
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
        self.ui.get_tree().iter().collect()
    }

    fn address(&self) -> PathBuf {
        PathBuf::from(self.ui.get_address().as_str())
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

    fn click_at(&self, e: &ElementHandle) {
        let (pos, size) = (e.absolute_position(), e.size());
        let position = LogicalPosition::new(pos.x + size.width / 2.0, pos.y + size.height / 2.0);
        let w = self.ui.window();
        w.dispatch_event(WindowEvent::PointerMoved { position });
        w.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
        w.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
    }

    fn click(&self, label: &str) {
        self.click_at(&self.element(label));
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
}

fn lists_folders_first_with_status_and_no_hidden_files() {
    let f = open_window();
    assert_eq!(f.names(), ["alpha", "beta", "a.png", "big.bin", "notes.txt"]);
    assert_eq!(f.ui.get_status(), "5 items");
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
    assert_eq!(f.ui.get_message(), "This folder is empty.");
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
    assert_eq!(f.ui.get_status(), "5 items, 2 selected (4.89 KB)");
    f.key(Key::Home);
    assert_eq!(f.selected(), ["alpha"]);
    f.chord(&[Key::Control], "a");
    assert_eq!(f.selected().len(), 5);

    f.key(Key::End);
    f.key(Key::Return);
    settle(&f);
    assert!(shell::launched().contains(&f.dir.join("notes.txt")), "{:?}", shell::launched());
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
    assert!(!f.ui.get_sort_ascending());
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
    f.ui.invoke_navigate_address(f.dir.join("alpha").display().to_string().into());
    wait_until(&f, "alpha", |f| f.names() == ["inside.txt"]);
    let missing = f.dir.join("missing");
    f.ui.invoke_navigate_address(missing.display().to_string().into());
    wait_until(&f, "the error", |f| f.ui.get_status().contains("Not found"));
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
    f.ui.invoke_tree_toggled(row as i32);
    wait_until(&f, "subfolders", |f| f.tree().iter().any(|r| r.name == "alpha"));
    let alpha = f.tree().iter().position(|r| r.name == "alpha").unwrap();
    f.ui.invoke_tree_clicked(alpha as i32);
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
    assert_eq!(f.ui.get_sort_column(), 1);
    assert!(!f.ui.get_sort_ascending());
    assert_eq!(f.names()[2], "big.bin", "largest file first");
    assert_eq!(f.ui.get_tree_width(), 300.0);
}

fn a_vanished_last_folder_opens_its_nearest_parent() {
    let f = open_window_with(|s, dir| s.put("left.tab.0", dir.join("alpha/was-here/deeper").display()), false);
    wait_until(&f, "the nearest existing parent", |f| f.address() == f.dir.join("alpha"));
    assert_eq!(f.names(), ["inside.txt"]);
}

fn saving_the_session_keeps_what_only_the_java_app_uses() {
    let f = open_window_with(
        |s, _| {
            s.put("left.tabs", 3);
            s.put("left.selected", 1);
            s.put("left.tab.0", "/java/tab0");
            s.put("left.tab.1", "/java/tab1");
            s.put("left.tab.2", "/java/tab2");
            s.put("right.tab.0", "/java/right");
            s.put("columns.size.width", 120);
            s.put("theme.name", "aurora");
        },
        true,
    );
    f.double_click("alpha");
    wait_until(&f, "alpha", |f| f.address() == f.dir.join("alpha"));
    f.chord(&[Key::Control], "h");
    app::collect_session(&f.app);
    app::session(&f.app).save();

    let saved = Session::load(app::session(&f.app).file());
    assert_eq!(saved.get("left.tab.1").map(PathBuf::from), Some(f.dir.join("alpha")), "the shown tab's folder");
    assert!(saved.get_bool("showHidden", false));
    for (k, v) in [
        ("left.tabs", "3"),
        ("left.tab.0", "/java/tab0"),
        ("left.tab.2", "/java/tab2"),
        ("right.tab.0", "/java/right"),
        ("columns.size.width", "120"),
        ("theme.name", "aurora"),
    ] {
        assert_eq!(saved.get(k).as_deref(), Some(v), "{k} left alone");
    }
    assert!(saved.get_f64("window.w", 0.0) > 0.0);
}
