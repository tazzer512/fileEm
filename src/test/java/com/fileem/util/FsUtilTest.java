package com.fileem.util;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.function.Predicate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

class FsUtilTest {
    @TempDir
    Path dir;

    @Test
    void uniqueNameReturnsWantedNameWhenFree() {
        assertEquals(dir.resolve("a.txt"), FsUtil.uniqueName(dir.resolve("a.txt"), false));
    }

    @Test
    void uniqueNameNumbersBeforeTheExtension() throws Exception {
        Files.writeString(dir.resolve("a.txt"), "");
        Files.writeString(dir.resolve("a (2).txt"), "");
        assertEquals(dir.resolve("a (3).txt"), FsUtil.uniqueName(dir.resolve("a.txt"), false));
    }

    @Test
    void uniqueNameCopySuffix() throws Exception {
        Files.writeString(dir.resolve("a.txt"), "");
        assertEquals(dir.resolve("a - Copy.txt"), FsUtil.uniqueName(dir.resolve("a.txt"), true));
        Files.writeString(dir.resolve("a - Copy.txt"), "");
        assertEquals(dir.resolve("a - Copy (2).txt"), FsUtil.uniqueName(dir.resolve("a.txt"), true));
    }

    @Test
    void uniqueNameKeepsDotsInFolderNames() throws Exception {
        Files.createDirectory(dir.resolve("v1.2"));
        assertEquals(dir.resolve("v1.2 (2)"), FsUtil.uniqueName(dir.resolve("v1.2"), false));
    }

    @Test
    void uniqueNameDotFilesAreNotSplitAtTheLeadingDot() throws Exception {
        Files.writeString(dir.resolve(".bashrc"), "");
        assertEquals(dir.resolve(".bashrc (2)"), FsUtil.uniqueName(dir.resolve(".bashrc"), false));
    }

    @Test
    void validateNameRejectsEmptyDotsAndSlashes() {
        assertNotNull(FsUtil.validateName(""));
        assertNotNull(FsUtil.validateName("   "));
        assertNotNull(FsUtil.validateName("."));
        assertNotNull(FsUtil.validateName(".."));
        assertNotNull(FsUtil.validateName("a/b"));
        assertNull(FsUtil.validateName("normal name.txt"));
        assertNull(FsUtil.validateName(".hidden"));
    }

    @Test
    void validateNameRejectsWindowsReservedCharactersOnWindows() {
        assumeTrue(OsUtil.isWindows());
        assertNotNull(FsUtil.validateName("a:b"));
        assertNotNull(FsUtil.validateName("a?"));
        assertNotNull(FsUtil.validateName("trailing."));
    }

    @Test
    void nameMatcherPlainTextIsCaseInsensitiveContains() {
        Predicate<String> m = FsUtil.nameMatcher("Rep");
        assertTrue(m.test("report.pdf"));
        assertTrue(m.test("MY_REPORT"));
        assertFalse(m.test("other"));
        assertTrue(FsUtil.nameMatcher("").test("anything"));
        assertTrue(FsUtil.nameMatcher(null).test("anything"));
    }

    @Test
    void nameMatcherWildcardsMatchTheWholeName() {
        Predicate<String> m = FsUtil.nameMatcher("*.TXT");
        assertTrue(m.test("a.txt"));
        assertFalse(m.test("a.txt.bak"));
        Predicate<String> q = FsUtil.nameMatcher("file?.log");
        assertTrue(q.test("file1.log"));
        assertFalse(q.test("file12.log"));
        assertTrue(FsUtil.nameMatcher("a+b(*").test("a+b(1)"), "regex characters are literal");
    }

    @Test
    void isSameOrInside() {
        assertTrue(FsUtil.isSameOrInside(dir.resolve("a/b"), dir.resolve("a")));
        assertTrue(FsUtil.isSameOrInside(dir.resolve("a"), dir.resolve("a")));
        assertTrue(FsUtil.isSameOrInside(dir.resolve("a/x/../b"), dir.resolve("a")));
        assertFalse(FsUtil.isSameOrInside(dir.resolve("ab"), dir.resolve("a")), "prefix of name is not inside");
        assertFalse(FsUtil.isSameOrInside(dir.resolve("a"), dir.resolve("a/b")));
    }

    @Test
    void hiddenFollowsPlatformRules() throws Exception {
        assumeTrue(!OsUtil.isWindows());
        assertTrue(FsUtil.isHidden(dir.resolve(".x")));
        assertFalse(FsUtil.isHidden(dir.resolve("x")));
        assertFalse(FsUtil.isJunction(dir), "junctions only exist on Windows");
    }

    @Test
    void displayNameOfRootIsItsPath() {
        Path root = dir.getRoot();
        assertEquals(root.toString(), FsUtil.displayName(root));
        assertEquals("x", FsUtil.displayName(dir.resolve("x")));
    }
}
