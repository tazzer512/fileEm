package com.fileem.ui;

import atlantafx.base.theme.Styles;
import com.fileem.ops.FileOperation;
import com.fileem.ops.FileOperation.Decision;
import com.fileem.ops.FileOperation.Kind;
import com.fileem.ops.FileOperation.Resolution;
import com.fileem.util.Bg;
import com.fileem.util.Formats;
import com.fileem.util.FsUtil;
import javafx.animation.PauseTransition;
import javafx.application.Platform;
import javafx.beans.binding.Bindings;
import javafx.concurrent.Worker;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.Scene;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.ButtonBar;
import javafx.scene.control.ButtonType;
import javafx.scene.control.CheckBox;
import javafx.scene.control.Label;
import javafx.scene.control.OverrunStyle;
import javafx.scene.control.ProgressBar;
import javafx.scene.layout.GridPane;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.scene.layout.Region;
import javafx.scene.layout.VBox;
import javafx.stage.Stage;
import javafx.stage.Window;
import javafx.util.Duration;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.HashSet;
import java.util.List;
import java.util.Optional;
import java.util.Set;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.FutureTask;
import java.util.function.Consumer;

/** Runs a {@link FileOperation} with a progress window and conflict prompts. */
final class OperationUi {
    private OperationUi() {
    }

    /** Starts the operation; {@code onDone} gets the folders whose contents changed. */
    static FileOperation run(Window owner, Kind kind, List<Path> sources, Path target, Consumer<Set<Path>> onDone) {
        FileOperation op = new FileOperation(kind, sources, target, (s, t) -> askOnFxThread(owner, s, t));
        String title = title(kind, sources, target);
        ProgressWindow window = new ProgressWindow(owner, op, title);
        op.stateProperty().addListener((obs, old, state) -> {
            if (state == Worker.State.SUCCEEDED || state == Worker.State.FAILED || state == Worker.State.CANCELLED) {
                window.close();
                Set<Path> affected = new HashSet<>();
                if (target != null) {
                    affected.add(target);
                }
                for (Path s : sources) {
                    if (s.getParent() != null) {
                        affected.add(s.getParent());
                    }
                }
                onDone.accept(affected);
                List<String> errors = op.errors();
                if (state == Worker.State.FAILED && op.getException() != null) {
                    Dialogs.error(owner, title, String.valueOf(op.getException().getMessage()));
                } else if (!errors.isEmpty()) {
                    Dialogs.errorList(owner, title,
                            errors.size() == 1 ? "1 item could not be processed." : errors.size() + " items could not be processed.",
                            errors);
                }
            }
        });
        Bg.POOL.execute(op);
        window.showDelayed();
        return op;
    }

    private static String title(Kind kind, List<Path> sources, Path target) {
        String what = sources.size() == 1 ? "\"" + FsUtil.displayName(sources.get(0)) + "\"" : sources.size() + " items";
        return switch (kind) {
            case COPY -> "Copying " + what + " to " + target;
            case MOVE -> "Moving " + what + " to " + target;
            case DELETE -> "Deleting " + what;
            case TRASH -> "Moving " + what + " to Trash";
            case COMPRESS -> "Compressing " + what;
            case EXTRACT -> "Extracting " + what;
        };
    }

    private static Decision askOnFxThread(Window owner, Path source, Path target) throws InterruptedException {
        FutureTask<Decision> task = new FutureTask<>(() -> askConflict(owner, source, target));
        Platform.runLater(task);
        try {
            return task.get();
        } catch (ExecutionException e) {
            return new Decision(Resolution.CANCEL, false);
        }
    }

    private static Decision askConflict(Window owner, Path source, Path target) {
        Alert a = new Alert(Alert.AlertType.WARNING);
        Dialogs.prepare(a, owner);
        a.setTitle("Replace or skip");
        boolean dir = Files.isDirectory(target, LinkOption.NOFOLLOW_LINKS);
        a.setHeaderText("The destination already has " + (dir ? "a folder" : "a file") + " named \""
                + FsUtil.displayName(target) + "\".");

        GridPane grid = new GridPane();
        grid.setHgap(12);
        grid.setVgap(6);
        grid.addRow(0, bold("Source:"), new Label(describe(source)));
        grid.addRow(1, bold("Existing:"), new Label(describe(target)));
        CheckBox all = new CheckBox("Do this for all remaining conflicts");
        VBox content = new VBox(12, grid, all);
        a.getDialogPane().setContent(content);

        ButtonType replace = new ButtonType("Replace", ButtonBar.ButtonData.OK_DONE);
        ButtonType skip = new ButtonType("Skip", ButtonBar.ButtonData.NO);
        ButtonType both = new ButtonType("Keep both", ButtonBar.ButtonData.OTHER);
        ButtonType cancel = new ButtonType("Cancel", ButtonBar.ButtonData.CANCEL_CLOSE);
        a.getButtonTypes().setAll(replace, skip, both, cancel);

        Optional<ButtonType> r = a.showAndWait();
        Resolution res = r.map(b -> b == replace ? Resolution.OVERWRITE
                : b == skip ? Resolution.SKIP
                : b == both ? Resolution.KEEP_BOTH
                : Resolution.CANCEL).orElse(Resolution.CANCEL);
        return new Decision(res, all.isSelected());
    }

    private static Label bold(String text) {
        Label l = new Label(text);
        l.getStyleClass().add(Styles.TEXT_BOLD);
        return l;
    }

    private static String describe(Path p) {
        try {
            BasicFileAttributes a = Files.readAttributes(p, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
            String size = a.isDirectory() ? "Folder" : Formats.size(a.size());
            return size + ", modified " + Formats.date(a.lastModifiedTime()) + "\n" + p;
        } catch (IOException e) {
            return p.toString();
        }
    }

    private static final class ProgressWindow {
        private final Stage stage = new Stage();
        private boolean finished;

        ProgressWindow(Window owner, FileOperation op, String title) {
            stage.initOwner(owner);
            stage.setTitle(title);

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

            Button cancel = new Button("Cancel");
            cancel.setCancelButton(true);
            cancel.setOnAction(e -> op.cancel());
            Region spacer = new Region();
            HBox.setHgrow(spacer, Priority.ALWAYS);
            HBox bottom = new HBox(8, detail, spacer, cancel);
            bottom.setAlignment(Pos.CENTER_LEFT);

            VBox box = new VBox(10, heading, current, bar, bottom);
            box.setPadding(new Insets(16));
            box.setPrefWidth(520);
            Scene scene = new Scene(box);
            scene.getStylesheets().add(Theme.APP_CSS);
            stage.setScene(scene);
            stage.setResizable(false);
            stage.setOnCloseRequest(e -> op.cancel());
        }

        void showDelayed() {
            PauseTransition delay = new PauseTransition(Duration.millis(350));
            delay.setOnFinished(e -> {
                if (!finished) {
                    stage.show();
                }
            });
            delay.play();
        }

        void close() {
            finished = true;
            stage.close();
        }
    }
}
