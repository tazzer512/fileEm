package com.fileem.ui;

import com.fileem.fs.FileEntry;
import javafx.scene.image.Image;
import javafx.stage.Stage;
import org.kordamp.ikonli.javafx.FontIcon;

import java.net.URL;
import java.util.ArrayList;
import java.util.List;

public final class Icons {
    private Icons() {
    }

    public static FontIcon icon(String literal) {
        return icon(literal, 16);
    }

    public static FontIcon icon(String literal, int size) {
        FontIcon i = new FontIcon(literal);
        i.setIconSize(size);
        return i;
    }

    /** Applies the icon literal and color class of {@code entry} to a reusable FontIcon. */
    public static void apply(FontIcon icon, FileEntry entry) {
        String literal = entry.isDirectory() && entry.isSymlink() ? "mdi2f-folder-move" : entry.category().icon;
        apply(icon, literal, entry.category().styleClass);
    }

    public static void apply(FontIcon icon, String literal, String styleClass) {
        if (!literal.equals(icon.getIconLiteral())) {
            icon.setIconLiteral(literal);
        }
        if (icon.getStyleClass().size() != 2 || !icon.getStyleClass().get(1).equals(styleClass)) {
            icon.getStyleClass().setAll("ikonli-font-icon", styleClass);
        }
    }

    private static final int[] APP_ICON_SIZES = {16, 24, 32, 48, 64, 128, 256, 512};
    private static List<Image> appIcons;

    /**
     * The app icon in every size (from {@code /com/fileem/icons}, drawn by {@code tools/make-icon.py}),
     * so the OS can pick the best one for title bars, task bars and switchers.
     */
    public static List<Image> appIcons() {
        if (appIcons == null) {
            List<Image> images = new ArrayList<>();
            for (int size : APP_ICON_SIZES) {
                URL url = Icons.class.getResource("/com/fileem/icons/fileem-" + size + ".png");
                if (url != null) {
                    images.add(new Image(url.toExternalForm()));
                }
            }
            appIcons = List.copyOf(images);
        }
        return appIcons;
    }

    /** Gives a window the app icon. */
    public static void applyAppIcon(Stage stage) {
        stage.getIcons().setAll(appIcons());
    }
}
