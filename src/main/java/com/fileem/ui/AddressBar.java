package com.fileem.ui;

import atlantafx.base.theme.Styles;
import com.fileem.util.OsUtil;
import javafx.geometry.Insets;
import javafx.scene.Node;
import javafx.scene.control.Button;
import javafx.scene.control.Label;
import javafx.scene.control.TextField;
import javafx.scene.input.KeyCode;
import javafx.scene.input.KeyEvent;
import javafx.scene.layout.Pane;
import javafx.scene.layout.StackPane;

import java.nio.file.InvalidPathException;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.List;
import java.util.function.Consumer;

/** Breadcrumb path display that turns into an editable text field on click or Ctrl+L. */
final class AddressBar extends StackPane {
    private final CrumbBox crumbs = new CrumbBox();
    private final TextField field = new TextField();
    private final Consumer<Path> onNavigate;
    private final Runnable onDone;
    private Path path;

    AddressBar(Consumer<Path> onNavigate, Runnable onDone) {
        this.onNavigate = onNavigate;
        this.onDone = onDone;
        getStyleClass().add("address-bar");
        setMinWidth(0);
        field.setVisible(false);
        field.getStyleClass().add("address-field");
        getChildren().addAll(crumbs, field);

        crumbs.setOnMouseClicked(e -> {
            if (e.getTarget() == crumbs) {
                edit();
            }
        });
        field.setOnAction(e -> commit());
        field.addEventHandler(KeyEvent.KEY_PRESSED, e -> {
            if (e.getCode() == KeyCode.ESCAPE) {
                e.consume();
                stopEditing();
                onDone.run();
            }
        });
        field.focusedProperty().addListener((o, was, focused) -> {
            if (!focused) {
                stopEditing();
            }
        });
    }

    void setPath(Path p) {
        path = p;
        crumbs.getChildren().clear();
        if (p == null) {
            return;
        }
        Deque<Path> chain = new ArrayDeque<>();
        for (Path cur = p; cur != null; cur = cur.getParent()) {
            chain.push(cur);
        }
        boolean first = true;
        for (Path segment : chain) {
            if (!first) {
                Label sep = new Label("›");
                sep.getStyleClass().add("crumb-separator");
                crumbs.getChildren().add(sep);
            }
            crumbs.getChildren().add(crumb(segment));
            first = false;
        }
    }

    private Node crumb(Path segment) {
        Button b = new Button();
        b.getStyleClass().addAll(Styles.FLAT, "crumb");
        b.setFocusTraversable(false);
        if (segment.getFileName() == null) {
            if (OsUtil.isWindows()) {
                b.setText(segment.toString().replace("\\", ""));
            } else {
                b.setGraphic(Icons.icon("mdi2h-harddisk"));
            }
        } else {
            b.setText(segment.getFileName().toString());
        }
        b.setOnAction(e -> onNavigate.accept(segment));
        return b;
    }

    boolean isEditing() {
        return field.isVisible();
    }

    void edit() {
        field.setText(path == null ? "" : path.toString());
        field.setVisible(true);
        crumbs.setVisible(false);
        field.requestFocus();
        field.selectAll();
    }

    private void stopEditing() {
        if (field.isVisible()) {
            field.setVisible(false);
            crumbs.setVisible(true);
        }
    }

    private void commit() {
        String text = field.getText().trim();
        if (text.equals("~") || text.startsWith("~/") || text.startsWith("~\\")) {
            text = OsUtil.home() + text.substring(1);
        }
        Path target;
        try {
            target = Path.of(text);
            if (!target.isAbsolute() && path != null) {
                target = path.resolve(target);
            }
            target = target.normalize();
        } catch (InvalidPathException e) {
            Dialogs.error(getScene().getWindow(), "Address", "Invalid path: " + text);
            return;
        }
        stopEditing();
        onDone.run();
        onNavigate.accept(target);
    }

    /** Lays crumbs out left to right, hiding leading crumbs when the path does not fit. */
    private static final class CrumbBox extends Pane {
        CrumbBox() {
            getStyleClass().add("crumbs");
            setPadding(new Insets(0, 4, 0, 4));
        }

        @Override
        protected void layoutChildren() {
            Insets in = getInsets();
            double avail = getWidth() - in.getLeft() - in.getRight();
            double h = getHeight() - in.getTop() - in.getBottom();
            List<Node> kids = getManagedChildren();
            int firstVisible = kids.size();
            double used = 0;
            for (int i = kids.size() - 1; i >= 0; i--) {
                double w = kids.get(i).prefWidth(-1);
                if (used + w > avail && i < kids.size() - 1) {
                    break;
                }
                used += w;
                firstVisible = i;
            }
            double x = in.getLeft();
            for (int i = 0; i < kids.size(); i++) {
                Node n = kids.get(i);
                boolean visible = i >= firstVisible;
                n.setVisible(visible);
                if (visible) {
                    double w = n.prefWidth(-1);
                    double ph = Math.min(h, n.prefHeight(w));
                    n.resizeRelocate(x, in.getTop() + (h - ph) / 2, w, ph);
                    x += w;
                }
            }
        }

        @Override
        protected double computeMinWidth(double height) {
            return 0;
        }

        @Override
        protected double computePrefHeight(double width) {
            double h = 0;
            for (Node n : getManagedChildren()) {
                h = Math.max(h, n.prefHeight(-1));
            }
            return h + getInsets().getTop() + getInsets().getBottom();
        }
    }
}
