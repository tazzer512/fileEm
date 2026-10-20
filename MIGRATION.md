# Migration plan: Java/JavaFX → Rust/Slint

The working plan for porting fileEm to Rust, kept up to date as stages land. Read this first
when continuing the migration; `RUST.md` has the build/test/layout reference, `AGENTS.md` the
rules (tests, integrity, conventions) that apply to both versions.

**Goal**: the Rust app replaces the Java app with the same features and behavior, as one native
executable with a much smaller memory footprint (measured: Java ~360 MB, Rust ~105 MB with
OpenGL, ~32 MB with the software renderer).

**Status (2026-10-20)**: stages 0–4 and 5a (view modes, thumbnails of common formats) done on
branch `rust`. The Java app in `src/` is still the complete, daily-use version; nothing was
removed from it. Next: stage 5b (PSD, camera raw, JPEG 2000 and Netpbm readers).

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
  theme etc. survive for the Java app. Not ported: the JVM shutdown hook (SIGTERM/logout save);
  the 2 s autosave covers most of it.) Original notes: (`Session` 153, `Autosave` 74; `SessionTest` 8): same file and keys
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
  Del/F8 Trash and Shift+Del delete after confirming, F2, F7/Ctrl+Shift+N. Still to do: new
  text file (menu only in Java), tabs/bookmarks following a renamed folder (stage 3/7).)
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
- Not yet (they need later stages): Open in new tab / other side, Copy/Move to other side
  (stage 3); View submenu (stage 5); Search in folder, bookmarks (stage 7); Eject (stage 7).
  The tree doesn't outline the right-clicked row (Java's `context-target`): Slint gives no
  "menu closed" event to remove the outline again.
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
  (no reordering yet). Extra column texts are filled only while shown (attributes look up the
  owner).
- [x] **Session**: all tabs of both sides with their selected tab, `dual`, `split.sides`, tree
  widths; Java's per-tab `.view`/`.size` keys are carried along unchanged. Missing tab folders
  walk up to an existing parent (as in Java). Renaming a folder moves the tabs inside it.
- Not yet: the F-key bar at the bottom (Java has one with F5 Copy, F6 Move, …), dragging tabs or
  columns to reorder them, per-tab view modes (stage 5).

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
- Not yet: the drag image (Slint draws none unless `drag-image` is set), drag & drop in the
  other view modes (stage 5).

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
- Not yet: Ctrl+wheel over a Details list that can scroll works only on the names (the list's
  scroll area takes the wheel elsewhere); double-click on the size slider doesn't reset it;
  List's column width is estimated from the name lengths, not measured; JPEG thumbnails ignore
  EXIF orientation (as in Java).

### Stage 5b: image readers

- Readers to port into core: `PsdReader` 327 (`PsdReaderTest` 10), `RawReader` 550 + `RawImages`
  (`RawReaderTest` 8, `RawFixtures`), `NetpbmReader` 160, JPEG 2000 (`ImageIoReader` 218,
  `ImageIoReaderTest` 9): pick `jpeg2k`/OpenJPEG or a pure-Rust decoder; keep resolution-level
  decoding for speed. Copy the test fixtures from `src/test/resources`. They plug into
  `thumbs::decode` (and the preview of stage 6). UI tests: `photoshopFilesShowAPreviewInTheIconViews`,
  `cameraRawFilesShowAPreviewInTheIconViews`, the thumbnail half of
  `jpeg2000AndPpmImagesArePreviewedAndShowAsThumbnails`.

### Stage 6: preview window

- `PreviewWindow` 645, `Preview` 126 (`PreviewTest` 4): F3, text (encoding sniffing, 256 KB),
  images with zoom/fit/pan, docking into the main window, settle delay (120 ms) and cancelling
  stale loads; session `preview.*`.

### Stage 7: search, bookmarks, volumes, eject

- `SearchWindow` 575 + `FileSearch` 209 (`FileSearchTest` 11) + `SearchHistory`
  (`SearchHistoryTest` 3): streaming results in batches, history combo, result commands.
- `Bookmarks` 158 + `BookmarksDialog` 205 (`BookmarksTest` 7): tree section, Ctrl+D,
  Ctrl+Shift+B.
- `Volumes` 461 (`VolumesTest` 6): mount table parsing, polling with per-volume timeouts,
  ejectable detection, eject (udisksctl / gio / umount; Windows, macOS), refused while busy.

### Stage 8: themes and about

- `Theme` 160 + theme CSS (`ThemeTest` 6): map the five families to Slint styles
  (fluent, cupertino, material, cosmic, …) plus our own palette globals; light/dark.
- About box with version and build date (`AppInfo`; one version number for both builds).

### Stage 9: packaging and cutover

- Linux: install script with `.desktop` entry and icons (like `tools/install-linux.sh`; the
  window class / app id must match the desktop file), AppImage or deb.
- Windows (`.exe`, icon resource, no console) and macOS (`.app` bundle, icns) builds; CI on
  all three OSes running `cargo test` and the release build.
- Decide the default renderer (OpenGL vs software) after testing big windows.
- Cutover: when every stage is ticked and the Rust app has been used daily for a while, move
  the Java app out (or to a `java` branch), make README/AGENTS describe the Rust app only.

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
- `notify`'s inotify backend reports opens and reads (`EventKind::Access`): filter them, or a
  watched folder refreshes itself every time it is listed.
