package com.fileem.ui;

import com.fileem.fs.FileEntry;
import javafx.scene.SnapshotParameters;
import javafx.scene.image.Image;
import javafx.scene.paint.Color;
import org.kordamp.ikonli.javafx.FontIcon;

import java.util.Optional;

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

    public static Optional<Image> appIcon() {
        try {
            FontIcon i = new FontIcon("mdi2f-folder-multiple");
            i.setIconSize(64);
            i.setIconColor(Color.web("#e0a52e"));
            SnapshotParameters params = new SnapshotParameters();
            params.setFill(Color.TRANSPARENT);
            return Optional.of(i.snapshot(params, null));
        } catch (RuntimeException e) {
            return Optional.empty();
        }
    }
}
