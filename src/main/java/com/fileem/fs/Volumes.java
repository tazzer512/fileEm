package com.fileem.fs;

import com.fileem.util.OsUtil;
import javafx.application.Platform;
import javafx.beans.property.ReadOnlyObjectProperty;
import javafx.beans.property.ReadOnlyObjectWrapper;

import java.io.IOException;
import java.nio.file.DirectoryStream;
import java.nio.file.FileStore;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.function.Function;
import java.util.function.Predicate;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Detects drives / mount points and keeps the list up to date by polling. */
public final class Volumes {
    public enum Kind { SYSTEM, LOCAL, REMOVABLE, NETWORK }

    /** {@code ejectable}: a drive the user plugs in and removes (USB stick, SD card, external disk). */
    public record Volume(Path root, String label, Kind kind, long free, long total, boolean ejectable) {
        public Volume(Path root, String label, Kind kind, long free, long total) {
            this(root, label, kind, free, total, kind == Kind.REMOVABLE);
        }
    }

    private static final Set<String> NETWORK_TYPES = Set.of(
            "nfs", "nfs4", "cifs", "smb3", "smbfs", "fuse.sshfs", "sshfs", "afpfs", "webdav", "davfs", "fuse.rclone");
    private static final Pattern OCTAL = Pattern.compile("\\\\([0-7]{3})");
    private static final ReadOnlyObjectWrapper<List<Volume>> CURRENT = new ReadOnlyObjectWrapper<>(List.of());
    private static boolean started;
    private static ScheduledExecutorService poller;
    /** Unmounting flushes everything to the device first, which can take a while for a slow stick. */
    private static final long EJECT_TIMEOUT_S = 120;

    private Volumes() {
    }

    /** The current volume list (FX thread). Starts the background monitor on first use. */
    public static ReadOnlyObjectProperty<List<Volume>> current() {
        if (!started) {
            started = true;
            poller = Executors.newSingleThreadScheduledExecutor(r -> {
                Thread t = new Thread(r, "fileem-volumes");
                t.setDaemon(true);
                return t;
            });
            poller.scheduleWithFixedDelay(Volumes::poll, 0, 5, TimeUnit.SECONDS);
        }
        return CURRENT.getReadOnlyProperty();
    }

    /** Re-reads the volumes now instead of at the next poll (after an eject). FX thread. */
    public static void pollNow() {
        if (poller != null) {
            poller.execute(Volumes::poll);
        }
    }

    private static void poll() {
        List<Volume> list = list();
        Platform.runLater(() -> {
            if (!list.equals(CURRENT.get())) {
                CURRENT.set(list);
            }
        });
    }

    public static List<Volume> list() {
        try {
            if (OsUtil.isWindows()) {
                return windows();
            }
            if (OsUtil.isMac()) {
                return mac();
            }
            return linux();
        } catch (RuntimeException e) {
            return List.of(volume(Path.of("/"), "File System", Kind.SYSTEM));
        }
    }

    private static List<Volume> windows() {
        List<Volume> out = new ArrayList<>();
        for (Path root : FileSystems.getDefault().getRootDirectories()) {
            String letter = root.toString().replace("\\", "");
            StoreInfo info = storeInfo(root);
            String label = info == null ? "Drive (" + letter + ")"
                    : (info.name() == null || info.name().isBlank() ? "Local Disk" : info.name()) + " (" + letter + ")";
            Kind kind = letter.equalsIgnoreCase("C:") ? Kind.SYSTEM
                    : info != null && info.removable() ? Kind.REMOVABLE : Kind.LOCAL;
            out.add(new Volume(root, label, kind, info == null ? -1 : info.free(), info == null ? -1 : info.total()));
        }
        return out;
    }

    private static List<Volume> mac() {
        List<Volume> out = new ArrayList<>();
        String bootName = "Macintosh HD";
        List<Volume> others = new ArrayList<>();
        Path volumes = Path.of("/Volumes");
        try (DirectoryStream<Path> ds = Files.newDirectoryStream(volumes)) {
            for (Path v : ds) {
                String name = v.getFileName().toString();
                if (name.startsWith(".")) {
                    continue;
                }
                try {
                    if (v.toRealPath().equals(Path.of("/"))) {
                        bootName = name;
                        continue;
                    }
                } catch (IOException e) {
                    continue;
                }
                Kind kind = Kind.REMOVABLE;
                try {
                    if (NETWORK_TYPES.contains(Files.getFileStore(v).type())) {
                        kind = Kind.NETWORK;
                    }
                } catch (IOException ignored) {
                    // keep removable
                }
                others.add(volume(v, name, kind));
            }
        } catch (IOException ignored) {
            // no /Volumes, just show the root
        }
        out.add(volume(Path.of("/"), bootName, Kind.SYSTEM));
        out.addAll(others);
        return out;
    }

    private static List<Volume> linux() {
        Map<String, Kind> mounts = new LinkedHashMap<>();
        Set<String> ejectableMounts = new HashSet<>();
        mounts.put("/", Kind.SYSTEM);
        try {
            for (String line : Files.readAllLines(Path.of("/proc/self/mounts"))) {
                String[] f = line.split(" ");
                if (f.length < 3) {
                    continue;
                }
                String dev = f[0], mp = unescape(f[1]), type = f[2];
                boolean network = NETWORK_TYPES.contains(type);
                boolean block = dev.startsWith("/dev/") && !dev.startsWith("/dev/loop") && !dev.startsWith("/dev/zram");
                if (!network && !block || mp.equals("/") || ignoredMount(mp)) {
                    continue;
                }
                boolean hotplug = block && isHotplug(dev);
                Kind kind = network ? Kind.NETWORK
                        : hotplug || mp.startsWith("/media/") || mp.startsWith("/run/media/") || mp.startsWith("/mnt/")
                        ? Kind.REMOVABLE : Kind.LOCAL;
                // what the desktop mounts for plugged-in drives, or a drive on a hot-pluggable bus;
                // not internal disks mounted below /mnt
                boolean ejectable = block && (hotplug || mp.startsWith("/media/") || mp.startsWith("/run/media/"));
                if (!mounts.containsKey(mp)) {
                    mounts.put(mp, kind);
                    if (ejectable) {
                        ejectableMounts.add(mp);
                    }
                }
            }
        } catch (IOException ignored) {
            // no /proc: root only
        }
        List<Volume> out = new ArrayList<>();
        mounts.forEach((mp, kind) -> {
            Path p = Path.of(mp);
            Volume v = volume(p, mp.equals("/") ? "File System" : p.getFileName().toString(), kind);
            out.add(new Volume(v.root(), v.label(), v.kind(), v.free(), v.total(), ejectableMounts.contains(mp)));
        });
        return out;
    }

    /** Whether a block device sits on a bus for pluggable drives (USB, SD/MMC, memory stick). */
    private static boolean isHotplug(String dev) {
        try {
            Path real = Path.of(dev).toRealPath(); // /dev/disk/by-uuid/... links to /dev/sdb1
            String sys = Path.of("/sys/class/block", real.getFileName().toString()).toRealPath().toString();
            return sys.contains("/usb") || sys.contains("/mmc") || sys.contains("/memstick");
        } catch (IOException | RuntimeException e) {
            return false;
        }
    }

    private static boolean attribute(FileStore store, String name) {
        try {
            return Boolean.TRUE.equals(store.getAttribute(name));
        } catch (IOException | RuntimeException e) {
            return false;
        }
    }

    // ---- eject ----

    /**
     * The mount points that go away when the drive holding {@code root} is ejected: every mounted
     * partition of the same physical drive, {@code root} first. Ejecting removes the whole drive (as
     * Windows and macOS do), so tabs on any of them must leave first. Reads sysfs: call off the FX thread.
     */
    public static List<Path> sameDrive(Path root) {
        if (!OsUtil.isLinux()) {
            return List.of(root);
        }
        try {
            return sameDriveMounts(Files.readAllLines(Path.of("/proc/self/mounts")), root, Volumes::diskOf);
        } catch (IOException | RuntimeException e) {
            return List.of(root);
        }
    }

    /** Mount points whose device is on the same disk as {@code root}'s device ({@code diskOf}: device → disk). */
    static List<Path> sameDriveMounts(List<String> mountLines, Path root, Function<String, String> diskOf) {
        String device = deviceFor(mountLines, root);
        String disk = device == null ? null : diskOf.apply(device);
        List<Path> out = new ArrayList<>(List.of(root));
        if (disk == null) {
            return out;
        }
        for (String line : mountLines) {
            String[] f = line.split(" ");
            if (f.length < 2 || !f[0].startsWith("/dev/")) {
                continue;
            }
            Path mp = Path.of(unescape(f[1]));
            if (!out.contains(mp) && disk.equals(diskOf.apply(unescape(f[0])))
                    && unescape(f[0]).equals(deviceFor(mountLines, mp))) {
                out.add(mp);
            }
        }
        return out;
    }

    /** The whole-disk device of a partition ("/dev/sdc1" → "/dev/sdc", "/dev/mmcblk0p1" → "/dev/mmcblk0"), or null. */
    private static String diskOf(String dev) {
        try {
            Path real = Path.of(dev).toRealPath();
            Path sys = Path.of("/sys/class/block", real.getFileName().toString()).toRealPath();
            Path disk = Files.exists(sys.resolve("partition")) ? sys.getParent() : sys;
            return "/dev/" + disk.getFileName();
        } catch (IOException | RuntimeException e) {
            return null;
        }
    }

    /**
     * Safely removes the drive holding {@code roots} (from {@link #sameDrive}): unmounts every one of
     * them (which writes out everything still cached) and, where the system can, powers the drive off.
     * Blocks until done; call it off the FX thread. Throws with the system's explanation when the
     * drive is busy or can't be ejected.
     */
    public static void eject(List<Path> roots) throws IOException {
        Path root = roots.get(0);
        if (OsUtil.isWindows()) {
            ejectWindows(root);
            return;
        }
        List<List<String>> steps;
        if (OsUtil.isMac()) {
            steps = List.of(List.of("diskutil", "eject", root.toString()));
        } else {
            List<String> mounts = Files.readAllLines(Path.of("/proc/self/mounts"));
            List<String> devices = new ArrayList<>();
            for (Path r : roots) {
                String dev = deviceFor(mounts, r);
                if (dev != null) {
                    devices.add(dev);
                }
            }
            String disk = devices.isEmpty() ? null : diskOf(devices.get(0));
            steps = linuxEjectCommands(root, devices, disk, OsUtil::onPath);
        }
        if (steps.isEmpty()) {
            throw new IOException("No tool to eject drives was found (udisksctl, gio or umount).");
        }
        for (List<String> step : steps) {
            try {
                run(step);
            } catch (IOException e) {
                if (!step.contains("power-off")) {
                    throw e;
                }
                // everything is unmounted, so the data is safe; the drive just stays powered
            }
        }
    }

    /**
     * The commands that eject a drive on Linux, in order: udisks (unmount each of its {@code devices},
     * then power off the {@code disk}), else GIO's eject (which removes the whole drive), else a plain
     * unmount of {@code root}. Empty when none of these tools is installed.
     */
    static List<List<String>> linuxEjectCommands(Path root, List<String> devices, String disk, Predicate<String> installed) {
        if (!devices.isEmpty() && devices.stream().allMatch(d -> d.startsWith("/dev/")) && installed.test("udisksctl")) {
            List<List<String>> steps = new ArrayList<>();
            for (String d : devices) {
                steps.add(List.of("udisksctl", "unmount", "--no-user-interaction", "-b", d));
            }
            steps.add(List.of("udisksctl", "power-off", "--no-user-interaction", "-b", disk != null ? disk : devices.get(0)));
            return steps;
        }
        if (installed.test("gio")) {
            return List.of(List.of("gio", "mount", "--eject", root.toString()));
        }
        if (installed.test("umount")) {
            return List.of(List.of("umount", root.toString()));
        }
        return List.of();
    }

    /** The device mounted at {@code root}, from /proc/self/mounts lines, or null. */
    static String deviceFor(List<String> mountLines, Path root) {
        String device = null;
        for (String line : mountLines) {
            String[] f = line.split(" ");
            if (f.length >= 2 && Path.of(unescape(f[1])).equals(root)) {
                device = unescape(f[0]); // the last mount on a path is the visible one
            }
        }
        return device;
    }

    private static void run(List<String> cmd) throws IOException {
        Process p = new ProcessBuilder(cmd).redirectErrorStream(true).start();
        try {
            p.getOutputStream().close();
            byte[] out = p.getInputStream().readAllBytes();
            if (!p.waitFor(EJECT_TIMEOUT_S, TimeUnit.SECONDS)) {
                p.destroyForcibly();
                throw new IOException("The drive did not respond.");
            }
            if (p.exitValue() != 0) {
                throw new IOException(ejectError(new String(out, java.nio.charset.StandardCharsets.UTF_8)));
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IOException("Interrupted");
        }
    }

    /** Turns a tool's error output into a message for people (busy drives are the common case). */
    static String ejectError(String output) {
        String text = output.strip();
        String lower = text.toLowerCase(java.util.Locale.ROOT);
        if (lower.contains("busy") || lower.contains("in use")) {
            return "The drive is in use. Close any files, programs or terminals using it and try again.";
        }
        if (lower.contains("not authorized") || lower.contains("permission") || lower.contains("not permitted")) {
            return "You are not allowed to eject this drive.";
        }
        return text.isEmpty() ? "The drive could not be ejected." : text;
    }

    /** Windows: the Explorer "Eject" verb, then waits until the drive is gone (the verb reports nothing). */
    private static void ejectWindows(Path root) throws IOException {
        String drive = root.toString().replace("\\", "");
        if (!drive.matches("[A-Za-z]:")) {
            throw new IOException("Only drive letters can be ejected.");
        }
        run(List.of("powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                "(New-Object -ComObject Shell.Application).Namespace(17).ParseName('" + drive + "').InvokeVerb('Eject')"));
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(15);
        while (System.nanoTime() < deadline) {
            boolean present = false;
            for (Path r : FileSystems.getDefault().getRootDirectories()) {
                present |= r.equals(root);
            }
            if (!present) {
                return;
            }
            try {
                Thread.sleep(250);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new IOException("Interrupted");
            }
        }
        throw new IOException("Windows could not eject the drive. Close any files or programs using it and try again.");
    }

    private static boolean ignoredMount(String mp) {
        return mp.startsWith("/boot") || mp.startsWith("/efi") || mp.startsWith("/snap") || mp.startsWith("/var/")
                || mp.startsWith("/sys") || mp.startsWith("/proc") || mp.startsWith("/dev")
                || mp.startsWith("/tmp") || (mp.startsWith("/run/") && !mp.startsWith("/run/media/"))
                || mp.startsWith("/usr");
    }

    private static String unescape(String s) {
        Matcher m = OCTAL.matcher(s);
        StringBuilder sb = new StringBuilder();
        while (m.find()) {
            m.appendReplacement(sb, Matcher.quoteReplacement(String.valueOf((char) Integer.parseInt(m.group(1), 8))));
        }
        m.appendTail(sb);
        return sb.toString();
    }

    private static Volume volume(Path root, String label, Kind kind) {
        StoreInfo info = storeInfo(root);
        return new Volume(root, label, kind, info == null ? -1 : info.free(), info == null ? -1 : info.total());
    }

    /** {@code removable}: Windows reports a removable or optical drive (other systems: false). */
    private record StoreInfo(String name, long free, long total, boolean removable) {
    }

    /** How long one poll waits for a volume's size before showing it as unknown. */
    static final long STORE_TIMEOUT_MS = 1500;
    /** Lookups still running, per root: a hung mount gets one stuck lookup, not one per poll. */
    private static final Map<Path, Future<StoreInfo>> LOOKUPS = new ConcurrentHashMap<>();
    private static final ExecutorService LOOKUP = Executors.newCachedThreadPool(r -> {
        Thread t = new Thread(r, "fileem-volume-lookup");
        t.setDaemon(true);
        return t;
    });

    /**
     * Name and space of the volume at {@code root}, or null when unknown. A dead network mount can
     * block these calls for minutes; the poll gives up after {@link #STORE_TIMEOUT_MS} so the other
     * volumes keep updating, and asks again once the stuck lookup has returned.
     */
    private static StoreInfo storeInfo(Path root) {
        Future<StoreInfo> f = LOOKUPS.computeIfAbsent(root, r -> LOOKUP.submit(() -> {
            FileStore store = Files.getFileStore(r);
            boolean removable = OsUtil.isWindows() && (attribute(store, "volume:isRemovable")
                    || attribute(store, "volume:isCdrom"));
            return new StoreInfo(store.name(), store.getUsableSpace(), store.getTotalSpace(), removable);
        }));
        try {
            StoreInfo info = f.get(STORE_TIMEOUT_MS, TimeUnit.MILLISECONDS);
            LOOKUPS.remove(root, f);
            return info;
        } catch (TimeoutException e) {
            return null; // still hanging: keep it, the next poll waits on the same lookup
        } catch (ExecutionException e) {
            LOOKUPS.remove(root, f);
            return null;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return null;
        }
    }
}
