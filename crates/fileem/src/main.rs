// No console window next to the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! fileEm: an Explorer-style file manager (Slint UI over `fileem-core`).

use fileem::{app, AppWindow};
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    let start = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(fileem_core::util::os::home);
    let ui = AppWindow::new()?;
    let _app = app::install(&ui, start);
    if let Some(out) = std::env::var_os("FILEEM_SNAPSHOT") {
        snapshot_later(&ui, out.into());
    }
    ui.run()
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
