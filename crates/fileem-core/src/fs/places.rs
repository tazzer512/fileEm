//! The tree's fixed entries: quick access folders (localized XDG user dirs on Linux) and volumes.

use std::path::{Path, PathBuf};

use crate::util::{fs_util, os};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
}

/// Home, Desktop, Documents, Downloads, Music, Pictures, Videos: the ones that exist.
pub fn quick_access() -> Vec<Place> {
    let home = os::home();
    let user_dirs = if os::is_linux() {
        std::fs::read_to_string(os::config_dir().parent().unwrap_or(Path::new("/")).join("user-dirs.dirs"))
            .map(|text| parse_user_dirs(&text, &home))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    quick_access_from(&home, &user_dirs)
}

fn quick_access_from(home: &Path, user_dirs: &[(String, PathBuf)]) -> Vec<Place> {
    let mut out = vec![Place { name: "Home".into(), path: home.to_path_buf() }];
    for (key, default) in [
        ("DESKTOP", "Desktop"),
        ("DOCUMENTS", "Documents"),
        ("DOWNLOAD", "Downloads"),
        ("MUSIC", "Music"),
        ("PICTURES", "Pictures"),
        ("VIDEOS", "Videos"),
    ] {
        let path = user_dirs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, p)| p.clone())
            .unwrap_or_else(|| home.join(default));
        // XDG points unused entries at $HOME itself
        if path != home && path.is_dir() {
            out.push(Place { name: fs_util::display_name(&path), path });
        }
    }
    out
}

/// `XDG_DESKTOP_DIR="$HOME/Bureaublad"` lines of `user-dirs.dirs` → ("DESKTOP", path).
pub fn parse_user_dirs(text: &str, home: &Path) -> Vec<(String, PathBuf)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (key, value) = line.strip_prefix("XDG_")?.split_once('=')?;
            let key = key.strip_suffix("_DIR")?;
            let value = value.trim().trim_matches('"');
            let path = match value.strip_prefix("$HOME") {
                Some(rest) => home.join(rest.trim_start_matches('/')),
                None if value.starts_with('/') => PathBuf::from(value),
                None => return None,
            };
            Some((key.to_string(), path))
        })
        .collect()
}

/// Mounted volumes: the file system root plus drives mounted by the desktop (Linux, macOS) or
/// the drive letters (Windows).
pub fn volumes() -> Vec<Place> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .map(|c| PathBuf::from(format!("{}:\\", c as char)))
            .filter(|p| p.exists())
            .map(|p| Place { name: p.display().to_string(), path: p })
            .collect()
    }
    #[cfg(not(windows))]
    {
        let mut out = vec![Place { name: "File System".into(), path: PathBuf::from("/") }];
        let user = std::env::var("USER").unwrap_or_default();
        let mut parents = vec![PathBuf::from("/mnt"), PathBuf::from("/Volumes")];
        if !user.is_empty() {
            parents.insert(0, Path::new("/media").join(&user));
            parents.insert(1, Path::new("/run/media").join(&user));
        }
        for parent in parents {
            let Ok(rd) = std::fs::read_dir(&parent) else { continue };
            let mut found: Vec<Place> = rd
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir() && !fs_util::is_hidden(p))
                // macOS lists the boot volume in /Volumes as a link to "/"
                .filter(|p| std::fs::canonicalize(p).map(|c| c != Path::new("/")).unwrap_or(true))
                .map(|p| Place { name: fs_util::display_name(&p), path: p })
                .collect();
            found.sort_by(|a, b| crate::util::natural_order::compare(&a.name, &b.name));
            out.extend(found);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_localized_user_dirs() {
        let home = Path::new("/home/u");
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Bureaublad\"\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n\
                    XDG_MUSIC_DIR=\"/data/muziek\"\nXDG_VIDEOS_DIR=\"$HOME/\"\nnonsense\n";
        let dirs = parse_user_dirs(text, home);
        assert_eq!(
            dirs,
            [
                ("DESKTOP".to_string(), PathBuf::from("/home/u/Bureaublad")),
                ("DOWNLOAD".to_string(), PathBuf::from("/home/u/Downloads")),
                ("MUSIC".to_string(), PathBuf::from("/data/muziek")),
                ("VIDEOS".to_string(), PathBuf::from("/home/u")),
            ]
        );
    }

    #[test]
    fn quick_access_lists_existing_folders_under_their_own_names() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir(home.join("Bureaublad")).unwrap();
        std::fs::create_dir(home.join("Documents")).unwrap();
        let user_dirs = vec![
            ("DESKTOP".to_string(), home.join("Bureaublad")),
            ("VIDEOS".to_string(), home.to_path_buf()), // unused: points at home
        ];
        let names: Vec<String> = quick_access_from(home, &user_dirs).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Home", "Bureaublad", "Documents"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_root_is_always_a_volume() {
        assert_eq!(volumes()[0].path, Path::new("/"));
    }
}
