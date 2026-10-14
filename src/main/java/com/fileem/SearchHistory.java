package com.fileem;

import java.util.ArrayList;
import java.util.List;

/**
 * The last {@value #MAX} search patterns, newest first, shared by every search window. Each change
 * goes into the session ({@code search.history.<i>}); {@link Autosave} writes it to disk.
 */
public final class SearchHistory {
    public static final int MAX = 15;
    private static final String PREFIX = "search.history.";

    private final Session session;
    private final List<String> items = new ArrayList<>();

    public SearchHistory(Session session) {
        this.session = session;
        for (int i = 0; i < MAX; i++) {
            String p = session.get(PREFIX + i, null);
            if (p != null && !p.isBlank() && !items.contains(p.strip())) {
                items.add(p.strip());
            }
        }
    }

    /** Newest first. */
    public List<String> items() {
        return List.copyOf(items);
    }

    /** Puts {@code pattern} first (moving it up when it was used before), keeping the newest {@value #MAX}. */
    public void add(String pattern) {
        if (pattern == null || pattern.isBlank()) {
            return;
        }
        String p = pattern.strip();
        if (!items.isEmpty() && items.get(0).equals(p)) {
            return;
        }
        items.remove(p);
        items.add(0, p);
        while (items.size() > MAX) {
            items.remove(items.size() - 1);
        }
        session.removePrefix(PREFIX);
        for (int i = 0; i < items.size(); i++) {
            session.put(PREFIX + i, items.get(i));
        }
    }
}
