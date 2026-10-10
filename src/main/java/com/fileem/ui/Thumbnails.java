package com.fileem.ui;

import com.fileem.fs.FileEntry;
import javafx.scene.image.Image;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;

/**
 * Image thumbnails for the icon views. JavaFX decodes and scales them on its own background threads
 * ({@code backgroundLoading}), so nothing here touches the disk on the FX thread. Kept in an LRU cache
 * bounded by pixel count; a changed file (size or date) gets a new entry.
 */
final class Thumbnails {
    /** Formats JavaFX can decode. */
    private static final Set<String> SUPPORTED = Set.of("png", "jpg", "jpeg", "gif", "bmp");
    /** Bigger files are not worth decoding just for a thumbnail. */
    private static final long MAX_FILE_SIZE = 40L << 20;
    /** About 64 MB of decoded ARGB pixels. */
    private static final long PIXEL_BUDGET = 16L << 20;

    private static final LinkedHashMap<String, Image> CACHE = new LinkedHashMap<>(256, 0.75f, true);
    private static long pixels;

    private Thumbnails() {
    }

    static boolean supports(FileEntry e) {
        return !e.isDirectory() && e.size() > 0 && e.size() <= MAX_FILE_SIZE && SUPPORTED.contains(e.extension());
    }

    /** A (possibly still loading) thumbnail of at most {@code size}×{@code size}, or null if unsupported. */
    static Image get(FileEntry e, int size) {
        if (!supports(e)) {
            return null;
        }
        String key = e.path() + "|" + e.size() + "|" + (e.modified() == null ? 0 : e.modified().toMillis()) + "|" + size;
        Image img = CACHE.get(key);
        if (img == null) {
            img = new Image(e.path().toUri().toString(), size, size, true, true, true);
            CACHE.put(key, img);
            pixels += (long) size * size;
            trim();
        }
        return img;
    }

    private static void trim() {
        var it = CACHE.entrySet().iterator();
        while (pixels > PIXEL_BUDGET && it.hasNext()) {
            Map.Entry<String, Image> eldest = it.next();
            String key = eldest.getKey();
            int size = Integer.parseInt(key.substring(key.lastIndexOf('|') + 1));
            pixels -= (long) size * size;
            it.remove();
        }
    }
}
