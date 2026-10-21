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
    #[cfg(windows)]
    {
        windows::shell_execute(p)
    }
    #[cfg(not(windows))]
    {
        spawn(Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(p))
    }
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
        return if installed("wt.exe") {
            cmd(&["wt.exe", "-d", &windows_terminal_escape(&d)])
        } else {
            cmd(&["cmd.exe", "/c", "start", "cmd.exe"])
        };
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

/// Windows Terminal splits its command line into commands at every `;`, also inside quotes: a
/// folder named `a;calc` would start `calc`. `\;` is its escape for a literal one.
pub fn windows_terminal_escape(arg: &str) -> String {
    arg.replace(';', "\\;")
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

/// Opening files on Windows: through the shell's own call, never `cmd /C start`, whose command
/// line runs `&`, `|` and friends in file names as commands (`x&calc` would start calc).
#[cfg(windows)]
mod windows {
    use std::ffi::{c_void, OsStr};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(hwnd: *mut c_void, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
        fn CoUninitialize();
    }

    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const COINIT_DISABLE_OLE1DDE: u32 = 0x4;
    const SW_SHOWNORMAL: i32 = 1;

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain([0]).collect()
    }

    /// `ShellExecuteW(open)` on a thread of its own (shell extensions want COM there, and a slow
    /// handler doesn't hold up the window for long): its error if it fails within 2 s.
    pub fn shell_execute(p: &Path) -> io::Result<()> {
        let file = wide(p.as_os_str());
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new().name("fileem-open".into()).spawn(move || {
            let op = wide(OsStr::new("open"));
            // SAFETY: NUL-terminated wide strings that outlive the calls; COM is set up and torn
            // down on this thread only
            let r = unsafe {
                let com = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
                let r = ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
                if com >= 0 {
                    CoUninitialize();
                }
                r
            };
            let _ = tx.send(r);
        })?;
        match rx.recv_timeout(Duration::from_secs(2)) {
            // values above 32 mean success; below are the old error codes
            Ok(r) if r > 32 => Ok(()),
            Ok(2 | 3) => Err(io::Error::new(io::ErrorKind::NotFound, "Not found")),
            Ok(5) => Err(io::Error::new(io::ErrorKind::PermissionDenied, "Access denied")),
            Ok(31) => Err(io::Error::other("No program is associated with this kind of file")),
            Ok(r) => Err(io::Error::other(format!("Windows could not open it (error {r})"))),
            Err(_) => Ok(()), // still starting
        }
    }
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

    #[test]
    fn semicolons_in_a_folder_for_windows_terminal_are_escaped() {
        assert_eq!(windows_terminal_escape(r"C:\a;calc"), r"C:\a\;calc");
        assert_eq!(windows_terminal_escape(r"C:\Users\me"), r"C:\Users\me");
    }

    #[test]
    fn files_are_never_opened_through_cmd() {
        // `cmd /C start` runs `&` in file names as a command separator
        let src = include_str!("shell.rs");
        let open = &src[src.find("pub fn open(").unwrap()..src.find("/// \"Show in").unwrap()];
        assert!(!open.contains("\"cmd"), "{open}");
        assert!(src.contains("ShellExecuteW(std::ptr::null_mut()"));
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
