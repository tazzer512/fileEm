package com.fileem.ui;

import atlantafx.base.theme.Styles;
import com.fileem.fs.FileEntry;
import com.fileem.util.Formats;
import com.fileem.util.FsUtil;
import javafx.application.Platform;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.control.ButtonType;
import javafx.scene.control.Dialog;
import javafx.scene.control.Label;
import javafx.scene.control.Separator;
import javafx.scene.control.TextField;
import javafx.scene.layout.GridPane;
import javafx.scene.layout.HBox;
import javafx.stage.Modality;
import javafx.stage.Window;
import org.kordamp.ikonli.javafx.FontIcon;

import java.io.IOException;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;

/** Properties of one or more items; folder sizes are calculated in the background. */
final class PropertiesDialog {
    private PropertiesDialog() {
    }

    static void show(Window owner, List<Path> paths) {
        Dialog<Void> d = new Dialog<>();
        Dialogs.prepare(d, owner);
        d.initModality(Modality.NONE);
        d.getDialogPane().getButtonTypes().add(ButtonType.CLOSE);
        d.setResizable(true);

        GridPane grid = new GridPane();
        grid.setHgap(14);
        grid.setVgap(8);
        grid.setPadding(new Insets(4, 4, 4, 4));
        grid.setMinWidth(440);
        int row = 0;

        Label sizeValue = new Label("Calculating…");
        Label containsValue = new Label("");
        boolean needsWalk;

        if (paths.size() == 1) {
            Path p = paths.get(0);
            FileEntry e = FileEntry.read(p);
            d.setTitle(FsUtil.displayName(p) + " Properties");

            FontIcon icon = Icons.icon("mdi2f-file-outline", 32);
            Icons.apply(icon, e);
            icon.setIconSize(32);
            TextField name = readOnly(FsUtil.displayName(p));
            name.getStyleClass().add(Styles.TITLE_4);
            HBox header = new HBox(12, icon, name);
            header.setAlignment(Pos.CENTER_LEFT);
            grid.add(header, 0, row++, 2, 1);
            grid.add(new Separator(), 0, row++, 2, 1);

            row = addRow(grid, row, "Type:", new Label(e.type()));
            row = addRow(grid, row, "Location:", readOnly(p.getParent() != null ? p.getParent().toString() : ""));
            if (e.isSymlink()) {
                String target;
                try {
                    target = Files.readSymbolicLink(p).toString();
                } catch (IOException ex) {
                    target = "(unreadable)";
                }
                row = addRow(grid, row, "Link target:", readOnly(target));
            }
            row = addRow(grid, row, "Size:", sizeValue);
            if (e.isDirectory()) {
                row = addRow(grid, row, "Contains:", containsValue);
                needsWalk = true;
            } else {
                sizeValue.setText(Formats.sizeWithExact(Math.max(0, e.size())));
                needsWalk = false;
            }
            grid.add(new Separator(), 0, row++, 2, 1);
            row = addRow(grid, row, "Created:", new Label(Formats.dateLong(e.created())));
            row = addRow(grid, row, "Modified:", new Label(Formats.dateLong(e.modified())));
            row = addRow(grid, row, "Accessed:", new Label(Formats.dateLong(e.accessed())));
            if (!e.attributes().isEmpty()) {
                grid.add(new Separator(), 0, row++, 2, 1);
                Label attrs = new Label(e.attributes());
                attrs.getStyleClass().add("mono");
                row = addRow(grid, row, "Attributes:", attrs);
            }
            row = addRow(grid, row, "Hidden:", new Label(e.isHidden() ? "Yes" : "No"));
        } else {
            d.setTitle(paths.size() + " items Properties");
            long files = paths.stream().filter(Files::isRegularFile).count();
            Label header = new Label(paths.size() + " items selected");
            header.getStyleClass().add(Styles.TITLE_4);
            grid.add(header, 0, row++, 2, 1);
            grid.add(new Separator(), 0, row++, 2, 1);
            Path parent = paths.get(0).getParent();
            boolean sameParent = paths.stream().allMatch(p -> parent != null && parent.equals(p.getParent()));
            if (sameParent) {
                row = addRow(grid, row, "Location:", readOnly(parent.toString()));
            }
            row = addRow(grid, row, "Selected:", new Label(files + " files, " + (paths.size() - files) + " folders"));
            row = addRow(grid, row, "Size:", sizeValue);
            row = addRow(grid, row, "Contains:", containsValue);
            needsWalk = true;
        }
        d.getDialogPane().setContent(grid);

        AtomicBoolean closed = new AtomicBoolean();
        d.setOnHidden(e -> closed.set(true));
        if (needsWalk) {
            Thread t = new Thread(() -> measure(paths, sizeValue, containsValue, closed), "fileem-properties");
            t.setDaemon(true);
            t.start();
        }
        d.show();
    }

    private static void measure(List<Path> paths, Label sizeValue, Label containsValue, AtomicBoolean closed) {
        long[] totals = new long[3]; // bytes, files, folders
        long[] lastUpdate = {0};
        Runnable publish = () -> {
            long bytes = totals[0], files = totals[1], dirs = totals[2];
            Platform.runLater(() -> {
                sizeValue.setText(Formats.sizeWithExact(bytes));
                containsValue.setText(Formats.count(files) + " files, " + Formats.count(dirs) + " folders");
            });
        };
        for (Path root : paths) {
            try {
                Files.walkFileTree(root, new SimpleFileVisitor<>() {
                    @Override
                    public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) {
                        if (closed.get()) {
                            return FileVisitResult.TERMINATE;
                        }
                        if (!dir.equals(root) || paths.size() > 1) {
                            totals[2]++;
                        }
                        return FsUtil.isJunction(dir) ? FileVisitResult.SKIP_SUBTREE : FileVisitResult.CONTINUE;
                    }

                    @Override
                    public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) {
                        totals[0] += attrs.isRegularFile() ? attrs.size() : 0;
                        totals[1]++;
                        long now = System.nanoTime();
                        if (now - lastUpdate[0] > 150_000_000L) {
                            lastUpdate[0] = now;
                            publish.run();
                        }
                        return closed.get() ? FileVisitResult.TERMINATE : FileVisitResult.CONTINUE;
                    }

                    @Override
                    public FileVisitResult visitFileFailed(Path file, IOException exc) {
                        return FileVisitResult.CONTINUE;
                    }
                });
            } catch (IOException ignored) {
                // partial result
            }
        }
        publish.run();
    }

    private static int addRow(GridPane grid, int row, String label, javafx.scene.Node value) {
        Label l = new Label(label);
        l.getStyleClass().add(Styles.TEXT_MUTED);
        grid.addRow(row, l, value);
        return row + 1;
    }

    private static TextField readOnly(String text) {
        TextField f = new TextField(text);
        f.setEditable(false);
        f.getStyleClass().add("readonly-field");
        f.setPrefColumnCount(30);
        return f;
    }
}
