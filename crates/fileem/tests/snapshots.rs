//! Visual checks without a desktop (`cargo test -p fileem --test snapshots -- --ignored`): renders
//! the main window (also with both sides, and in the grid views), a dialog and a right-click menu open, the progress window with a conflict
//! question and a Properties window, in the
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
}
