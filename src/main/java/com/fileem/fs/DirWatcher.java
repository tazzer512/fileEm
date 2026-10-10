package com.fileem.fs;

import javafx.application.Platform;

import java.io.IOException;
import java.nio.file.ClosedWatchServiceException;
import java.nio.file.FileSystems;
import java.nio.file.Path;
import java.nio.file.StandardWatchEventKinds;
import java.nio.file.WatchKey;
import java.nio.file.WatchService;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

/** Shared directory watcher; listeners run on the FX thread, debounced per directory. */
public final class DirWatcher {
    private static final long DEBOUNCE_MS = 400;
    private static DirWatcher instance;
    private static boolean failed;

    private final WatchService service;
    private final Map<Path, Entry> entries = new HashMap<>();
    private final ScheduledExecutorService debounce = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread t = new Thread(r, "fileem-watch-debounce");
        t.setDaemon(true);
        return t;
    });

    private static final class Entry {
        WatchKey key;
        final List<Runnable> listeners = new ArrayList<>();
        boolean pending;
    }

    private DirWatcher() throws IOException {
        service = FileSystems.getDefault().newWatchService();
        Thread t = new Thread(this::loop, "fileem-watch");
        t.setDaemon(true);
        t.start();
    }

    private static synchronized DirWatcher get() {
        if (instance == null && !failed) {
            try {
                instance = new DirWatcher();
            } catch (IOException e) {
                failed = true;
            }
        }
        return instance;
    }

    /**
     * Calls {@code onChange} whenever the content of {@code dir} changes. Close the handle to stop.
     * Registering touches the file system (it can block on a hung mount): call it off the FX thread.
     */
    public static AutoCloseable watch(Path dir, Runnable onChange) {
        DirWatcher w = get();
        return w == null ? () -> { } : w.register(dir, onChange);
    }

    private synchronized AutoCloseable register(Path dir, Runnable listener) {
        Entry e = entries.get(dir);
        if (e == null || !e.key.isValid()) {
            // a key turns invalid when its folder is deleted; a folder created again at the same
            // path needs a new one, or it would never be watched again
            try {
                WatchKey key = dir.register(service, StandardWatchEventKinds.ENTRY_CREATE,
                        StandardWatchEventKinds.ENTRY_DELETE, StandardWatchEventKinds.ENTRY_MODIFY);
                if (e == null) {
                    e = new Entry();
                    entries.put(dir, e);
                }
                e.key = key;
            } catch (IOException | RuntimeException ex) {
                return () -> { };
            }
        }
        e.listeners.add(listener);
        Entry entry = e;
        return () -> unregister(dir, entry, listener);
    }

    private synchronized void unregister(Path dir, Entry e, Runnable listener) {
        e.listeners.remove(listener);
        if (e.listeners.isEmpty() && entries.remove(dir, e)) {
            e.key.cancel();
        }
    }

    private void loop() {
        while (true) {
            WatchKey key;
            try {
                key = service.take();
            } catch (InterruptedException | ClosedWatchServiceException e) {
                return;
            }
            key.pollEvents();
            key.reset();
            Path dir = (Path) key.watchable();
            synchronized (this) {
                Entry e = entries.get(dir);
                if (e != null && e.key == key && !e.pending) {
                    e.pending = true;
                    debounce.schedule(() -> flush(e), DEBOUNCE_MS, TimeUnit.MILLISECONDS);
                }
            }
        }
    }

    private void flush(Entry e) {
        List<Runnable> listeners;
        synchronized (this) {
            e.pending = false;
            listeners = List.copyOf(e.listeners);
        }
        Platform.runLater(() -> listeners.forEach(Runnable::run));
    }
}
