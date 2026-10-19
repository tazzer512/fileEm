//! Maps file extensions to a category (icon + description).

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Folder,
    File,
    Image,
    Audio,
    Video,
    Archive,
    Pdf,
    Word,
    Spreadsheet,
    Presentation,
    Code,
    Text,
    Executable,
    Link,
}

impl Category {
    /// The word used in type descriptions ("PNG Image").
    pub fn label(self) -> &'static str {
        match self {
            Category::Folder => "Folder",
            Category::File => "File",
            Category::Image => "Image",
            Category::Audio => "Audio",
            Category::Video => "Video",
            Category::Archive => "Archive",
            Category::Pdf | Category::Word => "Document",
            Category::Spreadsheet => "Spreadsheet",
            Category::Presentation => "Presentation",
            Category::Code => "Source File",
            Category::Text => "Text Document",
            Category::Executable => "Application",
            Category::Link => "Shortcut",
        }
    }
}

fn by_extension() -> &'static HashMap<&'static str, Category> {
    static MAP: OnceLock<HashMap<&'static str, Category>> = OnceLock::new();
    MAP.get_or_init(|| {
        let groups: [(Category, &str); 12] = [
            (Category::Image, "jpg jpeg png gif bmp webp svg tif tiff ico heic heif raw cr2 cr3 nef nrw arw srf sr2 dng pef orf rw2 raf srw erf kdc dcr 3fr mef mos iiq rwl psd xcf avif jp2 j2k j2c jpc jpf jpx ppm pgm pbm pnm"),
            (Category::Audio, "mp3 wav flac ogg oga m4a aac wma opus mid midi aiff"),
            (Category::Video, "mp4 mkv avi mov wmv webm m4v mpg mpeg flv 3gp ts"),
            (Category::Archive, "zip rar 7z tar gz tgz bz2 xz zst lz lzma cab iso jar war"),
            (Category::Pdf, "pdf"),
            (Category::Word, "doc docx odt rtf pages"),
            (Category::Spreadsheet, "xls xlsx ods csv tsv numbers"),
            (Category::Presentation, "ppt pptx odp key"),
            (Category::Code, "java kt kts scala groovy py js mjs ts tsx jsx c h cpp hpp cc cs go rs rb php swift \
                html htm css scss less xml json yaml yml toml sh bash zsh fish bat cmd ps1 sql gradle properties \
                lua pl r dart vue fxml slint"),
            (Category::Text, "txt md markdown log ini cfg conf nfo readme"),
            (Category::Executable, "exe msi app dmg deb rpm appimage apk bin run com pkg snap flatpak"),
            (Category::Link, "lnk url desktop webloc"),
        ];
        let mut map = HashMap::new();
        for (cat, exts) in groups {
            for e in exts.split_whitespace() {
                map.insert(e, cat);
            }
        }
        map
    })
}

/// Lower-case extension without the dot; empty for folders, dot files and trailing dots.
pub fn extension(name: &str, directory: bool) -> String {
    if directory {
        return String::new();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot < name.len() - 1 => name[dot + 1..].to_lowercase(),
        _ => String::new(),
    }
}

pub fn category(extension: &str, directory: bool) -> Category {
    if directory {
        return Category::Folder;
    }
    by_extension().get(extension).copied().unwrap_or(Category::File)
}

/// "Folder", "File", "XYZ File", "PNG Image".
pub fn description(extension: &str, category: Category) -> String {
    if category == Category::Folder {
        return "Folder".into();
    }
    if extension.is_empty() {
        return "File".into();
    }
    let ext = extension.to_uppercase();
    if category == Category::File {
        format!("{ext} File")
    } else {
        format!("{ext} {}", category.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_lowercased_and_ignores_leading_and_trailing_dots() {
        assert_eq!(extension("Photo.JPG", false), "jpg");
        assert_eq!(extension("a.tar.gz", false), "gz");
        assert_eq!(extension(".bashrc", false), "");
        assert_eq!(extension("trailing.", false), "");
        assert_eq!(extension("noext", false), "");
        assert_eq!(extension("dir.with.dots", true), "");
    }

    #[test]
    fn categories_and_descriptions() {
        assert_eq!(category("", true), Category::Folder);
        assert_eq!(category("png", false), Category::Image);
        assert_eq!(category("java", false), Category::Code);
        assert_eq!(category("unknownext", false), Category::File);
        assert_eq!(description("", Category::Folder), "Folder");
        assert_eq!(description("", Category::File), "File");
        assert_eq!(description("xyz", Category::File), "XYZ File");
        assert_eq!(description("png", Category::Image), "PNG Image");
    }
}
