package com.fileem.fs;

import com.fileem.util.OsUtil;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.attribute.FileTime;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

class FileEntryTest {
    @TempDir
    Path dir;

    @Test
    void readsRegularFile() throws Exception {
        Path f = dir.resolve("Report.PDF");
        Files.writeString(f, "12345");
        Files.setLastModifiedTime(f, FileTime.fromMillis(1_000_000_000_000L));
        FileEntry e = FileEntry.read(f);
        assertEquals("Report.PDF", e.name());
        assertEquals(5, e.size());
        assertFalse(e.isDirectory());
        assertFalse(e.isSymlink());
        assertEquals("pdf", e.extension());
        assertEquals(FileTypes.Category.PDF, e.category());
        assertEquals(1_000_000_000_000L, e.modified().toMillis());
    }

    @Test
    void foldersHaveNoSize() throws Exception {
        FileEntry e = FileEntry.read(Files.createDirectory(dir.resolve("sub.d")));
        assertTrue(e.isDirectory());
        assertEquals(-1, e.size());
        assertEquals("Folder", e.type());
        assertEquals("", e.extension());
    }

    @Test
    void missingPathStillProducesARow() {
        FileEntry e = FileEntry.read(dir.resolve("gone"));
        assertEquals("gone", e.name());
        assertNull(e.modified());
        assertEquals("", e.attributes());
    }

    @Test
    void posixDotFilesAreHiddenAndAttributesShowPermissions() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Path f = Files.writeString(dir.resolve(".secret"), "x");
        FileEntry e = FileEntry.read(f);
        assertTrue(e.isHidden());
        assertTrue(e.attributes().matches("-[rwx-]{9}  .*"), e.attributes());
        assertTrue(FileEntry.read(dir).attributes().startsWith("d"));
    }

    @Test
    void symlinksReportTheTargetAndBrokenLinksDoNotFail() throws Exception {
        assumeTrue(OsUtil.isPosix());
        Path target = Files.writeString(dir.resolve("t.txt"), "abc");
        Path link = Files.createSymbolicLink(dir.resolve("link"), target);
        FileEntry e = FileEntry.read(link);
        assertTrue(e.isSymlink());
        assertEquals(3, e.size());
        assertTrue(e.attributes().startsWith("l"));

        Path dirLink = Files.createSymbolicLink(dir.resolve("dlink"), dir.resolve(".."));
        assertTrue(FileEntry.read(dirLink).isDirectory());
        assertEquals("Folder link", FileEntry.read(dirLink).type());

        Path broken = Files.createSymbolicLink(dir.resolve("broken"), dir.resolve("nothing"));
        FileEntry b = FileEntry.read(broken);
        assertTrue(b.isSymlink());
        assertFalse(b.isDirectory());
    }
}
