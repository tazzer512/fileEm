package com.fileem;

import com.fileem.ui.Icons;
import com.fileem.ui.MainWindow;
import com.fileem.ui.Theme;
import javafx.application.Application;
import javafx.geometry.Rectangle2D;
import javafx.scene.Scene;
import javafx.stage.Screen;
import javafx.stage.Stage;

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
        stage.setOnCloseRequest(e -> autosave.saveNow());
        stage.show();
        autosave.start();
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
