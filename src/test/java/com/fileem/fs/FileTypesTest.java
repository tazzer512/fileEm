package com.fileem.fs;

import com.fileem.fs.FileTypes.Category;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;

class FileTypesTest {
    @Test
    void extensionIsLowercasedAndIgnoresLeadingAndTrailingDots() {
        assertEquals("jpg", FileTypes.extension("Photo.JPG", false));
        assertEquals("gz", FileTypes.extension("a.tar.gz", false));
        assertEquals("", FileTypes.extension(".bashrc", false));
        assertEquals("", FileTypes.extension("trailing.", false));
        assertEquals("", FileTypes.extension("noext", false));
        assertEquals("", FileTypes.extension("dir.with.dots", true));
    }

    @Test
    void categoriesAndDescriptions() {
        assertEquals(Category.FOLDER, FileTypes.category("", true));
        assertEquals(Category.IMAGE, FileTypes.category("png", false));
        assertEquals(Category.CODE, FileTypes.category("java", false));
        assertEquals(Category.FILE, FileTypes.category("unknownext", false));
        assertEquals("Folder", FileTypes.description("", Category.FOLDER));
        assertEquals("File", FileTypes.description("", Category.FILE));
        assertEquals("XYZ File", FileTypes.description("xyz", Category.FILE));
        assertEquals("PNG Image", FileTypes.description("png", Category.IMAGE));
    }
}
