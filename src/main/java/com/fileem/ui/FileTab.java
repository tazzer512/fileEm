package com.fileem.ui;

import atlantafx.base.controls.CustomTextField;
import atlantafx.base.theme.Styles;
import com.fileem.fs.DirWatcher;
import com.fileem.fs.FileEntry;
import com.fileem.util.Bg;
import com.fileem.util.Formats;
import com.fileem.util.FsUtil;
import com.fileem.util.OsUtil;
import javafx.animation.PauseTransition;
import javafx.beans.InvalidationListener;
import javafx.beans.binding.Bindings;
import javafx.collections.FXCollections;
import javafx.collections.ListChangeListener;
import javafx.collections.ObservableList;
import javafx.collections.transformation.FilteredList;
import javafx.collections.transformation.SortedList;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.Node;
import javafx.scene.control.Button;
import javafx.scene.control.Label;
import javafx.scene.control.MenuButton;
import javafx.scene.control.MenuItem;
import javafx.scene.control.Slider;
import javafx.scene.control.Tab;
import javafx.scene.control.Tooltip;
import javafx.scene.input.KeyCode;
import javafx.scene.input.KeyEvent;
import javafx.scene.input.ScrollEvent;
import javafx.scene.layout.BorderPane;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.util.Duration;
import org.kordamp.ikonli.javafx.FontIcon;

import java.io.IOException;
import java.nio.file.AccessDeniedException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.NoSuchFileException;
import java.nio.file.NotDirectoryException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.EnumMap;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.Future;
import java.util.function.Supplier;

/** One tab of a side: address bar, filter, file list and status line for a single folder. */
public final class FileTab extends BorderPane {
    private static final int MAX_HISTORY = 100;

    private final MainWindow main;
    private final ExplorerSide side;
    private final Tab tab = new Tab();
    private final ObservableList<FileEntry> master = FXCollections.observableArrayList();
    private final FilteredList<FileEntry> filtered = new FilteredList<>(master);
    private final SortedList<FileEntry> sorted = new SortedList<>(filtered);
    private final FileTable table;
    private final AddressBar address;
    private final CustomTextField filter = new CustomTextField();
    private final Label status = new Label();
    private final InvalidationListener hiddenListener = o -> updatePredicate();
    private final List<Path> history = new ArrayList<>();
    private final StringBuilder typeAhead = new StringBuilder();
    private long lastTyped;
    private ViewMode viewMode = ViewMode.DETAILS;
    /** + / - size steps per view (icon views and Tiles); missing means the view's own size. */
    private final Map<ViewMode, Double> sizeSteps = new EnumMap<>(ViewMode.class);
    /** Item size: the middle is the view's own size, each half runs to the smallest / biggest size. */
    private final Slider sizeSlider = new Slider(-1, 1, 0);
    private boolean updatingSlider;
    private final MenuButton viewButton = new MenuButton(null, Icons.icon(ViewMode.DETAILS.icon()));
    private final MenuButton sortButton = new MenuButton(null, Icons.icon("mdi2s-sort-ascending"));
    private final Button smaller = new Button(null, Icons.icon("mdi2m-magnify-minus-outline"));
    private final Button bigger = new Button(null, Icons.icon("mdi2m-magnify-plus-outline"));
    /** Created when a non-Details view is first used. */
    private FileGrid grid;
    private double wheelZoom;

    private Path currentDir;
    private int historyIndex = -1;
    private int loadGeneration;
    /** The running listing (navigation or refresh), cancelled when a newer one starts. */
    private Future<?> load;
    private boolean navigating;
    private boolean refreshing;
    private boolean refreshPending;
    private volatile long lastListMillis;
    private long freeSpace = -1;
    /** The watch on the shown folder. */
    private Watch watch;
    /** The watch the navigation in flight registered, before listing its folder. */
    private Watch nextWatch;
    /** Test hook: runs in a navigation's job right after it listed its folder. */
    static volatile java.util.function.Consumer<Path> afterListing;

    FileTab(MainWindow main, ExplorerSide side, Path initial) {
        this.main = main;
        this.side = side;
        getStyleClass().add("file-tab");

        table = new FileTable(main, this);
        sorted.comparatorProperty().bind(Bindings.createObjectBinding(table::effectiveComparator, table.comparatorProperty()));
        table.setSortPolicy(t -> true);
        table.setItems(sorted);
        table.setOnDragOver(e -> {
            if (Dnd.accept(e, currentDir)) {
                e.consume();
            }
        });
        table.setOnDragDropped(e -> {
            if (Dnd.canDrop(e, currentDir)) {
                main.drop(e, currentDir);
            }
        });

        address = new AddressBar(this::navigateTo, this::focusTable);

        filter.setPromptText("Filter (*, ?)");
        filter.setLeft(Icons.icon("mdi2m-magnify"));
        filter.setPrefColumnCount(12);
        filter.getStyleClass().add("filter-field");
        filter.textProperty().addListener(o -> updatePredicate());
        filter.addEventHandler(KeyEvent.KEY_PRESSED, e -> {
            if (e.getCode() == KeyCode.ESCAPE) {
                filter.clear();
                focusTable();
                e.consume();
            } else if (e.getCode() == KeyCode.ENTER || e.getCode() == KeyCode.DOWN) {
                focusTable();
                e.consume();
            }
        });

        setUpMenuButton(viewButton, "view-button", "View (Ctrl+Shift+1…8, Ctrl+wheel)",
                () -> main.viewItems(this));
        setUpMenuButton(sortButton, "sort-button", "Sort by", () -> main.sortItems(table));
        sorted.comparatorProperty().addListener(o -> updateSortButton());
        updateSortButton();

        HBox top = new HBox(6, address, viewButton, sortButton, filter);
        top.setAlignment(Pos.CENTER_LEFT);
        top.setPadding(new Insets(6, 6, 6, 6));
        HBox.setHgrow(address, Priority.ALWAYS);

        status.getStyleClass().add("status-label");
        status.setMaxWidth(Double.MAX_VALUE);
        for (Button b : List.of(smaller, bigger)) {
            b.getStyleClass().addAll(Styles.BUTTON_ICON, Styles.FLAT, "size-button");
            b.setFocusTraversable(false);
        }
        smaller.setTooltip(new Tooltip("Smaller items (Ctrl+−)"));
        bigger.setTooltip(new Tooltip("Bigger items (Ctrl++)"));
        smaller.setOnAction(e -> resizeItems(-1));
        bigger.setOnAction(e -> resizeItems(+1));
        sizeSlider.getStyleClass().addAll(Styles.SMALL, "size-slider");
        sizeSlider.setFocusTraversable(false);
        sizeSlider.setPrefWidth(110);
        sizeSlider.setMajorTickUnit(1); // a tick in the middle: the default size
        sizeSlider.setShowTickMarks(true);
        sizeSlider.setMinorTickCount(0);
        sizeSlider.setTooltip(new Tooltip("Item size (middle: default)"));
        sizeSlider.valueProperty().addListener((o, a, v) -> {
            if (!updatingSlider) {
                double p = Math.abs(v.doubleValue()) < 0.04 ? 0 : v.doubleValue(); // snaps to the middle
                setItemSize(viewMode.stepsForSlider(p));
            }
        });
        // double-click: back to the default size
        sizeSlider.setOnMouseClicked(e -> {
            if (e.getClickCount() == 2) {
                setItemSize(0);
            }
        });
        HBox.setHgrow(status, Priority.ALWAYS);
        HBox bottomBar = new HBox(2, status, smaller, sizeSlider, bigger);
        bottomBar.setAlignment(Pos.CENTER_LEFT);
        bottomBar.getStyleClass().add("status-bar");

        setTop(top);
        setCenter(table);
        setBottom(bottomBar);
        updateSizeButtons();
        // Ctrl+wheel steps through the view modes, as in Explorer
        addEventFilter(ScrollEvent.SCROLL, e -> {
            if (e.isShortcutDown() && e.getDeltaY() != 0 && isInside(e.getTarget(), getCenter())) {
                wheelZoom += e.getDeltaY();
                if (Math.abs(wheelZoom) >= 30) {
                    setViewMode(viewMode.step(wheelZoom > 0 ? -1 : 1));
                    wheelZoom = 0;
                }
                e.consume();
            }
        });

        tab.setContent(this);
        tab.setUserData(this);
        tab.setGraphic(Icons.icon("mdi2f-folder", 14));
        tab.getGraphic().getStyleClass().add("icon-folder");

        main.showHiddenProperty().addListener(hiddenListener);
        filtered.addListener((ListChangeListener<FileEntry>) c -> updateStatus());
        table.getSelectionModel().getSelectedItems().addListener((ListChangeListener<FileEntry>) c -> {
            updateStatus();
            main.selectionChanged(this);
        });
        table.getFocusModel().focusedIndexProperty().addListener(o -> main.selectionChanged(this));
        updatePredicate();

        // a restored folder may be gone, or on a mount that hangs: find the nearest existing one off the FX thread
        go(initial, null, true, 0, false, true);
    }

    Tab tab() {
        return tab;
    }

    ExplorerSide side() {
        return side;
    }

    public Path currentDir() {
        return currentDir;
    }

    FileTable table() {
        return table;
    }

    ViewMode viewMode() {
        return viewMode;
    }

    /** The icon/list view, or null while this tab has only used Details. */
    FileGrid grid() {
        return grid;
    }

    /** The node that shows the files in the current view mode. */
    javafx.scene.control.Control view() {
        return viewMode == ViewMode.DETAILS ? table : grid;
    }

    /** Switches how the files are shown; selection, focus and sort carry over. */
    void setViewMode(ViewMode m) {
        if (m == viewMode) {
            return;
        }
        boolean hadFocus = view().isFocused();
        viewMode = m;
        if (m == ViewMode.DETAILS) {
            setCenter(table);
        } else {
            if (grid == null) {
                grid = new FileGrid(main, this, table);
            }
            grid.setMode(m);
            grid.setSizeSteps(sizeSteps(m));
            setCenter(grid);
        }
        updateSizeButtons();
        ((FontIcon) viewButton.getGraphic()).setIconLiteral(m.icon());
        int lead = table.getFocusModel().getFocusedIndex();
        if (hadFocus) {
            focusTable();
        }
        if (lead >= 0) {
            javafx.application.Platform.runLater(() -> ensureVisible(lead));
        }
    }

    // ---- view and sort buttons (between the address bar and the filter) ----

    /** An icon-only menu button whose items are rebuilt on every opening, so the checks are current. */
    private void setUpMenuButton(MenuButton b, String styleClass, String tooltip, Supplier<List<MenuItem>> items) {
        b.getStyleClass().addAll(Styles.BUTTON_ICON, Styles.FLAT, styleClass);
        b.setTooltip(new Tooltip(tooltip));
        b.setFocusTraversable(false);
        b.getItems().setAll(items.get()); // never empty when it opens
        b.setOnShowing(e -> b.getItems().setAll(items.get()));
        b.setOnHidden(e -> focusTable());
    }

    /** The sort button shows the direction (ascending or descending). */
    private void updateSortButton() {
        boolean asc = table.sortAscending();
        ((FontIcon) sortButton.getGraphic()).setIconLiteral(asc ? "mdi2s-sort-ascending" : "mdi2s-sort-descending");
        String column = table.sortColumnId();
        String label = column == null ? null : FileTable.SORT_COLUMNS.getOrDefault(column, column);
        sortButton.getTooltip().setText(label == null ? "Sort by" : "Sort by " + label + (asc ? ", ascending" : ", descending"));
    }

    MenuButton viewButton() {
        return viewButton;
    }

    MenuButton sortButton() {
        return sortButton;
    }

    // ---- item size (- / slider / +) ----

    private double sizeSteps(ViewMode m) {
        return sizeSteps.getOrDefault(m, 0.0);
    }

    /** Whether the items of the current view can get bigger (+1) or smaller (-1). */
    boolean canResizeItems(int direction) {
        return viewMode.canResize(sizeSteps(viewMode), direction);
    }

    /** One size step bigger (+1) or smaller (-1), for the current view only (icon views and Tiles). */
    void resizeItems(int direction) {
        if (canResizeItems(direction)) {
            setItemSize(viewMode.clampSteps(sizeSteps(viewMode) + Integer.signum(direction)));
        }
    }

    /** Sets the current view's size in (possibly fractional) steps, 0 being its own size. */
    void setItemSize(double steps) {
        if (!viewMode.resizable()) {
            return;
        }
        double s = viewMode.clampSteps(steps);
        if (Math.abs(s) < 1e-6) {
            sizeSteps.remove(viewMode);
            s = 0;
        } else {
            sizeSteps.put(viewMode, s);
        }
        grid.setSizeSteps(s);
        updateSizeButtons();
    }

    /** The - / slider / + controls only show in views whose items can be resized, and follow the size. */
    private void updateSizeButtons() {
        boolean show = viewMode.resizable();
        for (Node n : List.of(smaller, sizeSlider, bigger)) {
            n.setVisible(show);
            n.setManaged(show);
        }
        smaller.setDisable(!canResizeItems(-1));
        bigger.setDisable(!canResizeItems(+1));
        updatingSlider = true;
        try {
            sizeSlider.setValue(viewMode.sliderForSteps(sizeSteps(viewMode)));
        } finally {
            updatingSlider = false;
        }
    }

    Button smallerButton() {
        return smaller;
    }

    Button biggerButton() {
        return bigger;
    }

    Slider sizeSlider() {
        return sizeSlider;
    }

    /** The sizes for the session: "large-icons:2,tiles:-0.5" (views at their own size are left out). */
    String sizeStepsState() {
        return sizeSteps.entrySet().stream()
                .map(e -> e.getKey().id() + ":" + String.format(Locale.ROOT, "%.3f", e.getValue())
                        .replaceAll("\\.?0+$", ""))
                .collect(java.util.stream.Collectors.joining(","));
    }

    /** Restores {@link #sizeStepsState()}; unknown views are ignored, sizes beyond the limits are clamped. */
    void restoreSizeSteps(String state) {
        sizeSteps.clear();
        if (state != null && !state.isBlank()) {
            for (String part : state.split(",")) {
                String[] kv = part.split(":");
                if (kv.length != 2) {
                    continue;
                }
                ViewMode m = ViewMode.byId(kv[0].strip());
                try {
                    double steps = m.clampSteps(Double.parseDouble(kv[1].strip()));
                    if (m.resizable() && m.id().equals(kv[0].strip()) && Math.abs(steps) > 1e-6
                            && Double.isFinite(steps)) {
                        sizeSteps.put(m, steps);
                    }
                } catch (NumberFormatException ignored) {
                    // skip
                }
            }
        }
        if (grid != null) {
            grid.setSizeSteps(sizeSteps(viewMode));
        }
        updateSizeButtons();
    }

    private static boolean isInside(Object target, Node container) {
        for (Node n = target instanceof Node node ? node : null; n != null; n = n.getParent()) {
            if (n == container) {
                return true;
            }
        }
        return false;
    }

    boolean canGoBack() {
        return historyIndex > 0;
    }

    boolean canGoForward() {
        return historyIndex >= 0 && historyIndex < history.size() - 1;
    }

    boolean canGoUp() {
        return currentDir != null && currentDir.getParent() != null;
    }

    // ---- navigation ----

    /** Navigates to a folder; a file path opens its folder with the file selected. */
    public void navigateTo(Path p) {
        if (p == null) {
            return;
        }
        go(p, null, true, 0, true); // whether p is a file is checked off the FX thread (slow mounts)
    }

    /** Opens the folder holding {@code p} with {@code p} selected (a file or a folder). */
    void showItem(Path p) {
        if (p != null && p.getParent() != null) {
            go(p.getParent(), p, true);
        }
    }

    void back() {
        if (canGoBack()) {
            go(history.get(historyIndex - 1), null, false, -1);
        }
    }

    void forward() {
        if (canGoForward()) {
            go(history.get(historyIndex + 1), null, false, +1);
        }
    }

    void up() {
        if (canGoUp()) {
            go(currentDir.getParent(), currentDir, true);
        }
    }

    void refresh() {
        refresh(null);
    }

    /**
     * Refresh requested by the directory watcher. Coalesced: while a listing runs, further requests
     * collapse into one follow-up, delayed relative to how long listing takes (big folders under
     * heavy writes would otherwise be re-listed continuously).
     */
    void requestRefresh() {
        if (refreshing) {
            refreshPending = true;
            return;
        }
        refresh();
    }

    /** Reloads the listing, keeping the selection (or selecting {@code select} when given). */
    void refresh(Path select) {
        if (currentDir == null || navigating) {
            return; // a navigation in flight lists its folder fresh anyway; don't drop it
        }
        Path dir = currentDir;
        List<Path> keep = select != null ? List.of(select) : selectedPaths();
        int gen = ++loadGeneration;
        refreshing = true;
        startLoad(Bg.run(() -> timedList(dir), entries -> {
            finishRefresh();
            if (gen != loadGeneration) {
                return;
            }
            boolean foldersChanged = !folderPaths(master).equals(folderPaths(entries));
            master.setAll(entries);
            restoreSelection(keep, select != null);
            updateStatus();
            if (foldersChanged) {
                main.treesRefresh(dir);
            }
        }, err -> {
            finishRefresh();
            if (gen != loadGeneration) {
                return;
            }
            // the folder is gone: go to its nearest existing parent (found off the FX thread)
            go(dir, null, true, 0, false, true);
        }));
    }

    /** Starts a listing, cancelling the previous one: its result is stale and its work wasted. */
    private void startLoad(Future<?> next) {
        if (load != null) {
            load.cancel(true);
        }
        load = next;
    }

    private record Listing(Path dir, Path select, List<FileEntry> entries) {
    }

    private void finishRefresh() {
        refreshing = false;
        if (refreshPending) {
            refreshPending = false;
            PauseTransition delay = new PauseTransition(Duration.millis(Math.max(250, 2 * lastListMillis)));
            delay.setOnFinished(e -> requestRefresh());
            delay.play();
        }
    }

    private static Set<Path> folderPaths(List<FileEntry> entries) {
        Set<Path> out = new HashSet<>();
        for (FileEntry e : entries) {
            if (e.isDirectory()) {
                out.add(e.path());
            }
        }
        return out;
    }

    private void go(Path dir, Path select, boolean record) {
        go(dir, select, record, 0);
    }

    private void go(Path dir, Path select, boolean record, int historyStep) {
        go(dir, select, record, historyStep, false, false);
    }

    private void go(Path dir, Path select, boolean record, int historyStep, boolean mayBeFile) {
        go(dir, select, record, historyStep, mayBeFile, false);
    }

    /**
     * Loads a folder. All file system checks run in the background job: {@code mayBeFile} opens the
     * parent of a file with the file selected; {@code nearestExisting} walks up to a folder that exists.
     */
    private void go(Path dir, Path select, boolean record, int historyStep, boolean mayBeFile, boolean nearestExisting) {
        Path requested = dir.toAbsolutePath().normalize();
        Path previous = currentDir;
        int gen = ++loadGeneration;
        navigating = true;
        // a refresh still running is cancelled below and never calls back: clear its state here, or
        // requestRefresh would wait for it forever and the watcher would stop updating this tab
        refreshing = false;
        refreshPending = false;
        status.setText("Loading…");
        if (nextWatch != null) {
            nextWatch.close(); // its navigation is cancelled below
        }
        Watch w = new Watch();
        nextWatch = w;
        startLoad(Bg.run(() -> {
            Path target = requested;
            Path sel = select;
            if (nearestExisting) {
                while (target != null && !Files.isDirectory(target)) {
                    target = target.getParent();
                }
                if (target == null) {
                    throw new NoSuchFileException(requested.toString());
                }
            } else if (mayBeFile && requested.getParent() != null && Files.isRegularFile(requested)) {
                sel = requested;
                target = requested.getParent();
            }
            // watched before it is listed: a change in between is caught, not lost
            w.attach(DirWatcher.watch(target, () -> watchReported(w)));
            List<FileEntry> entries = timedList(target);
            java.util.function.Consumer<Path> hook = afterListing;
            if (hook != null) {
                hook.accept(target);
            }
            return new Listing(target, sel, entries);
        }, listing -> {
            if (gen != loadGeneration) {
                w.close();
                return;
            }
            navigating = false;
            closeWatch();
            watch = w;
            nextWatch = null;
            Path target = listing.dir();
            List<FileEntry> entries = listing.entries();
            Path selectAfter = listing.select() != null ? listing.select()
                    : previous != null && target.equals(previous.getParent()) ? previous : null;
            boolean changed = !target.equals(previous);
            currentDir = target;
            if (record) {
                pushHistory(target);
            } else {
                historyIndex += historyStep;
            }
            if (changed) {
                filter.clear();
                onDirectoryChanged();
            }
            master.setAll(entries);
            if (selectAfter != null) {
                restoreSelection(List.of(selectAfter), true);
            } else {
                table.getSelectionModel().clearSelection();
                if (!sorted.isEmpty()) {
                    table.getFocusModel().focus(0);
                    table.scrollTo(0);
                }
                if (grid != null) {
                    grid.scrollToStart();
                }
            }
            updateStatus();
            side.tabNavigated(this);
            if (w.changed) {
                requestRefresh(); // changed while it was listed
            }
        }, err -> {
            w.close();
            if (gen != loadGeneration) {
                return;
            }
            navigating = false;
            nextWatch = null;
            updateStatus();
            if (previous == null) {
                Path home = OsUtil.home();
                if (!requested.equals(home)) {
                    go(home, null, true);
                }
                return;
            }
            if (nearestExisting) {
                return; // nothing above the vanished folder exists either; keep showing what we have
            }
            Dialogs.error(getScene() != null ? getScene().getWindow() : null, "Open folder", describe(requested, err));
        }));
    }

    private static String describe(Path dir, Exception e) {
        if (e instanceof AccessDeniedException) {
            return "Access denied:\n" + dir;
        }
        if (e instanceof NoSuchFileException) {
            return "The folder does not exist:\n" + dir;
        }
        if (e instanceof NotDirectoryException) {
            return "Not a folder:\n" + dir;
        }
        return "Cannot open " + dir + ":\n" + e.getMessage();
    }

    private List<FileEntry> timedList(Path dir) throws IOException {
        long start = System.nanoTime();
        List<FileEntry> entries = list(dir);
        lastListMillis = (System.nanoTime() - start) / 1_000_000;
        return entries;
    }

    private static List<FileEntry> list(Path dir) throws IOException {
        List<FileEntry> out = new ArrayList<>();
        try (DirectoryStream<Path> ds = Files.newDirectoryStream(dir)) {
            for (Path p : ds) {
                if ((out.size() & 255) == 0) {
                    Bg.checkInterrupted(); // navigated away: stop reading a big folder
                }
                out.add(FileEntry.read(p));
            }
        }
        return out;
    }

    private void pushHistory(Path dir) {
        if (historyIndex >= 0 && history.get(historyIndex).equals(dir)) {
            return;
        }
        while (history.size() > historyIndex + 1) {
            history.remove(history.size() - 1);
        }
        history.add(dir);
        if (history.size() > MAX_HISTORY) {
            history.remove(0);
        }
        historyIndex = history.size() - 1;
    }

    private void onDirectoryChanged() {
        address.setPath(currentDir);
        String name = FsUtil.displayName(currentDir);
        tab.setText(name);
        tab.setTooltip(new Tooltip(currentDir.toString()));
        Path watched = currentDir;
        freeSpace = -1;
        Bg.run(() -> Files.getFileStore(watched).getUsableSpace(), free -> {
            if (watched.equals(currentDir)) {
                freeSpace = free;
                updateStatus();
            }
        }, null);
    }

    /** A report of the watcher (FX thread): the shown folder refreshes; one still loading notes it. */
    private void watchReported(Watch w) {
        if (w == watch) {
            requestRefresh();
        } else if (w == nextWatch) {
            w.changed = true;
        }
    }

    private void closeWatch() {
        if (watch != null) {
            watch.close();
            watch = null;
        }
    }

    /**
     * A watch registered in a navigation's background job. A navigation cancelled before it
     * calls back closes it through {@link #nextWatch}, also when the registration arrives late.
     */
    private static final class Watch {
        private AutoCloseable handle;
        private boolean closed;
        /** A change was reported before the listing was shown (FX thread). */
        boolean changed;

        synchronized void attach(AutoCloseable h) {
            if (closed) {
                closeQuietly(h);
            } else {
                handle = h;
            }
        }

        synchronized void close() {
            closed = true;
            if (handle != null) {
                closeQuietly(handle);
                handle = null;
            }
        }
    }

    /** Whether the directory watcher is registered for the current folder (test hook). */
    boolean isWatching() {
        return watch != null;
    }

    private static void closeQuietly(AutoCloseable c) {
        try {
            c.close();
        } catch (Exception ignored) {
            // nothing to do
        }
    }

    void dispose() {
        loadGeneration++;
        startLoad(null);
        closeWatch();
        if (nextWatch != null) {
            nextWatch.close();
            nextWatch = null;
        }
        main.showHiddenProperty().removeListener(hiddenListener);
    }

    // ---- selection & focus ----

    public List<Path> selectedPaths() {
        return table.getSelectionModel().getSelectedItems().stream()
                .filter(Objects::nonNull).map(FileEntry::path).toList();
    }

    /** What the preview shows: the focused item when it is selected, else the first selected one. */
    FileEntry previewEntry() {
        FileEntry focused = table.getFocusModel().getFocusedItem();
        if (focused != null && table.getSelectionModel().getSelectedItems().contains(focused)) {
            return focused;
        }
        return table.getSelectionModel().getSelectedItems().stream().filter(Objects::nonNull).findFirst().orElse(null);
    }

    List<FileEntry> selectedEntries() {
        return table.getSelectionModel().getSelectedItems().stream().filter(Objects::nonNull).toList();
    }

    private void restoreSelection(Collection<Path> paths, boolean scroll) {
        Set<Path> wanted = new HashSet<>(paths);
        var sm = table.getSelectionModel();
        sm.clearSelection();
        int[] indices = new int[Math.min(wanted.size(), sorted.size())];
        int found = 0;
        for (int i = 0; i < sorted.size() && found < indices.length; i++) {
            if (wanted.contains(sorted.get(i).path())) {
                indices[found++] = i;
            }
        }
        int first = found > 0 ? indices[0] : -1;
        if (found > 0) {
            // one selection change, not one per item: every change re-runs the selection listeners
            sm.selectIndices(first, Arrays.copyOfRange(indices, 1, found));
        }
        if (first >= 0) {
            table.getFocusModel().focus(first);
            if (scroll) {
                ensureVisible(first);
            }
        }
    }

    void selectAll() {
        table.getSelectionModel().selectAll();
    }

    /** Scrolls the current view just enough to show item {@code index}. */
    void ensureVisible(int index) {
        if (viewMode == ViewMode.DETAILS) {
            table.ensureVisible(index);
        } else {
            grid.ensureVisible(index);
        }
    }

    /** Space: selects or deselects the focused item, keeping the rest of the selection. */
    void toggleFocused() {
        int i = table.getFocusModel().getFocusedIndex();
        if (i < 0) {
            return;
        }
        var sm = table.getSelectionModel();
        if (sm.isSelected(i)) {
            sm.clearSelection(i);
        } else {
            sm.select(i);
        }
        table.getFocusModel().focus(i);
    }

    /** Whether a type-ahead search is going on (a space then belongs to the name typed). */
    boolean typingAhead() {
        return !typeAhead.isEmpty() && System.currentTimeMillis() - lastTyped <= 1000;
    }

    /** Type-ahead find in either view: typing a name's first letters selects it. */
    void typeAhead(KeyEvent e) {
        String ch = e.getCharacter();
        if (ch.isEmpty() || e.isControlDown() || e.isAltDown() || e.isMetaDown() || e.isShortcutDown()) {
            return;
        }
        char c = ch.charAt(0);
        if (Character.isISOControl(c)) {
            return;
        }
        long now = System.currentTimeMillis();
        if (now - lastTyped > 1000) {
            typeAhead.setLength(0);
        }
        lastTyped = now;
        if (typeAhead.isEmpty() && c == ' ') {
            return;
        }
        typeAhead.append(ch);
        String prefix = typeAhead.toString().toLowerCase(Locale.ROOT);
        for (int i = 0; i < sorted.size(); i++) {
            if (sorted.get(i).name().toLowerCase(Locale.ROOT).startsWith(prefix)) {
                table.getSelectionModel().clearAndSelect(i);
                table.getFocusModel().focus(i);
                ensureVisible(i);
                break;
            }
        }
        e.consume();
    }

    void focusTable() {
        view().requestFocus();
        if (table.getFocusModel().getFocusedIndex() < 0 && !sorted.isEmpty()) {
            table.getFocusModel().focus(0);
        }
    }

    void editAddress() {
        address.edit();
    }

    void focusFilter() {
        filter.requestFocus();
        filter.selectAll();
    }

    // ---- filter & status ----

    private void updatePredicate() {
        boolean showHidden = main.showHiddenProperty().get();
        var matcher = FsUtil.nameMatcher(filter.getText());
        filtered.setPredicate(e -> (showHidden || !e.isHidden()) && matcher.test(e.name()));
    }

    private void updateStatus() {
        int n = filtered.size();
        StringBuilder sb = new StringBuilder(Formats.count(n)).append(n == 1 ? " item" : " items");
        int hiddenCount = master.size() - n;
        if (hiddenCount > 0) {
            sb.append(" (").append(Formats.count(hiddenCount)).append(filter.getText().isBlank() ? " hidden)" : " filtered out)");
        }
        List<FileEntry> sel = selectedEntries();
        if (!sel.isEmpty()) {
            long bytes = sel.stream().filter(e -> !e.isDirectory()).mapToLong(FileEntry::size).sum();
            sb.append("     ").append(Formats.count(sel.size())).append(" selected");
            if (bytes > 0) {
                sb.append("  ·  ").append(Formats.size(bytes));
            }
        }
        if (freeSpace >= 0) {
            sb.append("     ").append(Formats.size(freeSpace)).append(" free");
        }
        status.setText(sb.toString());
    }
}
