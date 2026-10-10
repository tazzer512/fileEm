package com.fileem.ui;

import javafx.animation.AnimationTimer;
import javafx.geometry.Bounds;
import javafx.scene.Node;
import javafx.scene.control.IndexedCell;
import javafx.scene.control.ScrollBar;
import javafx.scene.control.TableCell;
import javafx.scene.control.TableRow;
import javafx.scene.control.skin.TableColumnHeader;
import javafx.scene.control.skin.VirtualFlow;
import javafx.scene.input.MouseButton;
import javafx.scene.input.MouseEvent;
import javafx.scene.shape.Rectangle;
import javafx.scene.text.Text;

import java.util.BitSet;

/**
 * Rubber-band selection in the file list: dragging from empty space, or from a row anywhere but its
 * name (icon + text, which drags the files), draws a dotted box and selects the rows it touches.
 * Shift adds to the selection, Ctrl/Cmd toggles. The list scrolls while the mouse is above or below it.
 * Rows are tracked in content coordinates (row index × row height), so the box stays anchored to the
 * row it started on while scrolling.
 */
final class Marquee {
    private static final double THRESHOLD = 4;

    private final FileTable table;
    private final Rectangle box = new Rectangle();
    private final AnimationTimer autoScroll = new AnimationTimer() {
        @Override
        public void handle(long now) {
            scrollIfOutside();
        }
    };

    private boolean armed;
    private boolean active;
    private double pressX;
    private double pressY;
    private double anchorContentY;
    private double mouseX;
    private double mouseY;
    private boolean toggle;
    private boolean add;
    private BitSet base = new BitSet();
    private int lastLo = -1;
    private int lastHi = -1;

    Marquee(FileTable table) {
        this.table = table;
        box.getStyleClass().add("marquee");
        box.setManaged(false);
        box.setMouseTransparent(true);
        box.setVisible(false);
        table.addEventFilter(MouseEvent.MOUSE_PRESSED, this::onPressed);
        table.addEventFilter(MouseEvent.MOUSE_DRAGGED, this::onDragged);
        table.addEventFilter(MouseEvent.DRAG_DETECTED, e -> {
            if (armed) {
                e.consume(); // a marquee, not a file drag
            }
        });
        table.addEventFilter(MouseEvent.MOUSE_RELEASED, this::onReleased);
    }

    boolean isActive() {
        return active;
    }

    private void onPressed(MouseEvent e) {
        armed = false;
        if (e.getButton() != MouseButton.PRIMARY || !startsMarquee(e)) {
            return;
        }
        VirtualFlow<?> flow = flow();
        if (flow == null) {
            return;
        }
        armed = true;
        pressX = e.getX();
        pressY = e.getY();
        mouseX = pressX;
        mouseY = pressY;
        toggle = e.isShortcutDown();
        add = e.isShiftDown() && !toggle;
        base = new BitSet();
        if (toggle || add) {
            table.getSelectionModel().getSelectedIndices().forEach(i -> {
                if (i != null && i >= 0) {
                    base.set(i);
                }
            });
        }
        anchorContentY = contentY(flow, pressY);
        lastLo = -1;
        lastHi = -1;
    }

    private void onDragged(MouseEvent e) {
        if (!armed) {
            return;
        }
        e.consume(); // keeps the cell behavior from drag-extending the selection itself
        mouseX = e.getX();
        mouseY = e.getY();
        if (!active) {
            if (Math.abs(mouseX - pressX) < THRESHOLD && Math.abs(mouseY - pressY) < THRESHOLD) {
                return;
            }
            active = true;
            table.requestFocus();
            table.showOverlay(box);
            box.setVisible(true);
            autoScroll.start();
        }
        update();
    }

    private void onReleased(MouseEvent e) {
        if (active) {
            e.consume(); // the cell behavior would otherwise reselect the pressed row on release
            autoScroll.stop();
            box.setVisible(false);
            active = false;
        }
        armed = false;
    }

    /** Whether a press here starts a marquee rather than a click/drag on a file or a scroll bar. */
    private boolean startsMarquee(MouseEvent e) {
        Node n = e.getPickResult().getIntersectedNode();
        boolean inRows = false;
        for (; n != null && n != table; n = n.getParent()) {
            if (n instanceof ScrollBar || n instanceof TableColumnHeader) {
                return false;
            }
            if (n instanceof TableRow<?> row && !row.isEmpty() && row.getItem() != null) {
                return true; // on a row but not on its name (checked below)
            }
            if (n instanceof TableCell<?, ?> cell && cell.getStyleClass().contains("name-cell")
                    && !cell.isEmpty() && onNameContent(cell, e)) {
                return false;
            }
            if (n instanceof VirtualFlow<?>) {
                inRows = true;
            }
        }
        return inRows;
    }

    /** Whether the mouse is on the icon or the text of a name cell (up to the end of the text). */
    private static boolean onNameContent(TableCell<?, ?> cell, MouseEvent e) {
        double x = cell.sceneToLocal(e.getSceneX(), e.getSceneY()).getX();
        double end = cell.snappedLeftInset();
        if (cell.getGraphic() != null) {
            end = Math.max(end, cell.getGraphic().getBoundsInParent().getMaxX());
        }
        for (Node child : cell.getChildrenUnmodifiable()) {
            if (child instanceof Text t) {
                end = Math.max(end, t.getBoundsInParent().getMaxX());
            }
        }
        return x <= end + 4;
    }

    private void scrollIfOutside() {
        VirtualFlow<?> flow = flow();
        if (flow == null) {
            return;
        }
        Bounds b = flow.getBoundsInParent();
        double dy = mouseY < b.getMinY() ? mouseY - b.getMinY() : mouseY > b.getMaxY() ? mouseY - b.getMaxY() : 0;
        if (dy != 0) {
            flow.scrollPixels(Math.max(-40, Math.min(40, dy / 2)));
            update();
        }
    }

    /** Redraws the box and selects the rows between the anchor and the mouse. */
    private void update() {
        VirtualFlow<?> flow = flow();
        IndexedCell<?> first = flow == null ? null : flow.getFirstVisibleCell();
        if (first == null) {
            return;
        }
        double rowH = first.getHeight();
        double offset = offset(first);
        Bounds b = flow.getBoundsInParent();
        double clipMaxY = b.getMinY() + flow.getHeight();
        double y1 = clamp(anchorContentY - offset, b.getMinY(), clipMaxY);
        double y2 = clamp(mouseY, b.getMinY(), clipMaxY);
        double x1 = clamp(pressX, b.getMinX(), b.getMaxX());
        double x2 = clamp(mouseX, b.getMinX(), b.getMaxX());
        box.setX(Math.min(x1, x2) + 0.5);
        box.setY(Math.min(y1, y2) + 0.5);
        box.setWidth(Math.abs(x2 - x1));
        box.setHeight(Math.abs(y2 - y1));
        box.toFront();

        int count = table.getItems().size();
        double c1 = anchorContentY;
        double c2 = mouseY + offset;
        int lo = (int) Math.floor(Math.max(0, Math.min(c1, c2)) / rowH);
        int hi = Math.min(count - 1, (int) Math.floor(Math.max(c1, c2) / rowH));
        if (Math.max(c1, c2) < 0 || lo > hi) {
            lo = 0;
            hi = -1; // the box touches no rows
        }
        if (lo == lastLo && hi == lastHi) {
            return;
        }
        lastLo = lo;
        lastHi = hi;
        select(lo, hi, (int) Math.floor(Math.max(0, c2) / rowH));
    }

    private void select(int lo, int hi, int focus) {
        BitSet sel = (BitSet) base.clone();
        if (hi >= lo) {
            if (toggle) {
                sel.flip(lo, hi + 1);
            } else {
                sel.set(lo, hi + 1);
            }
        }
        var sm = table.getSelectionModel();
        sm.clearSelection();
        int[] indices = sel.stream().toArray();
        if (indices.length > 0) {
            sm.selectIndices(indices[0], indices);
        }
        int count = table.getItems().size();
        if (count > 0) {
            table.getFocusModel().focus(Math.min(count - 1, focus));
        }
    }

    /** Converts a y in table coordinates to a y in content coordinates (0 = top of the first row). */
    private double contentY(VirtualFlow<?> flow, double y) {
        IndexedCell<?> first = flow.getFirstVisibleCell();
        return first == null ? y : y + offset(first);
    }

    /** Content y minus table y. */
    private double offset(IndexedCell<?> first) {
        double cellTop = table.sceneToLocal(first.localToScene(0, 0)).getY();
        return first.getIndex() * first.getHeight() - cellTop;
    }

    private VirtualFlow<?> flow() {
        return table.lookup(".virtual-flow") instanceof VirtualFlow<?> f ? f : null;
    }

    private static double clamp(double v, double min, double max) {
        return Math.max(min, Math.min(max, v));
    }
}
