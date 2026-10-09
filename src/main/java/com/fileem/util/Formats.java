package com.fileem.util;

import java.nio.file.attribute.FileTime;
import java.text.NumberFormat;
import java.time.ZoneId;
import java.time.format.DateTimeFormatter;
import java.time.format.FormatStyle;

public final class Formats {
    private static final String[] UNITS = {"KB", "MB", "GB", "TB", "PB", "EB"};
    private static final DateTimeFormatter DATE = DateTimeFormatter
            .ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT)
            .withZone(ZoneId.systemDefault());
    private static final DateTimeFormatter DATE_LONG = DateTimeFormatter
            .ofLocalizedDateTime(FormatStyle.LONG, FormatStyle.MEDIUM)
            .withZone(ZoneId.systemDefault());

    private Formats() {
    }

    /** Human readable size with binary units, empty for negative (unknown / folder). */
    public static String size(long bytes) {
        if (bytes < 0) {
            return "";
        }
        if (bytes < 1024) {
            return bytes + (bytes == 1 ? " byte" : " bytes");
        }
        double v = bytes;
        int unit = -1;
        do {
            v /= 1024;
            unit++;
        } while (v >= 1024 && unit < UNITS.length - 1);
        String num = v >= 100 ? String.format("%.0f", v) : v >= 10 ? String.format("%.1f", v) : String.format("%.2f", v);
        return num + " " + UNITS[unit];
    }

    public static String exact(long bytes) {
        return NumberFormat.getIntegerInstance().format(bytes) + " bytes";
    }

    public static String sizeWithExact(long bytes) {
        return bytes < 1024 ? size(bytes) : size(bytes) + " (" + exact(bytes) + ")";
    }

    public static String count(long n) {
        return NumberFormat.getIntegerInstance().format(n);
    }

    public static String date(FileTime t) {
        return t == null ? "" : DATE.format(t.toInstant());
    }

    public static String dateLong(FileTime t) {
        return t == null ? "" : DATE_LONG.format(t.toInstant());
    }
}
