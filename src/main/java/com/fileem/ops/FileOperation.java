package com.fileem.ops;

import com.fileem.fs.Trash;
import com.fileem.util.Formats;
import com.fileem.util.FsUtil;
import com.fileem.util.OsUtil;
import javafx.beans.property.ReadOnlyBooleanProperty;
import javafx.beans.property.ReadOnlyBooleanWrapper;
import javafx.concurrent.Task;

import java.io.BufferedOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.Charset;
import java.nio.charset.StandardCharsets;
import java.nio.channels.FileChannel;
import java.nio.file.AccessDeniedException;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.DirectoryNotEmptyException;
import java.nio.file.DirectoryStream;
import java.nio.file.FileAlreadyExistsException;
import java.nio.file.FileStore;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.NoSuchFileException;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.nio.file.attribute.DosFileAttributes;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Enumeration;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ThreadLocalRandom;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.locks.Condition;
import java.util.concurrent.locks.ReentrantLock;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import java.util.zip.ZipOutputStream;

/**
 * Copy / move / delete / trash of a list of paths, run as a background task.
 * Progress is in "work units": bytes plus one unit per entry.
 *
 * <p>Integrity rules:
 * <ul>
 *   <li>Files are written to a hidden temp file next to the target, size-checked, then renamed into
 *       place, so an existing target is only replaced by a complete copy and a failed or cancelled
 *       copy never leaves a truncated file behind.</li>
 *   <li>A move between file systems fsyncs the copy before the source is deleted, and a source is
 *       only deleted after its copy succeeded.</li>
 *   <li>Symbolic links are copied/deleted as links and never followed; Windows junctions are never
 *       descended into; special files (FIFOs, devices, sockets) are skipped.</li>
 *   <li>Something displaced by "Replace" (e.g. a folder replaced by a file) is moved aside first
 *       and restored if the replacement fails.</li>
 * </ul>
 */
public final class FileOperation extends Task<Void> {
    /** COMPRESS zips all sources into one archive in {@code targetDir}; EXTRACT unpacks one zip into it. */
    public enum Kind { COPY, MOVE, DELETE, TRASH, COMPRESS, EXTRACT }

    public enum Resolution { OVERWRITE, SKIP, KEEP_BOTH, CANCEL }

    public record Decision(Resolution resolution, boolean applyToAll) {
    }

    /** Called on the worker thread when a target already exists; may block for user input. */
    @FunctionalInterface
    public interface ConflictHandler {
        Decision resolve(Path source, Path target) throws InterruptedException;
    }

    private static final LinkOption NOFOLLOW = LinkOption.NOFOLLOW_LINKS;
    /** Files up to this size use Files.copy (fast, keeps all attributes); bigger ones are chunked for progress. */
    static final long FAST_COPY_LIMIT = 8L << 20;
    private static final long CHUNK = 16L << 20;
    private static final int TRASH_BATCH = 100;

    private final Kind kind;
    private final List<Path> sources;
    private final Path targetDir;
    private final ConflictHandler conflicts;
    private final List<String> errors = Collections.synchronizedList(new ArrayList<>());
    private final Map<Path, FileStore> storeCache = new HashMap<>();

    private Resolution conflictForAll;
    /** The current top-level move stays on one file system, so items can simply be renamed. */
    private boolean renameMoves;
    private long total = 1;
    /** The archive or extracted folder created by COMPRESS / EXTRACT. */
    private volatile Path result;
    private Charset zipCharset = StandardCharsets.UTF_8;
    private long done;

    /**
     * Pausing: the worker blocks at its next cancel check until resumed or cancelled. A Lock rather
     * than synchronized/wait, so a waiting worker never pins a carrier if it runs on a virtual thread.
     */
    private final ReentrantLock pauseLock = new ReentrantLock();
    private final Condition unpaused = pauseLock.newCondition();
    /** Opens when call() has returned, i.e. after cleanup of a cancelled or failed run. */
    private final CountDownLatch finished = new CountDownLatch(1);
    private volatile boolean started;
    private final ReadOnlyBooleanWrapper pausedState = new ReadOnlyBooleanWrapper(this, "paused");
    private volatile boolean paused;
    private volatile boolean waitingWhilePaused;
    /** The last message, readable on the worker thread (Task.getMessage is FX-thread only). */
    private String lastMessage = "";

    private static final class Cancelled extends RuntimeException {
        Cancelled() {
            super(null, null, false, false);
        }
    }

    public FileOperation(Kind kind, List<Path> sources, Path targetDir, ConflictHandler conflicts) {
        this.kind = kind;
        this.sources = List.copyOf(sources);
        this.targetDir = targetDir;
        this.conflicts = conflicts;
    }

    public Kind kind() {
        return kind;
    }

    public List<Path> sources() {
        return sources;
    }

    /** What COMPRESS / EXTRACT created (null otherwise, or when it failed). */
    public Path result() {
        return result;
    }

    public Path targetDir() {
        return targetDir;
    }

    /**
     * Pauses the operation: it stops at the next item or chunk (a file under 8 MB being copied is
     * finished first) and waits, holding no locks on the files, until {@link #resume} or cancel.
     * Call on the FX thread (it updates {@link #pausedProperty}).
     */
    public void pause() {
        paused = true;
        pausedState.set(true);
    }

    public void resume() {
        pauseLock.lock();
        try {
            paused = false;
            unpaused.signalAll();
        } finally {
            pauseLock.unlock();
        }
        pausedState.set(false);
    }

    public boolean isPaused() {
        return paused;
    }

    public ReadOnlyBooleanProperty pausedProperty() {
        return pausedState.getReadOnlyProperty();
    }

    /** Whether the worker is currently stopped at a pause point (it may still be finishing a step after pause()). */
    public boolean isWaitingWhilePaused() {
        return waitingWhilePaused;
    }

    @Override
    public boolean cancel(boolean mayInterruptIfRunning) {
        boolean cancelled = super.cancel(mayInterruptIfRunning);
        pauseLock.lock();
        try {
            unpaused.signalAll(); // a paused worker wakes up and stops
        } finally {
            pauseLock.unlock();
        }
        return cancelled;
    }

    /**
     * Waits until the worker has stopped, including the cleanup after a cancel (temp files removed,
     * displaced items restored). Returns false on timeout. An operation that never started counts as
     * stopped once it is cancelled or done.
     */
    public boolean awaitStopped(long timeout, TimeUnit unit) throws InterruptedException {
        if (!started && isDone()) {
            return true;
        }
        return finished.await(timeout, unit);
    }

    /** One line per item that could not be processed. */
    public List<String> errors() {
        synchronized (errors) {
            return List.copyOf(errors);
        }
    }

    @Override
    protected Void call() throws Exception {
        started = true;
        try {
            return work();
        } finally {
            finished.countDown();
        }
    }

    private Void work() throws Exception {
        updateMessage("Preparing…");
        try {
            List<Path> work = new ArrayList<>();
            for (Path src : sources) {
                if (kind != Kind.COPY && src.toAbsolutePath().getParent() == null) {
                    errors.add(src + ": Cannot " + kind.name().toLowerCase(java.util.Locale.ROOT) + " a drive or file system root");
                } else {
                    work.add(src);
                }
            }
            total = Math.max(1, measure(work));
            updateProgress(0, total);
            if (kind == Kind.TRASH) {
                trashAll(work);
            } else if (kind == Kind.COMPRESS) {
                compress(work);
            } else if (kind == Kind.EXTRACT) {
                for (Path zip : work) {
                    checkCancel();
                    extract(zip);
                }
            } else {
                for (Path src : work) {
                    checkCancel();
                    try {
                        switch (kind) {
                            case COPY -> copyTop(src);
                            case MOVE -> moveTop(src);
                            default -> deleteTree(src, true);
                        }
                    } catch (IOException e) {
                        error(src, e);
                    }
                }
            }
            updateProgress(total, total);
        } catch (Cancelled e) {
            cancel(false);
        }
        return null;
    }

    // ---- archives ----

    /** Name for a new archive: "name.zip" for one item (extension dropped for files), else "Archive.zip". */
    static String archiveName(List<Path> items) {
        if (items.size() != 1) {
            return "Archive.zip";
        }
        Path p = items.get(0);
        String name = FsUtil.displayName(p);
        int dot = name.lastIndexOf('.');
        if (dot > 0 && !Files.isDirectory(p, NOFOLLOW)) {
            name = name.substring(0, dot);
        }
        return name + ".zip";
    }

    private void compress(List<Path> work) throws IOException {
        if (work.isEmpty()) {
            return;
        }
        Path out = FsUtil.uniqueName(targetDir.resolve(archiveName(work)), false);
        Path tmp = tempSibling(out);
        boolean placed = false;
        try {
            try (ZipOutputStream zip = new ZipOutputStream(new BufferedOutputStream(
                    Files.newOutputStream(tmp, StandardOpenOption.CREATE_NEW), 1 << 16))) {
                for (Path src : work) {
                    addToZip(zip, src, src.getFileName().toString());
                }
            }
            place(tmp, out, false);
            placed = true;
            result = out;
        } finally {
            if (!placed) {
                deleteQuietly(tmp);
            }
        }
    }

    private void addToZip(ZipOutputStream zip, Path p, String entryName) throws IOException {
        checkCancel();
        BasicFileAttributes a = Files.readAttributes(p, BasicFileAttributes.class, NOFOLLOW);
        if (a.isSymbolicLink() || a.isOther() || (a.isDirectory() && FsUtil.isJunction(p))) {
            errors.add(p + ": Links and special files are not added to archives");
            advance(1);
            return;
        }
        updateMessage(p.toString());
        if (a.isDirectory()) {
            ZipEntry e = new ZipEntry(entryName + "/");
            e.setLastModifiedTime(a.lastModifiedTime());
            zip.putNextEntry(e);
            zip.closeEntry();
            advance(1);
            List<Path> children = new ArrayList<>();
            try (DirectoryStream<Path> ds = Files.newDirectoryStream(p)) {
                ds.forEach(children::add);
            } catch (IOException ex) {
                error(p, ex);
            }
            children.sort(null);
            for (Path child : children) {
                addToZip(zip, child, entryName + "/" + child.getFileName());
            }
            return;
        }
        // open before writing the entry header, so an unreadable file is skipped cleanly;
        // a failure while reading aborts the whole archive (the entry would be truncated)
        InputStream in;
        try {
            in = Files.newInputStream(p);
        } catch (IOException ex) {
            error(p, ex);
            advance(a.size() + 1);
            return;
        }
        try (in) {
            ZipEntry e = new ZipEntry(entryName);
            e.setLastModifiedTime(a.lastModifiedTime());
            zip.putNextEntry(e);
            copyStream(in, zip);
            zip.closeEntry();
        }
        advance(1);
    }

    /** {uncompressed bytes, entries}; also picks the name encoding (UTF-8, else the legacy CP437). */
    private long[] zipSize(Path zipFile) throws IOException {
        for (Charset cs : List.of(StandardCharsets.UTF_8, Charset.forName("IBM437"))) {
            try (ZipFile zf = new ZipFile(zipFile.toFile(), cs)) {
                long bytes = 0, count = 0;
                for (Enumeration<? extends ZipEntry> en = zf.entries(); en.hasMoreElements(); ) {
                    ZipEntry e = en.nextElement();
                    e.getName();
                    bytes += Math.max(0, e.getSize() >= 0 ? e.getSize() : e.getCompressedSize());
                    count++;
                }
                zipCharset = cs;
                return new long[]{bytes, count};
            } catch (IllegalArgumentException malformedNames) {
                // try the next charset
            }
        }
        throw new IOException("Cannot read the file names in this archive.");
    }

    /**
     * Extracts into a hidden temp folder, then renames it into place: "name/" for archives with
     * several top-level entries, or the single top-level item itself. Entries that would land
     * outside the target (zip-slip) are refused.
     */
    private void extract(Path zipFile) throws IOException {
        String base = FsUtil.displayName(zipFile);
        int dot = base.lastIndexOf('.');
        if (dot > 0) {
            base = base.substring(0, dot);
        }
        Path tmp = tempSibling(targetDir.resolve(base));
        Files.createDirectory(tmp);
        boolean placed = false;
        try {
            Path root = tmp.toAbsolutePath().normalize();
            try (ZipFile zf = new ZipFile(zipFile.toFile(), zipCharset)) {
                for (Enumeration<? extends ZipEntry> en = zf.entries(); en.hasMoreElements(); ) {
                    checkCancel();
                    ZipEntry e = en.nextElement();
                    Path dest = root.resolve(e.getName().replace('\\', '/')).normalize();
                    if (!dest.startsWith(root) || dest.equals(root)) {
                        if (!dest.equals(root)) {
                            errors.add(zipFile + ": Skipped unsafe entry \"" + e.getName() + "\"");
                        }
                        advance(1);
                        continue;
                    }
                    updateMessage(e.getName());
                    if (e.isDirectory()) {
                        Files.createDirectories(dest);
                    } else {
                        Files.createDirectories(dest.getParent());
                        try (InputStream in = zf.getInputStream(e);
                             OutputStream out = Files.newOutputStream(dest, StandardOpenOption.CREATE_NEW)) {
                            copyStream(in, out);
                        } catch (FileAlreadyExistsException dup) {
                            errors.add(zipFile + ": Duplicate entry \"" + e.getName() + "\" skipped");
                        }
                        if (e.getLastModifiedTime() != null) {
                            Files.setLastModifiedTime(dest, e.getLastModifiedTime());
                        }
                    }
                    advance(1);
                }
            }
            List<Path> top = new ArrayList<>();
            try (DirectoryStream<Path> ds = Files.newDirectoryStream(tmp)) {
                ds.forEach(top::add);
            }
            Path out;
            if (top.size() == 1) {
                out = FsUtil.uniqueName(targetDir.resolve(top.get(0).getFileName().toString()), false);
                Files.move(top.get(0), out);
                Files.delete(tmp);
            } else {
                out = FsUtil.uniqueName(targetDir.resolve(base), false);
                Files.move(tmp, out);
            }
            placed = true;
            result = out;
        } finally {
            if (!placed) {
                deleteQuietly(tmp);
            }
        }
    }

    private void copyStream(InputStream in, OutputStream out) throws IOException {
        byte[] buf = new byte[1 << 16];
        int n;
        while ((n = in.read(buf)) > 0) {
            checkCancel();
            out.write(buf, 0, n);
            advance(n);
        }
    }

    // ---- measuring ----

    private long measure(List<Path> work) throws IOException {
        long units = 0;
        long newBytes = 0;
        for (Path src : work) {
            checkCancel();
            switch (kind) {
                case TRASH -> units += 1;
                case COMPRESS -> {
                    long[] size = treeSize(src);
                    units += size[0] + size[1];
                }
                case EXTRACT -> {
                    long[] size = zipSize(src);
                    units += size[0] + size[1];
                    newBytes += size[0];
                }
                case DELETE -> units += treeSize(src)[1];
                case COPY, MOVE -> {
                    if (kind == Kind.MOVE && isRename(src)) {
                        units += 1;
                    } else {
                        long[] size = treeSize(src);
                        units += size[0] + size[1];
                        if (!Files.exists(targetDir.resolve(src.getFileName().toString()), NOFOLLOW)) {
                            newBytes += size[0];
                        }
                    }
                }
            }
        }
        if (newBytes > 0) {
            long usable = store(targetDir) != null ? store(targetDir).getUsableSpace() : Long.MAX_VALUE;
            if (newBytes > usable) {
                throw new IOException("Not enough free space on the destination: " + Formats.size(newBytes)
                        + " needed, " + Formats.size(usable) + " available.");
            }
        }
        return units;
    }

    private boolean isRename(Path src) {
        return sameStore(src) && !Files.exists(targetDir.resolve(src.getFileName().toString()), NOFOLLOW);
    }

    /** {bytes, entries} below {@code root}, not following links or junctions. */
    private long[] treeSize(Path root) {
        long[] sum = {0, 0};
        try {
            Files.walkFileTree(root, new SimpleFileVisitor<>() {
                @Override
                public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) {
                    checkCancel();
                    sum[1]++;
                    return FsUtil.isJunction(dir) ? FileVisitResult.SKIP_SUBTREE : FileVisitResult.CONTINUE;
                }

                @Override
                public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) {
                    long bytes = attrs.isRegularFile() ? attrs.size() : 0;
                    sum[0] += bytes;
                    sum[1]++;
                    counted(1, bytes);
                    return FileVisitResult.CONTINUE;
                }

                @Override
                public FileVisitResult visitFileFailed(Path file, IOException exc) {
                    return FileVisitResult.CONTINUE;
                }
            });
        } catch (IOException ignored) {
            // progress is only an estimate
        }
        return sum;
    }

    private long countedItems;
    private long countedBytes;
    private long lastCountReport;

    /** Live feedback while measuring, so a big job doesn't look stuck before copying starts. */
    private void counted(long items, long bytes) {
        countedItems += items;
        countedBytes += bytes;
        long now = System.nanoTime();
        if (now - lastCountReport > 200_000_000L) {
            lastCountReport = now;
            updateMessage("Counting… " + Formats.count(countedItems) + " items, " + Formats.size(countedBytes));
        }
    }

    private FileStore store(Path p) {
        Path key = p.toAbsolutePath();
        if (!storeCache.containsKey(key)) {
            FileStore fs;
            try {
                fs = Files.getFileStore(key);
            } catch (IOException | RuntimeException e) {
                fs = null;
            }
            storeCache.put(key, fs);
        }
        return storeCache.get(key);
    }

    private boolean sameStore(Path src) {
        Path parent = src.toAbsolutePath().getParent();
        FileStore a = parent != null ? store(parent) : null;
        FileStore b = store(targetDir);
        return a != null && a.equals(b);
    }

    // ---- copy / move ----

    private void copyTop(Path src) throws IOException {
        Path dest = targetDir.resolve(src.getFileName().toString());
        if (dest.equals(src)) {
            dest = FsUtil.uniqueName(src, true);
        } else if (Files.isDirectory(src, NOFOLLOW) && FsUtil.isSameOrInside(targetDir, src)) {
            throw new IOException("Cannot copy a folder into itself.");
        }
        renameMoves = false;
        transfer(src, dest, false);
    }

    private void moveTop(Path src) throws IOException {
        Path dest = targetDir.resolve(src.getFileName().toString());
        if (dest.equals(src)) {
            advance(1);
            return;
        }
        if (Files.isDirectory(src, NOFOLLOW) && FsUtil.isSameOrInside(targetDir, src)) {
            throw new IOException("Cannot move a folder into itself.");
        }
        renameMoves = sameStore(src);
        transfer(src, dest, true);
    }

    /** Copies (or moves, when {@code move}) {@code src} to {@code dst}, merging folders and asking on conflicts. */
    private void transfer(Path src, Path dst, boolean move) throws IOException {
        checkCancel();
        BasicFileAttributes a = Files.readAttributes(src, BasicFileAttributes.class, NOFOLLOW);
        boolean exists = Files.exists(dst, NOFOLLOW);

        if (exists && Files.isSameFile(src, dst)) {
            if (move) {
                throw new IOException("Source and destination are the same item.");
            }
            dst = FsUtil.uniqueName(dst, true);
            exists = false;
        }

        if (move && renameMoves && !exists) {
            updateMessage(src.toString());
            if (tryRename(src, dst, false)) {
                advance(a.isDirectory() ? 1 : a.size() + 1);
                return;
            }
            exists = Files.exists(dst, NOFOLLOW);
        }

        boolean junction = (a.isDirectory() || a.isOther()) && FsUtil.isJunction(src);
        if (junction) {
            throw new IOException("Junctions are not copied (move it within the same drive instead).");
        }
        if (a.isOther()) {
            throw new IOException("Special file skipped (pipe, socket or device).");
        }

        if (a.isDirectory()) {
            transferDirectory(src, dst, a, exists, move);
        } else {
            transferFile(src, dst, a, exists, move);
        }
    }

    private void transferDirectory(Path src, Path dst, BasicFileAttributes a, boolean exists, boolean move)
            throws IOException {
        Path displaced = null;
        if (exists && !Files.isDirectory(dst, NOFOLLOW)) {
            Resolution r = resolve(src, dst);
            if (r == Resolution.SKIP) {
                return;
            }
            if (r == Resolution.KEEP_BOTH) {
                dst = FsUtil.uniqueName(dst, false);
            } else {
                displaced = displace(dst);
            }
            exists = false;
        }
        if (!exists) {
            try {
                Files.createDirectory(dst);
            } catch (IOException e) {
                restore(displaced, dst);
                throw e;
            }
        }
        if (displaced != null) {
            deleteQuietly(displaced);
        }
        advance(1);
        try (DirectoryStream<Path> children = Files.newDirectoryStream(src)) {
            for (Path child : children) {
                try {
                    transfer(child, dst.resolve(child.getFileName().toString()), move);
                } catch (IOException e) {
                    error(child, e);
                }
            }
        }
        try {
            Files.setLastModifiedTime(dst, a.lastModifiedTime());
        } catch (IOException ignored) {
            // not important
        }
        if (move) {
            try {
                Files.delete(src);
            } catch (DirectoryNotEmptyException ignored) {
                // something inside was skipped or failed: keep the source folder
            }
        }
    }

    private void transferFile(Path src, Path dst, BasicFileAttributes a, boolean exists, boolean move)
            throws IOException {
        if (exists) {
            switch (resolve(src, dst)) {
                case SKIP -> {
                    advance(a.size() + 1);
                    return;
                }
                case KEEP_BOTH -> {
                    dst = FsUtil.uniqueName(dst, false);
                    exists = false;
                }
                default -> {
                    // replace, below
                }
            }
        }

        if (move && renameMoves) {
            updateMessage(src.toString());
            Path displaced = exists && Files.isDirectory(dst, NOFOLLOW) ? displace(dst) : null;
            if (tryRename(src, dst, exists)) {
                if (displaced != null) {
                    deleteQuietly(displaced);
                }
                advance(a.size() + 1);
                return;
            }
            restore(displaced, dst);
        }

        if (a.isSymbolicLink()) {
            copySymlink(src, dst, exists);
        } else {
            copyFile(src, dst, a, exists, move);
        }
        if (move) {
            Files.delete(src);
        }
    }

    /**
     * Renames within one file system. Returns false when the rename is impossible and copying should
     * be used instead; errors that copying would not fix (access denied, missing source) are thrown.
     */
    private boolean tryRename(Path src, Path dst, boolean replace) throws IOException {
        try {
            if (replace) {
                Files.move(src, dst, StandardCopyOption.REPLACE_EXISTING);
            } else {
                Files.move(src, dst);
            }
            return true;
        } catch (AccessDeniedException | NoSuchFileException e) {
            throw e;
        } catch (FileAlreadyExistsException e) {
            return false;
        } catch (IOException e) {
            return false;
        }
    }

    private void copySymlink(Path src, Path dst, boolean replace) throws IOException {
        Path tmp = tempSibling(dst);
        Files.copy(src, tmp, NOFOLLOW, StandardCopyOption.COPY_ATTRIBUTES);
        try {
            place(tmp, dst, replace);
        } catch (IOException e) {
            deleteQuietly(tmp);
            throw e;
        }
        advance(1);
    }

    private void copyFile(Path src, Path dst, BasicFileAttributes a, boolean replace, boolean durable)
            throws IOException {
        updateMessage(src.toString());
        Path tmp = tempSibling(dst);
        boolean placed = false;
        try {
            if (a.size() <= FAST_COPY_LIMIT) {
                Files.copy(src, tmp, StandardCopyOption.COPY_ATTRIBUTES);
                advance(a.size());
                if (durable) {
                    force(tmp);
                }
            } else {
                chunkedCopy(src, tmp, durable);
                copyMetadata(src, tmp, a);
            }
            long copied = Files.size(tmp);
            if (copied != a.size()) {
                throw new IOException("Copy is incomplete (" + Formats.exact(copied) + " of "
                        + Formats.exact(a.size()) + "); the source may have changed during the copy.");
            }
            Path displaced = replace && Files.isDirectory(dst, NOFOLLOW) ? displace(dst) : null;
            try {
                place(tmp, dst, replace);
            } catch (IOException e) {
                restore(displaced, dst);
                throw e;
            }
            placed = true;
            if (displaced != null) {
                deleteQuietly(displaced);
            }
        } finally {
            if (!placed) {
                deleteQuietly(tmp);
            }
        }
        advance(1);
    }

    private void chunkedCopy(Path src, Path tmp, boolean durable) throws IOException {
        try (FileChannel in = FileChannel.open(src, StandardOpenOption.READ);
             FileChannel out = FileChannel.open(tmp, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE)) {
            long size = in.size();
            long pos = 0;
            while (pos < size) {
                checkCancel();
                long n = in.transferTo(pos, Math.min(CHUNK, size - pos), out);
                if (n <= 0) {
                    break; // source shrank; the size check reports it
                }
                pos += n;
                advance(n);
            }
            if (durable) {
                out.force(true);
            }
        }
    }

    private static void copyMetadata(Path src, Path dst, BasicFileAttributes a) {
        try {
            Files.setLastModifiedTime(dst, a.lastModifiedTime());
            if (OsUtil.isPosix()) {
                Files.setPosixFilePermissions(dst, Files.getPosixFilePermissions(src, NOFOLLOW));
            } else if (OsUtil.isWindows()) {
                DosFileAttributes dos = Files.readAttributes(src, DosFileAttributes.class, NOFOLLOW);
                Files.setAttribute(dst, "dos:hidden", dos.isHidden(), NOFOLLOW);
                Files.setAttribute(dst, "dos:readonly", dos.isReadOnly(), NOFOLLOW);
            }
        } catch (IOException | RuntimeException ignored) {
            // the target file system may not support it
        }
    }

    /** Flushes a finished copy to disk (used before deleting the source of a cross-device move). */
    private static void force(Path p) throws IOException {
        if (OsUtil.isWindows()) {
            try (FileChannel ch = FileChannel.open(p, StandardOpenOption.WRITE)) {
                ch.force(true);
            } catch (AccessDeniedException ignored) {
                // read-only copy: cannot open for flushing on Windows
            }
        } else {
            try (FileChannel ch = FileChannel.open(p, StandardOpenOption.READ)) {
                ch.force(true);
            }
        }
    }

    /** Moves a finished temp file to its final name, atomically replacing the target when allowed. */
    private static void place(Path tmp, Path dst, boolean replace) throws IOException {
        if (replace) {
            try {
                Files.move(tmp, dst, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
            } catch (AtomicMoveNotSupportedException e) {
                Files.move(tmp, dst, StandardCopyOption.REPLACE_EXISTING);
            }
        } else {
            Files.move(tmp, dst);
        }
    }

    /** Renames {@code p} to a hidden sibling so it can be restored if replacing it fails. */
    private static Path displace(Path p) throws IOException {
        Path aside = tempSibling(p);
        Files.move(p, aside);
        return aside;
    }

    private static void restore(Path displaced, Path original) {
        if (displaced != null) {
            try {
                Files.move(displaced, original);
            } catch (IOException ignored) {
                // leave it under the temp name rather than lose it
            }
        }
    }

    static Path tempSibling(Path target) {
        return target.resolveSibling(".fileem-" + Long.toHexString(ThreadLocalRandom.current().nextLong()) + ".part");
    }

    private void deleteQuietly(Path p) {
        try {
            if (Files.isDirectory(p, NOFOLLOW)) {
                deleteTree(p, false);
            } else {
                Files.deleteIfExists(p);
            }
        } catch (IOException ignored) {
            // best effort cleanup
        }
    }

    private Resolution resolve(Path src, Path dst) {
        if (conflictForAll != null) {
            return conflictForAll;
        }
        Decision d;
        try {
            d = conflicts.resolve(src, dst);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new Cancelled();
        }
        if (d == null || d.resolution() == Resolution.CANCEL) {
            throw new Cancelled();
        }
        if (d.applyToAll()) {
            conflictForAll = d.resolution();
        }
        return d.resolution();
    }

    // ---- trash ----

    private void trashAll(List<Path> work) {
        List<Path> present = new ArrayList<>();
        for (Path p : work) {
            if (Files.exists(p, NOFOLLOW)) {
                present.add(p);
            } else {
                errors.add(p + ": Not found");
                advance(1);
            }
        }
        for (int i = 0; i < present.size(); i += TRASH_BATCH) {
            checkCancel();
            List<Path> batch = present.subList(i, Math.min(present.size(), i + TRASH_BATCH));
            updateMessage(batch.get(0).toString());
            if (batch.size() > 1 && Trash.moveAllToTrash(batch)) {
                advance(batch.size());
                continue;
            }
            // single item, or the batch failed part-way: items still present are retried one by one
            // so each failure gets its own error message
            for (Path p : batch) {
                checkCancel();
                if (batch.size() == 1 || Files.exists(p, NOFOLLOW)) {
                    updateMessage(p.toString());
                    try {
                        Trash.moveToTrash(p);
                    } catch (IOException e) {
                        error(p, e);
                    }
                }
                advance(1);
            }
        }
    }

    // ---- delete ----

    private void deleteTree(Path root, boolean report) throws IOException {
        Files.walkFileTree(root, new SimpleFileVisitor<>() {
            @Override
            public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) {
                checkCancel();
                if (FsUtil.isJunction(dir)) {
                    // remove the junction itself, never what it points to
                    try {
                        deleteOne(dir);
                    } catch (IOException e) {
                        error(dir, e);
                    }
                    if (report) {
                        advance(1);
                    }
                    return FileVisitResult.SKIP_SUBTREE;
                }
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) {
                checkCancel();
                if (report) {
                    updateMessage(file.toString());
                }
                try {
                    deleteOne(file);
                } catch (IOException e) {
                    error(file, e);
                }
                if (report) {
                    advance(1);
                }
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult visitFileFailed(Path file, IOException exc) {
                error(file, exc);
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult postVisitDirectory(Path dir, IOException exc) {
                checkCancel();
                try {
                    deleteOne(dir);
                } catch (DirectoryNotEmptyException e) {
                    // a child failed and was reported already
                } catch (IOException e) {
                    error(dir, e);
                }
                if (report) {
                    advance(1);
                }
                return FileVisitResult.CONTINUE;
            }
        });
    }

    private static void deleteOne(Path p) throws IOException {
        try {
            Files.delete(p);
        } catch (AccessDeniedException e) {
            if (!OsUtil.isWindows()) {
                throw e;
            }
            // read-only files cannot be deleted on Windows
            Files.setAttribute(p, "dos:readonly", false, NOFOLLOW);
            Files.delete(p);
        }
    }

    // ---- helpers ----

    private void advance(long units) {
        done += units;
        updateProgress(Math.min(done, total), total);
    }

    /** The worker's checkpoint between items and chunks: stops when cancelled, waits while paused. */
    private void checkCancel() {
        if (isCancelled()) {
            throw new Cancelled();
        }
        if (paused) {
            waitWhilePaused();
        }
    }

    @Override
    protected void updateMessage(String message) {
        lastMessage = message;
        super.updateMessage(message);
    }

    private void waitWhilePaused() {
        String message = lastMessage;
        updateMessage("Paused");
        pauseLock.lock();
        try {
            waitingWhilePaused = true;
            while (paused && !isCancelled()) {
                unpaused.await();
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new Cancelled();
        } finally {
            waitingWhilePaused = false;
            pauseLock.unlock();
        }
        if (isCancelled()) {
            throw new Cancelled();
        }
        updateMessage(message);
    }

    private void error(Path p, IOException e) {
        String reason;
        if (e instanceof AccessDeniedException) {
            reason = "Access denied";
        } else if (e instanceof NoSuchFileException) {
            reason = "Not found";
        } else if (e instanceof DirectoryNotEmptyException) {
            reason = "Folder is not empty";
        } else if (e instanceof FileAlreadyExistsException) {
            reason = "Already exists";
        } else {
            reason = e.getMessage() != null ? e.getMessage() : e.getClass().getSimpleName();
        }
        errors.add(p + ": " + reason);
    }
}
