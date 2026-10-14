package com.fileem.ui;

import atlantafx.base.theme.Styles;
import atlantafx.base.theme.Tweaks;
import com.fileem.Session;
import com.fileem.fs.FileEntry;
import com.fileem.fs.Preview;
import com.fileem.fs.Jpeg2000Reader;
import com.fileem.fs.PsdReader;
import com.fileem.fs.RawReader;
import com.fileem.util.Bg;
import com.fileem.util.Formats;
import javafx.animation.PauseTransition;
import javafx.beans.property.BooleanProperty;
import javafx.beans.property.SimpleBooleanProperty;
import javafx.beans.value.ChangeListener;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.Group;
import javafx.scene.Node;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.Label;
import javafx.scene.control.ScrollPane;
import javafx.scene.control.Tooltip;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;
import javafx.scene.image.PixelFormat;
import javafx.scene.image.WritableImage;
import javafx.scene.input.KeyCode;
import javafx.scene.input.ScrollEvent;
import javafx.scene.layout.BorderPane;
import javafx.scene.layout.FlowPane;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.scene.layout.Region;
import javafx.scene.layout.StackPane;
import javafx.scene.layout.VBox;
import javafx.scene.text.Font;
import javafx.scene.text.Text;
import javafx.stage.Stage;
import javafx.stage.Window;
import javafx.util.Duration;

import java.io.ByteArrayInputStream;
import java.util.Objects;
import java.util.concurrent.Future;

/**
 * A separate, resizable window that previews the selected file of the active tab: images (also
 * Photoshop files and camera raw previews) and text documents, with zoom and pan buttons; folders and other files show their
 * details. Everything is loaded in the background and a newer selection cancels an older load, so
 * previewing never blocks the app. It can be switched off (toolbar, F3, or closing the window), and
 * docked into the main window (a pane on its right, through {@link DockHost}) or float on its own.
 */
final class PreviewWindow {
    /** Where the docked preview goes: the main window shows or removes the pane. */
    interface DockHost {
        void showDocked(Node pane);

        void hideDocked(Node pane);
    }

    /** Images are decoded at most this big (enough to zoom in, bounded in memory). */
    static final int MAX_IMAGE_SIDE = 4096;
    static final double MIN_ZOOM = 0.05;
    static final double MAX_ZOOM = 32;
    static final double ZOOM_STEP = 1.25;
    /** Selection changes within this time load only the last one (arrow keys through a folder). */
    static final Duration SETTLE = Duration.millis(120);
    private static final double TEXT_SIZE = 13;

    private final Stage stage = new Stage();
    private final BooleanProperty enabled = new SimpleBooleanProperty(this, "enabled");
    private final BooleanProperty docked = new SimpleBooleanProperty(this, "docked");
    private final DockHost host;
    private final Window owner;
    private final BorderPane root;
    /** Stands in for the content in the window's scene while it is docked (a node has one parent). */
    private final Region placeholder = new Region();
    private final Label title = new Label();
    private final Label message = new Label();
    private final ScrollPane scroll = new ScrollPane();
    private final ImageView imageView = new ImageView();
    private final Text text = new Text();
    private final Button zoomIn = tool("mdi2m-magnify-plus-outline", "Zoom in", "preview-zoom-in");
    private final Button zoomOut = tool("mdi2m-magnify-minus-outline", "Zoom out", "preview-zoom-out");
    private final Button fit = tool("mdi2f-fit-to-page-outline", "Fit to window", "preview-fit");
    private final Button actual = tool("mdi2n-numeric-1-box-outline", "Actual size (100%)", "preview-actual");
    private final Button panLeft = tool("mdi2a-arrow-left", "Pan left", "preview-pan-left");
    private final Button panRight = tool("mdi2a-arrow-right", "Pan right", "preview-pan-right");
    private final Button panUp = tool("mdi2a-arrow-up", "Pan up", "preview-pan-up");
    private final Button panDown = tool("mdi2a-arrow-down", "Pan down", "preview-pan-down");
    private final Button dock = tool("mdi2d-dock-right", "Dock in the main window", "preview-dock");
    private final Button close = tool("mdi2c-close", "Close the preview (F3)", "preview-close");
    private final PauseTransition settle = new PauseTransition(SETTLE);

    private FileEntry wanted;
    private FileEntry shown;
    private Preview.Kind kind;
    private double zoom = 1;
    private boolean fitting = true;
    private int generation;
    private Future<?> load;
    private Image loadingImage;
    private ChangeListener<Number> loadingListener;
    private boolean placed;

    PreviewWindow(Window owner, Session session, DockHost host) {
        this.owner = owner;
        this.host = host;
        stage.initOwner(owner);
        Icons.applyAppIcon(stage);
        stage.setTitle("Preview");

        title.getStyleClass().add(Styles.TEXT_BOLD);
        title.setMaxWidth(Double.MAX_VALUE);
        HBox.setHgrow(title, Priority.ALWAYS);
        close.managedProperty().bind(close.visibleProperty());
        HBox header = new HBox(2, title, dock, close);
        header.setAlignment(Pos.CENTER_LEFT);
        header.setPadding(new Insets(4, 6, 4, 8));
        header.getStyleClass().add("preview-header");
        // wraps onto a second line in a narrow (docked) pane instead of pushing buttons out of sight
        FlowPane tools = new FlowPane(2, 2, zoomOut, zoomIn, fit, actual, gap(), panLeft, panRight, panUp, panDown);
        tools.setAlignment(Pos.CENTER);
        tools.setPadding(new Insets(4, 6, 4, 6));
        tools.getStyleClass().add("preview-tools");

        imageView.setPreserveRatio(true);
        imageView.setSmooth(true);
        text.setFont(Font.font("monospace", TEXT_SIZE));
        text.getStyleClass().add("preview-text");
        scroll.setPannable(true); // drag to pan
        scroll.getStyleClass().addAll("preview-scroll", Tweaks.EDGE_TO_EDGE);
        scroll.viewportBoundsProperty().addListener(o -> {
            if (fitting) {
                applyZoom();
            }
        });
        // Ctrl + wheel zooms, like the file views
        scroll.addEventFilter(ScrollEvent.SCROLL, e -> {
            if (e.isShortcutDown() && e.getDeltaY() != 0) {
                zoomBy(e.getDeltaY() > 0 ? ZOOM_STEP : 1 / ZOOM_STEP);
                e.consume();
            }
        });
        message.getStyleClass().addAll(Styles.TEXT_MUTED, "preview-message");
        message.setWrapText(true);
        message.setMaxWidth(360);
        StackPane center = new StackPane(scroll, message);
        center.getStyleClass().add("preview-center");

        root = new BorderPane(center);
        root.setTop(header);
        root.setBottom(tools);
        root.getStyleClass().add("preview-window");
        root.setMinWidth(200);
        Scene scene = new Scene(root, session.getDouble("preview.width", 560), session.getDouble("preview.height", 480));
        Theme.style(scene.getStylesheets());
        scene.setOnKeyPressed(e -> {
            if (e.getCode() == KeyCode.ESCAPE || e.getCode() == KeyCode.F3) {
                enabled.set(false);
            }
        });
        stage.setScene(scene);
        stage.setMinWidth(260);
        stage.setMinHeight(200);
        double x = session.getDouble("preview.x", Double.NaN);
        double y = session.getDouble("preview.y", Double.NaN);
        if (!Double.isNaN(x) && !Double.isNaN(y)) {
            stage.setX(x);
            stage.setY(y);
            placed = true;
        }
        stage.setOnCloseRequest(e -> enabled.set(false)); // closing switches the preview off

        zoomIn.setOnAction(e -> zoomBy(ZOOM_STEP));
        zoomOut.setOnAction(e -> zoomBy(1 / ZOOM_STEP));
        fit.setOnAction(e -> fitToWindow());
        actual.setOnAction(e -> setZoom(1));
        panLeft.setOnAction(e -> pan(-1, 0));
        panRight.setOnAction(e -> pan(1, 0));
        panUp.setOnAction(e -> pan(0, -1));
        panDown.setOnAction(e -> pan(0, 1));
        dock.setOnAction(e -> docked.set(!docked.get()));
        close.setOnAction(e -> enabled.set(false));
        settle.setOnFinished(e -> load(wanted));

        docked.set(session.getBool("preview.docked", false));
        enabled.addListener((o, was, on) -> {
            place();
            if (on) {
                load(wanted);
            } else {
                stopLoading();
            }
        });
        docked.addListener((o, was, now) -> place());
        place();
        showMessage("Select a file to preview it.");
    }

    BooleanProperty enabledProperty() {
        return enabled;
    }

    /** Docked: a pane in the main window instead of a window of its own. */
    BooleanProperty dockedProperty() {
        return docked;
    }

    /** The preview's content: in its own window, in the main window when docked, or nowhere when off. */
    Node pane() {
        return root;
    }

    /** Puts the content where enabled and docked say it belongs. */
    private void place() {
        dock.setGraphic(Icons.icon(docked.get() ? "mdi2d-dock-window" : "mdi2d-dock-right", 16));
        dock.getTooltip().setText(docked.get() ? "Float in its own window" : "Dock in the main window");
        close.setVisible(docked.get());
        if (!enabled.get()) {
            stage.hide();
            host.hideDocked(root);
            return;
        }
        if (docked.get()) {
            stage.hide();
            stage.getScene().setRoot(placeholder);
            host.showDocked(root);
            return;
        }
        host.hideDocked(root);
        stage.getScene().setRoot(root);
        if (!placed) {
            placeNextTo(owner);
        }
        stage.show();
        // keep typing in the file list: arrow keys through files update the preview
        if (owner instanceof Stage main) {
            javafx.application.Platform.runLater(main::requestFocus);
        }
    }

    Stage stage() {
        return stage;
    }

    /** Session keys preview.*: on/off, size and position. */
    void save(Session session) {
        session.put("preview.enabled", enabled.get());
        session.put("preview.docked", docked.get());
        if (stage.getWidth() > 0 && stage.getScene().getRoot() == root) {
            session.put("preview.width", Math.round(stage.getScene().getWidth()));
            session.put("preview.height", Math.round(stage.getScene().getHeight()));
        }
        if (placed || stage.isShowing()) {
            session.put("preview.x", Math.round(stage.getX()));
            session.put("preview.y", Math.round(stage.getY()));
        }
    }

    /** The selection changed: preview {@code e} (null: nothing selected) once the selection settles. */
    void select(FileEntry e) {
        if (same(e, wanted)) {
            return;
        }
        wanted = e;
        if (enabled.get()) {
            settle.playFromStart();
        }
    }

    private static boolean same(FileEntry a, FileEntry b) {
        if (a == null || b == null) {
            return a == b;
        }
        return a.path().equals(b.path()) && a.size() == b.size() && Objects.equals(a.modified(), b.modified());
    }

    // ---- loading ----

    private void load(FileEntry e) {
        settle.stop();
        if (same(e, shown) && kind != null) {
            return;
        }
        stopLoading();
        int gen = ++generation;
        shown = e;
        if (e == null) {
            kind = null;
            title.setText("");
            stage.setTitle("Preview");
            showMessage("Select a file to preview it.");
            return;
        }
        title.setText(e.name());
        stage.setTitle(e.name() + " — Preview");
        kind = Preview.kindOf(e.extension(), e.isDirectory());
        switch (kind) {
            case FOLDER -> showMessage(e.name() + "\nFolder");
            case IMAGE -> {
                showMessage("Loading…");
                // JavaFX enlarges an image to a requested size, so only bound images that are bigger:
                // read the dimensions from the header first (cheap), then decode in the background
                load = Bg.run(() -> imageSize(e.path()), size -> {
                    if (gen == generation) {
                        decodeImage(gen, e, size);
                    }
                }, err -> {
                    if (gen == generation) {
                        decodeImage(gen, e, null);
                    }
                });
            }
            case PSD -> {
                showMessage("Loading…");
                load = Bg.run(() -> PsdReader.read(e.path(), 2048), p -> {
                    if (gen != generation) {
                        return;
                    }
                    Image img = p == null ? null : p.argb() != null ? pixels(p)
                            : new Image(new ByteArrayInputStream(p.jpeg()));
                    if (img == null || img.isError()) {
                        showInfo(e, "No preview available for this Photoshop file.");
                    } else {
                        showImage(img);
                    }
                }, err -> {
                    if (gen == generation) {
                        showInfo(e, "Cannot read this file: " + err.getMessage());
                    }
                });
            }
            case RAW -> {
                showMessage("Loading…");
                // the camera's embedded JPEG preview, decoded and turned upright off the FX thread
                load = Bg.run(() -> RawImages.decode(RawReader.read(e.path(), Integer.MAX_VALUE), MAX_IMAGE_SIDE), img -> {
                    if (gen != generation) {
                        return;
                    }
                    if (img == null) {
                        showInfo(e, "This raw file has no embedded preview to show.");
                    } else {
                        showImage(img);
                    }
                }, err -> {
                    if (gen == generation) {
                        showInfo(e, "Cannot read this file: " + err.getMessage());
                    }
                });
            }
            case JPEG2000 -> {
                showMessage("Loading…");
                // like PSDs at 2048 px: a 6000 px image then decodes its 3000 px level (~1 s), not all (~5 s)
                load = Bg.run(() -> RawImages.of(Jpeg2000Reader.read(e.path(), 2048)), img -> {
                    if (gen != generation) {
                        return;
                    }
                    if (img == null) {
                        showInfo(e, "Cannot show this JPEG 2000 image.");
                    } else {
                        showImage(img);
                    }
                }, err -> {
                    if (gen == generation) {
                        showInfo(e, "Cannot read this file: " + err.getMessage());
                    }
                });
            }
            default -> { // TEXT, and unknown files that may be text after all
                showMessage("Loading…");
                load = Bg.run(() -> Preview.readText(e.path(), Preview.MAX_TEXT_BYTES), t -> {
                    if (gen != generation) {
                        return;
                    }
                    if (t == null) {
                        showInfo(e, "No preview available.");
                    } else {
                        kind = Preview.Kind.TEXT;
                        showText(t);
                    }
                }, err -> {
                    if (gen == generation) {
                        showInfo(e, "Cannot read this file: " + err.getMessage());
                    }
                });
            }
        }
    }

    /** Width and height from the image header (no decoding), or null when unknown. */
    static int[] imageSize(java.nio.file.Path file) throws java.io.IOException {
        try (var in = javax.imageio.ImageIO.createImageInputStream(file.toFile())) {
            if (in == null) {
                return null;
            }
            var readers = javax.imageio.ImageIO.getImageReaders(in);
            if (!readers.hasNext()) {
                return null;
            }
            var reader = readers.next();
            try {
                reader.setInput(in, true, true);
                return new int[]{reader.getWidth(0), reader.getHeight(0)};
            } finally {
                reader.dispose();
            }
        }
    }

    /** Decodes with JavaFX's background loading, at natural size or bounded to {@link #MAX_IMAGE_SIDE}. */
    private void decodeImage(int gen, FileEntry e, int[] size) {
        boolean big = size == null || size[0] > MAX_IMAGE_SIDE || size[1] > MAX_IMAGE_SIDE;
        String url = e.path().toUri().toString();
        Image img = big && size != null ? new Image(url, MAX_IMAGE_SIDE, MAX_IMAGE_SIDE, true, true, true)
                : new Image(url, true);
        loadingImage = img;
        loadingListener = (o, was, p) -> {
            if (p.doubleValue() >= 1) {
                imageLoaded(gen, img, e);
            }
        };
        img.progressProperty().addListener(loadingListener);
        if (img.getProgress() >= 1) {
            imageLoaded(gen, img, e);
        }
    }

    private void imageLoaded(int gen, Image img, FileEntry e) {
        img.progressProperty().removeListener(loadingListener);
        loadingImage = null;
        if (gen != generation) {
            return;
        }
        if (img.isError()) {
            showInfo(e, "Cannot show this image.");
        } else {
            showImage(img);
        }
    }

    private static Image pixels(PsdReader.Preview p) {
        WritableImage img = new WritableImage(p.width(), p.height());
        img.getPixelWriter().setPixels(0, 0, p.width(), p.height(), PixelFormat.getIntArgbInstance(), p.argb(), 0, p.width());
        return img;
    }

    private void stopLoading() {
        if (load != null) {
            load.cancel(true);
            load = null;
        }
        if (loadingImage != null) {
            loadingImage.progressProperty().removeListener(loadingListener);
            loadingImage.cancel();
            loadingImage = null;
        }
    }

    // ---- showing ----

    private void showMessage(String s) {
        message.setText(s);
        message.setVisible(true);
        scroll.setContent(null);
        imageView.setImage(null);
        text.setText("");
        setToolsEnabled(false);
    }

    private void showInfo(FileEntry e, String reason) {
        kind = Preview.Kind.OTHER;
        String size = e.isDirectory() ? "" : "\n" + Formats.size(e.size());
        showMessage(e.name() + "\n" + e.type() + size + "\n\n" + reason);
    }

    private void showImage(Image img) {
        message.setVisible(false);
        text.setText("");
        imageView.setImage(img);
        // at least as big as the window, so a smaller image sits in the middle
        StackPane centered = new StackPane(new Group(imageView));
        centered.minWidthProperty().bind(javafx.beans.binding.Bindings.createDoubleBinding(
                () -> scroll.getViewportBounds().getWidth(), scroll.viewportBoundsProperty()));
        centered.minHeightProperty().bind(javafx.beans.binding.Bindings.createDoubleBinding(
                () -> scroll.getViewportBounds().getHeight(), scroll.viewportBoundsProperty()));
        scroll.setContent(centered);
        setToolsEnabled(true);
        fitToWindow();
    }

    private void showText(Preview.Text t) {
        message.setVisible(false);
        imageView.setImage(null);
        text.setText(t.truncated() ? t.content() + "\n\n… (only the first " + Formats.size(Preview.MAX_TEXT_BYTES) + " is shown)"
                : t.content());
        VBox box = new VBox(text);
        box.setPadding(new Insets(8, 10, 8, 10));
        scroll.setContent(box);
        setToolsEnabled(true);
        fitting = false;
        setZoom(1);
        scroll.setHvalue(0);
        scroll.setVvalue(0);
    }

    private void setToolsEnabled(boolean on) {
        for (Button b : new Button[]{zoomIn, zoomOut, fit, actual, panLeft, panRight, panUp, panDown}) {
            b.setDisable(!on);
        }
    }

    // ---- zoom & pan ----

    /** Image: scale of its pixels (1 = 100%). Text: scale of the font. */
    double zoom() {
        return zoom;
    }

    boolean isFitting() {
        return fitting;
    }

    void zoomBy(double factor) {
        setZoom(zoom * factor);
    }

    void setZoom(double z) {
        fitting = false;
        zoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, z));
        applyZoom();
    }

    /** Images: the whole image in the window, never enlarged. Text: back to the normal size. */
    void fitToWindow() {
        if (kind == Preview.Kind.TEXT) {
            setZoom(1);
            return;
        }
        fitting = true;
        applyZoom();
    }

    private void applyZoom() {
        Image img = imageView.getImage();
        if (img != null) {
            if (fitting) {
                double vw = Math.max(1, scroll.getViewportBounds().getWidth());
                double vh = Math.max(1, scroll.getViewportBounds().getHeight());
                zoom = Math.min(1, Math.min(vw / img.getWidth(), vh / img.getHeight()));
            }
            imageView.setFitWidth(img.getWidth() * zoom);
            imageView.setFitHeight(img.getHeight() * zoom);
        } else if (kind == Preview.Kind.TEXT) {
            text.setFont(Font.font("monospace", TEXT_SIZE * zoom));
        }
    }

    /** Moves the view a quarter of the window in the given direction. */
    void pan(int dx, int dy) {
        Node content = scroll.getContent();
        if (content == null) {
            return;
        }
        double cw = content.getBoundsInLocal().getWidth(), ch = content.getBoundsInLocal().getHeight();
        double vw = scroll.getViewportBounds().getWidth(), vh = scroll.getViewportBounds().getHeight();
        if (dx != 0 && cw > vw) {
            scroll.setHvalue(clamp01(scroll.getHvalue() + dx * 0.25 * vw / (cw - vw)));
        }
        if (dy != 0 && ch > vh) {
            scroll.setVvalue(clamp01(scroll.getVvalue() + dy * 0.25 * vh / (ch - vh)));
        }
    }

    private static double clamp01(double v) {
        return Math.max(0, Math.min(1, v));
    }

    // ---- helpers ----

    /** First opening: to the right of the main window, or inside it near its right edge. */
    private void placeNextTo(Window owner) {
        if (owner == null || owner.getWidth() <= 0) {
            return;
        }
        double w = stage.getScene().getWidth();
        var screens = javafx.stage.Screen.getScreensForRectangle(owner.getX(), owner.getY(), owner.getWidth(), owner.getHeight());
        var bounds = (screens.isEmpty() ? javafx.stage.Screen.getPrimary() : screens.get(0)).getVisualBounds();
        double x = owner.getX() + owner.getWidth() + 8;
        if (x + w > bounds.getMaxX()) {
            x = Math.max(bounds.getMinX(), owner.getX() + owner.getWidth() - w - 24);
        }
        stage.setX(x);
        stage.setY(owner.getY() + 60);
        placed = true;
    }

    private static Region gap() {
        Region r = new Region();
        r.setPrefWidth(12);
        return r;
    }

    private static Button tool(String icon, String tooltip, String id) {
        Button b = new Button(null, Icons.icon(icon, 16));
        b.getStyleClass().addAll(Styles.BUTTON_ICON, Styles.FLAT);
        b.setTooltip(new Tooltip(tooltip));
        b.setFocusTraversable(false);
        b.setId(id);
        return b;
    }

    // ---- test hooks ----

    Preview.Kind kind() {
        return kind;
    }

    FileEntry shown() {
        return shown;
    }

    Image image() {
        return imageView.getImage();
    }

    String textContent() {
        return text.getText();
    }

    String messageText() {
        return message.isVisible() ? message.getText() : null;
    }

    ScrollPane scrollPane() {
        return scroll;
    }

    Button button(String id) {
        return (Button) root.lookup("#" + id);
    }
}
