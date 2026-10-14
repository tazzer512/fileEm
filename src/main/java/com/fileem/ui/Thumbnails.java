package com.fileem.ui;

import com.fileem.fs.FileEntry;
import com.fileem.fs.PsdReader;
import com.fileem.fs.RawReader;
import javafx.application.Platform;
import javafx.beans.value.ChangeListener;
import javafx.scene.image.Image;
import javafx.scene.image.PixelFormat;
import javafx.scene.image.WritableImage;

import java.io.ByteArrayInputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.function.Consumer;

/**
 * Image thumbnails for the icon views. Formats JavaFX decodes load with its own background loading;
 * Photoshop files ({@link PsdReader}) and camera raws ({@link RawReader}) are decoded on two background threads. Nothing touches the disk
 * on the FX thread. Kept in an LRU cache bounded by pixel count, keyed by file, size and date (a changed
 * file gets a new entry). Thumbnails load at a few fixed sizes ({@link #loadSize}) and the image view
 * scales them, so dragging the size slider doesn't decode every image again for every pixel.
 */
final class Thumbnails {
    /** Formats JavaFX can decode. */
    private static final Set<String> SUPPORTED = Set.of("png", "jpg", "jpeg", "gif", "bmp");
    /** Bigger files are not worth decoding just for a thumbnail. */
    private static final long MAX_FILE_SIZE = 40L << 20;
    /** Photoshop and raw previews read only a part of the file, so even big ones are fine. */
    private static final long MAX_PSD_SIZE = 8L << 30;
    /** About 64 MB of decoded ARGB pixels. */
    private static final long PIXEL_BUDGET = 16L << 20;

    private static final LinkedHashMap<String, Image> CACHE = new LinkedHashMap<>(256, 0.75f, true);
    /** Photoshop and raw previews being decoded, with whoever is waiting for them. */
    private static final Map<String, List<Consumer<Image>>> DECODING = new HashMap<>();
    /** Files that have no usable preview: not tried again until they change. */
    private static final Set<String> FAILED = new HashSet<>();
    private static final ExecutorService PSD = Executors.newFixedThreadPool(2, r -> {
        Thread t = new Thread(r, "fileem-psd-preview");
        t.setDaemon(true);
        return t;
    });
    private static long pixels;

    private Thumbnails() {
    }

    static boolean supports(FileEntry e) {
        if (e.isDirectory() || e.size() <= 0) {
            return false;
        }
        if (PsdReader.isPsd(e.extension()) || RawReader.isRaw(e.extension())) {
            return e.size() <= MAX_PSD_SIZE;
        }
        return e.size() <= MAX_FILE_SIZE && SUPPORTED.contains(e.extension());
    }

    /** The size a thumbnail for {@code px} pixels is loaded at: the next multiple of 32, at least 48. */
    static int loadSize(int px) {
        return Math.max(48, Math.min(ViewMode.MAX_ICON, (px + 31) / 32 * 32));
    }

    /**
     * Calls {@code onLoaded} on the FX thread with a thumbnail of at most {@code size}×{@code size}
     * (scaled for display) once it is ready: right away when cached, later when it loads, never for
     * unsupported files or failures. Call on the FX thread; check in the callback that the item shown is
     * still the same (cells are reused).
     */
    static void request(FileEntry e, int size, Consumer<Image> onLoaded) {
        if (!supports(e)) {
            return;
        }
        int load = loadSize(size);
        String key = e.path() + "|" + e.size() + "|" + (e.modified() == null ? 0 : e.modified().toMillis()) + "|" + load;
        if (FAILED.contains(key)) {
            return;
        }
        Image cached = CACHE.get(key);
        if (cached != null) {
            whenLoaded(cached, onLoaded);
            return;
        }
        if (decodedHere(e)) {
            List<Consumer<Image>> waiting = DECODING.get(key);
            if (waiting != null) {
                waiting.add(onLoaded);
                return;
            }
            waiting = new ArrayList<>(List.of(onLoaded));
            DECODING.put(key, waiting);
            PSD.execute(() -> {
                Image img = RawReader.isRaw(e.extension()) ? decodeRaw(e, load) : decodePsd(e, load);
                Platform.runLater(() -> {
                    List<Consumer<Image>> callbacks = DECODING.remove(key);
                    if (img == null) {
                        FAILED.add(key);
                        return;
                    }
                    put(key, img, load);
                    callbacks.forEach(c -> whenLoaded(img, c));
                });
            });
            return;
        }
        Image img = new Image(e.path().toUri().toString(), load, load, true, true, true);
        put(key, img, load);
        whenLoaded(img, onLoaded);
    }

    /** Photoshop and camera raw files: read by our own readers on the background pool. */
    private static boolean decodedHere(FileEntry e) {
        return PsdReader.isPsd(e.extension()) || RawReader.isRaw(e.extension());
    }

    /** On a background thread: the raw file's smallest embedded JPEG that is big enough, upright. */
    private static Image decodeRaw(FileEntry e, int load) {
        try {
            return RawImages.decode(RawReader.read(e.path(), load), load);
        } catch (Exception | OutOfMemoryError ex) {
            return null;
        }
    }

    /** On a background thread: the PSD's preview as an image, or null. */
    private static Image decodePsd(FileEntry e, int load) {
        try {
            PsdReader.Preview p = PsdReader.read(e.path(), load);
            if (p == null) {
                return null;
            }
            if (p.argb() != null) {
                WritableImage img = new WritableImage(p.width(), p.height());
                img.getPixelWriter().setPixels(0, 0, p.width(), p.height(), PixelFormat.getIntArgbInstance(),
                        p.argb(), 0, p.width());
                return img;
            }
            Image img = new Image(new ByteArrayInputStream(p.jpeg()), load, load, true, true);
            return img.isError() ? null : img;
        } catch (Exception | OutOfMemoryError ex) {
            return null;
        }
    }

    /** Calls back once {@code img} has loaded without error (immediately if it already has). */
    private static void whenLoaded(Image img, Consumer<Image> onLoaded) {
        if (img.getProgress() >= 1) {
            if (!img.isError()) {
                onLoaded.accept(img);
            }
            return;
        }
        ChangeListener<Number> listener = new ChangeListener<>() {
            @Override
            public void changed(javafx.beans.value.ObservableValue<? extends Number> o, Number was, Number now) {
                if (now.doubleValue() >= 1) {
                    img.progressProperty().removeListener(this);
                    if (!img.isError()) {
                        onLoaded.accept(img);
                    }
                }
            }
        };
        img.progressProperty().addListener(listener);
    }

    private static void put(String key, Image img, int load) {
        CACHE.put(key, img);
        pixels += (long) load * load;
        trim();
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
