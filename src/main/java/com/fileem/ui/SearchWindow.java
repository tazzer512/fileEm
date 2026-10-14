package com.fileem.ui;

import com.fileem.fs.FileEntry;
import com.fileem.fs.FileSearch;
import com.fileem.fs.Shell;
import com.fileem.util.Bg;
import com.fileem.util.Formats;
import com.fileem.util.FsUtil;
import javafx.application.Platform;
import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.collections.transformation.SortedList;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.Label;
import javafx.scene.control.OverrunStyle;
import javafx.scene.control.MenuItem;
import javafx.scene.control.SelectionMode;
import javafx.scene.control.TableCell;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableRow;
import javafx.scene.control.TableView;
import javafx.scene.control.TextField;
import javafx.scene.input.KeyCode;
import javafx.scene.input.MouseButton;
import javafx.scene.layout.BorderPane;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.stage.Stage;

import java.nio.file.Path;
import java.nio.file.attribute.FileTime;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Predicate;

/**
 * Finds files and folders by name (wildcards {@code *} and {@code ?}) in a folder and all folders
 * below it. A window of its own per search, owned by the main window. The search runs on a
 * {@link Bg} thread; results stream into the table in batches (at most one pending FX update), and
 * starting again, Stop or closing the window cancels it.
 */
final class SearchWindow {
    /** More results than anyone reads; stops a pattern like "*" on a whole disk. */
    static final int MAX_RESULTS = 50_000;

    private final MainWindow main;
    private final Path root;
    private final Stage stage = new Stage();
    private final TextField pattern = new TextField();
    private final CheckBox hidden = new CheckBox("Hidden files");
    private final Button go = new Button("Search");
    private final Label status = new Label();
    private final ObservableList<FileEntry> results = FXCollections.observableArrayList();
    private final TableView<FileEntry> table = new TableView<>();
    private Run current;

    /** One search: its results queue up here until the FX thread takes them. */
    private final class Run {
        final ConcurrentLinkedQueue<FileEntry> found = new ConcurrentLinkedQueue<>();
        final AtomicBoolean drainQueued = new AtomicBoolean();
        final AtomicLong folders = new AtomicLong();
        final AtomicLong unreadable = new AtomicLong();
        final AtomicLong count = new AtomicLong();
        final long started = System.nanoTime();
        Future<?> job;

        void add(FileEntry e) {
            found.add(e);
            count.incrementAndGet();
            if (drainQueued.compareAndSet(false, true)) {
                Platform.runLater(this::drain);
            }
        }

        void drain() {
            drainQueued.set(false);
            if (current != this) {
                return;
            }
            List<FileEntry> batch = new ArrayList<>();
            for (FileEntry e; (e = found.poll()) != null; ) {
                batch.add(e);
            }
            results.addAll(batch);
            status.setText("Searching…   " + summary());
        }

        String summary() {
            return Formats.count(results.size()) + (results.size() == 1 ? " item" : " items") + " found in "
                    + Formats.count(folders.get()) + (folders.get() == 1 ? " folder" : " folders");
        }
    }

    SearchWindow(MainWindow main, Stage owner, Path root, boolean showHidden) {
        this.main = main;
        this.root = root;
        stage.initOwner(owner);
        Icons.applyAppIcon(stage);
        stage.setTitle("Search in " + FsUtil.displayName(root));

        pattern.setId("search-pattern");
        pattern.setPromptText("Name or patterns, like *.jpg *.png or a*.* !*.mp3 (! excludes)");
        pattern.setTooltip(new javafx.scene.control.Tooltip(
                "* any text, ? one character. Separate patterns with spaces; ! in front excludes.\n"
                        + "Quotes keep spaces: \"my file*\". Text without wildcards matches anywhere in the name."));
        HBox.setHgrow(pattern, Priority.ALWAYS);
        pattern.setOnAction(e -> start());
        hidden.setId("search-hidden");
        hidden.setSelected(showHidden);
        go.setId("search-go");
        go.setDefaultButton(false);
        go.setOnAction(e -> {
            if (isRunning()) {
                stop();
            } else {
                start();
            }
        });
        Label in = new Label(root.toString());
        in.getStyleClass().add("search-root");
        in.setMnemonicParsing(false);
        HBox bar = new HBox(8, pattern, hidden, go);
        bar.setAlignment(Pos.CENTER_LEFT);
        HBox where = new HBox(6, Icons.icon("mdi2f-folder-search-outline"), in);
        where.setAlignment(Pos.CENTER_LEFT);
        BorderPane top = new BorderPane(bar, where, null, null, null);
        BorderPane.setMargin(bar, new Insets(6, 0, 0, 0));
        top.setPadding(new Insets(10));

        setUpTable();
        status.setId("search-status");
        status.getStyleClass().add("status-label");
        HBox bottom = new HBox(status);
        bottom.getStyleClass().add("status-bar");
        bottom.setPadding(new Insets(4, 10, 4, 10));

        BorderPane rootPane = new BorderPane(table, top, null, bottom, null);
        rootPane.getStyleClass().add("search-window");
        Scene scene = new Scene(rootPane, 820, 520);
        Theme.style(scene.getStylesheets());
        scene.setOnKeyPressed(e -> {
            if (e.getCode() == KeyCode.ESCAPE) {
                if (isRunning()) {
                    stop();
                } else {
                    stage.close();
                }
                e.consume();
            }
        });
        stage.setScene(scene);
        stage.setOnHidden(e -> {
            stop();
            main.searchClosed(this);
        });
        updateButton();
    }

    private void setUpTable() {
        table.setId("search-results");
        table.setPlaceholder(new Label("Type a name or a pattern and press Enter."));
        table.getSelectionModel().setSelectionMode(SelectionMode.MULTIPLE);
        TableColumn<FileEntry, FileEntry> name = column("Name", 240, e -> e);
        name.setCellFactory(c -> new FileTable.NameCell());
        name.setComparator((a, b) -> com.fileem.util.NaturalOrder.INSTANCE.compare(a.name(), b.name()));
        TableColumn<FileEntry, String> folder = column("Folder", 300, this::folderOf);
        folder.setCellFactory(c -> {
            TableCell<FileEntry, String> cell = text(v -> v, null);
            cell.setTextOverrun(OverrunStyle.LEADING_ELLIPSIS); // the end of a long path says the most
            return cell;
        });
        TableColumn<FileEntry, Long> size = column("Size", 90, e -> e.isDirectory() ? null : e.size());
        size.setCellFactory(c -> text(Formats::size, "align-right"));
        TableColumn<FileEntry, FileTime> modified = column("Date modified", 175, FileEntry::modified);
        modified.setCellFactory(c -> text(Formats::date, null));
        table.getColumns().setAll(List.of(name, folder, size, modified));
        SortedList<FileEntry> sorted = new SortedList<>(results);
        sorted.comparatorProperty().bind(table.comparatorProperty());
        table.setItems(sorted);
        table.setRowFactory(t -> {
            TableRow<FileEntry> row = new TableRow<>();
            row.setOnMouseClicked(e -> {
                if (e.getButton() == MouseButton.PRIMARY && e.getClickCount() == 2 && !row.isEmpty()) {
                    open(row.getItem());
                }
            });
            return row;
        });
        table.setOnKeyPressed(e -> {
            FileEntry sel = table.getSelectionModel().getSelectedItem();
            if (e.getCode() == KeyCode.ENTER && sel != null) {
                open(sel);
                e.consume();
            }
        });
        Views.dynamicContextMenu(table, this::menuItems);
    }

    private static <T> TableColumn<FileEntry, T> column(String title, double width,
                                                         java.util.function.Function<FileEntry, T> getter) {
        TableColumn<FileEntry, T> c = new TableColumn<>(title);
        c.setPrefWidth(width);
        c.setCellValueFactory(cd -> new ReadOnlyObjectWrapper<>(getter.apply(cd.getValue())));
        return c;
    }

    private static <T> TableCell<FileEntry, T> text(java.util.function.Function<T, String> format, String styleClass) {
        TableCell<FileEntry, T> cell = new TableCell<>() {
            @Override
            protected void updateItem(T value, boolean empty) {
                super.updateItem(value, empty);
                setText(empty || value == null ? null : format.apply(value));
            }
        };
        if (styleClass != null) {
            cell.getStyleClass().add(styleClass);
        }
        return cell;
    }

    /** The result's folder, relative to where the search started ("." for that folder itself). */
    private String folderOf(FileEntry e) {
        Path parent = e.path().getParent();
        if (parent == null || !parent.startsWith(root)) {
            return parent == null ? "" : parent.toString();
        }
        String rel = root.relativize(parent).toString();
        return rel.isEmpty() ? "." : rel;
    }

    private List<MenuItem> menuItems() {
        List<FileEntry> sel = List.copyOf(table.getSelectionModel().getSelectedItems());
        if (sel.isEmpty()) {
            return List.of();
        }
        FileEntry first = sel.get(0);
        List<MenuItem> items = new ArrayList<>();
        items.add(MainWindow.item("Open", "mdi2o-open-in-app", () -> sel.forEach(this::open)));
        items.add(MainWindow.item("Open containing folder", "mdi2f-folder-open-outline", () -> showInFolder(first)));
        items.add(MainWindow.item("Copy path", "mdi2c-clipboard-text-outline",
                () -> main.copyPaths(sel.stream().map(FileEntry::path).toList())));
        return items;
    }

    /** A folder opens in the main window's active tab; a file opens with its program. */
    void open(FileEntry e) {
        if (e.isDirectory()) {
            main.tab().navigateTo(e.path());
            focusMain();
        } else {
            Shell.open(e.path(), msg -> Dialogs.error(stage, "Open", "Cannot open " + e.name() + ":\n" + msg));
        }
    }

    /** Opens the folder holding {@code e} in the main window's active tab, with {@code e} selected. */
    void showInFolder(FileEntry e) {
        main.tab().showItem(e.path());
        focusMain();
    }

    private void focusMain() {
        if (stage.getOwner() instanceof Stage owner) {
            owner.toFront();
            owner.requestFocus();
        }
        main.tab().focusTable();
    }

    void show() {
        stage.show();
        stage.toFront();
        pattern.requestFocus();
    }

    /** Starts searching for the pattern in the field, replacing the results of an earlier search. */
    void start() {
        Predicate<String> match = FileSearch.matcher(pattern.getText());
        stop();
        results.clear();
        if (match == null) {
            status.setText("Type a name or a pattern to search for.");
            return;
        }
        Run run = new Run();
        current = run;
        boolean includeHidden = hidden.isSelected();
        status.setText("Searching…");
        run.job = Bg.run(() -> FileSearch.search(root, match, includeHidden, new FileSearch.Listener() {
            @Override
            public void found(FileEntry entry) {
                run.add(entry);
            }

            @Override
            public void entering(Path dir) {
                run.folders.incrementAndGet();
            }

            @Override
            public void failed(Path path) {
                run.unreadable.incrementAndGet();
            }

            @Override
            public boolean wantsMore() {
                return run.count.get() < MAX_RESULTS; // finish() tells why it stopped
            }
        }), complete -> finish(run, complete), err -> {
            if (current == run) {
                current = null;
                run.drain();
                status.setText("Cannot search " + root + ": " + err.getMessage());
                updateButton();
            }
        });
        updateButton();
    }

    private void finish(Run run, boolean complete) {
        if (current != run) {
            return;
        }
        run.drain();
        current = null;
        long seconds = (System.nanoTime() - run.started) / 1_000_000_000L;
        String text = run.summary();
        if (!complete && run.count.get() >= MAX_RESULTS) {
            text = "Stopped at " + Formats.count(MAX_RESULTS) + " items: use a narrower pattern.   " + text;
        }
        if (seconds > 0) {
            text += " (" + seconds + " s)";
        }
        if (run.unreadable.get() > 0) {
            text += "   ·   " + Formats.count(run.unreadable.get()) + " could not be read";
        }
        status.setText(text);
        updateButton();
    }

    /** Stops a running search; the results so far stay. */
    void stop() {
        Run run = current;
        if (run == null) {
            return;
        }
        current = null;
        run.job.cancel(true);
        List<FileEntry> rest = new ArrayList<>();
        for (FileEntry e; (e = run.found.poll()) != null; ) {
            rest.add(e);
        }
        results.addAll(rest);
        status.setText("Stopped.   " + run.summary());
        updateButton();
    }

    boolean isRunning() {
        return current != null;
    }

    private void updateButton() {
        boolean running = isRunning();
        go.setText(running ? "Stop" : "Search");
        go.setGraphic(Icons.icon(running ? "mdi2s-stop" : "mdi2m-magnify"));
    }

    // ---- test hooks ----

    Stage stage() {
        return stage;
    }

    Path root() {
        return root;
    }

    TextField patternField() {
        return pattern;
    }

    CheckBox hiddenBox() {
        return hidden;
    }

    TableView<FileEntry> table() {
        return table;
    }

    String statusText() {
        return status.getText();
    }
}
