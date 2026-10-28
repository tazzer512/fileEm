//! Visual checks without a desktop (`cargo test -p fileem --test snapshots -- --ignored`): renders
//! the main window (also with both sides, and in the grid views), the preview (floating and docked), a search window, bookmarks (tree + Manage window), drives, a dialog and a right-click menu open, the progress window with a conflict
//! question, a Properties window, every theme in both modes and the About box, in the
//! headless backend's software renderer, to `target/snapshots/*.ppm`. Look at them after UI
//! changes; they are not compared automatically.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use fileem::view_mode::ViewMode;
use fileem::{app, AppWindow};
use fileem_core::ops::Kind;
use fileem_core::session::Session;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model};

fn pump_until(what: &str, cond: impl Fn() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting until {what}");
        slint::Timer::single_shot(Duration::from_millis(5), || {
            let _ = slint::quit_event_loop();
        });
        slint::run_event_loop_until_quit().unwrap();
    }
}

fn save(window: &slint::Window, name: &str) {
    let buf = window.take_snapshot().expect("snapshot");
    let mut ppm = format!("P6\n{} {}\n255\n", buf.width(), buf.height()).into_bytes();
    for px in buf.as_slice() {
        ppm.extend([px.r, px.g, px.b]);
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    fs::create_dir_all(&dir).unwrap();
    let out: PathBuf = dir.join(format!("{name}.ppm"));
    fs::write(&out, ppm).unwrap();
    eprintln!("wrote {}", out.display());
}

#[test]
#[ignore = "writes images for a visual check; run with --ignored"]
fn snapshots() {
    std::env::set_var("FILEEM_NO_LAUNCH", "1");
    // the testing backend with a real rasterizer, so take_snapshot works headless
    slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
        i_slint_backend_testing::TestingBackendOptions {
            mock_time: false,
            threading: true,
            renderer_name: Some("software".into()),
        },
    )))
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("files");
    fs::create_dir_all(dir.join("alpha")).unwrap();
    for (n, data) in [("notes.txt", "1234567890"), ("photo.png", "png"), ("archive.zip", "zip")] {
        fs::write(dir.join(n), data).unwrap();
    }
    fs::write(dir.join("alpha/notes.txt"), "old").unwrap();

    let ui = AppWindow::new().unwrap();
    ui.window().set_size(slint::LogicalSize::new(900.0, 560.0));
    let a = app::install(&ui, Rc::new(Session::load(&tmp.path().join("s.properties"))), Some(dir.clone()));
    ui.show().unwrap();
    pump_until("listed", || ui.get_left().files.row_count() == 4);
    save(ui.window(), "main");

    // the second side, with a second tab and a filter on the left
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::F9.into() });
    ui.invoke_tab_new(0);
    pump_until("second tab", || ui.get_left().tabs.row_count() == 2 && ui.get_left().files.row_count() == 4);
    ui.invoke_filter_edited(0, "*.txt".into());
    pump_until("right side listed", || ui.get_right().files.row_count() > 0);
    save(ui.window(), "dual");
    ui.invoke_filter_edited(0, "".into());
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::F9.into() });

    ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::F7.into() });
    pump_until("dialog", || ui.get_dialog_open());
    save(ui.window(), "dialog-new-folder");
    ui.invoke_dialog_cancelled();

    app::run_operation(&a, Kind::Copy, vec![dir.join("notes.txt")], Some(dir.join("alpha")));
    pump_until("question", || app::ops(&a).rows().iter().any(|r| r.asking));
    let w = app::ops(&a).window().unwrap();
    w.window().set_size(slint::LogicalSize::new(520.0, 360.0));
    pump_until("laid out", || true);
    save(w.window(), "progress-conflict");
    app::ops(&a).running().iter().for_each(|h| h.cancel());

    // the question after dropping a file onto a folder
    ui.invoke_row_pressed(0, 2, false, false);
    ui.invoke_drag_started(0, 0, 2, 0.0, 0.0, 0.0, 0.0);
    ui.invoke_dropped(0, 0, 0);
    pump_until("drop question", || ui.get_dialog_open());
    save(ui.window(), "dialog-drop");
    ui.invoke_dialog_cancelled();
    // a question that wraps over several lines
    let long = dir.join("a rather long file name that has to wrap over more than one line of the dialog.txt");
    app::run_command(&a, fileem::menu::Command::Delete(vec![long]));
    pump_until("delete question", || ui.get_dialog_open());
    save(ui.window(), "dialog-delete");
    ui.invoke_dialog_cancelled();
    let unbroken = dir.join("IMG_20240101_123456_a_very_long_file_name_without_any_spaces_in_it_at_all.jpg");
    app::run_command(&a, fileem::menu::Command::Trash(vec![unbroken.clone()]));
    pump_until("trash question", || ui.get_dialog_open());
    save(ui.window(), "dialog-trash");
    ui.invoke_dialog_cancelled();
    ui.window().set_size(slint::LogicalSize::new(360.0, 400.0));
    app::run_command(&a, fileem::menu::Command::Delete(vec![unbroken]));
    pump_until("delete question", || ui.get_dialog_open());
    save(ui.window(), "dialog-delete-narrow");
    ui.invoke_dialog_cancelled();
    ui.window().set_size(slint::LogicalSize::new(900.0, 560.0));

    // the grid views, with a thumbnail
    image::RgbImage::from_fn(120, 80, |x, y| image::Rgb([(x * 2) as u8, (y * 3) as u8, 160])).save(dir.join("photo.png")).unwrap();
    ui.invoke_refresh();
    pump_until("relisted", || ui.get_left().files.iter().any(|r| r.name == "photo.png" && r.size != "3 bytes"));
    for (mode, name) in [(ViewMode::LargeIcons, "view-large-icons"), (ViewMode::Tiles, "view-tiles"), (ViewMode::Content, "view-content"), (ViewMode::List, "view-list")] {
        app::run_command(&a, fileem::menu::Command::SetView(mode));
        let wait = mode == ViewMode::LargeIcons;
        pump_until("thumbnail", || !wait || ui.get_left().files.iter().any(|r| r.thumb.size().width > 0));
        save(ui.window(), name);
    }
    app::run_command(&a, fileem::menu::Command::SetView(ViewMode::Details));

    // the preview: an image in its own window, then text docked
    ui.invoke_row_pressed(0, 3, false, false);
    ui.invoke_row_released(0, 3);
    ui.invoke_toggle_preview();
    let w = app::preview_window(&a).unwrap();
    w.window().set_size(slint::LogicalSize::new(560.0, 480.0));
    pump_until("the image", || w.get_view().mode == 1);
    save(w.window(), "preview-image");
    ui.invoke_preview_dock();
    ui.invoke_row_pressed(0, 2, false, false);
    ui.invoke_row_released(0, 2);
    pump_until("the text", || ui.get_preview().mode == 2);
    save(ui.window(), "preview-docked");
    ui.invoke_toggle_preview();
    ui.invoke_preview_dock();

    // a search window with results, and a rename asked over it
    let w = app::open_search(&a, dir.clone()).unwrap();
    w.window().set_size(slint::LogicalSize::new(820.0, 380.0));
    w.set_pattern("*.txt".into());
    w.invoke_search("*.txt".into());
    pump_until("results", || !w.get_running() && w.get_rows().row_count() == 2);
    save(w.window(), "search");
    w.invoke_row_pressed(0, false, false);
    w.invoke_key(slint::SharedString::from(Key::F2), false, false, false);
    pump_until("laid out", || w.get_dialog_open());
    save(w.window(), "search-rename");
    w.invoke_dialog_cancelled();

    // bookmarks in the tree (one of them on a missing folder) and the Manage window
    app::run_command(&a, fileem::menu::Command::AddBookmark(dir.clone()));
    app::run_command(&a, fileem::menu::Command::AddBookmark(dir.join("alpha")));
    app::run_command(&a, fileem::menu::Command::AddBookmark(dir.join("gone")));
    pump_until("laid out", || ui.get_bookmarked());
    let position = slint::LogicalPosition::new(50.0, 300.0);
    ui.window().dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerScrolled { position, delta_x: 0.0, delta_y: 100_000.0 });
    pump_until("laid out", || true);
    save(ui.window(), "bookmarks-tree");
    let b = app::bookmarks_window(&a).unwrap_or_else(|| {
        app::run_command(&a, fileem::menu::Command::ManageBookmarks);
        app::bookmarks_window(&a).unwrap()
    });
    b.window().set_size(slint::LogicalSize::new(640.0, 380.0));
    pump_until("checked", || b.get_rows().iter().any(|r| r.missing));
    save(b.window(), "bookmarks-manage");
    b.invoke_close_clicked();

    // drives: a stick (eject button, free space), a network share and the root
    {
        use fileem_core::fs::volumes::{Kind as VKind, Volume};
        let mut stick = Volume::new(dir.join("alpha"), "STICK", VKind::Removable);
        stick.space = Some((3 << 30, 16 << 30));
        let mut share = Volume::new(tmp.path().join("share"), "share", VKind::Network);
        share.space = Some((700 << 30, 2000 << 30));
        let mut root = Volume::new("/", "File System", VKind::System);
        root.space = Some((50 << 30, 500 << 30));
        app::show_volumes(&a, vec![stick, share, root]);
    }
    pump_until("laid out", || true);
    let position = slint::LogicalPosition::new(50.0, 300.0);
    ui.window().dispatch_event(WindowEvent::PointerScrolled { position, delta_x: 0.0, delta_y: 100_000.0 });
    pump_until("laid out", || true);
    save(ui.window(), "drives-tree");

    // a right-click menu on a file, and its Properties window
    ui.invoke_row_pressed(0, 1, false, false);
    ui.invoke_row_menu(0, 1);
    ui.invoke_show_menu(520.0, 120.0);
    pump_until("laid out", || true);
    save(ui.window(), "menu-file");
    app::run_command(&a, fileem::menu::Command::Properties(vec![dir.clone()]));
    pump_until("properties", || app::properties_windows(&a).first().is_some_and(|w| w.get_lines().iter().any(|l| l.value.contains("files,"))));
    let p = app::properties_windows(&a).remove(0);
    p.window().set_size(slint::LogicalSize::new(480.0, 420.0));
    pump_until("laid out", || true);
    save(p.window(), "properties-folder");

    // every theme in both modes, with the second side and a selection; a dark Properties window
    // and the About box
    p.invoke_close_clicked();
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Escape.into() });
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::F9.into() });
    ui.invoke_row_pressed(0, 2, false, false);
    for family in fileem::theme::Family::ALL {
        for dark in [false, true] {
            app::set_theme(&a, family, dark);
            pump_until("laid out", || true);
            save(ui.window(), &format!("theme-{}-{}", family.id(), if dark { "dark" } else { "light" }));
        }
    }
    app::run_command(&a, fileem::menu::Command::Properties(vec![dir.clone()]));
    pump_until("properties", || !app::properties_windows(&a).is_empty());
    let p = app::properties_windows(&a).remove(0);
    p.window().set_size(slint::LogicalSize::new(480.0, 420.0));
    pump_until("laid out", || p.get_lines().iter().any(|l| l.value.contains("files,")));
    save(p.window(), "theme-properties-dark");
    ui.invoke_about();
    pump_until("about", || ui.get_dialog_open());
    save(ui.window(), "about");
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Escape.into() });

    render_times(&ui, &a, tmp.path());
}

/// How long the software renderer takes to draw the whole main window at common screen sizes
/// (the worst case: a full repaint; scrolling and hovering repaint less). Printed, not checked:
/// the numbers behind the renderer choice in MIGRATION.md.
fn render_times(ui: &AppWindow, a: &app::Shared, tmp: &Path) {
    let many = tmp.join("many");
    fs::create_dir_all(&many).unwrap();
    for i in 0..300 {
        fs::write(many.join(format!("file {i:03}.txt")), "x").unwrap();
    }
    let path = many.to_string_lossy().to_string();
    ui.invoke_address_entered(0, path.clone().into());
    ui.invoke_address_entered(1, path.into());
    pump_until("listed", || ui.get_left().files.row_count() == 300 && ui.get_right().files.row_count() == 300);
    for family in [fileem::theme::Family::Classic, fileem::theme::Family::Aurora] {
        app::set_theme(a, family, false);
        for (w, h) in [(1280.0, 780.0), (1920.0, 1080.0), (2560.0, 1440.0), (3840.0, 2160.0)] {
            ui.window().set_size(slint::LogicalSize::new(w, h));
            pump_until("laid out", || true);
            let _ = ui.window().take_snapshot();
            let mut times: Vec<Duration> = (0..5)
                .map(|_| {
                    let start = Instant::now();
                    let _ = ui.window().take_snapshot().unwrap();
                    start.elapsed()
                })
                .collect();
            times.sort();
            eprintln!("render {:?} {w}x{h}: {:.1} ms", family, times[2].as_secs_f64() * 1000.0);
        }
    }
}
