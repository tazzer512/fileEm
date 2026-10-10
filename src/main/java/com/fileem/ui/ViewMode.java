package com.fileem.ui;

/**
 * How a tab shows its files, after Windows Explorer's View menu (same order, same Ctrl+Shift+1…8
 * shortcuts). Details is the table; the others are drawn by {@link FileGrid}.
 */
enum ViewMode {
    EXTRA_LARGE_ICONS("xl-icons", "Extra large icons", "mdi2v-view-agenda-outline", 192),
    LARGE_ICONS("large-icons", "Large icons", "mdi2v-view-grid-outline", 96),
    MEDIUM_ICONS("medium-icons", "Medium icons", "mdi2v-view-module-outline", 48),
    SMALL_ICONS("small-icons", "Small icons", "mdi2v-view-comfy-outline", 16),
    LIST("list", "List", "mdi2v-view-list-outline", 16),
    DETAILS("details", "Details", "mdi2t-table", 16),
    TILES("tiles", "Tiles", "mdi2v-view-dashboard-outline", 48),
    CONTENT("content", "Content", "mdi2v-view-stream-outline", 32);

    private final String id;
    private final String label;
    private final String icon;
    private final int iconSize;

    ViewMode(String id, String label, String icon, int iconSize) {
        this.id = id;
        this.label = label;
        this.icon = icon;
        this.iconSize = iconSize;
    }

    String id() {
        return id;
    }

    String label() {
        return label;
    }

    String icon() {
        return icon;
    }

    int iconSize() {
        return iconSize;
    }

    /** Icon views big enough to show image thumbnails instead of a file type icon. */
    boolean showsThumbnails() {
        return iconSize >= 48;
    }

    /** List fills columns top to bottom and scrolls sideways; the other grid views fill rows. */
    boolean flowsInColumns() {
        return this == LIST;
    }

    /** Big icon views put the name under the icon; the others put text beside it. */
    boolean textBelowIcon() {
        return this == EXTRA_LARGE_ICONS || this == LARGE_ICONS || this == MEDIUM_ICONS;
    }

    /** The mode with {@code id}, or Details for unknown or missing ids. */
    static ViewMode byId(String id) {
        for (ViewMode m : values()) {
            if (m.id.equals(id)) {
                return m;
            }
        }
        return DETAILS;
    }

    /** The next mode in Explorer's order, toward Content ({@code +1}) or Extra large icons ({@code -1}). */
    ViewMode step(int direction) {
        int i = Math.max(0, Math.min(values().length - 1, ordinal() + Integer.signum(direction)));
        return values()[i];
    }
}
