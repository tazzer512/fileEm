package com.fileem;

import com.fileem.util.FsUtil;
import javafx.collections.FXCollections;
import javafx.collections.ListChangeListener;
import javafx.collections.ObservableList;

import java.nio.file.InvalidPathException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;

/**
 * Ordered, named folder bookmarks shared by both sides. Every change is written to the session
 * file immediately, so bookmarks survive a crash.
 */
public final class Bookmarks {
    public record Bookmark(String name, Path path) {
    }

    private static final String PREFIX = "bookmarks.";

    private final Session session;
    private final ObservableList<Bookmark> items = FXCollections.observableArrayList();
    private final ObservableList<Bookmark> readOnly = FXCollections.unmodifiableObservableList(items);

    public Bookmarks(Session session) {
        this.session = session;
        load();
        items.addListener((ListChangeListener<Bookmark>) c -> save());
    }

    /** Live, read-only view in display order. */
    public ObservableList<Bookmark> items() {
        return readOnly;
    }

    public boolean contains(Path dir) {
        return indexOf(dir) >= 0;
    }

    public Optional<Bookmark> find(Path dir) {
        int i = indexOf(dir);
        return i < 0 ? Optional.empty() : Optional.of(items.get(i));
    }

    /** Adds {@code dir} named after the folder. Returns false when it is already bookmarked. */
    public boolean add(Path dir) {
        return add(dir, FsUtil.displayName(normalize(dir)));
    }

    public boolean add(Path dir, String name) {
        Path p = normalize(dir);
        if (contains(p)) {
            return false;
        }
        items.add(new Bookmark(cleanName(name, p), p));
        return true;
    }

    public void remove(Path dir) {
        int i = indexOf(dir);
        if (i >= 0) {
            items.remove(i);
        }
    }

    /** Renames a bookmark; a blank name falls back to the folder name. */
    public void rename(Path dir, String name) {
        int i = indexOf(dir);
        if (i >= 0) {
            Bookmark b = items.get(i);
            items.set(i, new Bookmark(cleanName(name, b.path()), b.path()));
        }
    }

    /** Moves a bookmark {@code delta} places up (negative) or down; clamped to the list. */
    public void move(Path dir, int delta) {
        int from = indexOf(dir);
        if (from < 0) {
            return;
        }
        int to = Math.max(0, Math.min(items.size() - 1, from + delta));
        if (to != from) {
            List<Bookmark> copy = new ArrayList<>(items);
            copy.add(to, copy.remove(from));
            items.setAll(copy); // one change, one save
        }
    }

    private int indexOf(Path dir) {
        Path p = normalize(dir);
        for (int i = 0; i < items.size(); i++) {
            if (items.get(i).path().equals(p)) {
                return i;
            }
        }
        return -1;
    }

    private static Path normalize(Path p) {
        return p.toAbsolutePath().normalize();
    }

    private static String cleanName(String name, Path path) {
        return name == null || name.isBlank() ? FsUtil.displayName(path) : name.strip();
    }

    private void load() {
        int count = session.getInt(PREFIX + "count", 0);
        for (int i = 0; i < count; i++) {
            String path = session.get(PREFIX + i + ".path", null);
            if (path == null || path.isBlank()) {
                continue;
            }
            try {
                Path p = normalize(Path.of(path));
                if (!contains(p)) {
                    items.add(new Bookmark(cleanName(session.get(PREFIX + i + ".name", null), p), p));
                }
            } catch (InvalidPathException ignored) {
                // skip entries that are not valid on this OS
            }
        }
    }

    private void save() {
        session.removePrefix(PREFIX);
        session.put(PREFIX + "count", items.size());
        for (int i = 0; i < items.size(); i++) {
            session.put(PREFIX + i + ".name", items.get(i).name());
            session.put(PREFIX + i + ".path", items.get(i).path().toString());
        }
        session.save();
    }
}
