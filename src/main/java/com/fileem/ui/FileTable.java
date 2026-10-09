package com.fileem.ui;

import com.fileem.Session;
import com.fileem.fs.FileEntry;
import com.fileem.util.Formats;
import com.fileem.util.NaturalOrder;
import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.collections.ListChangeListener;
import javafx.css.PseudoClass;
import javafx.scene.control.Label;
import javafx.scene.control.SelectionMode;
import javafx.scene.control.TableCell;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableRow;
import javafx.scene.control.TableView;
import javafx.scene.input.ClipboardContent;
import javafx.scene.input.Dragboard;
import javafx.scene.input.KeyEvent;
import javafx.scene.input.MouseButton;
import javafx.scene.input.TransferMode;
import org.kordamp.ikonli.javafx.FontIcon;

import java.nio.file.Path;
import java.nio.file.attribute.FileTime;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.function.Function;

/** The sortable, multi-column file list of one tab. */
final class FileTable extends TableView<FileEntry> {
    private static final PseudoClass HIDDEN = PseudoClass.getPseudoClass("hidden-file");
    private static final Set<String> DEFAULT_VISIBLE = Set.of("name", "size", "type", "modified");

    private final MainWindow main;
    private final FileTab tab;
    private final StringBuilder typeAhead = new StringBuilder();
    private long lastTyped;

    FileTable(MainWindow main, FileTab tab) {
        this.main = main;
        this.tab = tab;
        getStyleClass().addAll("file-table", atlantafx.base.theme.Styles.DENSE);
        getSelectionModel().setSelectionMode(SelectionMode.MULTIPLE);
        setTableMenuButtonVisible(true);
        setColumnResizePolicy(TableView.UNCONSTRAINED_RESIZE_POLICY);
        setPlaceholder(new Label("This folder is empty."));

        TableColumn<FileEntry, FileEntry> name = column("name", "Name", 300, e -> e);
        name.setComparator((a, b) -> NaturalOrder.INSTANCE.compare(a.name(), b.name()));
        name.setCellFactory(c -> new NameCell());

        TableColumn<FileEntry, Long> size = column("size", "Size", 95, FileEntry::size);
        size.setCellFactory(c -> formatted(Formats::size, true));

        TableColumn<FileEntry, String> type = column("type", "Type", 140, FileEntry::type);
        type.setComparator(NaturalOrder.INSTANCE);

        TableColumn<FileEntry, FileTime> modified = column("modified", "Date modified", 165, FileEntry::modified);
        modified.setCellFactory(c -> formatted(Formats::date, false));
        TableColumn<FileEntry, FileTime> created = column("created", "Date created", 165, FileEntry::created);
        created.setCellFactory(c -> formatted(Formats::date, false));
        TableColumn<FileEntry, FileTime> accessed = column("accessed", "Date accessed", 165, FileEntry::accessed);
        accessed.setCellFactory(c -> formatted(Formats::date, false));

        TableColumn<FileEntry, String> ext = column("extension", "Extension", 80, FileEntry::extension);
        TableColumn<FileEntry, String> attrs = column("attributes", "Attributes", 160, FileEntry::attributes);
        attrs.getStyleClass().add("mono");

        getColumns().setAll(List.of(name, size, type, modified, created, accessed, ext, attrs));
        restoreColumns(main.session());

        setRowFactory(tv -> new FileRow());
        addEventHandler(KeyEvent.KEY_TYPED, this::onKeyTyped);

        Views.dynamicContextMenu(this, () -> main.fileMenuItems(tab));
    }

    private static <T> TableColumn<FileEntry, T> column(String id, String title, double width, Function<FileEntry, T> getter) {
        TableColumn<FileEntry, T> c = new TableColumn<>(title);
        c.setId(id);
        c.setPrefWidth(width);
        c.setCellValueFactory(cd -> new ReadOnlyObjectWrapper<>(getter.apply(cd.getValue())));
        return c;
    }

    private static <T> TableCell<FileEntry, T> formatted(Function<T, String> format, boolean alignRight) {
        TableCell<FileEntry, T> cell = new TableCell<>() {
            @Override
            protected void updateItem(T value, boolean empty) {
                super.updateItem(value, empty);
                setText(empty || value == null ? null : format.apply(value));
            }
        };
        if (alignRight) {
            cell.getStyleClass().add("align-right");
        }
        return cell;
    }

    /** Folders always first, then the user's sort, then name. */
    Comparator<FileEntry> effectiveComparator() {
        Comparator<FileEntry> dirsFirst = Comparator.comparing(e -> !e.isDirectory());
        Comparator<FileEntry> byName = (a, b) -> NaturalOrder.INSTANCE.compare(a.name(), b.name());
        Comparator<FileEntry> user = getComparator();
        return user == null ? dirsFirst.thenComparing(byName) : dirsFirst.thenComparing(user).thenComparing(byName);
    }

    /** Id of the primary sort column, or null when unsorted. */
    String sortColumnId() {
        return getSortOrder().isEmpty() ? null : getSortOrder().get(0).getId();
    }

    boolean sortAscending() {
        return getSortOrder().isEmpty() || getSortOrder().get(0).getSortType() == TableColumn.SortType.ASCENDING;
    }

    /** Sorts by the column with {@code id} (making it visible), keeping the current direction. */
    void sortBy(String id, boolean ascending) {
        getColumns().stream().filter(c -> c.getId().equals(id)).findFirst().ifPresent(c -> {
            c.setVisible(true);
            c.setSortType(ascending ? TableColumn.SortType.ASCENDING : TableColumn.SortType.DESCENDING);
            getSortOrder().setAll(List.of(c));
        });
    }

    void ensureVisible(int index) {
        Views.ensureVisible(this, index);
    }

    private void onKeyTyped(KeyEvent e) {
        String ch = e.getCharacter();
        if (ch.isEmpty() || e.isControlDown() || e.isAltDown() || e.isMetaDown() || e.isShortcutDown()) {
            return;
        }
        char c = ch.charAt(0);
        if (Character.isISOControl(c)) {
            return;
        }
        long now = System.currentTimeMillis();
        if (now - lastTyped > 1000) {
            typeAhead.setLength(0);
        }
        lastTyped = now;
        if (typeAhead.isEmpty() && c == ' ') {
            return;
        }
        typeAhead.append(ch);
        String prefix = typeAhead.toString().toLowerCase(Locale.ROOT);
        List<FileEntry> items = getItems();
        for (int i = 0; i < items.size(); i++) {
            if (items.get(i).name().toLowerCase(Locale.ROOT).startsWith(prefix)) {
                getSelectionModel().clearAndSelect(i);
                getFocusModel().focus(i);
                ensureVisible(i);
                break;
            }
        }
        e.consume();
    }

    // ---- column persistence ----

    private void restoreColumns(Session s) {
        String order = s.get("columns.order", null);
        if (order != null) {
            Map<String, TableColumn<FileEntry, ?>> byId = new HashMap<>();
            getColumns().forEach(c -> byId.put(c.getId(), c));
            List<TableColumn<FileEntry, ?>> ordered = new ArrayList<>();
            for (String id : order.split(",")) {
                TableColumn<FileEntry, ?> c = byId.remove(id);
                if (c != null) {
                    ordered.add(c);
                }
            }
            getColumns().stream().filter(byId::containsValue).forEach(ordered::add);
            getColumns().setAll(ordered);
        }
        for (TableColumn<FileEntry, ?> c : getColumns()) {
            String id = c.getId();
            c.setPrefWidth(s.getDouble("columns." + id + ".width", c.getPrefWidth()));
            c.setVisible(s.getBool("columns." + id + ".visible", DEFAULT_VISIBLE.contains(id)));
            c.widthProperty().addListener((o, a, w) -> s.put("columns." + id + ".width", w.intValue()));
            c.visibleProperty().addListener((o, a, v) -> s.put("columns." + id + ".visible", v));
            c.sortTypeProperty().addListener((o, a, t) -> saveSort(s));
        }
        String sortId = s.get("sort.column", "name");
        getColumns().stream().filter(c -> c.getId().equals(sortId)).findFirst().ifPresent(c -> {
            c.setSortType(s.getBool("sort.ascending", true) ? TableColumn.SortType.ASCENDING : TableColumn.SortType.DESCENDING);
            getSortOrder().setAll(List.of(c));
        });
        getColumns().addListener((ListChangeListener<TableColumn<FileEntry, ?>>) ch ->
                s.put("columns.order", String.join(",", getColumns().stream().map(TableColumn::getId).toList())));
        getSortOrder().addListener((ListChangeListener<TableColumn<FileEntry, ?>>) ch -> saveSort(s));
    }

    private void saveSort(Session s) {
        if (getSortOrder().isEmpty()) {
            s.put("sort.column", "");
            return;
        }
        TableColumn<FileEntry, ?> c = getSortOrder().get(0);
        s.put("sort.column", c.getId());
        s.put("sort.ascending", c.getSortType() == TableColumn.SortType.ASCENDING);
    }

    // ---- cells ----

    private static final class NameCell extends TableCell<FileEntry, FileEntry> {
        private final FontIcon icon = Icons.icon("mdi2f-file-outline");

        NameCell() {
            getStyleClass().add("name-cell");
        }

        @Override
        protected void updateItem(FileEntry e, boolean empty) {
            super.updateItem(e, empty);
            if (empty || e == null) {
                setText(null);
                setGraphic(null);
            } else {
                Icons.apply(icon, e);
                setText(e.name());
                setGraphic(icon);
            }
        }
    }

    private final class FileRow extends TableRow<FileEntry> {
        FileRow() {
            setOnMousePressed(e -> {
                if (isEmpty()) {
                    getSelectionModel().clearSelection();
                }
            });
            setOnMouseClicked(e -> {
                if (e.getButton() == MouseButton.PRIMARY && e.getClickCount() == 2 && !isEmpty()) {
                    main.openSelection();
                }
            });
            setOnDragDetected(e -> {
                if (isEmpty()) {
                    return;
                }
                List<Path> paths = tab.selectedPaths();
                if (paths.isEmpty()) {
                    return;
                }
                Dragboard db = startDragAndDrop(TransferMode.COPY_OR_MOVE);
                ClipboardContent content = new ClipboardContent();
                content.putFiles(paths.stream().map(Path::toFile).toList());
                db.setContent(content);
                db.setDragView(snapshot(null, null));
                e.consume();
            });
            setOnDragOver(e -> {
                FileEntry it = getItem();
                if (it != null && it.isDirectory() && Dnd.accept(e, it.path())) {
                    e.consume();
                }
            });
            setOnDragEntered(e -> {
                FileEntry it = getItem();
                if (it != null && it.isDirectory() && Dnd.canDrop(e, it.path())) {
                    getStyleClass().add("drop-target");
                }
            });
            setOnDragExited(e -> getStyleClass().remove("drop-target"));
            setOnDragDropped(e -> {
                FileEntry it = getItem();
                if (it != null && it.isDirectory() && Dnd.canDrop(e, it.path())) {
                    main.drop(e, it.path());
                }
            });
        }

        @Override
        protected void updateItem(FileEntry e, boolean empty) {
            super.updateItem(e, empty);
            pseudoClassStateChanged(HIDDEN, !empty && e != null && e.isHidden());
        }
    }
}
