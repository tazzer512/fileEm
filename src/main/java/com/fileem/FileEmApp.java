package com.fileem;

import com.fileem.ui.Dialogs;
import com.fileem.ui.Icons;
import com.fileem.ui.OperationUi;
import com.fileem.util.Bg;
import com.fileem.ui.MainWindow;
import com.fileem.ui.Theme;
import javafx.application.Application;
import javafx.application.Platform;
import javafx.geometry.Rectangle2D;
import javafx.scene.Scene;
import javafx.stage.Screen;
import javafx.stage.Stage;

import java.time.Duration;

public class FileEmApp extends Application {
    private Session session;
    private MainWindow window;
    private Autosave autosave;

    public static void main(String[] args) {
        launch(args);
    }

    @Override
    public void start(Stage stage) {
        session = Session.load();
        Theme.apply(Theme.Family.byId(session.get("theme.name", "classic")), session.getBool("theme.dark", false));

        window = new MainWindow(stage, session);
        double w = session.getDouble("window.w", 1280);
        double h = session.getDouble("window.h", 780);
        Scene scene = new Scene(window, w, h);
        Theme.style(scene.getStylesheets());

        stage.setScene(scene);
        stage.setTitle("fileEm");
        stage.setMinWidth(640);
        stage.setMinHeight(400);
        Icons.applyAppIcon(stage);

        double x = session.getDouble("window.x", Double.NaN);
        double y = session.getDouble("window.y", Double.NaN);
        if (!Double.isNaN(x) && !Double.isNaN(y)
                && !Screen.getScreensForRectangle(new Rectangle2D(x, y, w, h)).isEmpty()) {
            stage.setX(x);
            stage.setY(y);
        }
        stage.setMaximized(session.getBool("window.maximized", false));
        autosave = new Autosave(session, () -> collectSession(stage));
        stage.setOnCloseRequest(e -> {
            autosave.saveNow();
            if (!OperationUi.running().isEmpty()) {
                e.consume();
                quitAfterOperations(stage);
            }
        });
        stage.show();
        autosave.start();
        // On logout or kill: stop running operations so they clean up their temp files first.
        Runtime.getRuntime().addShutdownHook(new Thread(
                () -> OperationUi.cancelAllAndWait(Duration.ofSeconds(3)), "fileem-ops-shutdown"));
    }

    /**
     * Closing while files are being copied, moved or deleted: ask, then cancel the operations and wait
     * for their cleanup (no half-written files left behind) before quitting.
     */
    private void quitAfterOperations(Stage stage) {
        int n = OperationUi.running().size();
        String what = n == 1 ? "An operation is" : n + " operations are";
        if (!Dialogs.confirm(stage, "Operations running",
                what + " still running.\n\nStop " + (n == 1 ? "it" : "them") + " and quit? Files already "
                        + "copied or moved stay where they are; nothing is left half-written.")) {
            return;
        }
        stage.getScene().getRoot().setDisable(true);
        stage.setTitle("Stopping operations… — fileEm");
        Bg.IO.execute(() -> {
            OperationUi.cancelAllAndWait(Duration.ofSeconds(15));
            Platform.runLater(Platform::exit);
        });
    }

    @Override
    public void stop() {
        if (autosave != null) {
            autosave.stop();
            autosave.saveNow();
        }
    }

    /** Copies the window geometry and the main window's state into the session. */
    private void collectSession(Stage stage) {
        session.put("window.maximized", stage.isMaximized());
        if (!stage.isMaximized() && !stage.isIconified()) {
            session.put("window.x", stage.getX());
            session.put("window.y", stage.getY());
            session.put("window.w", stage.getWidth());
            session.put("window.h", stage.getHeight());
        }
        window.saveSession();
    }
}
