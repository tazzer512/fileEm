//! Platform checks and well-known folders.

use std::env;
use std::path::PathBuf;

pub fn is_windows() -> bool {
    cfg!(windows)
}

pub fn is_mac() -> bool {
    cfg!(target_os = "macos")
}

pub fn is_linux() -> bool {
    !is_windows() && !is_mac()
}

pub fn is_posix() -> bool {
    cfg!(unix)
}

/// Whether `program` is an executable in one of the PATH folders.
pub fn on_path(program: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file() && is_executable(&candidate)
    })
}

#[cfg(unix)]
fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_: &std::path::Path) -> bool {
    true
}

pub fn home() -> PathBuf {
    let var = if is_windows() { "USERPROFILE" } else { "HOME" };
    env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Where settings live: `%APPDATA%\fileEm`, `~/Library/Application Support/fileEm`, or
/// `$XDG_CONFIG_HOME/fileem` (default `~/.config/fileem`). Same folders as the Java version.
pub fn config_dir() -> PathBuf {
    if is_windows() {
        return env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home).join("fileEm");
    }
    if is_mac() {
        return home().join("Library").join("Application Support").join("fileEm");
    }
    let base = match env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => home().join(".config"),
    };
    base.join("fileem")
}
