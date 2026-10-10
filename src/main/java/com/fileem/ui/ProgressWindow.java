package com.fileem.ui;

import atlantafx.base.theme.Styles;
import com.fileem.ops.FileOperation;
import com.fileem.ops.FileOperation.Decision;
import com.fileem.ops.FileOperation.Kind;
import com.fileem.ops.FileOperation.Resolution;
import com.fileem.util.Eta;
import com.fileem.util.Formats;
import javafx.animation.KeyFrame;
import javafx.animation.Timeline;
import javafx.beans.binding.Bindings;
import javafx.beans.property.SimpleStringProperty;
import javafx.beans.property.StringProperty;
import javafx.css.PseudoClass;
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
import javafx.scene.control.CheckBox;
import javafx.scene.layout.GridPane;
import javafx.stage.Window;
import javafx.util.Duration;
import org.kordamp.ikonli.javafx.FontIcon;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;

/**
 * One progress window per owner window that shows every running {@link FileOperation} stacked under
 * each other, each with its own Pause/Resume and Cancel buttons. It opens with the first operation and closes when the
 * last one is done. Closing the window cancels all of them.
 */
final class ProgressWindow {
    private static final Map<Window, ProgressWindow> OPEN = new HashMap<>();

    private final Window owner;
    private final Stage stage = new Stage();
    private final VBox rows = new VBox();
    private final Map<FileOperation, Row> shown = new LinkedHashMap<>();
    /** Samples every row's progress once a second for its speed and time-left estimate. */
    private final Timeline ticker = new Timeline(new KeyFrame(Duration.seconds(1), e -> tick(System.nanoTime())));

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
        ticker.setCycleCount(Timeline.INDEFINITE);
        stage.setOnShown(e -> ticker.play());
        stage.setOnHidden(e -> ticker.stop());
    }

    /** Updates every row's speed and time left (package-private: tests drive it with their own clock). */
    void tick(long nowNanos) {
        shown.values().forEach(r -> r.tick(nowNanos));
    }

    /** The progress line under {@code op}'s bar ("42%  ·  1.2 GB of 3 GB  ·  40 MB/s  ·  About 1 minute left"). */
    String detailText(FileOperation op) {
        Row r = shown.get(op);
        return r == null ? null : r.detail.getText();
    }

    /**
     * The progress line: percentage, amount, and (once known) speed and time left. Paused operations
     * say so instead of guessing a time.
     */
    static String detailText(double progress, long done, long total, boolean bytes, boolean paused, String estimate) {
        if (progress < 0) {
            return "";
        }
        String amount = bytes ? Formats.size(done) + " of " + Formats.size(total)
                : Formats.count(done) + " of " + Formats.count(total) + " items";
        String text = String.format("%.0f%%   ·   %s", progress * 100, amount);
        if (paused) {
            return "Paused   ·   " + text;
        }
        return estimate == null || estimate.isEmpty() ? text : text + "   ·   " + estimate;
    }

    /** The progress window of {@code owner}, or null while no operation is shown there. */
    static ProgressWindow of(Window owner) {
        return OPEN.get(owner);
    }

    /** Adds a row for {@code op} to the owner's progress window, opening the window if needed. */
    static void show(Window owner, FileOperation op, String title) {
        OPEN.computeIfAbsent(owner, ProgressWindow::new).add(op, title);
    }

    /**
     * Shows a conflict question in {@code op}'s row (adding the row and opening the window if needed)
     * and brings the window to the front. {@code answer} completes when a button is pressed.
     */
    static void ask(Window owner, FileOperation op, String title, OperationUi.Conflict conflict,
                    CompletableFuture<Decision> answer) {
        if (op.isDone()) {
            answer.complete(new Decision(Resolution.CANCEL, false));
            return;
        }
        ProgressWindow w = OPEN.computeIfAbsent(owner, ProgressWindow::new);
        w.addRow(op, title).ask(conflict, answer); // before showing, so the window opens at its full size
        w.layoutRows();
        w.showOnScreen();
        w.stage.setIconified(false);
        w.stage.toFront();
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

    /** The Pause/Resume button of {@code op}'s row. */
    Button pauseButton(FileOperation op) {
        Row r = shown.get(op);
        return r == null ? null : r.pause;
    }

    /** Whether {@code op}'s row is waiting for a conflict answer. */
    boolean isAsking(FileOperation op) {
        Row r = shown.get(op);
        return r != null && r.question != null;
    }

    /** A button ("Replace", "Skip", "Keep both") of the conflict question in {@code op}'s row. */
    Button conflictButton(FileOperation op, String text) {
        Row r = shown.get(op);
        if (r == null || r.question == null) {
            return null;
        }
        return r.question.getChildren().stream().filter(n -> n instanceof HBox).flatMap(n -> ((HBox) n).getChildren().stream())
                .filter(n -> n instanceof Button b && b.getText().equals(text)).map(n -> (Button) n).findFirst().orElse(null);
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
        addRow(op, title);
        layoutRows();
        showOnScreen();
    }

    private Row addRow(FileOperation op, String title) {
        return shown.computeIfAbsent(op, o -> new Row(o, title));
    }

    /** Shows the window and keeps it inside the screen as rows and questions make it taller. */
    private void showOnScreen() {
        if (!stage.isShowing()) {
            stage.show();
        }
        var screens = javafx.stage.Screen.getScreensForRectangle(stage.getX(), stage.getY(),
                Math.max(1, stage.getWidth()), Math.max(1, stage.getHeight()));
        var bounds = (screens.isEmpty() ? javafx.stage.Screen.getPrimary() : screens.get(0)).getVisualBounds();
        if (stage.getY() + stage.getHeight() > bounds.getMaxY()) {
            stage.setY(Math.max(bounds.getMinY(), bounds.getMaxY() - stage.getHeight()));
        }
        if (stage.getX() + stage.getWidth() > bounds.getMaxX()) {
            stage.setX(Math.max(bounds.getMinX(), bounds.getMaxX() - stage.getWidth()));
        }
    }

    private void remove(FileOperation op) {
        Row row = shown.remove(op);
        if (row == null) {
            return;
        }
        row.dropQuestion();
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

    private static final PseudoClass PAUSED = PseudoClass.getPseudoClass("paused");

    private final class Row {
        final String title;
        final VBox box;
        /** The open conflict question, or null. */
        VBox question;
        private CompletableFuture<Decision> answer;
        final Button cancel = new Button("Cancel");
        final Button pause = new Button("Pause", Icons.icon("mdi2p-pause"));
        final Label detail = new Label();
        private final FileOperation op;
        private final boolean bytes;
        private final Eta eta = new Eta();
        /** "40 MB/s   ·   About 1 minute left", or empty while unknown. */
        private final StringProperty estimate = new SimpleStringProperty("");

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

            this.op = op;
            bytes = op.kind() != Kind.DELETE && op.kind() != Kind.TRASH;
            detail.textProperty().bind(Bindings.createStringBinding(() -> detailText(op.getProgress(),
                    (long) op.getWorkDone(), (long) op.getTotalWork(), bytes, op.isPaused(), estimate.get()),
                    op.progressProperty(), op.pausedProperty(), estimate));

            pause.setOnAction(e -> {
                if (op.isPaused()) {
                    op.resume();
                } else {
                    op.pause();
                }
            });
            op.pausedProperty().addListener((o, was, paused) -> {
                pause.setText(paused ? "Resume" : "Pause");
                ((FontIcon) pause.getGraphic()).setIconLiteral(paused ? "mdi2p-play" : "mdi2p-pause");
                bar.pseudoClassStateChanged(PAUSED, paused);
            });

            cancel.setOnAction(e -> {
                cancel.setDisable(true);
                pause.setDisable(true);
                op.cancel();
            });
            Region spacer = new Region();
            HBox.setHgrow(spacer, Priority.ALWAYS);
            // the progress line gets its own row: with speed and time left it is too wide to share with the buttons
            detail.setMaxWidth(Double.MAX_VALUE);
            detail.setTextOverrun(OverrunStyle.LEADING_ELLIPSIS); // if it must be cut, keep the time left
            HBox bottom = new HBox(8, spacer, pause, cancel);
            bottom.setAlignment(Pos.CENTER_LEFT);

            box = new VBox(10, heading, current, bar, detail, bottom);
        }

        void tick(long nowNanos) {
            // waiting is not slow progress: measure afresh once the operation moves again
            if (op.isPaused() || question != null || op.getProgress() < 0 || op.isDone()) {
                eta.reset();
                estimate.set("");
                return;
            }
            eta.sample(nowNanos, (long) op.getWorkDone());
            double rate = eta.rate();
            String left = Eta.format(eta.secondsLeft((long) op.getTotalWork()));
            String speed = bytes && rate > 0 ? Formats.size((long) rate) + "/s" : "";
            estimate.set(speed.isEmpty() || left.isEmpty() ? left : speed + "   ·   " + left);
        }

        void ask(OperationUi.Conflict c, CompletableFuture<Decision> next) {
            dropQuestion();
            answer = next;
            Label q = new Label(c.question());
            q.getStyleClass().add(Styles.TEXT_BOLD);
            q.setWrapText(true);
            GridPane grid = new GridPane();
            grid.setHgap(12);
            grid.setVgap(4);
            grid.addRow(0, muted("Source:"), detail(c.source()));
            grid.addRow(1, muted("Existing:"), detail(c.existing()));
            CheckBox all = new CheckBox("Do this for all remaining conflicts");
            Button replace = new Button("Replace");
            Button skip = new Button("Skip");
            Button both = new Button("Keep both");
            replace.getStyleClass().add(Styles.ACCENT);
            replace.setOnAction(e -> answer(new Decision(Resolution.OVERWRITE, all.isSelected())));
            skip.setOnAction(e -> answer(new Decision(Resolution.SKIP, all.isSelected())));
            both.setOnAction(e -> answer(new Decision(Resolution.KEEP_BOTH, all.isSelected())));
            HBox buttons = new HBox(8, replace, skip, both);
            question = new VBox(8, q, grid, all, buttons);
            question.getStyleClass().add("conflict-question");
            box.getChildren().add(box.getChildren().size() - 1, question);
            pause.setDisable(true); // the operation is already waiting, for this answer
            replace.requestFocus();
        }

        private void answer(Decision d) {
            CompletableFuture<Decision> a = answer;
            answer = null; // answered: dropQuestion must not turn it into a Cancel
            dropQuestion();
            layoutRows();
            if (a != null) {
                a.complete(d);
            }
        }

        /** Removes the question; an unanswered one counts as Cancel (the operation is ending). */
        void dropQuestion() {
            if (question != null) {
                box.getChildren().remove(question);
                question = null;
                pause.setDisable(cancel.isDisabled());
            }
            if (answer != null && !answer.isDone()) {
                answer.complete(new Decision(Resolution.CANCEL, false));
            }
            answer = null;
        }

        private static Label muted(String text) {
            Label l = new Label(text);
            l.getStyleClass().add(Styles.TEXT_MUTED);
            return l;
        }

        private static Label detail(String text) {
            Label l = new Label(text);
            l.setWrapText(true);
            l.setMaxWidth(420);
            return l;
        }
    }
}
