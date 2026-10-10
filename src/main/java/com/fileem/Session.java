package com.fileem;

import com.fileem.util.OsUtil;

import java.io.IOException;
import java.io.Reader;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Map;
import java.util.Properties;
import java.util.TreeMap;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Window layout, open tabs, theme and other settings, persisted as a properties file in the OS config
 * dir. Writes go to a temp file that is renamed into place, so a crash mid-write never leaves a broken
 * session, and unchanged contents are not written again.
 */
public final class Session {
    /** One writer thread keeps background saves in order. */
    private static final ExecutorService WRITER = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "fileem-session-writer");
        t.setDaemon(true);
        return t;
    });

    private final Path file;
    private final Properties props = new Properties();
    /** The contents last handed to a write (guarded by {@code this}). */
    private Map<String, String> lastSaved;

    private Session(Path file) {
        this.file = file;
    }

    public static Session load() {
        return load(OsUtil.configDir().resolve("session.properties"));
    }

    /** Loads (or starts empty) a session stored at {@code file}. */
    public static Session load(Path file) {
        Session s = new Session(file);
        if (Files.isRegularFile(s.file)) {
            try (Reader r = Files.newBufferedReader(s.file, StandardCharsets.UTF_8)) {
                s.props.load(r);
            } catch (IOException | IllegalArgumentException e) {
                System.err.println("Could not read session: " + e.getMessage());
            }
        }
        return s;
    }

    /** Writes the session now if it changed since the last save. */
    public void save() {
        Map<String, String> snapshot = changedSnapshot();
        if (snapshot != null) {
            write(snapshot);
        }
    }

    /**
     * Writes the session on a background thread if it changed since the last save. Returns the
     * pending write, or null when nothing changed.
     */
    public CompletableFuture<Void> saveInBackground() {
        Map<String, String> snapshot = changedSnapshot();
        return snapshot == null ? null : CompletableFuture.runAsync(() -> write(snapshot), WRITER);
    }

    private synchronized Map<String, String> changedSnapshot() {
        Map<String, String> snapshot = new TreeMap<>();
        for (String key : props.stringPropertyNames()) {
            snapshot.put(key, props.getProperty(key));
        }
        if (snapshot.equals(lastSaved)) {
            return null;
        }
        lastSaved = snapshot;
        return snapshot;
    }

    private synchronized void write(Map<String, String> snapshot) {
        if (snapshot != lastSaved) {
            return; // a newer snapshot was taken (and is or was written), don't overwrite it with this one
        }
        Path tmp = null;
        try {
            Properties out = new Properties();
            out.putAll(snapshot);
            StringWriter text = new StringWriter();
            out.store(text, "fileEm session");
            Files.createDirectories(file.getParent());
            tmp = Files.createTempFile(file.getParent(), ".session-", ".tmp");
            Files.writeString(tmp, text.toString(), StandardCharsets.UTF_8);
            try {
                Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
            } catch (AtomicMoveNotSupportedException e) {
                Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING);
            }
            tmp = null;
        } catch (IOException e) {
            lastSaved = null; // try again next time
            System.err.println("Could not save session: " + e.getMessage());
        } finally {
            if (tmp != null) {
                try {
                    Files.deleteIfExists(tmp);
                } catch (IOException ignored) {
                    // best effort
                }
            }
        }
    }

    public String get(String key, String def) {
        return props.getProperty(key, def);
    }

    public int getInt(String key, int def) {
        try {
            return Integer.parseInt(props.getProperty(key));
        } catch (RuntimeException e) {
            return def;
        }
    }

    public double getDouble(String key, double def) {
        try {
            return Double.parseDouble(props.getProperty(key));
        } catch (RuntimeException e) {
            return def;
        }
    }

    public boolean getBool(String key, boolean def) {
        String v = props.getProperty(key);
        return v == null ? def : Boolean.parseBoolean(v);
    }

    public void put(String key, Object value) {
        props.setProperty(key, String.valueOf(value));
    }

    public void removePrefix(String prefix) {
        props.keySet().removeIf(k -> k.toString().startsWith(prefix));
    }
}
