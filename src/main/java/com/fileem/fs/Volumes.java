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
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Detects drives / mount points and keeps the list up to date by polling. */
public final class Volumes {
    public enum Kind { SYSTEM, LOCAL, REMOVABLE, NETWORK }

    public record Volume(Path root, String label, Kind kind, long free, long total) {
    }

    private static final Set<String> NETWORK_TYPES = Set.of(
            "nfs", "nfs4", "cifs", "smb3", "smbfs", "fuse.sshfs", "sshfs", "afpfs", "webdav", "davfs", "fuse.rclone");
    private static final Pattern OCTAL = Pattern.compile("\\\\([0-7]{3})");
    private static final ReadOnlyObjectWrapper<List<Volume>> CURRENT = new ReadOnlyObjectWrapper<>(List.of());
    private static boolean started;

    private Volumes() {
    }

    /** The current volume list (FX thread). Starts the background monitor on first use. */
    public static ReadOnlyObjectProperty<List<Volume>> current() {
        if (!started) {
            started = true;
            ScheduledExecutorService ses = Executors.newSingleThreadScheduledExecutor(r -> {
                Thread t = new Thread(r, "fileem-volumes");
                t.setDaemon(true);
                return t;
            });
            ses.scheduleWithFixedDelay(() -> {
                List<Volume> list = list();
                Platform.runLater(() -> {
                    if (!list.equals(CURRENT.get())) {
                        CURRENT.set(list);
                    }
                });
            }, 0, 5, TimeUnit.SECONDS);
        }
        return CURRENT.getReadOnlyProperty();
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
            Kind kind = letter.equalsIgnoreCase("C:") ? Kind.SYSTEM : Kind.LOCAL;
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
                Kind kind = network ? Kind.NETWORK
                        : mp.startsWith("/media/") || mp.startsWith("/run/media/") || mp.startsWith("/mnt/") ? Kind.REMOVABLE
                        : Kind.LOCAL;
                mounts.putIfAbsent(mp, kind);
            }
        } catch (IOException ignored) {
            // no /proc: root only
        }
        List<Volume> out = new ArrayList<>();
        mounts.forEach((mp, kind) -> {
            Path p = Path.of(mp);
            out.add(volume(p, mp.equals("/") ? "File System" : p.getFileName().toString(), kind));
        });
        return out;
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

    private record StoreInfo(String name, long free, long total) {
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
            return new StoreInfo(store.name(), store.getUsableSpace(), store.getTotalSpace());
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
