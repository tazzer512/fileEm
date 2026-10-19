//! Hands files to the operating system: open with the associated program, open a terminal, show
//! an item in the OS file manager (the Java `Shell`).
//!
//! With `FILEEM_NO_LAUNCH` set (tests), nothing is started: requests are only recorded in
//! [`launched`] ("open /path", "terminal /path", "reveal /path"), so tests never open the user's
//! editor, viewer or terminal.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

static LAUNCHED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn suppressed(what: &str, p: &Path) -> bool {
    let off = std::env::var_os("FILEEM_NO_LAUNCH").is_some();
    if off {
        LAUNCHED.lock().unwrap().push(format!("{what} {}", p.display()));
    }
    off
}

/// What was requested while launching is off.
pub fn launched() -> Vec<String> {
    LAUNCHED.lock().unwrap().clone()
}

/// Opens `p` with its default program, without waiting for it.
pub fn open(p: &Path) -> io::Result<()> {
    if suppressed("open", p) {
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
    spawn(&mut cmd)
}

/// "Show in …": the OS file manager's name.
pub fn file_manager_name() -> &'static str {
    if cfg!(windows) {
        "Explorer"
    } else if cfg!(target_os = "macos") {
        "Finder"
    } else {
        "file manager"
    }
}

/// Opens a terminal window in `dir`.
pub fn open_terminal(dir: &Path) -> io::Result<()> {
    if suppressed("terminal", dir) {
        return Ok(());
    }
    let terminal = std::env::var("TERMINAL").ok();
    let cmd = terminal_command(dir, terminal.as_deref(), fileem_core::util::os::on_path).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "No terminal application was found. Set the TERMINAL environment variable.")
    })?;
    // also the working directory, for terminals without a flag for it
    spawn(Command::new(&cmd[0]).args(&cmd[1..]).current_dir(dir))
}

/// The command that opens a terminal in `dir`, or `None` when none is installed.
pub fn terminal_command(dir: &Path, env_terminal: Option<&str>, installed: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    let d = dir.display().to_string();
    let cmd = |parts: &[&str]| Some(parts.iter().map(|s| s.to_string()).collect());
    if cfg!(windows) {
        return if installed("wt.exe") { cmd(&["wt.exe", "-d", &d]) } else { cmd(&["cmd.exe", "/c", "start", "cmd.exe"]) };
    }
    if cfg!(target_os = "macos") {
        return cmd(&["open", "-a", "Terminal", &d]);
    }
    if let Some(t) = env_terminal.map(str::trim).filter(|t| !t.is_empty() && installed(t)) {
        return cmd(&[t]);
    }
    let known: [(&str, Vec<String>); 10] = [
        ("x-terminal-emulator", vec![]),
        ("gnome-terminal", vec![format!("--working-directory={d}")]),
        ("konsole", vec!["--workdir".into(), d.clone()]),
        ("xfce4-terminal", vec![format!("--working-directory={d}")]),
        ("tilix", vec!["-w".into(), d.clone()]),
        ("kitty", vec!["--directory".into(), d.clone()]),
        ("alacritty", vec!["--working-directory".into(), d.clone()]),
        ("wezterm", vec!["start".into(), "--cwd".into(), d.clone()]),
        ("foot", vec![format!("--working-directory={d}")]),
        ("xterm", vec![]),
    ];
    known.into_iter().find(|(name, _)| installed(name)).map(|(name, args)| {
        let mut c = vec![name.to_string()];
        c.extend(args);
        c
    })
}

/// Shows `p` selected in the OS file manager (falls back to opening its folder). Blocks briefly on
/// Linux (a D-Bus call): call it off the UI thread.
pub fn reveal(p: &Path) -> io::Result<()> {
    if suppressed("reveal", p) {
        return Ok(());
    }
    if cfg!(windows) {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(p);
        return spawn(Command::new("explorer.exe").arg(arg));
    }
    if cfg!(target_os = "macos") {
        return spawn(Command::new("open").arg("-R").arg(p));
    }
    let shown = Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.freedesktop.FileManager1"])
        .args(["--object-path", "/org/freedesktop/FileManager1"])
        .args(["--method", "org.freedesktop.FileManager1.ShowItems"])
        .arg(show_items_argument(p))
        .arg("")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if shown {
        return Ok(());
    }
    spawn(Command::new("xdg-open").arg(p.parent().unwrap_or(p)))
}

/// The GVariant array literal for ShowItems. URIs keep `'` unescaped, which would end the quoted
/// string early (the call fails or names other items), so it is percent-encoded too.
pub fn show_items_argument(p: &Path) -> String {
    format!("['{}']", crate::clipboard::to_uri(p).replace('\'', "%27"))
}

fn spawn(cmd: &mut Command) -> io::Result<()> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    // reap the launcher when it exits, so it doesn't linger as a zombie
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = "/work/dir";

    #[cfg(unix)]
    #[test]
    fn show_in_file_manager_quotes_apostrophes_in_the_item_uri() {
        assert_eq!(show_items_argument(Path::new("/home/u/it's here.txt")), "['file:///home/u/it%27s%20here.txt']");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_prefers_the_terminal_environment_variable() {
        let installed = |p: &str| ["foot", "xterm"].contains(&p);
        assert_eq!(terminal_command(Path::new(DIR), Some("foot"), installed), Some(vec!["foot".to_string()]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_passes_the_folder_to_known_terminals() {
        let dir = Path::new(DIR);
        assert_eq!(
            terminal_command(dir, None, |p| ["konsole", "xterm"].contains(&p)).unwrap(),
            ["konsole", "--workdir", DIR]
        );
        assert_eq!(
            terminal_command(dir, Some("not-installed"), |p| p == "gnome-terminal").unwrap(),
            ["gnome-terminal", "--working-directory=/work/dir"]
        );
        assert_eq!(terminal_command(dir, None, |_| false), None);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn mac_and_windows_always_have_a_terminal() {
        let first = terminal_command(Path::new(DIR), None, |_| false).unwrap().remove(0);
        assert_eq!(first, if cfg!(target_os = "macos") { "open" } else { "cmd.exe" });
    }
}
