//! Opening files with the program the OS associates with them.
//!
//! With `FILEEM_NO_LAUNCH` set (tests), nothing is started: requests are only recorded in
//! [`launched`], so tests never open the user's editor or viewer.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

static LAUNCHED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn no_launch() -> bool {
    std::env::var_os("FILEEM_NO_LAUNCH").is_some()
}

/// Paths that `open` was asked to open while launching is off.
pub fn launched() -> Vec<PathBuf> {
    LAUNCHED.lock().unwrap().clone()
}

/// Opens `p` with its default program, without waiting for it.
pub fn open(p: &Path) -> io::Result<()> {
    if no_launch() {
        LAUNCHED.lock().unwrap().push(p.to_path_buf());
        return Ok(());
    }
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        // the empty string is `start`'s window title, so a quoted path isn't taken for one
        c.args(["/C", "start", ""]).arg(p);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(p);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(p);
        c
    };
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    // reap the launcher when it exits, so it doesn't linger as a zombie
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
