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

    /** Icons at least this big show image thumbnails instead of a file type icon. */
    static final int THUMBNAIL_MIN = 48;


    /** List fills columns top to bottom and scrolls sideways; the other grid views fill rows. */
    boolean flowsInColumns() {
        return this == LIST;
    }

    /** Big icon views put the name under the icon; the others put text beside it. */
    boolean textBelowIcon() {
        return this == EXTRA_LARGE_ICONS || this == LARGE_ICONS || this == MEDIUM_ICONS;
    }

    /** Item sizes are bounded so the smallest icons stay legible and the biggest fit a window. */
    static final int MIN_ICON = 16;
    static final int MAX_ICON = 320;
    /** Each + / - step scales the icons by this factor. */
    static final double SIZE_STEP = 1.25;

    /** Views whose items can be made bigger or smaller with + / -: the icon views and Tiles. */
    boolean resizable() {
        return this == EXTRA_LARGE_ICONS || this == LARGE_ICONS || this == MEDIUM_ICONS
                || this == SMALL_ICONS || this == TILES;
    }

    /**
     * The icon size after {@code steps} size steps (0: the view's own size). Steps may be fractional
     * (the size slider); each whole step scales by {@link #SIZE_STEP}. Clamped to the limits.
     */
    int iconSize(double steps) {
        return resizable() ? (int) Math.round(iconSize * Math.pow(SIZE_STEP, clampSteps(steps))) : iconSize;
    }

    /** The fewest steps allowed (negative, or 0 when the view is already at the smallest size). */
    double minSteps() {
        return resizable() ? Math.min(0, Math.log((double) MIN_ICON / iconSize) / Math.log(SIZE_STEP)) : 0;
    }

    /** The most steps allowed. */
    double maxSteps() {
        return resizable() ? Math.max(0, Math.log((double) MAX_ICON / iconSize) / Math.log(SIZE_STEP)) : 0;
    }

    double clampSteps(double steps) {
        return Math.max(minSteps(), Math.min(maxSteps(), steps));
    }

    /** Whether one more step in {@code direction} (+1 bigger, -1 smaller) changes anything. */
    boolean canResize(double steps, int direction) {
        if (!resizable()) {
            return false;
        }
        return direction > 0 ? steps < maxSteps() - 1e-6 : steps > minSteps() + 1e-6;
    }

    /**
     * Size slider position (-1 … 1, 0 in the middle is the view's own size) to size steps: each half
     * covers the range to its limit, so the middle always means the default size.
     */
    double stepsForSlider(double position) {
        double p = Math.max(-1, Math.min(1, position));
        return p < 0 ? -p * minSteps() : p * maxSteps();
    }

    /** The slider position that shows {@code steps}. */
    double sliderForSteps(double steps) {
        double s = clampSteps(steps);
        if (s < 0) {
            return minSteps() == 0 ? 0 : -s / minSteps();
        }
        return maxSteps() == 0 ? 0 : s / maxSteps();
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
