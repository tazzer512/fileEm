package com.fileem.fs;

import java.util.HashMap;
import java.util.Locale;
import java.util.Map;

/** Maps file extensions to a category (icon + description). */
public final class FileTypes {
    public enum Category {
        FOLDER("mdi2f-folder", "icon-folder", "Folder"),
        FILE("mdi2f-file-outline", "icon-file", "File"),
        IMAGE("mdi2f-file-image", "icon-image", "Image"),
        AUDIO("mdi2f-file-music", "icon-audio", "Audio"),
        VIDEO("mdi2f-file-video", "icon-video", "Video"),
        ARCHIVE("mdi2z-zip-box", "icon-archive", "Archive"),
        PDF("mdi2f-file-pdf-box", "icon-pdf", "Document"),
        WORD("mdi2f-file-word", "icon-word", "Document"),
        SPREADSHEET("mdi2f-file-excel", "icon-excel", "Spreadsheet"),
        PRESENTATION("mdi2f-file-powerpoint", "icon-ppt", "Presentation"),
        CODE("mdi2f-file-code", "icon-code", "Source File"),
        TEXT("mdi2f-file-document", "icon-text", "Text Document"),
        EXECUTABLE("mdi2a-application", "icon-exe", "Application"),
        LINK("mdi2f-file-link", "icon-file", "Shortcut");

        public final String icon;
        public final String styleClass;
        public final String label;

        Category(String icon, String styleClass, String label) {
            this.icon = icon;
            this.styleClass = styleClass;
            this.label = label;
        }
    }

    private static final Map<String, Category> BY_EXT = new HashMap<>();

    static {
        reg(Category.IMAGE, "jpg jpeg png gif bmp webp svg tif tiff ico heic heif raw cr2 cr3 nef nrw arw srf sr2 dng pef orf rw2 raf srw erf kdc dcr 3fr mef mos iiq rwl psd xcf avif jp2 j2k j2c jpc jpf jpx");
        reg(Category.AUDIO, "mp3 wav flac ogg oga m4a aac wma opus mid midi aiff");
        reg(Category.VIDEO, "mp4 mkv avi mov wmv webm m4v mpg mpeg flv 3gp ts");
        reg(Category.ARCHIVE, "zip rar 7z tar gz tgz bz2 xz zst lz lzma cab iso jar war");
        reg(Category.PDF, "pdf");
        reg(Category.WORD, "doc docx odt rtf pages");
        reg(Category.SPREADSHEET, "xls xlsx ods csv tsv numbers");
        reg(Category.PRESENTATION, "ppt pptx odp key");
        reg(Category.CODE, "java kt kts scala groovy py js mjs ts tsx jsx c h cpp hpp cc cs go rs rb php swift "
                + "html htm css scss less xml json yaml yml toml sh bash zsh fish bat cmd ps1 sql gradle properties "
                + "lua pl r dart vue fxml");
        reg(Category.TEXT, "txt md markdown log ini cfg conf nfo readme");
        reg(Category.EXECUTABLE, "exe msi app dmg deb rpm appimage apk bin run com pkg snap flatpak");
        reg(Category.LINK, "lnk url desktop webloc");
    }

    private FileTypes() {
    }

    private static void reg(Category c, String exts) {
        for (String e : exts.split(" ")) {
            BY_EXT.put(e, c);
        }
    }

    public static String extension(String name, boolean directory) {
        if (directory) {
            return "";
        }
        int dot = name.lastIndexOf('.');
        return dot > 0 && dot < name.length() - 1 ? name.substring(dot + 1).toLowerCase(Locale.ROOT) : "";
    }

    public static Category category(String extension, boolean directory) {
        if (directory) {
            return Category.FOLDER;
        }
        return BY_EXT.getOrDefault(extension, Category.FILE);
    }

    public static String description(String extension, Category category) {
        if (category == Category.FOLDER) {
            return "Folder";
        }
        if (extension.isEmpty()) {
            return "File";
        }
        String ext = extension.toUpperCase(Locale.ROOT);
        return category == Category.FILE ? ext + " File" : ext + " " + category.label;
    }
}
