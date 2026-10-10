package com.fileem.ui;

import com.fileem.util.FsUtil;
import javafx.application.Platform;
import javafx.scene.control.Alert;
import javafx.scene.control.ButtonBar;
import javafx.scene.control.ButtonType;
import javafx.scene.control.Dialog;
import javafx.scene.control.Label;
import javafx.scene.control.TextArea;
import javafx.scene.control.TextField;
import javafx.scene.control.TextInputDialog;
import javafx.scene.layout.VBox;
import javafx.stage.Stage;
import javafx.stage.Window;

import java.nio.file.InvalidPathException;
import java.nio.file.Path;
import java.util.List;
import java.util.Optional;
import java.util.function.Function;

public final class Dialogs {
    private Dialogs() {
    }

    public static void prepare(Dialog<?> d, Window owner) {
        if (owner != null) {
            d.initOwner(owner);
        }
        Theme.style(d.getDialogPane().getStylesheets());
        if (d.getDialogPane().getScene().getWindow() instanceof Stage s) {
            Icons.applyAppIcon(s);
        }
    }

    public static boolean confirm(Window owner, String title, String message) {
        Alert a = new Alert(Alert.AlertType.CONFIRMATION, message, ButtonType.OK, ButtonType.CANCEL);
        prepare(a, owner);
        a.setTitle(title);
        a.setHeaderText(null);
        return a.showAndWait().filter(b -> b == ButtonType.OK).isPresent();
    }

    /**
     * Asks what a drop should do: Move (the default button), Copy, or nothing. Returns true for move,
     * false for copy, empty when cancelled.
     */
    static Optional<Boolean> moveOrCopy(Window owner, String what, Path target) {
        ButtonType move = new ButtonType("Move", ButtonBar.ButtonData.OK_DONE);
        ButtonType copy = new ButtonType("Copy", ButtonBar.ButtonData.OTHER);
        Alert a = new Alert(Alert.AlertType.CONFIRMATION, null, move, copy, ButtonType.CANCEL);
        prepare(a, owner);
        a.setTitle("Move or copy");
        a.setHeaderText("Move " + what + " to \"" + FsUtil.displayName(target) + "\"?");
        a.setContentText("To: " + target + "\n\nChoose Copy to keep the originals where they are.");
        return a.showAndWait().filter(b -> b == move || b == copy).map(b -> b == move);
    }

    /** A short notice that doesn't block (the app stays usable while it shows). */
    static Alert info(Window owner, String title, String message) {
        Alert a = new Alert(Alert.AlertType.INFORMATION, message, ButtonType.OK);
        prepare(a, owner);
        a.initModality(javafx.stage.Modality.NONE);
        a.setTitle(title);
        a.setHeaderText(null);
        a.show();
        return a;
    }

    public static void error(Window owner, String title, String message) {
        Alert a = new Alert(Alert.AlertType.ERROR, message, ButtonType.OK);
        prepare(a, owner);
        a.setTitle(title);
        a.setHeaderText(null);
        a.showAndWait();
    }

    public static void errorList(Window owner, String title, String header, List<String> lines) {
        Alert a = new Alert(Alert.AlertType.ERROR);
        prepare(a, owner);
        a.setTitle(title);
        a.setHeaderText(header);
        TextArea area = new TextArea(String.join("\n", lines));
        area.setEditable(false);
        area.setWrapText(false);
        area.setPrefRowCount(Math.min(12, lines.size() + 1));
        area.setPrefColumnCount(60);
        a.getDialogPane().setContent(area);
        a.setResizable(true);
        a.showAndWait();
    }

    /**
     * Asks for a file name. Only the base name is preselected so typing keeps the extension.
     * {@code validator} returns an error message or null.
     */
    public static Optional<String> askName(Window owner, String title, String prompt, String initial,
                                           Function<String, String> validator) {
        TextInputDialog d = new TextInputDialog(initial);
        prepare(d, owner);
        d.setTitle(title);
        d.setHeaderText(null);
        d.setContentText(prompt);
        TextField editor = d.getEditor();
        editor.setPrefColumnCount(32);
        d.setOnShown(e -> Platform.runLater(() -> {
            editor.requestFocus();
            int dot = initial.lastIndexOf('.');
            editor.selectRange(0, dot > 0 ? dot : initial.length());
        }));
        Label error = new Label();
        error.getStyleClass().add("dialog-error");
        d.getDialogPane().lookupButton(ButtonType.OK).addEventFilter(javafx.event.ActionEvent.ACTION, e -> {
            String msg = FsUtil.validateName(editor.getText().trim());
            if (msg == null && validator != null) {
                msg = validator.apply(editor.getText().trim());
            }
            if (msg != null) {
                error.setText(msg);
                if (!(d.getDialogPane().getContent() instanceof VBox)) {
                    d.getDialogPane().setContent(new VBox(8, new Label(prompt), editor, error));
                }
                e.consume();
            }
        });
        return d.showAndWait().map(String::trim);
    }

    /** Free text prompt (e.g. a bookmark name); blank answers are treated as cancel. */
    public static Optional<String> askText(Window owner, String title, String prompt, String initial) {
        TextInputDialog d = new TextInputDialog(initial);
        prepare(d, owner);
        d.setTitle(title);
        d.setHeaderText(null);
        d.setContentText(prompt);
        d.getEditor().setPrefColumnCount(32);
        d.setOnShown(e -> Platform.runLater(() -> {
            d.getEditor().requestFocus();
            d.getEditor().selectAll();
        }));
        return d.showAndWait().map(String::strip).filter(t -> !t.isEmpty());
    }

    /** Commander style "copy to" prompt with an editable target folder. */
    public static Optional<Path> askTarget(Window owner, String title, String message, Path initial) {
        TextInputDialog d = new TextInputDialog(initial == null ? "" : initial.toString());
        prepare(d, owner);
        d.setTitle(title);
        d.setHeaderText(message);
        d.setContentText("To:");
        d.getEditor().setPrefColumnCount(42);
        Optional<String> text = d.showAndWait();
        if (text.isEmpty() || text.get().isBlank()) {
            return Optional.empty();
        }
        try {
            return Optional.of(Path.of(text.get().trim()));
        } catch (InvalidPathException e) {
            error(owner, title, "Invalid path: " + text.get());
            return Optional.empty();
        }
    }
}
