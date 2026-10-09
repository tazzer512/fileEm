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

    public static void main(String[] args) {
        launch(args);
    }

    @Override
    public void start(Stage stage) {
        session = Session.load();
        Theme.apply(session.getBool("theme.dark", false));

        window = new MainWindow(stage, session);
        double w = session.getDouble("window.w", 1280);
        double h = session.getDouble("window.h", 780);
        Scene scene = new Scene(window, w, h);
        scene.getStylesheets().add(Theme.APP_CSS);

        stage.setScene(scene);
        stage.setTitle("fileEm");
        stage.setMinWidth(640);
        stage.setMinHeight(400);
        Icons.appIcon().ifPresent(stage.getIcons()::add);

        double x = session.getDouble("window.x", Double.NaN);
        double y = session.getDouble("window.y", Double.NaN);
        if (!Double.isNaN(x) && !Double.isNaN(y)
                && !Screen.getScreensForRectangle(new Rectangle2D(x, y, w, h)).isEmpty()) {
            stage.setX(x);
            stage.setY(y);
        }
        stage.setMaximized(session.getBool("window.maximized", false));
        stage.setOnCloseRequest(e -> saveSession(stage));
        stage.show();
    }

    private void saveSession(Stage stage) {
        session.put("window.maximized", stage.isMaximized());
        if (!stage.isMaximized() && !stage.isIconified()) {
            session.put("window.x", stage.getX());
            session.put("window.y", stage.getY());
            session.put("window.w", stage.getWidth());
            session.put("window.h", stage.getHeight());
        }
        session.put("theme.dark", Theme.isDark());
        window.saveSession();
        session.save();
    }
}
