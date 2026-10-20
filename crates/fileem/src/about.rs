//! The About box's text (the Java `AppInfo`): version, build date, toolkit and system. build.rs
//! fills in the build date, the compiler and the Slint version.

/// The version, like "0.1.0": the workspace's, which must match the pom's `app.version` (the
/// Java app and the packages use that one).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The app id: Wayland's app id and X11's window class, so the desktop links the windows to the
/// launcher entry of the same name (`packaging/linux/install.sh`) and shows its icon. Also the
/// macOS bundle id. Not the Java app's (`com.fileem.FileEmApp`): both can be installed.
pub const APP_ID: &str = "com.fileem.FileEm";

/// The build date (yyyy-mm-dd).
pub fn build_date() -> Option<&'static str> {
    option_env!("FILEEM_BUILT").filter(|d| !d.is_empty())
}

/// The About box's text.
pub fn text() -> String {
    compose(VERSION, build_date(), env!("FILEEM_RUSTC"), env!("FILEEM_SLINT"), &system())
}

/// "Version …", "Built …", then the toolkit and the system, like the Java app's.
pub fn compose(version: &str, built: Option<&str>, rustc: &str, slint: &str, system: &str) -> String {
    let mut s = format!("Version {version}");
    if let Some(d) = built {
        s += &format!("\nBuilt {d}");
    }
    let or_unknown = |v: &str| if v.is_empty() { "?".to_owned() } else { v.to_owned() };
    s += &format!("\n\nRust {}  ·  Slint {}\n{system}", or_unknown(rustc), or_unknown(slint));
    s
}

/// The system's name, version (where known) and architecture, like "Linux 6.8.0 (x86_64)".
pub fn system() -> String {
    let name = match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "macOS",
        "windows" => "Windows",
        other => other,
    };
    match os_version() {
        Some(v) => format!("{name} {v} ({})", std::env::consts::ARCH),
        None => format!("{name} ({})", std::env::consts::ARCH),
    }
}

/// The kernel's release on Linux (what Java's `os.version` shows); elsewhere none.
fn os_version() -> Option<String> {
    if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_has_version_date_toolkit_and_system() {
        let t = compose("1.2.3", Some("2026-10-20"), "1.90.0", "1.18.1", "Linux 6.8 (x86_64)");
        assert_eq!(t, "Version 1.2.3\nBuilt 2026-10-20\n\nRust 1.90.0  ·  Slint 1.18.1\nLinux 6.8 (x86_64)");
        let t = compose("1.2.3", None, "", "", "Windows (x86_64)");
        assert_eq!(t, "Version 1.2.3\n\nRust ?  ·  Slint ?\nWindows (x86_64)");
    }

    #[test]
    fn build_info_is_filled_in() {
        let d = build_date().expect("build.rs sets the date");
        assert!(d.len() == 10 && d.as_bytes()[4] == b'-' && d.as_bytes()[7] == b'-', "{d}");
        assert!(d[..4].parse::<u32>().unwrap() >= 2026, "{d}");
        assert!(env!("FILEEM_RUSTC").starts_with("1."), "{}", env!("FILEEM_RUSTC"));
        assert!(env!("FILEEM_SLINT").starts_with("1."), "{}", env!("FILEEM_SLINT"));
        assert!(text().starts_with(&format!("Version {VERSION}\nBuilt {d}\n\nRust ")));
        assert!(system().contains(std::env::consts::ARCH));
    }

    /// One version for both apps (the Java `AppInfoTest`): the pom's `app.version` is what the
    /// packages carry.
    #[test]
    fn the_version_matches_the_poms_app_version() {
        let pom = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../pom.xml")).expect("pom.xml");
        let app_version = pom
            .split_once("<app.version>")
            .and_then(|(_, rest)| rest.split_once("</app.version>"))
            .map(|(v, _)| v.trim())
            .expect("the pom has <app.version>");
        assert_eq!(VERSION, app_version);
    }
}
