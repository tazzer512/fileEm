package com.fileem.ui;

import com.fileem.fs.FileEntry;
import com.fileem.util.Formats;
import javafx.animation.AnimationTimer;
import javafx.collections.ListChangeListener;
import javafx.css.PseudoClass;
import javafx.geometry.HPos;
import javafx.geometry.Insets;
import javafx.geometry.Orientation;
import javafx.geometry.VPos;
import javafx.scene.control.Control;
import javafx.scene.control.Label;
import javafx.scene.control.OverrunStyle;
import javafx.scene.control.ScrollBar;
import javafx.scene.control.SkinBase;
import javafx.scene.control.TableView;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;
import javafx.scene.input.ClipboardContent;
import javafx.scene.input.DragEvent;
import javafx.scene.input.Dragboard;
import javafx.scene.input.KeyCode;
import javafx.scene.input.KeyEvent;
import javafx.scene.input.MouseButton;
import javafx.scene.input.MouseEvent;
import javafx.scene.input.ScrollEvent;
import javafx.scene.input.TransferMode;
import javafx.scene.layout.Pane;
import javafx.scene.layout.Region;
import javafx.scene.layout.StackPane;
import javafx.scene.shape.Rectangle;
import javafx.scene.text.Text;
import javafx.scene.text.TextAlignment;
import org.kordamp.ikonli.javafx.FontIcon;

import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.Deque;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * The icon, list, tiles and content views of a tab (every {@link ViewMode} but Details). It shows the
 * tab's {@link FileTable} items and uses the table's selection and focus models, so commands, the
 * status line and selection restore work the same in every view.
 * <p>
 * Virtualized: only items in view get a node, and nodes are reused while scrolling. Items sit in fixed
 * slots ({@code cellW × cellH}); List fills columns and scrolls sideways, the other views fill rows.
 */
final class FileGrid extends Control {
    private static final PseudoClass SELECTED = PseudoClass.getPseudoClass("selected");
    private static final PseudoClass LEAD = PseudoClass.getPseudoClass("lead");
    private static final PseudoClass HIDDEN = PseudoClass.getPseudoClass("hidden-file");
    private static final double PAD = 6;
    private static final double DRAG_THRESHOLD = 4;

    private final MainWindow main;
    private final FileTab tab;
    private final TableView<FileEntry> model;
    private final ScrollBar bar = new ScrollBar();
    private final Pane layer = new Pane();
    private final Rectangle clip = new Rectangle();
    private final Rectangle marquee = new Rectangle();
    private final Label placeholder = new Label("This folder is empty.");
    private final Map<Integer, ItemCell> active = new HashMap<>();
    private final Deque<ItemCell> pool = new ArrayDeque<>();
    private final AnimationTimer autoScroll = new AnimationTimer() {
        @Override
        public void handle(long now) {
            autoScrollStep();
        }
    };

    private ViewMode mode = ViewMode.LARGE_ICONS;
    private boolean dirty = true;
    /** List view column width; measured once per change of the items (-1 = stale). */
    private double listWidth = -1;
    // geometry of the last layout
    private double cellW;
    private double cellH;
    private double viewW;
    private double viewH;
    /** Slots across the non-scrolling axis: columns (row flow) or rows (List). */
    private int lanes = 1;

    // mouse state
    private int anchor = -1;
    private int pressedIndex = -1;
    private boolean deferredSelect;
    private boolean dragStarted;
    private boolean bandArmed;
    private boolean bandActive;
    private double pressX;
    private double pressY;
    private double bandAnchorX;
    private double bandAnchorY;
    private double mouseX;
    private double mouseY;
    private boolean bandToggle;
    private BitSet bandBase = new BitSet();
    private ItemCell dropCell;

    FileGrid(MainWindow main, FileTab tab, TableView<FileEntry> model) {
        this.main = main;
        this.tab = tab;
        this.model = model;
        getStyleClass().add("file-grid");
        setFocusTraversable(true);

        layer.setClip(clip);
        layer.setManaged(false);
        marquee.getStyleClass().add("marquee");
        marquee.setManaged(false);
        marquee.setMouseTransparent(true);
        marquee.setVisible(false);
        placeholder.getStyleClass().add("grid-placeholder");
        placeholder.setManaged(false);
        bar.setManaged(false);
        bar.valueProperty().addListener(o -> requestLayout());

        model.getItems().addListener((ListChangeListener<FileEntry>) c -> markDirty());
        model.itemsProperty().addListener((o, a, b) -> markDirty());
        model.getSelectionModel().getSelectedIndices().addListener((ListChangeListener<Integer>) c -> refreshStates());
        model.getFocusModel().focusedIndexProperty().addListener(o -> refreshStates());
        focusedProperty().addListener(o -> refreshStates());

        addEventHandler(MouseEvent.MOUSE_PRESSED, this::onPressed);
        addEventHandler(MouseEvent.MOUSE_DRAGGED, this::onDragged);
        addEventHandler(MouseEvent.DRAG_DETECTED, this::onDragDetected);
        addEventHandler(MouseEvent.MOUSE_RELEASED, this::onReleased);
        addEventHandler(MouseEvent.MOUSE_CLICKED, this::onClicked);
        addEventHandler(ScrollEvent.SCROLL, this::onScroll);
        addEventHandler(KeyEvent.KEY_PRESSED, this::onKey);
        addEventHandler(KeyEvent.KEY_TYPED, tab::typeAhead);
        addEventHandler(DragEvent.DRAG_OVER, this::onDragOver);
        addEventHandler(DragEvent.DRAG_DROPPED, this::onDragDropped);
        addEventHandler(DragEvent.DRAG_EXITED, e -> setDropCell(null));

        Views.dynamicContextMenu(this, () -> main.fileMenuItems(tab));
    }

    @Override
    protected javafx.scene.control.Skin<?> createDefaultSkin() {
        return new GridSkin(this);
    }

    ViewMode mode() {
        return mode;
    }

    /** + / - size steps of the current view (0: its own size). */
    private int sizeSteps;

    /** Makes the items {@code steps} size steps bigger (or smaller, negative) than the view's own size. */
    void setSizeSteps(int steps) {
        if (steps == sizeSteps) {
            return;
        }
        sizeSteps = steps;
        // cells take their icon size when created, like their mode: start with fresh ones
        layer.getChildren().removeIf(n -> n instanceof ItemCell);
        active.clear();
        pool.clear();
        markDirty();
        int lead = model.getFocusModel().getFocusedIndex();
        if (lead >= 0) {
            javafx.application.Platform.runLater(() -> ensureVisible(lead));
        }
    }

    int sizeSteps() {
        return sizeSteps;
    }

    /** The icon size of the current view and size steps. */
    int iconPx() {
        return mode.iconSize(sizeSteps);
    }

    void setMode(ViewMode m) {
        if (m == mode) {
            return;
        }
        mode = m;
        bar.setValue(0);
        layer.getChildren().removeIf(n -> n instanceof ItemCell);
        active.clear();
        pool.clear();
        markDirty();
    }

    private List<FileEntry> items() {
        return model.getItems();
    }

    private void markDirty() {
        dirty = true;
        listWidth = -1;
        requestLayout();
    }

    // ---- geometry ----

    private boolean columns() {
        return mode.flowsInColumns();
    }

    /** Lays out the visible items; called by the skin. */
    private void layoutGrid(double x, double y, double w, double h) {
        int n = items().size();
        double barSize = bar.prefWidth(-1);
        measureCells(w, n);
        // Work out the viewport, leaving room for the scroll bar only when the content overflows.
        viewW = w;
        viewH = h;
        updateLanes();
        boolean overflow = contentLength(n) > (columns() ? viewW : viewH);
        if (overflow) {
            if (columns()) {
                viewH = Math.max(0, h - barSize);
            } else {
                viewW = Math.max(0, w - barSize);
                measureCells(viewW, n);
            }
            updateLanes();
        }
        double content = contentLength(n);
        double view = columns() ? viewW : viewH;
        bar.setOrientation(columns() ? Orientation.HORIZONTAL : Orientation.VERTICAL);
        bar.setVisible(overflow);
        bar.setMax(Math.max(0, content - view));
        bar.setVisibleAmount(view);
        bar.setUnitIncrement(columns() ? cellW : cellH);
        bar.setBlockIncrement(view * 0.9);
        bar.setValue(clamp(bar.getValue(), 0, bar.getMax()));
        if (columns()) {
            bar.resizeRelocate(x, y + h - barSize, w, barSize);
        } else {
            bar.resizeRelocate(x + w - barSize, y, barSize, h);
        }
        layer.resizeRelocate(x, y, viewW, viewH);
        clip.setWidth(viewW);
        clip.setHeight(viewH);

        placeholder.setVisible(n == 0);
        if (n == 0) {
            placeholder.autosize();
            placeholder.relocate(x + (w - placeholder.getWidth()) / 2, y + 24);
        }

        int first;
        int last;
        double offset = bar.getValue();
        int firstLine = (int) Math.max(0, Math.floor((offset - PAD) / slotAlong()));
        int lastLine = (int) Math.floor((offset + view - PAD) / slotAlong());
        first = firstLine * lanes;
        last = Math.min(n - 1, (lastLine + 1) * lanes - 1);

        // recycle cells that scrolled away
        var it = active.entrySet().iterator();
        while (it.hasNext()) {
            var e = it.next();
            if (e.getKey() < first || e.getKey() > last || e.getKey() >= n) {
                e.getValue().setVisible(false);
                pool.push(e.getValue());
                it.remove();
            }
        }
        var sm = model.getSelectionModel();
        int lead = model.getFocusModel().getFocusedIndex();
        for (int i = first; i <= last; i++) {
            ItemCell cell = active.get(i);
            FileEntry entry = items().get(i);
            if (cell == null) {
                cell = pool.isEmpty() ? newCell() : pool.pop();
                active.put(i, cell);
                cell.setVisible(true);
                cell.show(entry);
            } else if (dirty || cell.entry != entry) {
                cell.show(entry);
            }
            cell.pseudoClassStateChanged(SELECTED, sm.isSelected(i));
            cell.pseudoClassStateChanged(LEAD, i == lead);
            cell.resizeRelocate(slotX(i), slotY(i), cellW, cellH);
        }
        dirty = false;
        if (bandActive) {
            updateBand();
        }
        marquee.toFront();
    }

    private ItemCell newCell() {
        ItemCell c = new ItemCell();
        c.setManaged(false); // the grid sizes and places cells; the layer pane must not autosize them
        layer.getChildren().add(c);
        return c;
    }

    /** Slot size per view (grown or shrunk with the icon for + / -); List fits the longest names (within limits). */
    private void measureCells(double width, int n) {
        measureBaseCells(width, n);
        if (mode.resizable()) {
            int delta = iconPx() - mode.iconSize(); // the text area keeps its size
            cellW += delta;
            cellH += delta;
            if (mode == ViewMode.TILES) {
                cellH = Math.max(cellH, 56); // three lines of text
            } else if (mode == ViewMode.SMALL_ICONS) {
                cellH = Math.max(cellH, 24);
            }
        }
    }

    private void measureBaseCells(double width, int n) {
        switch (mode) {
            case EXTRA_LARGE_ICONS -> {
                cellW = 228;
                cellH = 244;
            }
            case LARGE_ICONS -> {
                cellW = 124;
                cellH = 146;
            }
            case MEDIUM_ICONS -> {
                cellW = 92;
                cellH = 98;
            }
            case SMALL_ICONS -> {
                cellW = 220;
                cellH = 24;
            }
            case LIST -> {
                if (listWidth < 0) {
                    listWidth = listColumnWidth();
                }
                cellW = listWidth;
                cellH = 24;
            }
            case TILES -> {
                cellW = 264;
                cellH = 64;
            }
            case CONTENT -> {
                cellW = Math.max(200, width - 2 * PAD);
                cellH = 56;
            }
            default -> throw new IllegalStateException("Details is the table");
        }
    }

    private double listColumnWidth() {
        // Measure only the longest names: measuring every name of a big folder costs too much.
        // one pass for the longest length, a second for names close to it (wide letters vary)
        int maxLen = 0;
        for (FileEntry e : items()) {
            maxLen = Math.max(maxLen, e.name().length());
        }
        List<String> longest = new ArrayList<>();
        for (FileEntry e : items()) {
            if (e.name().length() >= maxLen * 0.8 && longest.size() < 24) {
                longest.add(e.name());
            }
        }
        Text probe = new Text();
        probe.setFont(placeholder.getFont());
        double max = 0;
        for (String s : longest) {
            probe.setText(s);
            max = Math.max(max, probe.getLayoutBounds().getWidth());
        }
        return clamp(max + 16 + 6 + 16, 140, 420);
    }

    private void updateLanes() {
        double across = columns() ? viewH : viewW;
        double slot = columns() ? cellH : cellW;
        lanes = Math.max(1, (int) Math.floor((across - 2 * PAD) / slot));
    }

    /** Size of one line of slots along the scroll axis. */
    private double slotAlong() {
        return columns() ? cellW : cellH;
    }

    private double contentLength(int n) {
        int lines = (n + lanes - 1) / lanes;
        return 2 * PAD + lines * slotAlong();
    }

    /** Slot position in the viewport. */
    private double slotX(int i) {
        if (columns()) {
            return PAD + (i / lanes) * cellW - bar.getValue();
        }
        return PAD + (i % lanes) * cellW;
    }

    private double slotY(int i) {
        if (columns()) {
            return PAD + (i % lanes) * cellH;
        }
        return PAD + (i / lanes) * cellH - bar.getValue();
    }

    /** The item at a point of the viewport, or -1 for empty space. */
    int indexAt(double x, double y) {
        double offset = bar.getValue();
        double cx = x + (columns() ? offset : 0) - PAD;
        double cy = y + (columns() ? 0 : offset) - PAD;
        if (cx < 0 || cy < 0) {
            return -1;
        }
        int col = (int) (cx / cellW);
        int row = (int) (cy / cellH);
        int i;
        if (columns()) {
            if (row >= lanes) {
                return -1;
            }
            i = col * lanes + row;
        } else {
            if (col >= lanes) {
                return -1;
            }
            i = row * lanes + col;
        }
        return i < items().size() ? i : -1;
    }

    /** Scrolls just enough to show item {@code i}. */
    void ensureVisible(int i) {
        if (i < 0 || i >= items().size()) {
            return;
        }
        if (getWidth() <= 0) {
            // not laid out yet: retry once it is
            javafx.application.Platform.runLater(() -> {
                if (getWidth() > 0) {
                    ensureVisible(i);
                }
            });
            return;
        }
        layout();
        double start = PAD + (i / lanes) * slotAlong();
        double end = start + slotAlong();
        double view = columns() ? viewW : viewH;
        double value = bar.getValue();
        if (start - PAD < value) {
            bar.setValue(Math.max(0, start - PAD));
        } else if (end + PAD > value + view) {
            bar.setValue(Math.min(bar.getMax(), end + PAD - view));
        }
    }

    void scrollToStart() {
        bar.setValue(0);
    }

    /** Package-private test hooks. */
    ItemCell cellFor(int index) {
        layout();
        return active.get(index);
    }

    /** Items per row (row flow) or per column (List). */
    int lanes() {
        layout();
        return lanes;
    }

    List<Integer> visibleIndices() {
        layout();
        return active.keySet().stream().sorted().toList();
    }

    /** Center of item {@code i} in screen coordinates. */
    javafx.geometry.Point2D screenCenterOf(int i) {
        ensureVisible(i);
        layout();
        return layer.localToScreen(slotX(i) + cellW / 2, slotY(i) + cellH / 2);
    }

    private void refreshStates() {
        var sm = model.getSelectionModel();
        int lead = model.getFocusModel().getFocusedIndex();
        active.forEach((i, cell) -> {
            cell.pseudoClassStateChanged(SELECTED, sm.isSelected(i));
            cell.pseudoClassStateChanged(LEAD, i == lead);
        });
    }

    // ---- mouse ----

    private void onPressed(MouseEvent e) {
        requestFocus();
        pressedIndex = itemAt(e);
        deferredSelect = false;
        dragStarted = false;
        bandArmed = false;
        var sm = model.getSelectionModel();
        if (e.getButton() == MouseButton.SECONDARY) {
            // right-click acts on the clicked item; on empty space it targets the folder itself
            if (pressedIndex < 0) {
                sm.clearSelection();
            } else if (!sm.isSelected(pressedIndex)) {
                select(pressedIndex, false, false);
            }
            return;
        }
        if (e.getButton() != MouseButton.PRIMARY) {
            return;
        }
        if (pressedIndex >= 0) {
            boolean toggle = e.isShortcutDown();
            if (!toggle && !e.isShiftDown() && sm.isSelected(pressedIndex)) {
                // keep a multi-selection for dragging; a plain click selects just this one on release
                deferredSelect = true;
                model.getFocusModel().focus(pressedIndex);
            } else {
                select(pressedIndex, e.isShiftDown(), toggle);
            }
        } else {
            bandArmed = true;
            bandToggle = e.isShortcutDown();
            Pointer p = toLayer(e);
            pressX = p.x;
            pressY = p.y;
            mouseX = pressX;
            mouseY = pressY;
            bandAnchorX = pressX + (columns() ? bar.getValue() : 0);
            bandAnchorY = pressY + (columns() ? 0 : bar.getValue());
            bandBase = new BitSet();
            if (e.isShortcutDown() || e.isShiftDown()) {
                sm.getSelectedIndices().forEach(i -> {
                    if (i != null && i >= 0) {
                        bandBase.set(i);
                    }
                });
            }
        }
    }

    private void onDragged(MouseEvent e) {
        if (!bandArmed) {
            return;
        }
        e.consume();
        Pointer p = toLayer(e);
        mouseX = p.x;
        mouseY = p.y;
        if (!bandActive) {
            if (Math.abs(mouseX - pressX) < DRAG_THRESHOLD && Math.abs(mouseY - pressY) < DRAG_THRESHOLD) {
                return;
            }
            bandActive = true;
            if (!layer.getChildren().contains(marquee)) {
                layer.getChildren().add(marquee);
            }
            marquee.setVisible(true);
            autoScroll.start();
        }
        updateBand();
    }

    private void onDragDetected(MouseEvent e) {
        if (bandArmed || pressedIndex < 0 || e.getButton() != MouseButton.PRIMARY) {
            return;
        }
        List<Path> paths = tab.selectedPaths();
        if (paths.isEmpty()) {
            return;
        }
        dragStarted = true;
        deferredSelect = false;
        Dragboard db = startDragAndDrop(TransferMode.COPY_OR_MOVE);
        ClipboardContent content = new ClipboardContent();
        content.putFiles(paths.stream().map(Path::toFile).toList());
        db.setContent(content);
        ItemCell cell = active.get(pressedIndex);
        if (cell != null) {
            db.setDragView(cell.snapshot(null, null));
        }
        e.consume();
    }

    private void onReleased(MouseEvent e) {
        if (bandActive) {
            autoScroll.stop();
            marquee.setVisible(false);
            bandActive = false;
        } else if (bandArmed && !e.isShortcutDown() && !e.isShiftDown()) {
            model.getSelectionModel().clearSelection(); // a click on empty space
        } else if (deferredSelect && !dragStarted && pressedIndex >= 0) {
            select(pressedIndex, false, false);
        }
        bandArmed = false;
        deferredSelect = false;
    }

    private void onClicked(MouseEvent e) {
        if (e.getButton() == MouseButton.PRIMARY && e.getClickCount() == 2 && itemAt(e) >= 0) {
            main.openSelection();
        }
    }

    private void onScroll(ScrollEvent e) {
        if (e.isShortcutDown()) {
            return; // Ctrl+wheel changes the view mode (FileTab)
        }
        double delta = columns() && e.getDeltaX() == 0 ? e.getDeltaY() : columns() ? e.getDeltaX() : e.getDeltaY();
        bar.setValue(clamp(bar.getValue() - delta, 0, bar.getMax()));
        e.consume();
    }

    private int itemAt(MouseEvent e) {
        Pointer p = toLayer(e);
        return indexAt(p.x, p.y);
    }

    private Pointer toLayer(MouseEvent e) {
        var p = layer.sceneToLocal(e.getSceneX(), e.getSceneY());
        return new Pointer(p.getX(), p.getY());
    }

    private record Pointer(double x, double y) {
    }

    // ---- rubber band ----

    private void updateBand() {
        double offX = columns() ? bar.getValue() : 0;
        double offY = columns() ? 0 : bar.getValue();
        double ax = bandAnchorX - offX;
        double ay = bandAnchorY - offY;
        double mx = clamp(mouseX, 0, viewW);
        double my = clamp(mouseY, 0, viewH);
        double x1 = clamp(ax, 0, viewW);
        double y1 = clamp(ay, 0, viewH);
        marquee.setX(Math.min(x1, mx) + 0.5);
        marquee.setY(Math.min(y1, my) + 0.5);
        marquee.setWidth(Math.abs(mx - x1));
        marquee.setHeight(Math.abs(my - y1));

        // the band in content coordinates (minus padding)
        double minX = Math.min(bandAnchorX, mouseX + offX) - PAD;
        double maxX = Math.max(bandAnchorX, mouseX + offX) - PAD;
        double minY = Math.min(bandAnchorY, mouseY + offY) - PAD;
        double maxY = Math.max(bandAnchorY, mouseY + offY) - PAD;
        BitSet sel = (BitSet) bandBase.clone();
        int n = items().size();
        int c1 = (int) Math.floor(Math.max(0, minX) / cellW);
        int c2 = (int) Math.floor(maxX / cellW);
        int r1 = (int) Math.floor(Math.max(0, minY) / cellH);
        int r2 = (int) Math.floor(maxY / cellH);
        if (maxX >= 0 && maxY >= 0) {
            BitSet hit = new BitSet();
            for (int r = r1; r <= r2; r++) {
                for (int c = c1; c <= c2; c++) {
                    int i;
                    if (columns()) {
                        if (r >= lanes) {
                            continue;
                        }
                        i = c * lanes + r;
                    } else {
                        if (c >= lanes) {
                            continue;
                        }
                        i = r * lanes + c;
                    }
                    if (i < n) {
                        hit.set(i);
                    }
                }
            }
            if (bandToggle) {
                sel.xor(hit);
            } else {
                sel.or(hit);
            }
        }
        applySelection(sel);
    }

    private void applySelection(BitSet sel) {
        var sm = model.getSelectionModel();
        int[] indices = sel.stream().toArray();
        List<Integer> current = new ArrayList<>(sm.getSelectedIndices());
        if (current.size() == indices.length && sel.equals(toBits(current))) {
            return;
        }
        sm.clearSelection();
        if (indices.length > 0) {
            sm.selectIndices(indices[0], indices);
        }
    }

    private static BitSet toBits(List<Integer> list) {
        BitSet b = new BitSet();
        list.forEach(i -> {
            if (i != null && i >= 0) {
                b.set(i);
            }
        });
        return b;
    }

    private void autoScrollStep() {
        double view = columns() ? viewW : viewH;
        double pos = columns() ? mouseX : mouseY;
        double d = pos < 0 ? pos : pos > view ? pos - view : 0;
        if (d != 0) {
            bar.setValue(clamp(bar.getValue() + clamp(d / 2, -40, 40), 0, bar.getMax()));
            updateBand();
        }
    }

    // ---- keyboard ----

    private void onKey(KeyEvent e) {
        int n = items().size();
        if (n == 0) {
            return;
        }
        var fm = model.getFocusModel();
        int cur = fm.getFocusedIndex();
        int across = lanes;
        int page = Math.max(1, (int) ((columns() ? viewW : viewH) / slotAlong())) * lanes;
        int target;
        KeyCode c = e.getCode();
        if (c == KeyCode.SPACE) {
            if (cur >= 0) {
                if (e.isShortcutDown()) {
                    select(cur, false, true);
                } else if (!model.getSelectionModel().isSelected(cur)) {
                    select(cur, e.isShiftDown(), false);
                }
                e.consume();
            }
            return;
        }
        int along = columns() ? 1 : across; // up/down in row flow jumps a row
        int side = columns() ? across : 1;
        switch (c) {
            case LEFT -> target = cur - side;
            case RIGHT -> target = cur + side;
            case UP -> target = cur - along;
            case DOWN -> target = cur + along;
            case HOME -> target = 0;
            case END -> target = n - 1;
            case PAGE_UP -> target = cur - page;
            case PAGE_DOWN -> target = cur + page;
            default -> {
                return;
            }
        }
        if (e.isAltDown()) {
            return; // Alt+arrows are history navigation
        }
        // past an edge: stop at the first or last item (Down from a full row to a shorter last row
        // lands on the last item, as in Explorer)
        target = cur < 0 ? 0 : clamp(target, 0, n - 1);
        if (e.isShortcutDown() && !e.isShiftDown()) {
            fm.focus(target);
        } else {
            select(target, e.isShiftDown(), false);
        }
        ensureVisible(target);
        e.consume();
    }

    private static int clamp(int v, int min, int max) {
        return Math.max(min, Math.min(max, v));
    }

    // ---- selection ----

    /** Explorer-style selection: plain click selects one, Shift selects a range, Ctrl/Cmd toggles. */
    private void select(int i, boolean range, boolean toggle) {
        var sm = model.getSelectionModel();
        var fm = model.getFocusModel();
        if (range) {
            int from = anchor >= 0 && anchor < items().size() ? anchor : Math.max(0, fm.getFocusedIndex());
            if (!toggle) {
                sm.clearSelection();
            }
            sm.selectRange(Math.min(from, i), Math.max(from, i) + 1);
            fm.focus(i);
            return;
        }
        if (toggle) {
            if (sm.isSelected(i)) {
                sm.clearSelection(i);
            } else {
                sm.select(i);
            }
            fm.focus(i);
        } else {
            sm.clearAndSelect(i);
        }
        anchor = i;
    }

    // ---- drag & drop target ----

    private void onDragOver(DragEvent e) {
        ItemCell cell = folderCellAt(e.getSceneX(), e.getSceneY());
        if (cell != null && Dnd.accept(e, cell.entry.path())) {
            setDropCell(Dnd.canDrop(e, cell.entry.path()) ? cell : null);
            e.consume();
            return;
        }
        setDropCell(null);
        if (Dnd.accept(e, tab.currentDir())) {
            e.consume();
        }
    }

    private void onDragDropped(DragEvent e) {
        setDropCell(null);
        ItemCell cell = folderCellAt(e.getSceneX(), e.getSceneY());
        Path target = cell != null && Dnd.canDrop(e, cell.entry.path()) ? cell.entry.path() : tab.currentDir();
        if (Dnd.canDrop(e, target)) {
            main.drop(e, target);
        }
    }

    private ItemCell folderCellAt(double sceneX, double sceneY) {
        var p = layer.sceneToLocal(sceneX, sceneY);
        int i = indexAt(p.getX(), p.getY());
        if (i < 0) {
            return null;
        }
        ItemCell cell = active.get(i);
        return cell != null && cell.entry != null && cell.entry.isDirectory() ? cell : null;
    }

    private void setDropCell(ItemCell cell) {
        if (dropCell == cell) {
            return;
        }
        if (dropCell != null) {
            dropCell.getStyleClass().remove("drop-target");
        }
        dropCell = cell;
        if (cell != null) {
            cell.getStyleClass().add("drop-target");
        }
    }

    private static double clamp(double v, double min, double max) {
        return Math.max(min, Math.min(max, v));
    }

    // ---- item node ----

    /** One item: icon or thumbnail plus name, and type/size/date lines in Tiles and Content. */
    final class ItemCell extends Region {
        private final StackPane iconBox = new StackPane();
        private final FontIcon icon = new FontIcon("mdi2f-file-outline");
        private final ImageView thumb = new ImageView();
        private final Label name = new Label();
        private final Label line2 = new Label();
        private final Label line3 = new Label();
        private final Label right1 = new Label();
        private final Label right2 = new Label();
        private final ViewMode cellMode = mode;
        private final int iconPx = iconPx();
        private FileEntry entry;
        private Image image;
        private final javafx.beans.value.ChangeListener<Number> imageProgress = (o, a, p) -> showImageIfReady();

        ItemCell() {
            getStyleClass().addAll("grid-item", cellMode.id() + "-item");
            int size = iconPx;
            // inline, so the stylesheets' -fx-icon-size for list icons can't shrink it
            icon.setStyle("-fx-icon-size: " + size + "px;");
            thumb.setFitWidth(size);
            thumb.setFitHeight(size);
            thumb.setPreserveRatio(true);
            thumb.setSmooth(true);
            iconBox.getChildren().add(icon);
            iconBox.setMinSize(size, size);
            iconBox.setPrefSize(size, size);
            iconBox.setMaxSize(size, size);
            name.getStyleClass().add("item-name");
            for (Label l : List.of(line2, line3, right1, right2)) {
                l.getStyleClass().add("item-detail");
                l.setTextOverrun(OverrunStyle.ELLIPSIS);
            }
            if (cellMode.textBelowIcon()) {
                name.setWrapText(true);
                name.setTextAlignment(TextAlignment.CENTER);
                name.setAlignment(javafx.geometry.Pos.TOP_CENTER);
                name.setTextOverrun(OverrunStyle.ELLIPSIS);
            } else {
                name.setTextOverrun(OverrunStyle.ELLIPSIS);
            }
            getChildren().addAll(iconBox, name);
            if (cellMode == ViewMode.TILES) {
                getChildren().addAll(line2, line3);
            } else if (cellMode == ViewMode.CONTENT) {
                getChildren().addAll(line2, right1, right2);
            }
        }

        FileEntry entry() {
            return entry;
        }

        /** Whether an image thumbnail (loaded) is shown instead of the type icon. */
        boolean showsThumbnail() {
            return iconBox.getChildren().contains(thumb);
        }

        String nameText() {
            return name.getText();
        }

        void show(FileEntry e) {
            entry = e;
            Icons.apply(icon, e);
            name.setText(e.name());
            pseudoClassStateChanged(HIDDEN, e.isHidden());
            switch (cellMode) {
                case TILES -> {
                    line2.setText(e.type());
                    line3.setText(e.isDirectory() ? "" : Formats.size(e.size()));
                }
                case CONTENT -> {
                    line2.setText(e.type());
                    right1.setText(e.modified() == null ? "" : "Date modified: " + Formats.date(e.modified()));
                    right2.setText(e.isDirectory() ? "" : "Size: " + Formats.size(e.size()));
                }
                default -> {
                    // name only
                }
            }
            if (image != null) {
                image.progressProperty().removeListener(imageProgress);
            }
            // big enough icons show the image itself (also Small icons or Tiles made big with +)
            image = iconPx >= ViewMode.THUMBNAIL_MIN ? Thumbnails.get(e, iconPx) : null;
            iconBox.getChildren().setAll(icon);
            if (image != null) {
                image.progressProperty().addListener(imageProgress);
                showImageIfReady();
            }
        }

        private void showImageIfReady() {
            if (image != null && image.getProgress() >= 1 && !image.isError()) {
                thumb.setImage(image);
                iconBox.getChildren().setAll(thumb);
            }
        }

        @Override
        protected void layoutChildren() {
            Insets in = getInsets();
            double x = in.getLeft();
            double y = in.getTop();
            double w = getWidth() - in.getLeft() - in.getRight();
            double h = getHeight() - in.getTop() - in.getBottom();
            int size = iconPx;
            if (cellMode.textBelowIcon()) {
                layoutInArea(iconBox, x, y, w, size, 0, HPos.CENTER, VPos.CENTER);
                double textTop = y + size + 4;
                double lineH = name.getFont().getSize() * 1.35;
                double nameH = Math.min(name.prefHeight(w), Math.min(h - size - 4, lineH * 2 + 1));
                name.resizeRelocate(x, textTop, w, Math.max(0, nameH));
                return;
            }
            double gap = 8;
            layoutInArea(iconBox, x, y, size, h, 0, HPos.CENTER, VPos.CENTER);
            double tx = x + size + gap;
            double tw = Math.max(0, w - size - gap);
            switch (cellMode) {
                case TILES -> {
                    double lh = name.prefHeight(-1);
                    double top = y + (h - 3 * lh) / 2;
                    name.resizeRelocate(tx, top, tw, lh);
                    line2.resizeRelocate(tx, top + lh, tw, lh);
                    line3.resizeRelocate(tx, top + 2 * lh, tw, lh);
                }
                case CONTENT -> {
                    double rightW = Math.min(240, tw * 0.45);
                    double lw = tw - rightW - gap;
                    double lh = name.prefHeight(-1);
                    double top = y + (h - 2 * lh) / 2;
                    name.resizeRelocate(tx, top, lw, lh);
                    line2.resizeRelocate(tx, top + lh, lw, lh);
                    right1.resizeRelocate(tx + lw + gap, top, rightW, lh);
                    right2.resizeRelocate(tx + lw + gap, top + lh, rightW, lh);
                }
                default -> name.resizeRelocate(tx, y, tw, h);
            }
        }
    }

    // ---- skin ----

    private static final class GridSkin extends SkinBase<FileGrid> {
        GridSkin(FileGrid grid) {
            super(grid);
            getChildren().addAll(grid.layer, grid.bar, grid.placeholder);
        }

        @Override
        protected void layoutChildren(double x, double y, double w, double h) {
            getSkinnable().layoutGrid(x, y, w, h);
        }

        @Override
        protected double computePrefWidth(double height, double top, double right, double bottom, double left) {
            return 400;
        }

        @Override
        protected double computePrefHeight(double width, double top, double right, double bottom, double left) {
            return 300;
        }
    }
}
