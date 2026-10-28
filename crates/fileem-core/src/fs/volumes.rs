//! Drives and mount points (the Java `Volumes`): what the tree lists under "This computer", their
//! free space, which ones can be ejected, and ejecting them. Polling is the caller's job: call
//! [`list`] off the UI thread every few seconds.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::util::{fs_util, os};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    System,
    Local,
    Removable,
    Network,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub root: PathBuf,
    pub label: String,
    pub kind: Kind,
    /// Free and total bytes; `None` while unknown (a lookup that hangs or failed).
    pub space: Option<(u64, u64)>,
    /// A drive people plug in and take out (USB stick, SD card, external disk).
    pub ejectable: bool,
}

impl Volume {
    /// A volume of `kind`, ejectable when removable (as the Java record's short constructor).
    pub fn new(root: impl Into<PathBuf>, label: impl Into<String>, kind: Kind) -> Volume {
        Volume { root: root.into(), label: label.into(), kind, space: None, ejectable: kind == Kind::Removable }
    }
}

const NETWORK_TYPES: [&str; 11] =
    ["nfs", "nfs4", "cifs", "smb3", "smbfs", "fuse.sshfs", "sshfs", "afpfs", "webdav", "davfs", "fuse.rclone"];

/// How long one listing waits for a volume's space before showing it as unknown.
pub const SPACE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Unmounting writes out everything still cached first, which can take a while for a slow stick.
const EJECT_TIMEOUT: Duration = Duration::from_secs(120);

/// The volumes with their free space (each lookup waits at most [`SPACE_TIMEOUT`]). Blocks: call
/// it off the UI thread.
pub fn list() -> Vec<Volume> {
    let mut v = mounts();
    for vol in &mut v {
        vol.space = space(&vol.root);
    }
    v
}

/// The volumes without their space: only the mount table (or drive letters) is read, so it is
/// cheap enough for the window's first layout.
pub fn mounts() -> Vec<Volume> {
    #[cfg(windows)]
    {
        windows()
    }
    #[cfg(not(windows))]
    {
        if os::is_mac() {
            mac()
        } else {
            let lines = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
            let lines: Vec<&str> = lines.lines().collect();
            linux(&lines, is_hotplug)
        }
    }
}

#[cfg(windows)]
fn windows() -> Vec<Volume> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    let mut out = Vec::new();
    for c in b'A'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", c as char));
        let wide: Vec<u16> = root.to_string_lossy().encode_utf16().chain([0]).collect();
        // SAFETY: a NUL-terminated wide string that lives for the call
        let t = unsafe { GetDriveTypeW(wide.as_ptr()) };
        if t <= 1 {
            continue; // unknown or no such drive
        }
        let letter = format!("{}:", c as char);
        let kind = match t {
            _ if letter.eq_ignore_ascii_case("C:") => Kind::System,
            DRIVE_REMOVABLE | DRIVE_CDROM => Kind::Removable,
            DRIVE_REMOTE => Kind::Network,
            _ => Kind::Local,
        };
        let label = if kind == Kind::Removable { format!("Drive ({letter})") } else { format!("Local Disk ({letter})") };
        out.push(Volume::new(root, label, kind));
    }
    out
}

#[cfg(not(windows))]
fn mac() -> Vec<Volume> {
    let mut boot = "Macintosh HD".to_string();
    let mut others = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for e in rd.flatten() {
            let p = e.path();
            let name = fs_util::display_name(&p);
            if name.starts_with('.') {
                continue;
            }
            match std::fs::canonicalize(&p) {
                Ok(real) if real == Path::new("/") => boot = name,
                Ok(_) => others.push(Volume::new(p, name, Kind::Removable)),
                Err(_) => {}
            }
        }
    }
    others.sort_by(|a, b| crate::util::natural_order::compare(&a.label, &b.label));
    let mut out = vec![Volume::new("/", boot, Kind::System)];
    out.extend(others);
    out
}

#[cfg_attr(windows, allow(dead_code))]
/// The Linux volumes from `/proc/self/mounts` lines: the root, block devices and network shares,
/// without system mounts. `hotplug` tells whether a device sits on a bus for pluggable drives.
fn linux(lines: &[&str], hotplug: impl Fn(&str) -> bool) -> Vec<Volume> {
    let mut out = vec![Volume::new("/", "File System", Kind::System)];
    for line in lines {
        let f: Vec<&str> = line.split(' ').collect();
        if f.len() < 3 {
            continue;
        }
        let (dev, mp, fs_type) = (unescape(f[0]), unescape(f[1]), f[2]);
        let network = NETWORK_TYPES.contains(&fs_type);
        let block = dev.starts_with("/dev/") && !dev.starts_with("/dev/loop") && !dev.starts_with("/dev/zram");
        if (!network && !block) || mp == "/" || ignored_mount(&mp) {
            continue;
        }
        let plugged = block && hotplug(&dev);
        let desktop = mp.starts_with("/media/") || mp.starts_with("/run/media/");
        let kind = if network {
            Kind::Network
        } else if plugged || desktop || mp.starts_with("/mnt/") {
            Kind::Removable
        } else {
            Kind::Local
        };
        let root = PathBuf::from(&mp);
        if out.iter().any(|v| v.root == root) {
            continue;
        }
        // what the desktop mounts for plugged-in drives, or a drive on a pluggable bus; not
        // internal disks mounted below /mnt
        let ejectable = block && (plugged || desktop);
        out.push(Volume { label: fs_util::display_name(&root), root, kind, space: None, ejectable });
    }
    out
}

fn ignored_mount(mp: &str) -> bool {
    ["/boot", "/efi", "/snap", "/var/", "/sys", "/proc", "/dev", "/tmp", "/usr"].iter().any(|p| mp.starts_with(p))
        || (mp.starts_with("/run/") && !mp.starts_with("/run/media/"))
}

/// Mount table fields escape spaces and friends as `\040`.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let octal = b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c));
        if octal {
            let v = (b[i + 1] - b'0') as u32 * 64 + (b[i + 2] - b'0') as u32 * 8 + (b[i + 3] - b'0') as u32;
            out.push(v as u8);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The sysfs entry of a block device ("/dev/disk/by-uuid/…" → "/sys/devices/…/sdb/sdb1").
fn sys_block(dev: &str) -> Option<PathBuf> {
    let real = std::fs::canonicalize(dev).ok()?;
    std::fs::canonicalize(Path::new("/sys/class/block").join(real.file_name()?)).ok()
}

#[cfg_attr(windows, allow(dead_code))]
/// Whether a block device sits on a bus for pluggable drives (USB, SD/MMC, memory stick).
fn is_hotplug(dev: &str) -> bool {
    sys_block(dev).is_some_and(|s| {
        let s = s.to_string_lossy();
        s.contains("/usb") || s.contains("/mmc") || s.contains("/memstick")
    })
}

/// The whole-disk device of a partition ("/dev/sdc1" → "/dev/sdc", "/dev/mmcblk0p1" →
/// "/dev/mmcblk0").
fn disk_of(dev: &str) -> Option<String> {
    let sys = sys_block(dev)?;
    let disk = if sys.join("partition").exists() { sys.parent()?.to_path_buf() } else { sys };
    Some(format!("/dev/{}", disk.file_name()?.to_string_lossy()))
}

// ---- free space ----

type Lookup = Receiver<Option<(u64, u64)>>;

/// Lookups still running, per root: a hung mount keeps one stuck lookup (reused by later
/// listings), never one more thread per poll.
fn lookups() -> &'static Mutex<HashMap<PathBuf, Lookup>> {
    static LOOKUPS: OnceLock<Mutex<HashMap<PathBuf, Lookup>>> = OnceLock::new();
    LOOKUPS.get_or_init(Default::default)
}

/// Free and total space of the volume at `root`, or `None` when unknown. A dead network mount can
/// block these calls for minutes; this gives up after [`SPACE_TIMEOUT`] so the other volumes
/// keep updating, and asks again once the stuck lookup has returned.
pub fn space(root: &Path) -> Option<(u64, u64)> {
    let rx = {
        let mut map = lookups().lock().unwrap();
        match map.remove(root) {
            Some(rx) => rx,
            None => {
                let (tx, rx) = mpsc::channel();
                let r = root.to_path_buf();
                let started = std::thread::Builder::new().name("fileem-volume-space".into()).spawn(move || {
                    let s = fs4::available_space(&r).ok().zip(fs4::total_space(&r).ok());
                    let _ = tx.send(s);
                });
                if started.is_err() {
                    return None;
                }
                rx
            }
        }
    };
    match rx.recv_timeout(SPACE_TIMEOUT) {
        Ok(s) => s.filter(|&(_, total)| total > 0),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // still hanging: the next listing waits on the same lookup
            lookups().lock().unwrap().insert(root.to_path_buf(), rx);
            None
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
    }
}

// ---- eject ----

/// The mount points that go away when the drive holding `root` is ejected: every mounted
/// partition of the same physical drive, `root` first. Ejecting removes the whole drive (as
/// Windows and macOS do), so tabs on any of them must leave first. Reads sysfs: call it off the
/// UI thread.
pub fn same_drive(root: &Path) -> Vec<PathBuf> {
    if !os::is_linux() {
        return vec![root.to_path_buf()];
    }
    match std::fs::read_to_string("/proc/self/mounts") {
        Ok(text) => same_drive_mounts(&text.lines().collect::<Vec<_>>(), root, disk_of),
        Err(_) => vec![root.to_path_buf()],
    }
}

/// Mount points whose device is on the same disk as `root`'s device (`disk_of`: device → disk).
pub fn same_drive_mounts(lines: &[&str], root: &Path, disk_of: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    let Some(disk) = device_for(lines, root).and_then(|d| disk_of(&d)) else { return out };
    for line in lines {
        let f: Vec<&str> = line.split(' ').collect();
        if f.len() < 2 || !f[0].starts_with("/dev/") {
            continue;
        }
        let (dev, mp) = (unescape(f[0]), PathBuf::from(unescape(f[1])));
        if !out.contains(&mp) && disk_of(&dev).as_deref() == Some(disk.as_str()) && device_for(lines, &mp).as_deref() == Some(dev.as_str()) {
            out.push(mp);
        }
    }
    out
}

/// The device mounted at `root`, from mount table lines (the last mount on a path is the one
/// visible).
pub fn device_for(lines: &[&str], root: &Path) -> Option<String> {
    lines
        .iter()
        .rev()
        .find_map(|line| {
            let mut f = line.split(' ');
            let (dev, mp) = (f.next()?, f.next()?);
            (Path::new(&unescape(mp)) == root).then(|| unescape(dev))
        })
}

/// The commands that eject a drive on Linux, in order: udisks (unmount each of its `devices`,
/// then power off the `disk`), else GIO's eject (which removes the whole drive), else a plain
/// unmount of `root`. Empty when none of these tools is installed.
pub fn linux_eject_commands(root: &Path, devices: &[String], disk: Option<&str>, installed: impl Fn(&str) -> bool) -> Vec<Vec<String>> {
    let r = root.display().to_string();
    if !devices.is_empty() && devices.iter().all(|d| d.starts_with("/dev/")) && installed("udisksctl") {
        let mut steps: Vec<Vec<String>> = devices
            .iter()
            .map(|d| ["udisksctl", "unmount", "--no-user-interaction", "-b", d].map(String::from).to_vec())
            .collect();
        let disk = disk.unwrap_or(&devices[0]);
        steps.push(["udisksctl", "power-off", "--no-user-interaction", "-b", disk].map(String::from).to_vec());
        return steps;
    }
    if installed("gio") {
        return vec![["gio", "mount", "--eject", &r].map(String::from).to_vec()];
    }
    if installed("umount") {
        return vec![["umount", &r].map(String::from).to_vec()];
    }
    Vec::new()
}

/// Safely removes the drive holding `roots` (from [`same_drive`]): unmounts every one of them
/// (which writes out everything still cached) and, where the system can, powers the drive off.
/// Blocks until done: call it off the UI thread. The error is the system's explanation, worded
/// for people (a busy drive is the common case).
pub fn eject(roots: &[PathBuf]) -> Result<(), String> {
    let Some(root) = roots.first() else { return Ok(()) };
    let steps: Vec<Vec<String>> = if os::is_windows() {
        let drive = root.display().to_string().replace('\\', "");
        if drive.len() != 2 || !drive.ends_with(':') {
            return Err("Only drive letters can be ejected.".into());
        }
        let script = format!("(New-Object -ComObject Shell.Application).Namespace(17).ParseName('{drive}').InvokeVerb('Eject')");
        vec![["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", &script].map(String::from).to_vec()]
    } else if os::is_mac() {
        vec![vec!["diskutil".into(), "eject".into(), root.display().to_string()]]
    } else {
        let text = std::fs::read_to_string("/proc/self/mounts").map_err(|e| e.to_string())?;
        let lines: Vec<&str> = text.lines().collect();
        let devices: Vec<String> = roots.iter().filter_map(|r| device_for(&lines, r)).collect();
        let disk = devices.first().and_then(|d| disk_of(d));
        linux_eject_commands(root, &devices, disk.as_deref(), os::on_path)
    };
    if steps.is_empty() {
        return Err("No tool to eject drives was found (udisksctl, gio or umount).".into());
    }
    for step in &steps {
        if let Err(e) = run(step) {
            // everything is unmounted by then, so the data is safe; the drive just stays powered
            if !step.iter().any(|a| a == "power-off") {
                return Err(e);
            }
        }
    }
    if os::is_windows() {
        // the verb reports nothing: wait until the drive is gone
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if !root.exists() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        return Err("Windows could not eject the drive. Close any files or programs using it and try again.".into());
    }
    Ok(())
}

fn run(cmd: &[String]) -> Result<(), String> {
    run_within(cmd, EJECT_TIMEOUT)
}

/// Runs `cmd`, killing it after `timeout`; its output is the error.
fn run_within(cmd: &[String], timeout: Duration) -> Result<(), String> {
    let mut child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
    std::thread::spawn(move || {
        use std::io::Read;
        let mut text = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_string(&mut text);
        }
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_string(&mut text);
        }
        let _ = tx.send(text);
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let text = rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
                return if status.success() { Ok(()) } else { Err(eject_error(&text)) };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait(); // reaped, not left behind as a zombie
                return Err("The drive did not respond.".into());
            }
            Err(e) => return Err(io::Error::other(e).to_string()),
        }
    }
}

/// Turns a tool's error output into a message for people.
pub fn eject_error(output: &str) -> String {
    let text = output.trim();
    let lower = text.to_lowercase();
    if lower.contains("busy") || lower.contains("in use") {
        "The drive is in use. Close any files, programs or terminals using it and try again.".into()
    } else if lower.contains("not authorized") || lower.contains("permission") || lower.contains("not permitted") {
        "You are not allowed to eject this drive.".into()
    } else if text.is_empty() {
        "The drive could not be ejected.".into()
    } else {
        text.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[cfg(target_os = "linux")]
    #[test]
    fn a_tool_that_hangs_is_stopped_and_reaped() {
        let cmd = ["sleep", "37"].map(String::from);
        let start = Instant::now();
        assert_eq!(run_within(&cmd, Duration::from_millis(200)), Err("The drive did not respond.".into()));
        assert!(start.elapsed() < Duration::from_secs(5));
        // no sleep of ours is left, not even as a zombie
        let me = std::process::id().to_string();
        for proc in std::fs::read_dir("/proc").unwrap().filter_map(Result::ok) {
            let Ok(stat) = std::fs::read_to_string(proc.path().join("stat")) else { continue };
            // pid (comm) state ppid ...
            let after = &stat[stat.rfind(')').map_or(0, |i| i + 2)..];
            let ppid = after.split(' ').nth(1).unwrap_or("");
            assert!(!(ppid == me && stat.contains("(sleep)")), "left behind: {stat}");
        }
    }

    #[test]
    fn lists_at_least_the_system_volume_with_unique_existing_roots() {
        let vols = list();
        assert!(!vols.is_empty());
        let roots: HashSet<&PathBuf> = vols.iter().map(|v| &v.root).collect();
        assert_eq!(roots.len(), vols.len(), "no duplicate mount points");
        for v in &vols {
            assert!(!v.label.trim().is_empty(), "label for {}", v.root.display());
            assert!(v.root.is_absolute());
        }
        if !os::is_windows() {
            assert_eq!(vols[0].root, Path::new("/"), "the root file system comes first");
            assert!(vols[0].root.is_dir());
            assert!(vols[0].space.is_some_and(|(_, total)| total > 0), "space is reported for the root");
        }
    }

    #[test]
    fn linux_volumes_come_from_the_mount_table() {
        let lines = [
            "/dev/nvme0n1p2 / ext4 rw 0 0",
            "proc /proc proc rw 0 0",
            "/dev/nvme0n1p1 /boot/efi vfat rw 0 0",
            "/dev/sdb1 /media/me/MY\\040STICK vfat rw 0 0",
            "/dev/sdc1 /mnt/data ext4 rw 0 0",
            "/dev/sdd1 /mnt/usbdisk ext4 rw 0 0",
            "/dev/nvme0n1p3 /home ext4 rw 0 0",
            "/dev/loop3 /snap/core/1 squashfs ro 0 0",
            "/dev/loop4 /opt/img squashfs ro 0 0",
            "server:/export /net/share nfs4 rw 0 0",
            "tmpfs /run/user/1000 tmpfs rw 0 0",
            "/dev/sdb1 /media/me/MY\\040STICK vfat rw 0 0",
        ];
        let vols = linux(&lines, |dev| dev == "/dev/sdd1");
        let shown: Vec<(&str, &str, Kind, bool)> =
            vols.iter().map(|v| (v.root.to_str().unwrap(), v.label.as_str(), v.kind, v.ejectable)).collect();
        assert_eq!(
            shown,
            [
                ("/", "File System", Kind::System, false),
                ("/media/me/MY STICK", "MY STICK", Kind::Removable, true),
                ("/mnt/data", "data", Kind::Removable, false),
                ("/mnt/usbdisk", "usbdisk", Kind::Removable, true),
                ("/home", "home", Kind::Local, false),
                ("/net/share", "share", Kind::Network, false),
            ],
            "system mounts, loop devices and duplicates left out; internal disks under /mnt aren't ejectable"
        );
    }

    #[test]
    fn linux_eject_unmounts_every_partition_then_powers_off_the_disk() {
        let root = Path::new("/media/me/STICK");
        let devs = vec!["/dev/sdb1".to_string(), "/dev/sdb2".to_string()];
        let cmds = |tools: &[&str], devs: &[String], disk: Option<&str>| {
            let tools: Vec<String> = tools.iter().map(|t| t.to_string()).collect();
            linux_eject_commands(root, devs, disk, |t| tools.iter().any(|x| x == t))
        };
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            cmds(&["udisksctl", "gio", "umount"], &devs, Some("/dev/sdb")),
            [
                v(&["udisksctl", "unmount", "--no-user-interaction", "-b", "/dev/sdb1"]),
                v(&["udisksctl", "unmount", "--no-user-interaction", "-b", "/dev/sdb2"]),
                v(&["udisksctl", "power-off", "--no-user-interaction", "-b", "/dev/sdb"]),
            ]
        );
        assert_eq!(cmds(&["gio", "umount"], &devs, Some("/dev/sdb")), [v(&["gio", "mount", "--eject", "/media/me/STICK"])]);
        assert_eq!(cmds(&["udisksctl", "gio"], &[], None), [v(&["gio", "mount", "--eject", "/media/me/STICK"])], "no device: gio");
        assert_eq!(cmds(&["umount"], &devs, Some("/dev/sdb")), [v(&["umount", "/media/me/STICK"])]);
        assert!(cmds(&[], &devs, Some("/dev/sdb")).is_empty());
    }

    #[test]
    fn same_drive_finds_the_other_mounted_partitions_of_the_disk() {
        let mounts = [
            "/dev/nvme0n1p2 / ext4 rw 0 0",
            "/dev/sdc1 /media/me/photos exfat rw 0 0",
            "/dev/sdd1 /media/me/other exfat rw 0 0",
            "/dev/sdc2 /media/me/storage exfat rw 0 0",
            "tmpfs /media/me/storage/cache tmpfs rw 0 0",
        ];
        let disk_of = |d: &str| Some(d.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches('p').to_string());
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(same_drive_mounts(&mounts, Path::new("/media/me/storage"), disk_of), [p("/media/me/storage"), p("/media/me/photos")]);
        assert_eq!(same_drive_mounts(&mounts, Path::new("/media/me/other"), disk_of), [p("/media/me/other")]);
        assert_eq!(same_drive_mounts(&mounts, Path::new("/media/me/gone"), disk_of), [p("/media/me/gone")]);
    }

    #[test]
    fn device_for_reads_escaped_mount_points_and_takes_the_top_mount() {
        let mounts = [
            "/dev/nvme0n1p2 / ext4 rw 0 0",
            "/dev/sdb1 /media/me/MY\\040STICK vfat rw 0 0",
            "/dev/sdc1 /mnt/x ext4 rw 0 0",
            "/dev/sdd1 /mnt/x ext4 rw 0 0",
        ];
        assert_eq!(device_for(&mounts, Path::new("/media/me/MY STICK")).as_deref(), Some("/dev/sdb1"));
        assert_eq!(device_for(&mounts, Path::new("/mnt/x")).as_deref(), Some("/dev/sdd1"), "the last mount is the visible one");
        assert_eq!(device_for(&mounts, Path::new("/media/me/OTHER")), None);
        assert_eq!(unescape("a\\040b\\134c\\0"), "a b\\c\\0", "a cut-off escape stays as it is");
    }

    #[test]
    fn eject_errors_are_explained_for_people() {
        assert!(eject_error("Error unmounting /dev/sdb1: target is busy").contains("in use"));
        assert!(eject_error("Not authorized to perform operation").contains("not allowed"));
        assert_eq!(eject_error("  something odd\n"), "something odd");
        assert!(!eject_error("").trim().is_empty());
    }

    #[test]
    fn only_removable_volumes_are_ejectable_by_default() {
        assert!(Volume::new("/media/x", "x", Kind::Removable).ejectable);
        assert!(!Volume::new("/", "/", Kind::System).ejectable);
        assert!(!Volume::new("/net", "n", Kind::Network).ejectable);
        for v in list() {
            if v.kind == Kind::System {
                assert!(!v.ejectable, "the system drive is never ejectable");
            }
        }
    }

    #[test]
    fn a_missing_root_has_no_space() {
        let tmp = tempfile::tempdir().unwrap();
        // Windows answers for any path on a drive, existing or not: a drive letter that isn't there
        #[cfg(windows)]
        let missing = ('D'..='Z').rev().map(|c| PathBuf::from(format!("{c}:\\"))).find(|p| !p.exists()).unwrap();
        #[cfg(not(windows))]
        let missing = tmp.path().join("not-there");
        assert_eq!(space(&missing), None);
        assert!(space(tmp.path()).is_some());
    }
}
