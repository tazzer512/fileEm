// No console window next to the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! fileEm: an Explorer-style file manager (Slint UI over `fileem-core`).

use std::rc::Rc;

use fileem::{app, AppWindow};
use fileem_core::session::Session;
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    // before any thread starts: they all inherit the blocked termination signals
    let signals = fileem::signals::block();
    let start = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let session = Rc::new(Session::load_default());
    let ui = AppWindow::new()?;
    // before the first window shows (the backend exists once a window was created)
    if let Err(e) = slint::set_xdg_app_id(fileem::about::APP_ID) {
        eprintln!("fileem: app id: {e}");
    }
    let w = session.get_f64("window.w", 1280.0).clamp(480.0, 10_000.0);
    let h = session.get_f64("window.h", 780.0).clamp(320.0, 10_000.0);
    ui.window().set_size(slint::LogicalSize::new(w as f32, h as f32));
    ui.window().set_maximized(session.get_bool("window.maximized", false));
    let app = app::install(&ui, session.clone(), start);

    let snapshot = std::env::var_os("FILEEM_SNAPSHOT");
    // a snapshot is a look at the window, not a session: it never writes the settings
    let _autosave = if let Some(out) = snapshot {
        snapshot_later(&ui, out.into());
        None
    } else {
        Some(autosave(&ui, &app))
    };
    // logout, kill, Ctrl+C: save and stop operations first
    let weak = ui.as_weak();
    let waiting = signals.and_then(|s| {
        s.wait_in_background(std::time::Duration::from_secs(5), move || {
            let _ = weak.upgrade_in_event_loop(|ui| {
                ui.invoke_terminate();
                let _ = slint::quit_event_loop();
            });
        })
    });
    if let Err(e) = waiting {
        eprintln!("fileem: termination signals: {e}");
    }
    ui.run()
}

/// Keeps the session file up to date while the app runs, so settings survive a crash or a kill,
/// not only a normal close: every 2 s the state is collected and, if it changed, written in the
/// background. Closing the window saves right away.
fn autosave(ui: &AppWindow, app: &app::Shared) -> slint::Timer {
    let timer = slint::Timer::default();
    let a = app.clone();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(2), move || {
        app::collect_session(&a);
        app::session(&a).save_in_background();
    });
    let a = app.clone();
    ui.window().on_close_requested(move || {
        app::collect_session(&a);
        app::session(&a).save();
        // with operations running it asks first, and quits once they have cleaned up
        if app::close_requested(&a) {
            app::close_windows(&a);
            slint::CloseRequestResponse::HideWindow
        } else {
            slint::CloseRequestResponse::KeepWindowShown
        }
    });
    timer
}

/// Visual checks (Wayland has no desktop screenshots): `FILEEM_SNAPSHOT=out.ppm fileem DIR` renders
/// the window once the folder has loaded, writes it as a PPM image and quits.
fn snapshot_later(ui: &AppWindow, out: std::path::PathBuf) {
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(1500), move || {
        let Some(ui) = weak.upgrade() else { return };
        match ui.window().take_snapshot() {
            Ok(buf) => {
                let mut ppm = format!("P6\n{} {}\n255\n", buf.width(), buf.height()).into_bytes();
                for px in buf.as_slice() {
                    ppm.extend([px.r, px.g, px.b]);
                }
                if let Err(e) = std::fs::write(&out, ppm) {
                    eprintln!("snapshot: {e}");
                }
            }
            Err(e) => eprintln!("snapshot: {e}"),
        }
        let _ = slint::quit_event_loop();
    });
}
