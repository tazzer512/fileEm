# Migration plan: Java/JavaFX → Rust/Slint

The working plan for porting fileEm to Rust, kept up to date as stages land. Read this first
when continuing the migration; `RUST.md` has the build/test/layout reference, `AGENTS.md` the
rules (tests, integrity, conventions) that apply to both versions.

**Goal**: the Rust app replaces the Java app with the same features and behavior, as one native
executable with a much smaller memory footprint (measured: Java ~360 MB, Rust ~105 MB with
OpenGL, ~32 MB with the software renderer).

**Status (2026-10-21)**: stages 0–6 (5a view modes and thumbnails, 5b image readers, 6 the
preview) done on branch `rust`, and the leftovers of stages 1–5 fixed except drag & drop with
other applications (decided: not possible with Slint 1.18). The Java app in `src/` is still the
complete, daily-use version; nothing was removed from it. Stage 7 (search, bookmarks,
drives and eject), stage 8 (themes, About) and stage 9's packaging (Linux install script and
deb, Windows exe, macOS bundle, CI) done; open: the cutover, after daily use.

## How to work a stage

1. Read the Java sources and tests listed for the stage (they are the specification).
2. Port the logic into `fileem-core` (no UI) where possible, with the Java tests ported as Rust
   tests; UI state goes into toolkit-free structs in `crates/fileem/src` (like `tab.rs`,
   `tree.rs`) with unit tests; Slint only lays out and draws.
3. Port the stage's `MainWindowUiTest` cases into `crates/fileem/tests/ui.rs` scenarios
   (find them with `grep -n "void .*()" src/test/java/com/fileem/ui/MainWindowUiTest.java`).
4. Check visually with `FILEEM_SNAPSHOT` (light and dark), measure memory if the stage could
   change it.
5. `cargo test --workspace`, `cargo clippy --workspace --all-targets` (warning free) and
   `mvn test` (Java stays green) before committing.
6. Update this file: tick the stage, add decisions and lessons learned; update `RUST.md` if the
   layout or commands changed, and `README.md` once the Rust app is the shipped one.

## Done

### Stage 0: core and a browsing window (`d825f36`)

- `fileem-core`:
  - `util`: `natural_order`, `formats`, `fs_util` (unique names, name validation, `*`/`?`
    matcher, lexical normalize), `os` (platform, home, config dir).
  - `fs`: `entry` (`FileEntry`, lazy cached owner lookup), `file_types`, `listing` (cancellable
    list, subfolders, folders-first compare), `places` (quick access with localized XDG dirs,
    volumes: `/` + `/media/$USER`, `/run/media/$USER`, `/mnt`, `/Volumes`, Windows drives),
    `trash` (gio + freedesktop fallback on Linux, `trash` crate elsewhere).
  - `ops::file_operation`: copy/move/delete/trash/compress/extract with every integrity rule,
    pause/resume/cancel, `Handle` for progress/state/errors, conflict handler, batched syncfs
    flushes. All 42 `FileOperationTest` cases ported (+ extras), mutation-checked.
- `fileem` (Slint): toolbar (back/forward/up/refresh, address bar, hidden toggle), folder tree
  (sections, lazy loading, reveal + current box + bold path, folds what it opened, scrolls to
  the folder, shows a hidden current folder), splitter, details list (icons, folders-first
  sort by column), status line, keys (Enter, Backspace/Alt+Up, Alt+Left/Right, arrows, PgUp/
  PgDn/Home/End with Shift/Ctrl, Ctrl+Space/A/R/H/L, Del/F8 to Trash, type-ahead).
- Tests: 82 core, 13 tab/tree unit, 1 UI test with 10 scenarios (headless testing backend).

## Next stages

Order chosen so the Rust app becomes usable for daily work as early as possible. Line counts
are the Java sources, as a size hint.

### Stage 1: everyday file work

- [x] **Session + autosave** (done: `fileem-core::session` with a Java-compatible `.properties`
  reader/writer, checked against real `Properties.load`; restore of hidden, sort, tree width,
  window size/maximized and the selected tab's folder with nearest-parent fallback; 2 s
  autosave + save on close. Only keys the Rust app knows are written, so other tabs, columns,
  theme etc. survive for the Java app. The JVM shutdown hook's part came later: SIGTERM,
  SIGHUP and SIGINT save the session, cancel operations and wait up to 3 s for their cleanup,
  then quit (`signals.rs`: blocked in `main` before any thread starts, one thread `sigwait`s; a
  hung UI thread or a second signal still ends the process after 5 s). Unix only; on Windows
  the 2 s autosave covers a logout.) Original notes: (`Session` 153, `Autosave` 74; `SessionTest` 8): same file and keys
  in `OsUtil.configDir()` (`window.*`, `left.*`, `columns.*`, `sort.*`, `showHidden`, …) so
  both versions share settings during the transition; atomic writes, change-only, background
  save every 2 s, save on close. Core: `session.rs`.
- [x] **Directory watcher** (done: `fs::watcher` on the `notify` crate, debounced per folder,
  re-watched after delete + create; the app registers off the UI thread, reports reach it via a
  Slint callback with a token, refreshes are coalesced and keep scroll/focus/selection, a
  vanished folder opens its nearest parent, changed subfolders reload the tree. Our own
  operations still refresh explicitly.) Original notes: `notify` crate; coalesced
  refresh throttled to 2× the last listing time (`FileTab.requestRefresh`); skip while a
  navigation loads; refresh after our own operations.
- [x] **Progress window** (done: `util::eta` with `EtaTest` ported; `progress.rs` rows
  (title, detail line, estimate, question) unit-tested; `ops_ui.rs` runs each operation on its
  own thread, a row after 350 ms in a separate `ProgressWindow` (`ui/progress.slint`),
  Pause/Resume/Cancel per row, Escape with one row, closing cancels all; conflicts inline (the
  worker waits on a channel and checks for cancel); errors listed afterwards; closing the main
  window with operations running asks, cancels and waits for cleanup.) Original notes: a
  second Slint window per main window; one row per running op after 350 ms; Pause/Resume and
  Cancel per row; speed + time left from `Eta`; conflicts asked inline in the row (operation
  thread waits on a channel; never a modal dialog); closing cancels all; app close with running
  ops asks, then cancels and waits (`Handle::await_stopped`).
- [x] **Clipboard** (done: `clipboard.rs` with our own encodings (`x-special/gnome-copied-files`
  with copy/cut, `text/uri-list`, text; Windows CF_HDROP + Preferred DropEffect; macOS file
  URLs) over `clipboard-rs` (X11, Windows, macOS; on Wayland through XWayland, as GNOME 46 has
  no data-control protocol); an in-app fallback without a system clipboard; Ctrl+C/X/V, a
  pasted cut moves once and clears the clipboard; tests use `clipboard::Memory`. Checked by
  hand on GNOME (Wayland, via XWayland): copy and paste work both ways with Files.) Original notes: OS clipboard with file lists (Linux:
  `x-special/gnome-copied-files` + `text/uri-list`; Windows: `CF_HDROP` + Preferred DropEffect;
  macOS: file URLs). Check `arboard` for custom formats first; otherwise per-platform code.
  Ctrl+C/X/V.
- [x] **Dialogs** (done: an overlay dialog in the main window (`dialog-*` properties,
  `app.rs` `Dialog`/`DialogSpec`) for confirm, name entry with validation and error lists;
  `ops::names` (rename never replaces, case-only renames, create folder/file) with tests;
  Del/F8 Trash and Shift+Del delete after confirming, F2, F7/Ctrl+Shift+N. New text file came
  with stage 2's menus, tabs following a renamed folder with stage 3, bookmarks following it
  with stage 7.)
  Original notes: F2 rename (validate with `fs_util::validate_name`), F7/Ctrl+Shift+N new folder,
  Shift+Del with confirmation, error dialog for operation errors.
- UI tests from `MainWindowUiTest`: clipboard, rename, new folder, delete, progress window
  (pause/cancel, stacked ops, conflict buttons), autosave/session.

### Stage 2: menus and properties (done)

- [x] **Right-click menus** (`menu.rs`, toolkit-free and unit-tested; one `ContextMenuArea` in
  app.slint with six sections plus Sort by / New submenus, shown from code only). File menu:
  Open, Open in terminal, Show in file manager, Cut/Copy/Paste (Paste into folder on a folder),
  Duplicate, Compress to ZIP, Extract here (zips), Rename, Move to Trash, Delete permanently,
  Copy path, Properties. Empty space: Paste, Show hidden files, Refresh, Sort by, New (Folder,
  Text file), Open in terminal, Open in file manager, Copy path, Properties. Tree: the same per
  folder; quick access and drives are shortcuts (no cut, rename, delete, duplicate; drives no
  copy or zip either); a right-click never navigates. Right-click on an unselected row selects
  it alone, on a selected one keeps the selection, on empty space clears it. Menu key and
  Shift+F10 open the menu at the focused row. Compress/Extract select what they made; the tree
  reloads the folders an operation, rename or new folder touched.
- [x] **Properties** (`properties.rs` sheet + `ui/properties.slint`; a window per request, not
  modal): type, location, link target, size, created/modified/accessed, attributes with owner,
  hidden; folders and several items are counted in the background
  (`fileem-core::fs::measure`, links not followed, stops when the window closes). Alt+Enter,
  menus. Closing the main window closes them.
- [x] **Shell** (`shell.rs`, `ShellTest` ported): terminal per platform (`$TERMINAL`, then
  known terminals with their folder flag), Show in file manager (FileManager1 D-Bus `ShowItems`
  with `'` escaped, else open the folder), launches recorded as "open/terminal/reveal <path>"
  under `FILEEM_NO_LAUNCH`.
- Later stages added Open in new tab / other side, Copy/Move to other side (stage 3) and the
  View submenu (stage 5), Search in folder, the bookmark items and Eject (stage 7).
  The tree outlines the right-clicked row while its menu is open (Java's `context-target`;
  added after 5b): Slint has no "menu closed" event, but a menu takes the keys while open and
  they come back however it closes, so the tree's right-click gives the window's key handler
  the focus first and the outline ends when it has it again (`menu-closed`).
- Not ported from Java: "Open with…" never existed there either (the plan's old note).

### Stage 3: tabs, dual pane, address bar, filter (done)

- [x] **Structure**: `app.rs` (state, session, wiring, view sync) + `app/nav.rs` (listings per
  tab, watching, trees, tabs, sides) + `app/commands.rs` (operations, clipboard, dialogs, menus,
  properties) + `app/keys.rs`. `App.sides: [Side; 2]`, each with a `Tree` and `Vec<TabState>`;
  every tab has an id that async results (listings, watch tokens) look up, so closing or adding
  tabs never misroutes a result. Each tab watches its own folder. Slint: `ui/side.slint`
  (`SidePane`, driven by one `SideView` struct per side; requests such as "scroll to row" or
  "edit the address" are sequence numbers that change); app.slint shows one or two panes.
- [x] **Tabs** per side: tab bar with close buttons and +, Ctrl+T (current folder) / Ctrl+W
  (never the last), middle-click closes, tab menu (Duplicate tab, Open in other side, Close
  tab, Close other tabs). New tabs sort like the tab you were in.
- [x] **Second side**: F9 / toolbar button, Tab switches sides, a press anywhere in a side
  activates it (accent line on its tab bar), draggable divider (`split.sides`). F5/F6 ask for
  the target (prefilled with the other side's folder, typed paths resolved and checked in the
  background); only with the second side shown. Menus: Open in new tab / other side, Copy /
  Move to other side (greyed out without the second side). Operations refresh every tab and
  tree showing a touched folder.
- [x] **Address bar**: breadcrumbs (`address.rs`; the root as a drive, names as they are, the
  end stays visible); a click beside the crumbs or Ctrl+L edits; `~`, relative paths and `..`
  resolve; a file path opens its folder with the file selected; Escape cancels.
- [x] **Filter** (Ctrl+F): `*`/`?` wildcards, else "contains"; Escape clears, Enter/Down back
  to the list; a new folder clears it; status "n items (m filtered out)" / "(m hidden)".
- [x] **Columns** (`columns.rs`): Name, Size, Type, Date modified/created/accessed, Extension,
  Attributes; header right-click chooses, the header's right edge drags the width, click sorts
  (Created and Extension sort too); `columns.<id>.visible/width` saved, `columns.order` read
  (and saved since columns can be dragged into another order, see below). Extra column texts are filled only while shown (attributes look up the
  owner).
- [x] **Session**: all tabs of both sides with their selected tab, `dual`, `split.sides`, tree
  widths; Java's per-tab `.view`/`.size` keys are carried along unchanged. Missing tab folders
  walk up to an existing parent (as in Java). Renaming a folder moves the tabs inside it.
- [x] **F-key bar** (added after stage 5b): F2 Rename, F5 Copy, F6 Move (greyed out without the
  second side), F7 New folder, F8 Delete, F9 Dual pane, Tab Switch side; each button sends its key
  through the normal key handler (`function-key` in app.slint), then gives the keys back to the
  list. F3 Preview joins it with stage 6.
- [x] **Reordering** (added after 5b): tabs and columns are dragged to another place (a left
  press moved 6 px; an accent line marks the gap; `tab::reordered`, `Columns::move_to_gap`);
  the shown tab stays shown, the column order is saved in `columns.order`. Name stays the
  first column (the rows draw icon and name there; Java could move it too).

### Stage 4: drag & drop and selection box (done)

- [x] **Selection box** (Java `Marquee`): a press on a row past its icon + name (4 px slack), or
  on empty space, draws a box once moved 4 px; it selects the rows it touches (`tab::box_rows`,
  content coordinates so autoscroll keeps the anchor), Shift adds, Ctrl/Cmd toggles, plain
  replaces; the focus follows the mouse row. Above/below the list it scrolls (a Slint `Timer`).
  `Tab::start_box / box_select / end_box`.
- [x] **Drag & drop inside the window**: list rows (by icon + name) and tree folders (not
  section titles or drives) drag; drop targets are folder rows, the list's empty space (the
  shown folder), tree folders, in either side. Refused (no mark, no question) into the dragged
  items themselves or where they all already are (`fs_util::can_drop`, paths only, cached per
  hovered row). Every drop asks Move (default) / Copy / Cancel in the dialog overlay, after the
  drop event (`Dialog::Drop`, the dialog's new `alt` button). A press on a row of a larger
  selection keeps the selection until release (`Tab::press`), so several files drag together.
  `app/dnd.rs`; Slint: `DragArea`/`DropArea` in `side.slint`.
- [x] Fixed on the way (stage 3): showing/hiding the second side rebuilt the left side (scroll
  positions lost): both panes always exist now. The tree's scroll to the current folder could be
  undone by the list clamping before it had measured new rows.
- **Decided**: drags to and from other applications are not possible with Slint 1.18: its winit
  backend implements no native drag (`WindowAdapterInternal::start_drag` stays the default) and
  ignores dropped files. The clipboard works with other file managers instead. Revisit when Slint
  adds native drag (tracking issue slint-ui/slint#1967); `DataTransfer::set_file_paths` is
  already there, so the payload would carry the paths then.
- [x] **Drag image** (added after 5b): the dragged row's icon and name (a grid item, a tree
  row) follow the mouse, faded: at the drag's start the window is snapshot and the item's
  rectangle cut out (`dnd::show_drag_image`), like Java's snapshot of the cell. Drag & drop in
  the other views came with stage 5.

### Stage 5a: view modes and thumbnails (done)

- [x] **The eight Explorer views** per tab (`view_mode.rs`: ids, sizes, steps, slider mapping,
  the Java session format; `grid.rs`: slots, lanes, hit tests, box hits, key moves). Slint
  reports the grid's size (`grid-resized`) and the slots in view (`grid-range`); Rust computes
  the layout and hands it over in `SideView`; Slint builds only the slots in view (`GridItem`
  in `ui/grid.slint`) on a slot model from Rust. Same rows, selection, sort, filter, menus, drag
  & drop and selection box as Details. `app/views.rs`.
- [x] View button and sort button between address and filter (menus from `menu::view_menu` /
  `sort_menu`), "View" submenu in the folder's menu, Ctrl+Shift+1…8 (digits or the US layout's
  shifted symbols), Ctrl+wheel, arrow/page keys that follow the grid, item size (− / slider /
  +, Ctrl+= / Ctrl+−) per view, sizes and views saved per tab (`<side>.tab.<i>.view/.size`, same
  format as Java), new tabs open in the current view.
- [x] **Thumbnails** (`thumbs.rs`): PNG/JPEG/GIF/BMP via the `image` crate on two threads, only for
  the slots in view (newest wishes replace older ones), fixed load sizes, LRU bounded to 8 M
  pixels (~32 MB; Java: 16 M), failures remembered per file version.
- [x] Fixed on the way: the directory watcher reported *opens* (notify's inotify mask includes
  `OPEN`), so every listing, thumbnail or read by another program refreshed the folder again,
  forever (about every 0.5 s); reads are ignored now (`watcher::is_change`). A selection box
  started on a row lost its release once that row scrolled away (the box stayed armed and kept
  scrolling); rows now take presses only on icon + name, the rest goes to the list's fixed
  background area. Status messages ("Cannot open …") survive view updates.
- [x] Fixed after 5b: Ctrl+wheel works anywhere over a Details list that scrolls (a layer over
  the list, enabled only while Ctrl is held, takes the wheel first; its presses do what the area
  behind does); a double-click on the size slider resets the view's size (two `released` within
  500 ms); List's column fits the widest name *measured* in the items' font (the 32 roughly
  widest names, `grid::names_to_measure`, go to a hidden multi-line `Text`; its width comes back
  through `list-measured`), no longer estimated from name lengths. JPEG thumbnails ignored EXIF
  orientation (as in Java; fixed in 5b).

### Stage 5b: image readers (done)

- [x] `fileem_core::images`: `Format::of` (by extension) and `decode(path, max_side)` → RGBA
  `Pixels` (+ full size), upright, never enlarged, `None` for what it can't show (a panicking
  decoder included: release builds unwind now). Ported with their Java tests: `psd` (PsdReader:
  modes, depths, RLE/raw, PSB, JPEG fallback, damaged files), `raw` (RawReader: CR2/NEF/RW2/RAF/CR3,
  choice by size, orientation, damaged files; `fixtures` ports RawFixtures), `netpbm`
  (NetpbmReader: P1–P6, 16-bit, hostile headers), `jpeg2000` (ImageIoReader's JPEG 2000 half with
  the pure-Rust `hayro-jpeg2000`; its `target_resolution` decodes the smallest stored level at
  least the asked size, like `J2KImageReadParam.setResolution`; same OpenJPEG fixtures in
  `crates/fileem-core/testdata`). Thumbnails use it (`thumbs::supports` per format: 40 MB for
  the common formats, 256 MB for JPEG 2000, 8 GB for the partial readers).
- [x] UI test `photoshop_raw_jpeg2000_and_netpbm_files_show_thumbnails` (the Java PSD, raw and
  JPEG 2000/PPM thumbnail cases, with pixel checks).
- Beyond Java: JPEG (and other common format) thumbnails follow their EXIF orientation.
- [x] Fixed after 5b: a decode whose thumbnail is no longer wanted (scrolled away) is stopped
  (`images::decode_until` with a flag per job; Netpbm and Photoshop check it every row, the
  others between steps; a stopped decode is neither shown nor remembered as failed). JPEG 2000
  files over 256 MB are memory-mapped (`memmap2`) instead of skipped, so the size limit is 8 GB
  like the other partial readers.
- Not yet: the preview half of these readers (stage 6). A decoder of a third-party crate that is
  already running (`image`, `hayro-jpeg2000`) can't be stopped halfway.

### Stage 6: preview window (done)

- [x] `fileem_core::fs::preview` (`Preview`, `PreviewTest` ported): kinds by name, text read
  up to 256 KB with binary sniffing, UTF-8 then Latin-1, BOM dropped, a character cut by the
  limit dropped; `image_side` (4096 px for the common formats and raws, 2048 for Photoshop,
  JPEG 2000 and Netpbm, as in Java).
- [x] `src/preview.rs` (zoom, fit never enlarges, limits) and `src/app/preview.rs` (the Java
  `PreviewWindow`): F3, the toolbar toggle, the F-key bar's F3; the active tab's focused
  selected row (else the first selected) after a 120 ms settle; every load bumps a generation
  and stops the previous one (`images::decode_until`); messages for folders, binary files,
  images without a preview (raw without an embedded JPEG, Photoshop, damaged files) and read
  errors; long text says it was cut. Images: all `fileem_core::images` formats, upright.
- [x] `ui/preview.slint`: one `PreviewPane` (title, dock / close buttons, image or text in a
  scroll view, zoom and pan buttons, Ctrl+wheel zooms) shown docked right of the sides (divider,
  `split.preview`) or in `PreviewWindow` (Escape and F3 switch it off, closing it too; other
  keys go to the main window's handler, so arrows still move through the files). Both draw the
  same `PreviewView`.
- [x] Session: `preview.enabled`, `preview.docked`, `preview.width/height/x/y` (position where
  the platform reports one; Wayland doesn't), `split.preview`.
- UI tests: on/off (F3, toolbar, F-key bar, Escape, closing), image zoom/fit/pan/Ctrl+wheel,
  text (zoom = font, unknown files sniffed, binary, folder, long files), quick selection,
  Photoshop / raw upright / no embedded preview / JPEG 2000 / PPM / damaged, docking, session.
- Differences: showing the floating window doesn't hand the keyboard back to the main window
  (Slint can't activate a window); instead the preview window passes keys on. Docked, the pan
  buttons take a second row (Java wraps by width; from the width here is a binding loop).

### Stage 7: search, bookmarks, volumes, eject (done)

- [x] **Search** (`fileem_core::fs::search` with `FileSearchTest` ported, `search_history` with
  `SearchHistoryTest` ported; `src/search.rs` results, `src/app/search.rs`, `ui/search.slint`):
  "Search in folder…" in the file, background and tree menus and Ctrl+Shift+F; a window per
  search with the folder, the pattern field (Enter searches; the last 15 searches under the
  arrow button or Alt+Down, a pick searches at once; session `search.history.<i>`), Hidden files
  (starts like the list), Search / Stop; the search on a thread of its own (a stop flag, at most
  50,000 results), results reach the window in batches (one queued UI update at a time); a
  table (Name, Folder relative to the searched folder, Size, Type, Date modified) as found until
  a header is clicked (natural order, again reverses), resizable columns; Escape stops, then
  closes. Results: double-click / Enter open (folders in the main window's active tab, files with
  their programs), the menu (Open, Open containing folder, Cut, Copy, Rename, Trash, Delete,
  Copy path, Properties) and keys (F2, Del / F8, Shift+Del, Ctrl+C / X / A, Alt+Enter, arrows).
  Rename and delete are asked over the search window (`ui/dialog.slint` is now shared by both
  windows); a renamed result and what was found inside a renamed folder are re-read at their new
  paths; deleted results leave the list when the operation ends.
- UI tests: menu + results + types + folder column + excludes + open containing folder + open
  file / folder + Escape; history in every window, Alt+Down and a pick, after a restart; rename
  (file, folder with results inside), Shift+Del cancel / delete, Ctrl+C then paste; Ctrl+Shift+F,
  hidden files, empty pattern, stopping with Escape.
- Differences: opening a folder result can't bring the main window to the front (Slint can't
  activate a window). The history list is our own popup under the field (Slint has no editable
  combo box).
- [x] **Bookmarks** (`fileem_core::bookmarks` with `BookmarksTest` ported (its read-only test is
  the type system's: `items()` is a slice); `tree.rs` `set_bookmarks`; `src/app/bookmarks.rs`;
  `ui/bookmarks.slint`): one list per window, saved at once (`bookmarks.count`,
  `bookmarks.<i>.name/.path`, the Java keys). Both trees show a Bookmarks section after Quick
  access (hidden while empty; bookmarks are lazy folders kept by path, so renaming or reordering
  keeps them open; reveal walks only the drives). Ctrl+D and the toolbar button (filled and
  "Remove bookmark (Ctrl+D)" when the folder shown is bookmarked) toggle the active tab's folder;
  "Add to bookmarks" / "Remove from bookmarks" in the menus of a folder, empty space and tree
  folders; a bookmark's menu has Rename bookmark… (asked over the main window), Remove bookmark,
  Move up / down (disabled at the ends) and Manage bookmarks…, the section title only the
  latter; a bookmark is a shortcut (no cut, rename, trash, delete, duplicate). Manage bookmarks
  (Ctrl+Shift+B; one window, asking again shows it): name and folder per row, "(not found)" for
  missing folders (checked in the background), Open / Rename… / Remove / Move up / Move down /
  Close, keys Up/Down, Enter, Del, F2, Alt+Up/Down, Escape, double-click opens; live (changes from
  the main window show up). Renaming or moving a folder (list, tree, search window) takes its
  bookmarks and those inside it along; a bookmark named after the folder takes the new name.
- UI tests: Ctrl+D and the toolbar (both trees, section placement, the button following the
  folder); file / empty space / tree menus, rename from the menu, a click on a bookmark opens it;
  the bookmark menu (no destructive items, move up/down, rename, remove, the section title's
  menu) and a restart from the file; the Manage window (missing folders, buttons, keys, rename
  over the window, live list, Enter opens and closes, empty hint, Escape); a renamed folder takes
  its bookmarks (default name follows, custom name stays). `FILEEM_UI_ONLY=<part of a name>`
  runs only the matching UI scenarios while working on them.
- [x] **Drives and eject** (`fileem_core::fs::volumes` with `VolumesTest` ported plus a mount
  table test; `tree.rs` `set_volumes`; `src/app/volumes.rs`): Linux lists the root, block
  devices and network shares from `/proc/self/mounts` (system mounts, loop and zram devices
  left out; `/media`, `/run/media`, `/mnt` and pluggable buses are removable; ejectable: on a
  USB/MMC bus or mounted by the desktop, not internal disks under `/mnt`), macOS `/Volumes`,
  Windows the drive letters (`GetDriveTypeW`). Free space per drive with a 1.5 s timeout; a hung
  mount keeps one stuck lookup, reused by later polls. A thread polls every 5 s (and right after
  an eject) and hands over only changed lists; the window starts with the mount table alone (no
  space yet). The tree shows "free of" text and, for ejectable drives, an eject button before it
  (the free space is cut off first) and an Eject menu item. Ejecting removes the whole drive
  (`same_drive`: its other mounted partitions via sysfs), is refused while a running operation
  uses it, moves tabs on it home and waits until they left (up to 5 s), then runs the eject in
  the background (Linux: `udisksctl unmount` each partition + `power-off`, a failed power-off
  still counts; else `gio mount --eject`; else `umount`. macOS `diskutil eject`, Windows the
  Explorer verb then waits for the drive to go); "Ejecting…" meanwhile, no second eject; the
  result or the reason in a dialog. `places::volumes` is gone.
- UI tests: space texts, icons, order, only plugged-in drives offer Eject (menu, row button);
  eject from the row button (tabs leave first, no second eject meanwhile, the notice); refused
  while an operation uses the drive; the whole drive (a tab in the other side on the other
  partition leaves, "The drive with “photos” and “storage”"); a failure shows the reason and
  can be retried. Tests swap the real eject (`app::set_ejector`) and show their own drives
  (`app::show_volumes`, which stops the polling); nothing is ever ejected for real.
- Differences: the Windows drive labels are "Local Disk (C:)" / "Drive (E:)" without the
  volume name (Java asks the file store; that needs `GetVolumeInformationW`), and the Windows
  and macOS code paths are untested (no such machine here; the Java ones are untested too).

### Toolbar (done)

- [x] The Java toolbar's buttons in its order and with its tooltips: back/forward/up/refresh,
  new folder, cut, copy, paste, rename, delete, properties, new tab, bookmark; on the right the
  view menu (the active side's tab), preview, second side, hidden files, then (stage 8) the
  theme menu, light / dark and About. Icons from the MDI font the Java app uses. Each does what
  its key does (`commands::tool`), on the active side. The window can't get narrower than the
  toolbar (`toolbar-min-width`, about 680 px) so every button stays reachable (Java folds
  buttons into an overflow menu instead).
- UI tests: every button clicked once (dialogs asked, copy/cut + paste, properties, new tab);
  the view menu changes only the active side's view.

### Stage 8: themes and about (done)

- [x] **Themes** (`src/theme.rs` toolkit-free, `src/app/theme.rs`, `ui/theme.slint`): the
  Java families Classic, Windows 11, macOS, Adwaita (GNOME) and Aurora, each light and dark, as
  tables of the Java theme files' colors (Classic: AtlantaFX Primer), corner radii and font
  (Segoe UI Variable, SF Pro, Cantarell, Inter; the system's where missing). Every `.slint` file
  draws with the `Theme` global (surfaces: window, toolbar, tab bar, active tab, list, tree,
  status line; text, muted, borders, accent, selection with and without focus, hover, tree
  current, danger); Aurora's gradients (window, tab bar, tree, the active side's bar, the
  focused selection) are built in `theme.slint` from `ThemeFill` (Rust can't make gradient
  brushes through Slint's public API). Globals belong to one window, so every window (main,
  preview, search, bookmarks, properties, progress) gets the theme when created and on every
  change; the std widgets get the matching light / dark scheme. Toolbar: the theme menu
  (families, then Light / Dark, marks for the current ones), the light / dark button. Session
  `theme.name` / `theme.dark` (the Java ids; unknown names are Classic; default Classic light).
- [x] **About** (`src/about.rs`, `build.rs`): the toolbar's last button ("About fileEm
  0.1.0"); a dialog "fileEm 0.1.0" with Version, Built (yyyy-mm-dd, `SOURCE_DATE_EPOCH` for
  reproducible builds), Rust and Slint versions, and the system (Linux: the kernel release,
  like Java's `os.version`) and architecture. One version number: the workspace's must equal the
  pom's `app.version` (`AppInfoTest` ported). Escape now closes every OK-only dialog (as OK).
- Tests: unit tests for ids, both modes of every family, text contrast on every surface (WCAG:
  text 7, muted 3, selected 4.5), color math, the theme menu's marks, the About text and the
  build info; UI scenarios: the theme menu switches family and mode in all six kinds of window
  (and later windows start in it), the light / dark button, the session round trip, unknown
  names; the About dialog and the toolbar at the window's smallest width. Snapshots of every
  theme in both modes (`theme-<id>-<mode>`), a dark Properties window and the About box.
- Differences: the std widgets (buttons, fields, check boxes, scroll bars, the menu popups) stay
  Fluent and only follow light / dark: Slint picks a style at build time, and its accent can't
  be themed. About is a dialog over the main window, not a separate window that stays open.
  Texts without a color of their own take the std style's foreground (close to the theme's).

### Stage 9: packaging and cutover (packaging done; cutover open)

- [x] **App id and icon**: `about::APP_ID` = `com.fileem.FileEm`, set with
  `slint::set_xdg_app_id` before the first window shows (Wayland `set_app_id`, X11 WM_CLASS
  class; checked with `WAYLAND_DEBUG` and `xprop`). Not the Java app's id
  (`com.fileem.FileEmApp`): both can be installed side by side. Every window's `icon` is
  `Theme.app-icon` (the 256 px PNG, embedded), so title bars, X11 and the Windows taskbar show it;
  Wayland docks take the launcher entry's. Icons live in `crates/fileem/icons/` (all PNG sizes,
  ico, icns), written by `tools/make-icon.py` with the Java ones.
- [x] **Linux** (`crates/fileem/packaging/linux/`): `com.fileem.FileEm.desktop` (the one entry
  both scripts use; named after the app id, `StartupWMClass` too); `install.sh` installs per user
  into `~/.local/share/fileem-rust` + icons + entry (`--uninstall` removes only those, never the
  Java app's install); `make-deb.sh` builds `target/dist/fileem_<version>_<arch>.deb`
  (`/usr/bin/fileem`, entry, icons; needs `dpkg-deb`). The entry is called "fileEm (Rust)" until
  the cutover, so it can't be mixed up with the Java app's. No AppImage: the deb and the per-user
  script cover the two ways the Java app is installed now.
- [x] **Windows**: release builds have no console (`windows_subsystem`), `build.rs` embeds the
  icon and version info in `fileem.exe` (`winresource`, a build dependency only on Windows).
- [x] **macOS**: `packaging/macos/make-app.sh` builds `target/dist/fileEm.app` (Info.plist with the
  app id as bundle id, the version, the icns). Unsigned.
- [x] **CI** (`.github/workflows/rust.yml`): Linux, Windows and macOS run clippy (warnings are
  errors), `cargo test --workspace` and the release build, then package (deb, exe, zipped .app)
  and upload them as artifacts. Not run yet: the repository has no GitHub remote here.
- [x] **Renderer**: OpenGL (femtovg) stays the default; Slint falls back to the software
  renderer by itself where OpenGL is missing, and `SLINT_BACKEND=winit-software` picks it for
  the smaller memory use. Measured (snapshot test's `render_times`, release build, i7-4810MQ):
  a full repaint of the main window with 300 rows per side takes 25 ms at 1280×780, 52 ms at
  1920×1080, 86 ms at 2560×1440 and 186 ms at 3840×2160 (same with Aurora's gradients). Fine
  for small windows, too slow for smooth scrolling on big or HiDPI screens.
- Tests: `tests/packaging.rs` (the entry is named after the app id and valid per
  `desktop-file-validate`, every `.slint` window binds the icon, icons in every size and format,
  the install / uninstall / refusal of `install.sh`, the deb's files and version, the bundle's
  Info.plist; the scripts run on a stand-in executable into temp folders); UI scenario
  `windows_carry_the_app_icon`.
- Untested: the Windows and macOS builds and packages (no such machines here; CI will be the
  first run), signing (Windows SmartScreen and macOS Gatekeeper warn about unsigned apps).
- [ ] **Cutover**, once the Rust app has been used daily for a while: move the Java app out (or
  to a `java` branch), name the launcher entry "fileEm", make README/AGENTS describe the Rust app
  only.

## Decisions

- **Slint** for the UI (chosen by the user over egui, iced, Tauri), `fluent` style; no Qt
  backend (winit + femtovg OpenGL, software renderer as fallback).
- Cargo workspace in the repo root next to the Java app; `crates/fileem-core` must stay free
  of UI code; `crates/fileem` is a lib (for UI tests) plus a thin `main.rs`.
- Settings stay in the Java app's config folder and format, so switching back and forth works.
- Threads instead of virtual threads: one OS thread per background job (`bg::run`), results to
  the UI thread through `slint::invoke_from_event_loop`; operations on their own thread.

## Lessons learned (Slint 1.18)

- Linux build needs `libfontconfig1-dev` and `pkg-config` (fontique links fontconfig; its dlopen
  mode is not supported). `sudo` can't run from the agent's shell: ask the user to run it in a
  terminal.
- A `ListView` inside a plain `Rectangle` doesn't stretch; put it in a layout.
- `viewport-y` is deprecated: use `content-y`. Scrolling right after a model change gets clamped
  (rows not measured yet, also a moment *after* the scroll was set): keep a pending target and
  re-apply it in `changed visible-height` / `changed content-height` handlers until the row is
  in view *and* `content-height` reaches it (`apply-tree-scroll`).
- Special keys arrive as private-use characters: compare with `char::from(Key::…)`.
- Testing backend: init once per process (one `#[test]` with scenarios); compile the UI with
  `with_debug_info(true)` (debug builds only) or element search finds nothing; give the window
  a size or list views create no rows; pump the event loop with a single-shot timer +
  `run_event_loop_until_quit`; clicks via `WindowEvent::Pointer*` at element positions.
- Background threads reach the UI through `slint::Weak::upgrade_in_event_loop` + a Slint
  callback (the window handle is `Send`, the controller isn't): see `dir-changed`.
- Element search by accessible label can hit a container with the same label (a dialog card
  titled "Rename" vs. its "Rename" button): tests find buttons by role (`Fixture::button`).
- Headless rendering: `TestingBackend` with `renderer_name: Some("software")` makes
  `take_snapshot` work (see `tests/snapshots.rs`); the default testing backend can't render.
- `FILEEM_SNAPSHOT` renders the window for visual checks; on Wayland the new window takes
  keyboard focus, so the user's typing reaches it (it once "went up" by itself in a snapshot).
- The tree must show a hidden folder while it is the current folder or one of its parents.
- Context menus: `ContextMenuArea` takes models (`for e in model: MenuItem`, `if …: Menu`), and
  separators at the ends or next to each other are dropped, so fixed sections fed from Rust
  work. A row's `TouchArea` takes the right press, so the area never sees it: keep it
  `enabled: false` (it would show a stale menu elsewhere) and call `show()` from the handlers
  after Rust built the menu. A `ListView` (Flickable) passes non-left presses through, so a
  `TouchArea` behind it catches right-clicks on empty space.
- Testing backend: elements inside a popup report positions relative to the popup; add the
  popup's `ChildWindow` location (`i_slint_core::window::WindowInner::active_popups`, dev-dep
  pinned like the testing backend) before clicking menu items (`Fixture::pick`).
- Sizing children from the window's width inside a layout (`width: root.width * f`) is a binding
  loop; place such children by hand in a plain `Rectangle` (the two sides). A `for` can't give
  its instances ids, so per-instance requests go in as properties (sequence numbers +
  `changed`). In an `on!` style wiring macro, take the closure's variable name from the call
  site (macro hygiene).
- A `TouchArea` takes presses of every button: a right-click on a header cell never reaches a
  handler behind it; handle it in the cell.
- `LineEdit` has `key-pressed` (Escape in the address and filter fields) and a min-width of
  160px: set `min-width` (or preferred/max) to let it shrink, never `width` together with it.
- Rows as wide as all columns reach past a half-width side: test clicks go near an element's
  left end (`Fixture::click_at`).
- `Window` already has `icon` and similar properties: a component inheriting `Window` can't
  declare its own (`item-icon`); `GridLayout` rows from a `for` set `col` themselves.
- `DragArea` checks `enabled` on every move: never bind it to the child `TouchArea`'s
  `has-hover`, which turns false the moment the drag intercepts the child (the drag cancels
  itself). Its `data` must be non-empty before the press. `changed dragging` runs later than
  the drag starts but before the drop, so the controller learns the source there.
- A `ListView` (Flickable) holds a left press back for 100 ms when it can scroll; UI tests wait
  that long after pressing before they move (`Fixture::drag_to`).
- A `for s in (cond ? 2 : 1)` repeater rebuilds every instance when the count changes; keep a
  fixed count and hide what isn't shown. Worse, `for k in some-int-property` (and a model
  expression like `cond ? model : []`) is a *new* model whenever its binding re-evaluates (any
  change of the struct it reads), so every instance is rebuilt and a press in progress is lost
  (double-clicks, drags). Repeat over models from Rust that keep their identity (`Side::slots`,
  `list-files` / `no_files`).
- A `ListView` destroys rows that scroll out of view, with their `TouchArea` and its mouse grab:
  a gesture that may scroll (selection box) must be handled by an element that isn't a row.
- `ListView` recomputes `content-y` from its own row-height estimate and can nudge a programmatic
  scroll back a few pixels; tests wait for the final state, not the first moment it holds.
- An element given only `width` inside a plain `Rectangle` doesn't fill its height (and a
  component instance doesn't fill its parent at all): set `height` (and `width`) explicitly.
- A `Flickable` gets wheel events after its children: rows can catch Ctrl+wheel, the area behind
  a scrollable list can't.
- `Slider.changed` fires only for the user's moves, not for `value` set from code; `released`
  is the moment to give the keyboard focus back.
- `release` used `panic = "abort"`; a panic in a third-party decoder (damaged files) would kill
  the app, so it unwinds now and `images::decode` catches it.
- A `Flickable` takes every wheel event (`interactive` doesn't matter), after its children; to
  catch Ctrl+wheel anywhere over a list, a `TouchArea` on top is enabled only while Ctrl is held.
  The window's `FocusScope` sees Ctrl through `capture-key-pressed` / `capture-key-released`
  even while a text field has the focus.
- Slint's `Slider` calls `released` after every left click (not only after drags): two within
  the double-click time make a double-click.
- A `changed` handler on the `preferred-width` of a layout over a repeater did not fire again
  when the repeater's model changed; a single `Text` with one name per line measures the widest
  line reliably.
- `changed` handlers run between event-loop iterations, not while an event is dispatched: a
  release right after a move can't rely on a value a handler keeps up to date. The tab and
  column drags let the element under the mouse compute the gap from the last position when the
  release is handled, and copy a value into a `let` before changing what it depends on (a
  binding re-evaluates at once).
- The UI tests run on the testing backend with the software renderer (like the snapshots), so
  `take_snapshot` works there (drag images); it costs nothing until something is rendered.
- A test that sends signals to its own process needs a binary of its own (`harness = false`):
  the test harness's threads don't block them, and the default action ends the process.
- `ComponentHandle::show()`/`hide()` run change handlers of other windows, which may call back
  into the controller: never call them while holding the app's `RefCell` borrow.
- A `Window`'s own `close` callback shadows the builtin `close()`; and an element id equal to an
  enum value (`image`) breaks `accessible-role: image`.
- An element id equal to an accessible role name (`list`) breaks `accessible-role: list` too.
- `Path::join("")` adds a trailing separator (`new.txt/`), which no longer names a file.
- `notify`'s inotify backend reports opens and reads (`EventKind::Access`): filter them, or a
  watched folder refreshes itself every time it is listed.
- `slint::set_xdg_app_id` needs the platform to exist (it returns `NoPlatform` before): call it
  after the first `AppWindow::new()` and before `run()` / `show()`; every later window gets it.
  Check it with `WAYLAND_DEBUG=1` (`xdg_toplevel.set_app_id`) or, under X11, `xprop WM_CLASS`.
- Timing renders: debug builds are ~10× slower than release; measure `take_snapshot` with
  `cargo test --release`.
- `image`'s `ImageReader::into_decoder` sets the limits but doesn't reserve the picture's buffer
  (only `decode()` does): a lying header (a PNG claiming 1,000,000² pixels) then allocates
  terabytes, and a failed allocation aborts the process; `catch_unwind` doesn't help. Check
  `total_bytes()` first (`images::MAX_DECODE_BYTES`). `hayro-jpeg2000` needs ~27 bytes per
  decoded pixel and trusts the header too: `jpeg2000::MAX_DECODE_PIXELS` caps the level decoded.
- Linux file names are bytes: join `Path::file_name()` (an `OsStr`), never `display_name`
  (lossy), onto a target. Two non-UTF-8 names both became `�.txt` in a copy, and a move asked
  to replace one with the other.
- Windows: `cmd /C start <path>` runs `&` in file names as a command (`x&calc`); files open
  through `ShellExecuteW`. Windows Terminal splits its command line at `;`, also inside quotes:
  escape it as `\;` (`shell::windows_terminal_escape`). Check Windows-only code with
  `cargo check --workspace --target x86_64-pc-windows-msvc` (needs `rustup target add`).
- UI tests: wait for a condition, not a fixed time, before scrolling the tree. The tree reveals
  the current folder in the background and scrolls to it, so under load it scrolled away again
  after `settle` (the drives scenario failed in 3 of 4 full runs); `tree_to_top` waits for it.
- With 100,000 files an arrow key took ~65 ms in release: `sync_selection` read every row and
  the status walked all rows to find the selected ones. `Side::flagged` remembers the rows drawn
  selected or current, so a change touches only those and the new ones, and
  `Tab::selected_rows` costs the selection's size (now ~4 ms). Select all (~140 ms) and the first
  move after it still touch every row; type-ahead compares names without lowercased copies.
