# fileEm

A cross-platform (Linux, macOS, Windows) file explorer in JavaFX: Explorer-style folder tree and
detail view, with an optional second side for Norton Commander / Directory Opus style copy and move.

## Run

Requires JDK 27+ and Maven.

```sh
mvn javafx:run
```

## Install as an app

`mvn -Pdist -DskipTests package` builds a native app with its own trimmed Java runtime and the
fileEm icon (jpackage, for the OS you build on) in `target/dist/app`: `fileEm/bin/fileEm` on Linux,
`fileEm/fileEm.exe` on Windows, `fileEm.app` on macOS. Add `-Djpackage.type=deb` (or `rpm`, `dmg`,
`pkg`, `msi`, `exe`) for an installer; that needs the OS's packaging tools.

On Linux, install it for your user with a launcher entry, so it appears in the app grid / menu and
the dock shows its icon (no root needed):

```sh
mvn -Pdist -DskipTests package && tools/install-linux.sh   # again to update
tools/install-linux.sh --uninstall                          # settings are kept
```

## Test

```sh
mvn test                    # unit + end-to-end UI tests, headless (no display needed)
mvn test -Dheadless=false   # watch the UI tests run on screen
```

## Features

- **Folder tree**: Quick access (Home, Desktop, Documents, … including localized XDG folders on Linux)
  and all volumes/mounts with free space. Removable drives show up automatically (polled every 5 s).
- **Safely remove drives**: USB sticks, SD cards and external disks get an eject button next to
  their name in the tree (also *Eject* in their right-click menu). The whole drive is ejected, with
  all its partitions. fileEm first moves tabs off it, refuses while a copy or move still uses it,
  then unmounts it (writing out everything cached) and powers it off, and tells you when it can be
  unplugged. Linux uses `udisksctl` (or
  `gio`, else `umount`), macOS `diskutil eject`, Windows Explorer's Eject.
- **Bookmarks**: Ctrl+D (or the bookmark button) bookmarks the current folder; bookmarks appear
  in their own tree section on both sides. Right-click a bookmark to rename, reorder or remove it,
  or use *Manage bookmarks* (Ctrl+Shift+B). Bookmarks are saved immediately.
- **File list**: Name, Size, Type, Date modified by default; Date created, Date accessed, Extension and
  Attributes/permissions can be turned on from the `⋮` header menu. Click headers to sort (folders stay
  on top, natural "file2 < file10" order). Column widths, order, visibility and sort are remembered.
- **Search** (*Search in folder…* in the right-click menu of a folder, the tree or empty space in
  the list, or Ctrl+Shift+F): finds files and folders by name in that folder and every folder below
  it. Wildcards: `*` (any text) and `?` (one character), e.g. `*.jpg`, `IMG_????.*`. Separate several
  with spaces (or `;`): `*.jpg *.png` finds either; `!` in front excludes, so `a*.* !*.mp3` finds
  names starting with "a" except mp3s (only `!` terms: everything else). Quotes keep spaces in a
  term (`"my file*"`). Text without wildcards matches anywhere in the name; case is ignored.
  Results stream in while it searches (Stop or Escape halts it). Double-click or Enter opens a
  result (folders open in the active tab), and *Open containing folder* shows it selected in its
  folder. Hidden files are included when the box is ticked (it starts as the list's setting);
  links are listed but not followed, and unreadable folders are skipped and counted.
- **Preview** (F3 or the eye button in the toolbar): shows the selected file in a separate,
  resizable window, or **docked** as a pane on the right of the main window (the dock button in its
  header switches; drag the divider to size it). Images (also Photoshop files and camera raw photos)
  with zoom in / out, fit, 100% and pan buttons (or drag, Ctrl+wheel to zoom), text documents (the
  first 256 KB; zoom changes the font), and details for folders and other files. It loads in the
  background, so browsing stays instant; quick selection changes only load the last one. Closing it
  (or F3 / Escape) switches it off; whether it's on, docked, its size and position are remembered.
- **Camera raw photos** (Canon CR2/CR3, Nikon NEF, Sony ARW, DNG, Fujifilm RAF, Olympus ORF,
  Panasonic RW2, Pentax PEF and more) show the JPEG the camera embedded in the file, turned upright,
  in the preview and as thumbnails. Raws without an embedded preview (e.g. merged HDR DNGs) show their
  details only.
- **View and Sort by buttons** in every tab, between the address bar and the filter box: the view
  button shows the tab's current view and switches it; the sort button shows the direction and
  picks the column (Name, Size, Type, Date modified, Date created, Extension) and the order.
- **View modes** like Windows Explorer, per tab, from the View button in the toolbar or the tab, the
  right-click menu on empty space, Ctrl+Shift+1…8 or Ctrl+mouse wheel: Extra large / Large / Medium /
  Small icons, List, Details, Tiles and Content. The big icon views show thumbnails of images (PNG,
  JPEG, GIF, BMP), Photoshop files (PSD/PSB, read without loading the whole document) and camera raws. Selection, sorting, keyboard navigation, the selection box and drag & drop work in
  every view. New tabs open in the view you're using; each tab's view is remembered.
  In the icon views and Tiles, the **size slider** at the bottom right sizes the items freely (the
  middle is the view's default size, double-click it to go back); the **− / +** buttons beside it
  (or Ctrl+− / Ctrl++) step 25% at a time. Each view keeps its own size per tab.
- **Tabs** on each side, **dual pane** (F9), breadcrumb address bar (click or Ctrl+L to type a path),
  live filter box with `*` / `?` wildcards, type-ahead find, auto-refresh when folder contents change.
- **Operations** (copy, move, delete to Trash, permanent delete, compress, extract) each run on their
  own thread, all at the same time: starting one never stops or slows down the others. They share
  one progress window, stacked, each with its own **Pause/Resume** and **Cancel**. Each row shows the
  percentage, amount done, speed and **time left** (measured over the last 10 seconds, so it follows
  changes in speed; no guess while paused or waiting for an answer). Pausing stops at
  the next file or 16 MB chunk. Name conflicts are asked right in the operation's progress row
  (Replace / Skip / Keep both, optionally for all), so the rest of the app and other operations keep
  working while a question waits. Quitting while operations run asks first, then stops them cleanly (no half-written files).
- **Responsive**: folders are listed on virtual threads, so a slow or hanging network folder never
  blocks the window or other folders, and leaving a big folder before it finished loading stops
  reading it.
- **Safe file operations**: copies are written to a temp file, size-checked, then renamed into
  place, so a failed or cancelled copy never damages an existing file. Moves between drives write
  the copies to disk in batches before deleting the originals (fast even for thousands of small
  files on a USB stick). Links are never followed, special files are
  skipped, and free space is checked up front.
- **Right-click menus** on files, folders, empty space and the tree: open (in a tab, the other
  side, a terminal or the OS file manager), cut/copy/paste (into a folder), copy/move to the other
  side, duplicate, compress to ZIP / extract here, new folder / text file, rename, trash, delete,
  bookmark, sort by, show hidden files, copy path, properties. Quick access folders, bookmarks and
  drives in the tree can't be cut, renamed or deleted from there.
- **Drag & drop** between sides, tabs, the tree and other applications. A drop **moves** the files,
  after asking: *Move* (default), *Copy* to keep the originals, or *Cancel*. Moves only delete an
  original after its copy succeeded, also for files dropped from other applications. Files are
  dragged by their icon or name.
- **Selection box**: drag from empty space, or from a row outside its name, to draw a dotted box that
  selects every row it touches (Shift adds to the selection, Ctrl/Cmd toggles; the list scrolls when
  dragging past its edge).
- **Clipboard** uses the OS clipboard, so files can be copied/pasted to and from Explorer, Finder or
  Nautilus.
- **Properties** dialog with background folder-size calculation.
- **Themes**, each with a light and a dark mode, picked from the palette button in the toolbar (the
  button next to it switches light/dark): **Classic**, **Windows 11** (Fluent: mica, rounded
  corners, accent underlines), **macOS** (Finder: striped rows, solid blue selection, segmented
  tabs), **Adwaita** (GNOME Files) and **Aurora**, fileEm's own northern-lights theme with gradient
  edge lights, pill controls and keycap F-keys. Fonts fall back to the system font when the
  theme's font (Segoe UI Variable, SF Pro, Cantarell, Inter) isn't installed.
- **About** (the *i* button at the right end of the toolbar): the version, build date, and the Java,
  JavaFX and OS it runs on. The version is `app.version` in `pom.xml` (also the packaged app's).
- Session restore (window, tabs and their folders, dual mode, hidden files, columns, theme and
  light/dark mode, bookmarks). Saved automatically while the app runs, so it survives a crash or logout.

## Keyboard

| Key | Action |
| --- | --- |
| Enter / double-click | Open folder or file (default app) |
| Backspace, Alt+Up | Up one folder |
| Alt+Left / Alt+Right | Back / Forward |
| Tab | Switch side (dual pane) |
| F2 | Rename |
| F3 | Preview on / off (window or docked) |
| F5 / F6 | Copy / Move selection to the other side (target is editable; only with the dual pane on) |
| F7, Ctrl+Shift+N | New folder |
| Del, F8 | Move to Trash |
| Shift+Del | Delete permanently |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / Cut / Paste |
| Ctrl+A | Select all |
| Alt+Enter | Properties |
| Ctrl+T / Ctrl+W | New tab / Close tab |
| Ctrl+L | Edit address |
| Ctrl+F | Filter |
| Ctrl+Shift+F | Search this folder and all folders below it |
| Ctrl+H (Cmd+Shift+. on macOS) | Show hidden files |
| Ctrl+R | Refresh |
| Ctrl+D | Bookmark / un-bookmark the current folder |
| Ctrl+Shift+B | Manage bookmarks |
| F9 | Toggle dual pane |
| Ctrl+Shift+1…8, Ctrl+wheel | View: Extra large, Large, Medium, Small icons, List, Details, Tiles, Content |
| Ctrl++ / Ctrl+− | Bigger / smaller items (icon views and Tiles) |
| Arrows, Home/End, PgUp/PgDn | Move in the list or grid (Shift extends, Ctrl moves without selecting, Ctrl+Space toggles) |

On macOS, Ctrl is Cmd; Cmd+Backspace deletes and Cmd+Up/Down go up/open.

## Settings location

- Linux: `$XDG_CONFIG_HOME/fileem/session.properties` (default `~/.config/fileem`)
- macOS: `~/Library/Application Support/fileEm/session.properties`
- Windows: `%APPDATA%\fileEm\session.properties`
