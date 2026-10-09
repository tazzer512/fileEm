package com.fileem.ui;

import atlantafx.base.theme.Styles;
import com.fileem.Bookmarks;
import com.fileem.Bookmarks.Bookmark;
import javafx.beans.binding.Bindings;
import javafx.beans.binding.BooleanBinding;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.control.Button;
import javafx.scene.control.ButtonType;
import javafx.scene.control.Dialog;
import javafx.scene.control.Label;
import javafx.scene.control.ListCell;
import javafx.scene.control.ListView;
import javafx.scene.input.KeyCode;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.scene.layout.VBox;
import javafx.stage.Modality;
import javafx.stage.Window;
import org.kordamp.ikonli.javafx.FontIcon;

import java.nio.file.Files;
import java.nio.file.Path;

/** Lists all bookmarks with open / rename / remove / reorder. Changes apply immediately. */
final class BookmarksDialog {
    private BookmarksDialog() {
    }

    static void show(Window owner, MainWindow main) {
        Bookmarks bookmarks = main.bookmarks();
        Dialog<Void> d = new Dialog<>();
        Dialogs.prepare(d, owner);
        d.initModality(Modality.NONE);
        d.setTitle("Manage bookmarks");
        d.setResizable(true);
        d.getDialogPane().getButtonTypes().add(ButtonType.CLOSE);

        ListView<Bookmark> list = new ListView<>(bookmarks.items());
        list.getStyleClass().add("bookmark-list");
        list.setPrefSize(520, 320);
        list.setPlaceholder(new Label("No bookmarks yet.\nOpen a folder and press Ctrl+D to add one."));
        list.setCellFactory(lv -> new BookmarkCell());

        Button open = action("Open", "mdi2f-folder-open");
        Button rename = action("Rename…", "mdi2p-pencil");
        Button remove = action("Remove", "mdi2b-bookmark-remove");
        Button up = action("Move up", "mdi2a-arrow-up");
        Button down = action("Move down", "mdi2a-arrow-down");

        BooleanBinding none = list.getSelectionModel().selectedItemProperty().isNull();
        open.disableProperty().bind(none);
        rename.disableProperty().bind(none);
        remove.disableProperty().bind(none);
        up.disableProperty().bind(none.or(list.getSelectionModel().selectedIndexProperty().lessThanOrEqualTo(0)));
        down.disableProperty().bind(none.or(list.getSelectionModel().selectedIndexProperty()
                .greaterThanOrEqualTo(Bindings.size(bookmarks.items()).subtract(1))));

        Runnable openSelected = () -> {
            Bookmark b = list.getSelectionModel().getSelectedItem();
            if (b != null) {
                main.openBookmark(b.path());
                d.close();
            }
        };
        open.setOnAction(e -> openSelected.run());
        rename.setOnAction(e -> {
            Bookmark b = list.getSelectionModel().getSelectedItem();
            if (b != null) {
                Dialogs.askText(d.getDialogPane().getScene().getWindow(), "Rename bookmark", "Name:", b.name())
                        .ifPresent(name -> {
                            bookmarks.rename(b.path(), name);
                            reselect(list, b.path());
                        });
            }
        });
        remove.setOnAction(e -> {
            Bookmark b = list.getSelectionModel().getSelectedItem();
            if (b != null) {
                int index = list.getSelectionModel().getSelectedIndex();
                bookmarks.remove(b.path());
                if (!bookmarks.items().isEmpty()) {
                    list.getSelectionModel().select(Math.min(index, bookmarks.items().size() - 1));
                }
            }
        });
        up.setOnAction(e -> moveSelected(list, bookmarks, -1));
        down.setOnAction(e -> moveSelected(list, bookmarks, 1));

        list.setOnMouseClicked(e -> {
            if (e.getClickCount() == 2) {
                openSelected.run();
            }
        });
        list.setOnKeyPressed(e -> {
            if (e.getCode() == KeyCode.ENTER) {
                openSelected.run();
            } else if (e.getCode() == KeyCode.DELETE) {
                remove.fire();
            } else if (e.getCode() == KeyCode.F2) {
                rename.fire();
            } else if (e.isAltDown() && e.getCode() == KeyCode.UP) {
                up.fire();
            } else if (e.isAltDown() && e.getCode() == KeyCode.DOWN) {
                down.fire();
            } else {
                return;
            }
            e.consume();
        });

        VBox buttons = new VBox(6, open, rename, remove, up, down);
        buttons.setFillWidth(true);
        HBox content = new HBox(10, list, buttons);
        HBox.setHgrow(list, Priority.ALWAYS);
        content.setPadding(new Insets(4));
        d.getDialogPane().setContent(content);
        if (!bookmarks.items().isEmpty()) {
            list.getSelectionModel().select(0);
        }
        d.show();
        list.requestFocus();
    }

    private static void moveSelected(ListView<Bookmark> list, Bookmarks bookmarks, int delta) {
        Bookmark b = list.getSelectionModel().getSelectedItem();
        if (b != null) {
            bookmarks.move(b.path(), delta);
            reselect(list, b.path());
        }
    }

    private static void reselect(ListView<Bookmark> list, Path path) {
        for (int i = 0; i < list.getItems().size(); i++) {
            if (list.getItems().get(i).path().equals(path)) {
                list.getSelectionModel().select(i);
                list.scrollTo(i);
                return;
            }
        }
    }

    private static Button action(String text, String icon) {
        Button b = new Button(text, Icons.icon(icon));
        b.setMaxWidth(Double.MAX_VALUE);
        b.setAlignment(Pos.CENTER_LEFT);
        return b;
    }

    private static final class BookmarkCell extends ListCell<Bookmark> {
        private final FontIcon icon = Icons.icon("mdi2b-bookmark", 18);
        private final Label name = new Label();
        private final Label path = new Label();
        private final HBox box;

        BookmarkCell() {
            path.getStyleClass().addAll(Styles.TEXT_MUTED, Styles.TEXT_SMALL);
            VBox text = new VBox(2, name, path);
            box = new HBox(10, icon, text);
            box.setAlignment(Pos.CENTER_LEFT);
        }

        @Override
        protected void updateItem(Bookmark b, boolean empty) {
            super.updateItem(b, empty);
            setText(null);
            if (empty || b == null) {
                setGraphic(null);
                return;
            }
            boolean exists = Files.isDirectory(b.path());
            Icons.apply(icon, exists ? "mdi2b-bookmark" : "mdi2b-bookmark-remove", exists ? "icon-bookmark" : "icon-missing");
            name.setText(b.name());
            path.setText(exists ? b.path().toString() : b.path() + "  (not found)");
            setGraphic(box);
        }
    }
}
