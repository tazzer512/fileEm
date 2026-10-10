package com.fileem.ops;

import com.fileem.FxTestSupport;
import com.fileem.ops.FileOperation.ConflictHandler;
import com.fileem.ops.FileOperation.Decision;
import com.fileem.ops.FileOperation.Kind;
import com.fileem.ops.FileOperation.Resolution;
import com.fileem.util.OsUtil;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.attribute.FileTime;
import java.nio.file.attribute.PosixFilePermission;
import java.nio.file.attribute.PosixFilePermissions;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Random;
import java.util.Set;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.stream.Collectors;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTimeoutPreemptively;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

class FileOperationTest {
    private static final ConflictHandler FAIL_ON_CONFLICT = (s, t) -> {
        throw new AssertionError("unexpected conflict " + s + " -> " + t);
    };

    @TempDir
    Path base;
    Path src;
    Path dst;

    @BeforeAll
    static void fx() {
        FxTestSupport.startToolkit();
    }

    @BeforeEach
    void setUp() throws IOException {
        src = Files.createDirectory(base.resolve("src"));
        dst = Files.createDirectory(base.resolve("dst"));
    }

    // ---- helpers ----

    private static FileOperation run(Kind kind, List<Path> sources, Path target, ConflictHandler handler) {
        FileOperation op = new FileOperation(kind, sources, target, handler);
        op.run();
        return op;
    }

    /** Task properties may only be read on the FX thread, after its queued updates ran. */
    private static <T> T onFx(java.util.concurrent.Callable<T> read) throws Exception {
        FxTestSupport.drainFx();
        return org.testfx.util.WaitForAsyncUtils.asyncFx(read).get();
    }

    private static void assertProgressComplete(FileOperation op) throws Exception {
        double[] work = onFx(() -> new double[]{op.getWorkDone(), op.getTotalWork()});
        assertEquals(work[1], work[0], 0.0, "progress completes");
    }

    private static ConflictHandler always(Resolution r) {
        return (s, t) -> new Decision(r, false);
    }

    private static byte[] randomBytes(int size, long seed) {
        byte[] b = new byte[size];
        new Random(seed).nextBytes(b);
        return b;
    }

    private static String tree(Path root) throws IOException {
        try (Stream<Path> s = Files.walk(root)) {
            return s.filter(p -> !p.equals(root)).map(p -> root.relativize(p).toString().replace('\\', '/'))
                    .sorted().collect(Collectors.joining(","));
        }
    }

    private void assertNoTempFiles() throws IOException {
        try (Stream<Path> s = Files.walk(base)) {
            List<Path> leftovers = s.filter(p -> p.getFileName().toString().startsWith(".fileem-")).toList();
            assertTrue(leftovers.isEmpty(), "temp files left behind: " + leftovers);
        }
    }

    private Path bigFile(Path p, long seed) throws IOException {
        // larger than the fast-copy limit, so the chunked (cancellable, fsync-able) path is used
        Files.write(p, randomBytes((int) FileOperation.FAST_COPY_LIMIT + 3 * 1024 * 1024 + 17, seed));
        return p;
    }

    // ---- copy ----

    @Test
    void copiesTreesByteForByteAndKeepsModificationTimes() throws Exception {
        Files.createDirectories(src.resolve("sub/deep"));
        byte[] small = randomBytes(4096, 1);
        Files.write(src.resolve("a.bin"), small);
        Files.writeString(src.resolve("sub/b.txt"), "B");
        bigFile(src.resolve("sub/deep/big.bin"), 2);
        Files.createDirectory(src.resolve("empty"));
        FileTime time = FileTime.fromMillis(1_600_000_000_000L);
        Files.setLastModifiedTime(src.resolve("a.bin"), time);
        Files.setLastModifiedTime(src.resolve("sub/deep/big.bin"), time);

        FileOperation op = run(Kind.COPY, List.of(src.resolve("a.bin"), src.resolve("sub"), src.resolve("empty")),
                dst, FAIL_ON_CONFLICT);

        assertTrue(op.errors().isEmpty(), op.errors().toString());
        assertEquals("a.bin,empty,sub,sub/b.txt,sub/deep,sub/deep/big.bin", tree(dst));
        assertArrayEquals(small, Files.readAllBytes(dst.resolve("a.bin")));
        assertArrayEquals(Files.readAllBytes(src.resolve("sub/deep/big.bin")),
                Files.readAllBytes(dst.resolve("sub/deep/big.bin")));
        assertEquals(time, Files.getLastModifiedTime(dst.resolve("a.bin")));
        assertEquals(time, Files.getLastModifiedTime(dst.resolve("sub/deep/big.bin")));
        assertTrue(Files.exists(src.resolve("sub/deep/big.bin")), "copy keeps the source");
        assertProgressComplete(op);
        assertNoTempFiles();
    }

    @Test
    void copyKeepsPosixPermissionsOnBothCopyPaths() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Set<PosixFilePermission> exec = PosixFilePermissions.fromString("rwxr-x---");
        Path small = Files.writeString(src.resolve("run.sh"), "#!/bin/sh");
        Path big = bigFile(src.resolve("big.run"), 3);
        Files.setPosixFilePermissions(small, exec);
        Files.setPosixFilePermissions(big, exec);
        run(Kind.COPY, List.of(small, big), dst, FAIL_ON_CONFLICT);
        assertEquals(exec, Files.getPosixFilePermissions(dst.resolve("run.sh")));
        assertEquals(exec, Files.getPosixFilePermissions(dst.resolve("big.run")));
    }

    @Test
    void copyIntoSameFolderMakesACopyName() throws Exception {
        Files.writeString(src.resolve("a.txt"), "A");
        run(Kind.COPY, List.of(src.resolve("a.txt")), src, FAIL_ON_CONFLICT);
        run(Kind.COPY, List.of(src.resolve("a.txt")), src, FAIL_ON_CONFLICT);
        assertEquals("a - Copy (2).txt,a - Copy.txt,a.txt", tree(src));
    }

    @Test
    void copyFolderIntoItselfIsRefused() throws Exception {
        Files.createDirectories(src.resolve("sub/deep"));
        FileOperation op = run(Kind.COPY, List.of(src.resolve("sub")), src.resolve("sub/deep"), FAIL_ON_CONFLICT);
        assertEquals(1, op.errors().size());
        assertEquals("sub,sub/deep", tree(src));
    }

    @Test
    void copyingOrMovingAFolderIntoItselfThroughALinkIsRefused() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Files.createDirectories(src.resolve("sub/deep"));
        Files.writeString(src.resolve("sub/file.txt"), "f");
        Path viaLink = Files.createSymbolicLink(dst.resolve("link"), src.resolve("sub"));
        String before = tree(src);
        for (Kind kind : List.of(Kind.COPY, Kind.MOVE)) {
            FileOperation op = run(kind, List.of(src.resolve("sub")), viaLink.resolve("deep"), FAIL_ON_CONFLICT);
            assertEquals(1, op.errors().size(), kind + ": " + op.errors());
            assertTrue(op.errors().get(0).contains("into itself"), op.errors().toString());
            assertEquals(before, tree(src), kind + " left everything as it was");
        }
        assertNoTempFiles();
    }

    // ---- conflicts ----

    @Test
    void conflictResolutions() throws Exception {
        Files.writeString(src.resolve("a.txt"), "new");
        Files.writeString(dst.resolve("a.txt"), "old");

        run(Kind.COPY, List.of(src.resolve("a.txt")), dst, always(Resolution.SKIP));
        assertEquals("old", Files.readString(dst.resolve("a.txt")));

        run(Kind.COPY, List.of(src.resolve("a.txt")), dst, always(Resolution.KEEP_BOTH));
        assertEquals("new", Files.readString(dst.resolve("a (2).txt")));
        assertEquals("old", Files.readString(dst.resolve("a.txt")));

        run(Kind.COPY, List.of(src.resolve("a.txt")), dst, always(Resolution.OVERWRITE));
        assertEquals("new", Files.readString(dst.resolve("a.txt")));

        Files.writeString(dst.resolve("a.txt"), "old");
        FileOperation cancelled = run(Kind.COPY, List.of(src.resolve("a.txt")), dst, always(Resolution.CANCEL));
        assertTrue(cancelled.isCancelled());
        assertEquals("old", Files.readString(dst.resolve("a.txt")));
        assertNoTempFiles();
    }

    @Test
    void applyToAllAsksOnlyOnce() throws Exception {
        for (String n : List.of("1", "2", "3")) {
            Files.writeString(src.resolve(n), "new" + n);
            Files.writeString(dst.resolve(n), "old");
        }
        AtomicInteger asked = new AtomicInteger();
        run(Kind.COPY, List.of(src.resolve("1"), src.resolve("2"), src.resolve("3")), dst, (s, t) -> {
            asked.incrementAndGet();
            return new Decision(Resolution.OVERWRITE, true);
        });
        assertEquals(1, asked.get());
        assertEquals("new3", Files.readString(dst.resolve("3")));
    }

    @Test
    void foldersMergeAndOnlyConflictingFilesAsk() throws Exception {
        Files.createDirectories(src.resolve("d"));
        Files.createDirectories(dst.resolve("d"));
        Files.writeString(src.resolve("d/new.txt"), "n");
        Files.writeString(src.resolve("d/same.txt"), "src");
        Files.writeString(dst.resolve("d/same.txt"), "dst");
        Files.writeString(dst.resolve("d/keep.txt"), "k");
        List<Path> asked = new ArrayList<>();
        run(Kind.COPY, List.of(src.resolve("d")), dst, (s, t) -> {
            asked.add(t);
            return new Decision(Resolution.SKIP, false);
        });
        assertEquals(List.of(dst.resolve("d/same.txt")), asked);
        assertEquals("d,d/keep.txt,d/new.txt,d/same.txt", tree(dst));
        assertEquals("dst", Files.readString(dst.resolve("d/same.txt")));
    }

    @Test
    void replacingAFolderWithAFileAndBack() throws Exception {
        Files.writeString(src.resolve("x"), "file");
        Files.createDirectories(dst.resolve("x/inner"));
        run(Kind.COPY, List.of(src.resolve("x")), dst, always(Resolution.OVERWRITE));
        assertEquals("file", Files.readString(dst.resolve("x")));

        Path srcDir = Files.createDirectories(base.resolve("other/y/inner"));
        Files.writeString(dst.resolve("y"), "file");
        run(Kind.COPY, List.of(srcDir.getParent()), dst, always(Resolution.OVERWRITE));
        assertTrue(Files.isDirectory(dst.resolve("y/inner")));
        assertNoTempFiles();
    }

    // ---- integrity on failure / cancel ----

    @Test
    void cancellingDuringAnOverwriteKeepsTheOriginalIntact() throws Exception {
        bigFile(src.resolve("big.bin"), 4);
        byte[] original = randomBytes(1000, 5);
        Files.write(dst.resolve("big.bin"), original);
        FileOperation[] holder = new FileOperation[1];
        holder[0] = new FileOperation(Kind.COPY, List.of(src.resolve("big.bin")), dst, (s, t) -> {
            holder[0].cancel(); // cancel right as the replacement copy starts
            return new Decision(Resolution.OVERWRITE, false);
        });
        holder[0].run();
        assertTrue(holder[0].isCancelled());
        assertArrayEquals(original, Files.readAllBytes(dst.resolve("big.bin")));
        assertNoTempFiles();
    }

    @Test
    void failedOverwriteKeepsTheOriginalIntact() throws Exception {
        assumeTrue(OsUtil.isPosix() && !"root".equals(System.getProperty("user.name")));
        Path unreadable = Files.writeString(src.resolve("a.txt"), "secret");
        Files.setPosixFilePermissions(unreadable, PosixFilePermissions.fromString("-w-------"));
        Files.writeString(dst.resolve("a.txt"), "original");
        try {
            FileOperation op = run(Kind.COPY, List.of(unreadable), dst, always(Resolution.OVERWRITE));
            assertEquals(1, op.errors().size());
            assertEquals("original", Files.readString(dst.resolve("a.txt")));
            assertNoTempFiles();
        } finally {
            Files.setPosixFilePermissions(unreadable, PosixFilePermissions.fromString("rw-------"));
        }
    }

    @Test
    void specialFilesAreSkippedInsteadOfHanging() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Path fifo = src.resolve("pipe");
        Process p;
        try {
            p = new ProcessBuilder("mkfifo", fifo.toString()).start();
        } catch (IOException e) {
            assumeTrue(false, "mkfifo not available");
            return;
        }
        assumeTrue(p.waitFor() == 0);
        Files.writeString(src.resolve("ok.txt"), "x");
        FileOperation op = assertTimeoutPreemptively(Duration.ofSeconds(20),
                () -> run(Kind.COPY, List.of(src), dst, FAIL_ON_CONFLICT));
        assertEquals(1, op.errors().size(), op.errors().toString());
        assertTrue(Files.exists(dst.resolve("src/ok.txt")));
    }

    // ---- links ----

    @Test
    void symlinksAreCopiedAsLinks() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Files.writeString(src.resolve("t.txt"), "t");
        Files.createSymbolicLink(src.resolve("rel-link"), Path.of("t.txt"));
        Files.createSymbolicLink(src.resolve("dir-link"), base);
        run(Kind.COPY, List.of(src.resolve("rel-link"), src.resolve("dir-link")), dst, FAIL_ON_CONFLICT);
        assertTrue(Files.isSymbolicLink(dst.resolve("rel-link")));
        assertEquals(Path.of("t.txt"), Files.readSymbolicLink(dst.resolve("rel-link")));
        assertTrue(Files.isSymbolicLink(dst.resolve("dir-link")), "linked folder is not copied recursively");
    }

    @Test
    void deletingAFolderNeverFollowsLinksOutOfIt() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Path outside = Files.createDirectory(base.resolve("outside"));
        Files.writeString(outside.resolve("precious.txt"), "keep me");
        Files.createSymbolicLink(src.resolve("link-to-outside"), outside);
        Files.createSymbolicLink(src.resolve("file-link"), outside.resolve("precious.txt"));

        FileOperation op = run(Kind.DELETE, List.of(src), null, FAIL_ON_CONFLICT);
        assertTrue(op.errors().isEmpty(), op.errors().toString());
        assertFalse(Files.exists(src, LinkOption.NOFOLLOW_LINKS));
        assertEquals("keep me", Files.readString(outside.resolve("precious.txt")));
    }

    @Test
    void deletingALinkItselfKeepsItsTarget() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Path outside = Files.createDirectory(base.resolve("outside"));
        Files.writeString(outside.resolve("f"), "x");
        Path link = Files.createSymbolicLink(src.resolve("l"), outside);
        run(Kind.DELETE, List.of(link), null, FAIL_ON_CONFLICT);
        assertFalse(Files.exists(link, LinkOption.NOFOLLOW_LINKS));
        assertTrue(Files.exists(outside.resolve("f")));
    }

    // ---- move ----

    @Test
    void moveOnTheSameFileSystemRenamesWithoutCopying() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Path d = Files.createDirectories(src.resolve("d"));
        bigFile(d.resolve("big.bin"), 6);
        Object inode = Files.getAttribute(d.resolve("big.bin"), "unix:ino");
        FileOperation op = run(Kind.MOVE, List.of(d), dst, FAIL_ON_CONFLICT);
        assertTrue(op.errors().isEmpty(), op.errors().toString());
        assertFalse(Files.exists(d));
        assertEquals(inode, Files.getAttribute(dst.resolve("d/big.bin"), "unix:ino"), "same inode: renamed, not copied");
    }

    @Test
    void moveMergesAndKeepsSourcesThatWereSkipped() throws Exception {
        Files.createDirectories(src.resolve("d"));
        Files.createDirectories(dst.resolve("d"));
        Files.writeString(src.resolve("d/new.txt"), "n");
        Files.writeString(src.resolve("d/conflict.txt"), "src");
        Files.writeString(dst.resolve("d/conflict.txt"), "dst");

        FileOperation op = run(Kind.MOVE, List.of(src.resolve("d")), dst, always(Resolution.SKIP));
        assertTrue(op.errors().isEmpty(), op.errors().toString());
        assertEquals("n", Files.readString(dst.resolve("d/new.txt")));
        assertEquals("dst", Files.readString(dst.resolve("d/conflict.txt")));
        assertEquals("d,d/conflict.txt", tree(src), "skipped file and its folder stay at the source");
    }

    @Test
    void moveWithOverwriteReplacesAndRemovesTheSource() throws Exception {
        Files.writeString(src.resolve("a.txt"), "new");
        Files.writeString(dst.resolve("a.txt"), "old");
        run(Kind.MOVE, List.of(src.resolve("a.txt")), dst, always(Resolution.OVERWRITE));
        assertEquals("new", Files.readString(dst.resolve("a.txt")));
        assertFalse(Files.exists(src.resolve("a.txt")));
    }

    @Test
    void moveIntoOwnFolderOrSameFolderDoesNothing() throws Exception {
        Files.createDirectories(src.resolve("d/inner"));
        Files.writeString(src.resolve("f"), "x");
        FileOperation into = run(Kind.MOVE, List.of(src.resolve("d")), src.resolve("d/inner"), FAIL_ON_CONFLICT);
        assertEquals(1, into.errors().size());
        FileOperation same = run(Kind.MOVE, List.of(src.resolve("f")), src, FAIL_ON_CONFLICT);
        assertTrue(same.errors().isEmpty());
        assertEquals("d,d/inner,f", tree(src));
    }

    @Test
    void movingAFileOntoItselfThroughAnotherPathIsRefused() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Files.writeString(src.resolve("a.txt"), "data");
        Path alias = Files.createSymbolicLink(base.resolve("alias"), src);
        FileOperation op = run(Kind.MOVE, List.of(src.resolve("a.txt")), alias, always(Resolution.OVERWRITE));
        assertEquals(1, op.errors().size(), op.errors().toString());
        assertEquals("data", Files.readString(src.resolve("a.txt")), "file must survive");

        run(Kind.COPY, List.of(src.resolve("a.txt")), alias, FAIL_ON_CONFLICT);
        assertEquals("data", Files.readString(src.resolve("a - Copy.txt")), "copying onto itself makes a copy");
    }

    // ---- delete / trash ----

    @Test
    void deleteRemovesTreesAndReportsProgress() throws Exception {
        Files.createDirectories(src.resolve("a/b/c"));
        for (int i = 0; i < 50; i++) {
            Files.writeString(src.resolve("a/b/f" + i), "x");
        }
        FileOperation op = run(Kind.DELETE, List.of(src.resolve("a")), null, FAIL_ON_CONFLICT);
        assertTrue(op.errors().isEmpty());
        assertFalse(Files.exists(src.resolve("a")));
        assertProgressComplete(op);
    }

    @Test
    void deleteContinuesPastFailuresAndKeepsWhatItCannotDelete() throws Exception {
        assumeTrue(OsUtil.isPosix() && !"root".equals(System.getProperty("user.name")));
        Path locked = Files.createDirectories(src.resolve("locked"));
        Files.writeString(locked.resolve("f"), "x");
        Files.writeString(src.resolve("free.txt"), "x");
        Files.setPosixFilePermissions(locked, PosixFilePermissions.fromString("r-xr-xr-x"));
        try {
            FileOperation op = run(Kind.DELETE, List.of(locked, src.resolve("free.txt")), null, FAIL_ON_CONFLICT);
            assertFalse(op.errors().isEmpty());
            assertTrue(Files.exists(locked.resolve("f")));
            assertFalse(Files.exists(src.resolve("free.txt")), "other items are still deleted");
        } finally {
            Files.setPosixFilePermissions(locked, PosixFilePermissions.fromString("rwxr-xr-x"));
        }
    }

    @Test
    void trashOfAMissingItemReportsAnErrorAndTouchesNothing() {
        FileOperation op = run(Kind.TRASH, List.of(base.resolve("does-not-exist")), null, FAIL_ON_CONFLICT);
        assertEquals(1, op.errors().size(), op.errors().toString());
    }

    @Test
    void notEnoughSpaceFailsBeforeWritingAnything() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Path sparse = src.resolve("huge.bin");
        try (RandomAccessFile f = new RandomAccessFile(sparse.toFile(), "rw")) {
            f.setLength(Files.getFileStore(dst).getUsableSpace() + (1L << 30)); // sparse: costs no space
        }
        FileOperation op = run(Kind.COPY, List.of(sparse), dst, FAIL_ON_CONFLICT);
        Throwable failure = onFx(op::getException);
        assertTrue(failure != null && failure.getMessage().contains("Not enough free space"), String.valueOf(failure));
        assertEquals("", tree(dst));
    }

    // ---- archives ----

    @Test
    void archiveNames() throws Exception {
        Path file = Files.writeString(src.resolve("report.final.txt"), "x");
        Path dir = Files.createDirectory(src.resolve("my.folder"));
        assertEquals("report.final.zip", FileOperation.archiveName(List.of(file)));
        assertEquals("my.folder.zip", FileOperation.archiveName(List.of(dir)));
        assertEquals("Archive.zip", FileOperation.archiveName(List.of(file, dir)));
    }

    @Test
    void compressAndExtractRoundTripsContentAndTimes() throws Exception {
        Files.createDirectories(src.resolve("d/inner/empty"));
        byte[] data = randomBytes(300_000, 7);
        Files.write(src.resolve("d/inner/data.bin"), data);
        Files.writeString(src.resolve("top.txt"), "top");
        FileTime time = FileTime.fromMillis(1_600_000_000_000L);
        Files.setLastModifiedTime(src.resolve("top.txt"), time);

        FileOperation zip = run(Kind.COMPRESS, List.of(src.resolve("d"), src.resolve("top.txt")), src, FAIL_ON_CONFLICT);
        assertTrue(zip.errors().isEmpty(), zip.errors().toString());
        assertEquals(src.resolve("Archive.zip"), zip.result());
        assertProgressComplete(zip);

        Files.move(zip.result(), dst.resolve("Archive.zip"));
        FileOperation unzip = run(Kind.EXTRACT, List.of(dst.resolve("Archive.zip")), dst, FAIL_ON_CONFLICT);
        assertTrue(unzip.errors().isEmpty(), unzip.errors().toString());
        Path out = dst.resolve("Archive");
        assertEquals(out, unzip.result());
        assertEquals("d,d/inner,d/inner/data.bin,d/inner/empty,top.txt", tree(out));
        assertArrayEquals(data, Files.readAllBytes(out.resolve("d/inner/data.bin")));
        long delta = Math.abs(Files.getLastModifiedTime(out.resolve("top.txt")).toMillis() - time.toMillis());
        assertTrue(delta <= 2000, "modification time kept (zip has 2 s precision), off by " + delta);
        assertNoTempFiles();
    }

    @Test
    void extractingASingleTopLevelItemDoesNotNestItAndNeverOverwrites() throws Exception {
        Files.createDirectories(src.resolve("project/sub"));
        Files.writeString(src.resolve("project/sub/a.txt"), "a");
        FileOperation zip = run(Kind.COMPRESS, List.of(src.resolve("project")), src, FAIL_ON_CONFLICT);
        assertEquals(src.resolve("project.zip"), zip.result());

        Files.move(zip.result(), dst.resolve("project.zip"));
        Files.createDirectory(dst.resolve("project")); // already there: must not be merged into
        FileOperation unzip = run(Kind.EXTRACT, List.of(dst.resolve("project.zip")), dst, FAIL_ON_CONFLICT);
        assertEquals(dst.resolve("project (2)"), unzip.result());
        assertEquals("a", Files.readString(dst.resolve("project (2)/sub/a.txt")));
        assertEquals("", tree(dst.resolve("project")));
    }

    @Test
    void extractRefusesEntriesThatEscapeTheTarget() throws Exception {
        Path zip = dst.resolve("evil.zip");
        try (var out = new java.util.zip.ZipOutputStream(Files.newOutputStream(zip))) {
            for (String name : List.of("../escaped.txt", "a/../../escaped2.txt", "/abs-escaped.txt", "ok.txt")) {
                out.putNextEntry(new java.util.zip.ZipEntry(name));
                out.write("x".getBytes());
                out.closeEntry();
            }
        }
        FileOperation op = run(Kind.EXTRACT, List.of(zip), dst, FAIL_ON_CONFLICT);
        assertEquals(3, op.errors().size(), op.errors().toString());
        assertFalse(Files.exists(base.resolve("escaped.txt")));
        assertFalse(Files.exists(base.resolve("escaped2.txt")));
        assertFalse(Files.exists(Path.of("/abs-escaped.txt")));
        assertEquals("x", Files.readString(dst.resolve("ok.txt")), "safe entries are extracted");
        assertNoTempFiles();
    }

    @Test
    void corruptArchiveFailsWithoutLeavingAnything() throws Exception {
        Path zip = Files.writeString(dst.resolve("broken.zip"), "this is not a zip file");
        FileOperation op = run(Kind.EXTRACT, List.of(zip), dst, FAIL_ON_CONFLICT);
        assertTrue(onFx(op::getException) != null || !op.errors().isEmpty());
        assertEquals("broken.zip", tree(dst));
        assertNoTempFiles();
    }

    /** A zip whose entry "big.bin" really holds {@code realSize} zero bytes but declares only 10 in its directory. */
    private static Path zipWithUnderstatedSize(Path zip, int realSize) throws IOException {
        try (var out = new java.util.zip.ZipOutputStream(Files.newOutputStream(zip))) {
            out.putNextEntry(new java.util.zip.ZipEntry("big.bin"));
            out.write(new byte[realSize]);
            out.closeEntry();
        }
        byte[] b = Files.readAllBytes(zip);
        boolean patched = false;
        for (int i = 0; i + 28 < b.length; i++) {
            if (b[i] == 0x50 && b[i + 1] == 0x4b && b[i + 2] == 0x01 && b[i + 3] == 0x02) { // central directory header
                java.nio.ByteBuffer.wrap(b, i + 24, 4).order(java.nio.ByteOrder.LITTLE_ENDIAN).putInt(10);
                patched = true;
            }
        }
        assertTrue(patched, "central directory found");
        Files.write(zip, b);
        return zip;
    }

    @Test
    void extractStopsAtTheDeclaredEntrySizeSoAZipBombCannotFillTheDisk() throws Exception {
        Path zip = zipWithUnderstatedSize(dst.resolve("bomb.zip"), 1 << 20);
        FileOperation op = run(Kind.EXTRACT, List.of(zip), dst, FAIL_ON_CONFLICT);
        assertEquals(1, op.errors().size(), op.errors().toString());
        assertTrue(op.errors().get(0).contains("larger than the archive says"), op.errors().toString());
        assertNull(op.result());
        assertEquals("bomb.zip", tree(dst), "nothing placed");
        assertNoTempFiles();
    }

    @Test
    void oneFailingArchiveDoesNotStopTheOthers() throws Exception {
        Path bomb = zipWithUnderstatedSize(dst.resolve("bomb.zip"), 1 << 20);
        Path good = dst.resolve("good.zip");
        try (var out = new java.util.zip.ZipOutputStream(Files.newOutputStream(good))) {
            out.putNextEntry(new java.util.zip.ZipEntry("fine.txt"));
            out.write("ok".getBytes());
            out.closeEntry();
        }
        FileOperation op = run(Kind.EXTRACT, List.of(bomb, good), dst, FAIL_ON_CONFLICT);
        assertNull(onFx(op::getException));
        assertEquals(1, op.errors().size(), op.errors().toString());
        assertEquals("ok", Files.readString(dst.resolve("fine.txt")));
        assertNoTempFiles();
    }

    @Test
    void extractSkipsEntriesWhoseNameIsNotAValidPath() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Path zip = dst.resolve("odd.zip");
        try (var out = new java.util.zip.ZipOutputStream(Files.newOutputStream(zip))) {
            for (String name : List.of("bad\u0000name.txt", "ok.txt")) {
                out.putNextEntry(new java.util.zip.ZipEntry(name));
                out.write("x".getBytes());
                out.closeEntry();
            }
        }
        FileOperation op = run(Kind.EXTRACT, List.of(zip), dst, FAIL_ON_CONFLICT);
        assertNull(onFx(op::getException), "an invalid name does not abort the extraction");
        assertEquals(1, op.errors().size(), op.errors().toString());
        assertEquals("x", Files.readString(dst.resolve("ok.txt")));
        assertNoTempFiles();
    }

    @Test
    void archivesSkipLinksInsteadOfFollowingThem() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Files.createDirectory(src.resolve("d"));
        Files.writeString(src.resolve("d/real.txt"), "r");
        Files.createSymbolicLink(src.resolve("d/loop"), src);
        FileOperation zip = run(Kind.COMPRESS, List.of(src.resolve("d")), dst, FAIL_ON_CONFLICT);
        assertEquals(1, zip.errors().size(), zip.errors().toString());
        FileOperation unzip = run(Kind.EXTRACT, List.of(zip.result()), base, FAIL_ON_CONFLICT);
        assertEquals("real.txt", tree(unzip.result()));
    }

    // ---- pause / resume ----

    /** Runs {@code op} on its own thread, as the app's background pool does. */
    private static Thread start(FileOperation op) {
        Thread t = new Thread(op, "op-under-test");
        t.setDaemon(true);
        t.start();
        return t;
    }

    private static void awaitPaused(FileOperation op) {
        assertTimeoutPreemptively(Duration.ofSeconds(10), () -> {
            while (!op.isWaitingWhilePaused()) {
                Thread.onSpinWait();
            }
        });
    }

    private Set<String> names(Path dir) throws IOException {
        try (Stream<Path> s = Files.list(dir)) {
            return s.map(p -> p.getFileName().toString()).collect(Collectors.toSet());
        }
    }

    @Test
    void aPausedOperationWaitsAndThenFinishesOnResume() throws Exception {
        List<Path> files = new ArrayList<>();
        for (String n : List.of("a.txt", "b.txt", "c.txt")) {
            files.add(Files.writeString(src.resolve(n), n));
        }
        FileOperation op = new FileOperation(Kind.COPY, files, dst, FAIL_ON_CONFLICT);
        op.pause();
        Thread t = start(op);
        awaitPaused(op);
        assertTrue(op.isPaused());
        assertEquals(Set.of(), names(dst), "nothing is copied while paused");

        op.resume();
        assertTimeoutPreemptively(Duration.ofSeconds(10), () -> t.join());
        assertFalse(op.isPaused());
        assertEquals(Set.of("a.txt", "b.txt", "c.txt"), names(dst));
        assertFalse(onFx(op::isCancelled));
        assertProgressComplete(op);
    }

    @Test
    void pausingStopsBetweenItemsAndResumeContinuesWithTheNextOne() throws Exception {
        List<Path> files = new ArrayList<>();
        for (String n : List.of("a.txt", "b.txt", "c.txt")) {
            files.add(Files.writeString(src.resolve(n), n));
        }
        Files.writeString(dst.resolve("b.txt"), "existing");
        FileOperation[] holder = new FileOperation[1];
        // The conflict on b.txt pauses the operation, as a click on Pause would at that moment.
        holder[0] = new FileOperation(Kind.COPY, files, dst, (s, d) -> {
            holder[0].pause();
            return new Decision(Resolution.SKIP, false);
        });
        FileOperation op = holder[0];
        Thread t = start(op);
        awaitPaused(op);
        assertEquals(Set.of("a.txt", "b.txt"), names(dst), "a copied, b skipped, c waits");
        assertEquals("existing", Files.readString(dst.resolve("b.txt")));

        op.resume();
        assertTimeoutPreemptively(Duration.ofSeconds(10), () -> t.join());
        assertEquals("c.txt", Files.readString(dst.resolve("c.txt")));
    }

    @Test
    void cancellingAPausedOperationStopsItAndLeavesNoPartialFiles() throws Exception {
        Path big = src.resolve("big.bin");
        Files.write(big, randomBytes(1 << 16, 7));
        Path small = Files.writeString(src.resolve("small.txt"), "small");
        FileOperation op = new FileOperation(Kind.MOVE, List.of(big, small), dst.resolve("sub"), FAIL_ON_CONFLICT);
        Files.createDirectory(dst.resolve("sub"));
        op.pause();
        Thread t = start(op);
        awaitPaused(op);

        op.cancel();
        assertTimeoutPreemptively(Duration.ofSeconds(10), () -> t.join());
        assertTrue(onFx(op::isCancelled));
        assertEquals(Set.of(), names(dst.resolve("sub")), "nothing moved, no .part files");
        assertTrue(Files.exists(big) && Files.exists(small), "sources intact");
    }

    @Test
    void awaitStoppedReturnsOnlyAfterTheCancelledWorkerCleanedUp() throws Exception {
        Path a = Files.writeString(src.resolve("a.txt"), "a");
        Path b = Files.writeString(src.resolve("b.txt"), "b");
        FileOperation op = new FileOperation(Kind.COPY, List.of(a, b), dst, FAIL_ON_CONFLICT);
        op.pause();
        Thread t = start(op);
        awaitPaused(op);
        assertFalse(op.awaitStopped(50, java.util.concurrent.TimeUnit.MILLISECONDS), "still running while paused");

        op.cancel();
        assertTrue(op.awaitStopped(10, java.util.concurrent.TimeUnit.SECONDS));
        assertFalse(t.isAlive() && op.isWaitingWhilePaused(), "worker left the pause");
        assertEquals(Set.of(), names(dst), "no files and no .part leftovers");
    }

    @Test
    void anOperationCancelledBeforeItStartedCountsAsStopped() throws Exception {
        FileOperation op = new FileOperation(Kind.COPY, List.of(Files.writeString(src.resolve("a.txt"), "a")), dst,
                FAIL_ON_CONFLICT);
        op.cancel();
        assertTrue(op.awaitStopped(1, java.util.concurrent.TimeUnit.SECONDS));
        op.run(); // a cancelled task never runs
        assertEquals(Set.of(), names(dst));
    }
}
