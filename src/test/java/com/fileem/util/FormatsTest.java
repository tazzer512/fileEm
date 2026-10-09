package com.fileem.util;

import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.nio.file.attribute.FileTime;
import java.util.Locale;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

class FormatsTest {
    private static Locale saved;

    @BeforeAll
    static void useFixedLocale() {
        saved = Locale.getDefault();
        Locale.setDefault(Locale.US);
    }

    @AfterAll
    static void restoreLocale() {
        Locale.setDefault(saved);
    }

    @Test
    void sizesUseBinaryUnitsWithThreeSignificantDigits() {
        assertEquals("", Formats.size(-1));
        assertEquals("0 bytes", Formats.size(0));
        assertEquals("1 byte", Formats.size(1));
        assertEquals("1023 bytes", Formats.size(1023));
        assertEquals("1.00 KB", Formats.size(1024));
        assertEquals("1.50 KB", Formats.size(1536));
        assertEquals("10.0 KB", Formats.size(10 * 1024));
        assertEquals("100 KB", Formats.size(100 * 1024));
        assertEquals("1.00 MB", Formats.size(1L << 20));
        assertEquals("1.00 GB", Formats.size(1L << 30));
        assertEquals("8.00 EB", Formats.size(Long.MAX_VALUE));
    }

    @Test
    void exactAndCountUseGrouping() {
        assertEquals("1,234,567 bytes", Formats.exact(1_234_567));
        assertEquals("1,000", Formats.count(1000));
        assertEquals("1.18 MB (1,234,567 bytes)", Formats.sizeWithExact(1_234_567));
        assertEquals("12 bytes", Formats.sizeWithExact(12));
    }

    @Test
    void datesHandleNull() {
        assertEquals("", Formats.date(null));
        assertFalse(Formats.date(FileTime.fromMillis(0)).isEmpty());
        assertFalse(Formats.dateLong(FileTime.fromMillis(0)).isEmpty());
    }
}
