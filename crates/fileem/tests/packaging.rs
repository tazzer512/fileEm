//! Keeps the packages consistent with the app (packaging/: the Linux install script, the .deb,
//! the macOS bundle; icons/): the app id the windows carry names the launcher entry, every window
//! gets the icon, and the scripts install what they should. The scripts run with a stand-in
//! executable into temp folders, never into the real ~/.local or /usr.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use fileem::about::{APP_ID, VERSION};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    fs::read_to_string(crate_dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The value of `key=` in a desktop entry.
fn entry(desktop: &str, key: &str) -> String {
    desktop.lines().find_map(|l| l.strip_prefix(&format!("{key}="))).unwrap_or_else(|| panic!("no {key}")).to_owned()
}

fn png_size(p: &Path) -> (u32, u32) {
    let b = fs::read(p).unwrap();
    assert_eq!(&b[1..4], b"PNG", "{}", p.display());
    let be = |i: usize| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
    (be(16), be(20))
}

#[test]
fn the_launcher_entry_is_named_after_the_app_id() {
    // Wayland links windows to the entry whose file name is their app id, X11 by StartupWMClass
    let desktop = read(&format!("packaging/linux/{APP_ID}.desktop"));
    assert_eq!(entry(&desktop, "StartupWMClass"), APP_ID);
    assert_eq!(entry(&desktop, "Icon"), APP_ID);
    // its own name in the app grid, next to the Java app's "fileEm"
    assert_eq!(entry(&desktop, "Name"), "rustyFileEm");
    assert!(entry(&desktop, "Exec").starts_with("fileem "), "the scripts rewrite or keep this");
    for script in ["packaging/linux/install.sh", "packaging/linux/make-deb.sh", "packaging/macos/make-app.sh"] {
        assert!(read(script).contains(&format!("app_id=\"{APP_ID}\"")), "{script} uses the app id");
    }
    assert!(read("src/main.rs").contains("set_xdg_app_id(fileem::about::APP_ID)"), "the windows carry it");
    // the freedesktop checker, where installed
    if let Ok(out) = Command::new("desktop-file-validate").arg(crate_dir().join(format!("packaging/linux/{APP_ID}.desktop"))).output() {
        assert!(out.status.success() && out.stdout.is_empty(), "{}", String::from_utf8_lossy(&out.stdout));
    }
}

#[test]
fn every_window_shows_the_app_icon() {
    for f in fs::read_dir(crate_dir().join("ui")).unwrap() {
        let path = f.unwrap().path();
        let src = fs::read_to_string(&path).unwrap();
        let windows = src.matches("inherits Window {").count();
        assert_eq!(src.matches("icon: Theme.app-icon;").count(), windows, "{}", path.display());
    }
    assert!(read("ui/theme.slint").contains("@image-url(\"../icons/fileem-256.png\")"));
}

#[test]
fn icons_exist_in_every_size_and_format() {
    for size in [16, 24, 32, 48, 64, 128, 256, 512] {
        assert_eq!(png_size(&crate_dir().join(format!("icons/fileem-{size}.png"))), (size, size));
    }
    let ico = fs::read(crate_dir().join("icons/fileem.ico")).unwrap();
    assert_eq!(&ico[..4], &[0, 0, 1, 0], "an ICO file");
    let icns = fs::read(crate_dir().join("icons/fileem.icns")).unwrap();
    assert_eq!(&icns[..4], b"icns");
    assert!(read("build.rs").contains("set_icon(\"icons/fileem.ico\")"), "the .exe gets it");
}

/// A shell script the packaging scripts install in place of the app.
fn stand_in(dir: &Path) -> PathBuf {
    let bin = dir.join("fileem-built");
    fs::write(&bin, "#!/bin/sh\necho fileem\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

fn run(script: &str, args: &[&str], env: &[(&str, &Path)]) -> String {
    let mut c = Command::new("bash");
    c.arg(crate_dir().join(script)).args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("bash runs");
    assert!(out.status.success(), "{script}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[cfg(unix)]
#[test]
fn the_linux_install_puts_app_entry_and_icons_in_place_and_uninstall_removes_only_them() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("share");
    let bin = stand_in(tmp.path());
    // the Java app's install sits next to it and must survive both
    let java_icon = data.join("icons/hicolor/48x48/apps/fileem.png");
    fs::create_dir_all(java_icon.parent().unwrap()).unwrap();
    fs::write(&java_icon, "java").unwrap();
    fs::create_dir_all(data.join("fileem/app")).unwrap();

    let env = [("XDG_DATA_HOME", data.as_path()), ("FILEEM_BIN", bin.as_path())];
    run("packaging/linux/install.sh", &[], &env);
    let installed = data.join("fileem-rust/fileem");
    assert_eq!(fs::read(&installed).unwrap(), fs::read(&bin).unwrap());
    let desktop = fs::read_to_string(data.join(format!("applications/{APP_ID}.desktop"))).unwrap();
    assert_eq!(entry(&desktop, "Exec"), format!("\"{}\" %f", installed.display()));
    assert_eq!(entry(&desktop, "StartupWMClass"), APP_ID);
    for size in [16, 48, 512] {
        let icon = data.join(format!("icons/hicolor/{size}x{size}/apps/{APP_ID}.png"));
        assert_eq!(png_size(&icon), (size, size));
    }
    // installing again replaces the executable
    fs::write(&bin, "#!/bin/sh\necho newer\n").unwrap();
    run("packaging/linux/install.sh", &[], &env);
    assert_eq!(fs::read_to_string(&installed).unwrap(), "#!/bin/sh\necho newer\n");
    assert!(!data.join("fileem-rust/fileem.new").exists());

    run("packaging/linux/install.sh", &["--uninstall"], &env);
    assert!(!data.join("fileem-rust").exists());
    assert!(!data.join(format!("applications/{APP_ID}.desktop")).exists());
    assert!(!data.join(format!("icons/hicolor/48x48/apps/{APP_ID}.png")).exists());
    assert_eq!(fs::read_to_string(&java_icon).unwrap(), "java", "the Java app's icon stays");
    assert!(data.join("fileem/app").is_dir(), "the Java app stays");
}

#[cfg(unix)]
#[test]
fn the_linux_install_refuses_without_a_build() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("share");
    let out = Command::new("bash")
        .arg(crate_dir().join("packaging/linux/install.sh"))
        .env("XDG_DATA_HOME", &data)
        .env("FILEEM_BIN", tmp.path().join("missing"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cargo build --release"));
    assert!(!data.exists(), "nothing half installed");
}

#[cfg(unix)]
#[test]
fn the_deb_holds_the_executable_entry_and_icons_with_the_version() {
    if Command::new("dpkg-deb").arg("--version").output().is_err() {
        eprintln!("no dpkg-deb here: skipped");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let bin = stand_in(tmp.path());
    let dist = tmp.path().join("dist");
    let deb = run("packaging/linux/make-deb.sh", &[], &[("FILEEM_BIN", bin.as_path()), ("FILEEM_DIST", dist.as_path())]);
    let deb = PathBuf::from(deb.trim());
    assert!(deb.starts_with(&dist) && deb.file_name().unwrap().to_string_lossy().starts_with(&format!("fileem_{VERSION}_")));
    let field = |f: &str| {
        let o = Command::new("dpkg-deb").args(["-f"]).arg(&deb).arg(f).output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_owned()
    };
    assert_eq!(field("Package"), "fileem");
    assert_eq!(field("Version"), VERSION);
    let list = Command::new("dpkg-deb").arg("-c").arg(&deb).output().unwrap();
    let list = String::from_utf8_lossy(&list.stdout);
    for path in [
        "./usr/bin/fileem".to_owned(),
        format!("./usr/share/applications/{APP_ID}.desktop"),
        format!("./usr/share/icons/hicolor/256x256/apps/{APP_ID}.png"),
    ] {
        assert!(list.lines().any(|l| l.ends_with(&path)), "{path} in\n{list}");
    }
    assert!(list.contains("-rwxr-xr-x root/root"), "owned by root, executable:\n{list}");
}

#[cfg(unix)]
#[test]
fn the_macos_bundle_has_executable_icon_and_info_with_id_and_version() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = stand_in(tmp.path());
    let dist = tmp.path().join("dist");
    let app = run("packaging/macos/make-app.sh", &[], &[("FILEEM_BIN", bin.as_path()), ("FILEEM_DIST", dist.as_path())]);
    let app = PathBuf::from(app.trim());
    assert_eq!(app, dist.join("fileEm.app"));
    assert_eq!(fs::read(app.join("Contents/MacOS/fileem")).unwrap(), fs::read(&bin).unwrap());
    assert_eq!(fs::read(app.join("Contents/Resources/fileem.icns")).unwrap(), fs::read(crate_dir().join("icons/fileem.icns")).unwrap());
    let plist = fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
    for (key, value) in [
        ("CFBundleIdentifier", APP_ID),
        ("CFBundleExecutable", "fileem"),
        ("CFBundleIconFile", "fileem.icns"),
        ("CFBundleShortVersionString", VERSION),
    ] {
        assert!(plist.contains(&format!("<key>{key}</key><string>{value}</string>")), "{key} in\n{plist}");
    }
    // building again starts from scratch
    fs::write(app.join("Contents/stale"), "").unwrap();
    run("packaging/macos/make-app.sh", &[], &[("FILEEM_BIN", bin.as_path()), ("FILEEM_DIST", dist.as_path())]);
    assert!(!app.join("Contents/stale").exists());
}
