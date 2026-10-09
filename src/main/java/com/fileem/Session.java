package com.fileem;

import com.fileem.util.OsUtil;

import java.io.IOException;
import java.io.Reader;
import java.io.Writer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Properties;

/** Window layout, open tabs and column settings, persisted as a properties file in the OS config dir. */
public final class Session {
    private final Path file;
    private final Properties props = new Properties();

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

    public void save() {
        try {
            Files.createDirectories(file.getParent());
            try (Writer w = Files.newBufferedWriter(file, StandardCharsets.UTF_8)) {
                props.store(w, "fileEm session");
            }
        } catch (IOException e) {
            System.err.println("Could not save session: " + e.getMessage());
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
