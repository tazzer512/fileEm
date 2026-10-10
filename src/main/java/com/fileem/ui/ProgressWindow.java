package com.fileem.ui;

import atlantafx.base.theme.Styles;
import com.fileem.ops.FileOperation;
import com.fileem.ops.FileOperation.Kind;
import com.fileem.util.Formats;
import javafx.beans.binding.Bindings;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.Node;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.Label;
import javafx.scene.control.OverrunStyle;
import javafx.scene.control.ProgressBar;
import javafx.scene.control.Separator;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.scene.layout.Region;
import javafx.scene.layout.VBox;
import javafx.stage.Stage;
import javafx.stage.Window;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * One progress window per owner window that shows every running {@link FileOperation} stacked under
 * each other, each with its own Cancel button. It opens with the first operation and closes when the
 * last one is done. Closing the window cancels all of them.
 */
final class ProgressWindow {
    private static final Map<Window, ProgressWindow> OPEN = new HashMap<>();

    private final Window owner;
    private final Stage stage = new Stage();
    private final VBox rows = new VBox();
    private final Map<FileOperation, Row> shown = new LinkedHashMap<>();

    private ProgressWindow(Window owner) {
        this.owner = owner;
        stage.initOwner(owner);
        Icons.applyAppIcon(stage);
        rows.setPadding(new Insets(16));
        rows.setPrefWidth(520);
        Scene scene = new Scene(rows);
        Theme.style(scene.getStylesheets());
        stage.setScene(scene);
        stage.setResizable(false);
        stage.setOnCloseRequest(e -> List.copyOf(shown.keySet()).forEach(FileOperation::cancel));
    }

    /** The progress window of {@code owner}, or null while no operation is shown there. */
    static ProgressWindow of(Window owner) {
        return OPEN.get(owner);
    }

    /** Adds a row for {@code op} to the owner's progress window, opening the window if needed. */
    static void show(Window owner, FileOperation op, String title) {
        OPEN.computeIfAbsent(owner, ProgressWindow::new).add(op, title);
    }

    /** Removes the row of a finished operation; the window closes when it was the last one. */
    static void finished(Window owner, FileOperation op) {
        ProgressWindow w = OPEN.get(owner);
        if (w != null) {
            w.remove(op);
        }
    }

    Stage stage() {
        return stage;
    }

    /** The operations shown, in the order they were added. */
    List<FileOperation> operations() {
        return List.copyOf(shown.keySet());
    }

    /** The Cancel button of {@code op}'s row. */
    Button cancelButton(FileOperation op) {
        Row r = shown.get(op);
        return r == null ? null : r.cancel;
    }

    private void add(FileOperation op, String title) {
        if (shown.containsKey(op)) {
            return;
        }
        Row row = new Row(op, title);
        shown.put(op, row);
        layoutRows();
        if (!stage.isShowing()) {
            stage.show();
        }
    }

    private void remove(FileOperation op) {
        if (shown.remove(op) == null) {
            return;
        }
        if (shown.isEmpty()) {
            OPEN.remove(owner);
            stage.close();
        } else {
            layoutRows();
        }
    }

    private void layoutRows() {
        List<Node> children = new ArrayList<>();
        for (Row r : shown.values()) {
            if (!children.isEmpty()) {
                Separator sep = new Separator();
                sep.setPadding(new Insets(12, 0, 12, 0));
                children.add(sep);
            }
            children.add(r.box);
            // Escape cancels only when that can't hit the wrong operation.
            r.cancel.setCancelButton(shown.size() == 1);
        }
        rows.getChildren().setAll(children);
        stage.setTitle(shown.size() == 1 ? shown.values().iterator().next().title
                : shown.size() + " operations in progress");
        stage.sizeToScene();
    }

    private static final class Row {
        final String title;
        final VBox box;
        final Button cancel = new Button("Cancel");

        Row(FileOperation op, String title) {
            this.title = title;
            Label heading = new Label(title);
            heading.getStyleClass().add(Styles.TITLE_4);
            heading.setTextOverrun(OverrunStyle.CENTER_ELLIPSIS);
            heading.setMaxWidth(Double.MAX_VALUE);

            Label current = new Label();
            current.textProperty().bind(op.messageProperty());
            current.setTextOverrun(OverrunStyle.CENTER_ELLIPSIS);
            current.setMaxWidth(Double.MAX_VALUE);
            current.getStyleClass().add(Styles.TEXT_MUTED);

            ProgressBar bar = new ProgressBar();
            bar.progressProperty().bind(op.progressProperty());
            bar.setMaxWidth(Double.MAX_VALUE);

            boolean bytes = op.kind() != Kind.DELETE && op.kind() != Kind.TRASH;
            Label detail = new Label();
            detail.textProperty().bind(Bindings.createStringBinding(() -> {
                double p = op.getProgress();
                if (p < 0) {
                    return "";
                }
                long d = (long) op.getWorkDone(), t = (long) op.getTotalWork();
                String amount = bytes ? Formats.size(d) + " of " + Formats.size(t)
                        : Formats.count(d) + " of " + Formats.count(t) + " items";
                return String.format("%.0f%%   ·   %s", p * 100, amount);
            }, op.progressProperty()));

            cancel.setOnAction(e -> {
                cancel.setDisable(true);
                op.cancel();
            });
            Region spacer = new Region();
            HBox.setHgrow(spacer, Priority.ALWAYS);
            HBox bottom = new HBox(8, detail, spacer, cancel);
            bottom.setAlignment(Pos.CENTER_LEFT);

            box = new VBox(10, heading, current, bar, bottom);
        }
    }
}
