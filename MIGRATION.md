# Migration plan: Java/JavaFX → Rust/Slint

The working plan for porting fileEm to Rust, kept up to date as stages land. Read this first
when continuing the migration; `RUST.md` has the build/test/layout reference, `AGENTS.md` the
rules (tests, integrity, conventions) that apply to both versions.

**Goal**: the Rust app replaces the Java app with the same features and behavior, as one native
executable with a much smaller memory footprint (measured: Java ~360 MB, Rust ~105 MB with
OpenGL, ~32 MB with the software renderer).

**Status (2026-10-19)**: stage 0 done (commit `d825f36` on branch `rust`). The Java app in
`src/` is still the complete, daily-use version; nothing was removed from it. Next: stage 1.

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
- **Directory watcher** (`DirWatcher` 125; `DirWatcherTest` 3): `notify` crate; coalesced
  refresh throttled to 2× the last listing time (`FileTab.requestRefresh`); skip while a
  navigation loads; refresh after our own operations.
- **Progress window** (`OperationUi` 161, `ProgressWindow` 382, `Eta` 98; `EtaTest` 6): a
  second Slint window per main window; one row per running op after 350 ms; Pause/Resume and
  Cancel per row; speed + time left from `Eta`; conflicts asked inline in the row (operation
  thread waits on a channel; never a modal dialog); closing cancels all; app close with running
  ops asks, then cancels and waits (`Handle::await_stopped`).
- **Clipboard** (`FileClipboard` 63): OS clipboard with file lists (Linux:
  `x-special/gnome-copied-files` + `text/uri-list`; Windows: `CF_HDROP` + Preferred DropEffect;
  macOS: file URLs). Check `arboard` for custom formats first; otherwise per-platform code.
  Ctrl+C/X/V.
- **Dialogs**: F2 rename (validate with `fs_util::validate_name`), F7/Ctrl+Shift+N new folder,
  Shift+Del with confirmation, error dialog for operation errors.
- UI tests from `MainWindowUiTest`: clipboard, rename, new folder, delete, progress window
  (pause/cancel, stacked ops, conflict buttons), autosave/session.

### Stage 2: menus and properties

- Right-click menus on files, folders, empty space and the tree (`MainWindow.fileMenuItems` /
  `treeMenuItems`): Slint `ContextMenuArea` + `Menu`. Built per request; tree shortcut items
  (quick access, bookmarks, volumes) get no cut/rename/delete. Right-click in the tree must not
  navigate.
- Properties dialog (`PropertiesDialog` 189): attributes, owner, dates, background folder size.
- Open with / open terminal here (`Shell` 191; `ShellTest` 4).

### Stage 3: tabs, dual pane, address bar, filter

- Tabs per side (`ExplorerSide` 246, `FileTab` 872), Ctrl+T/W; dual pane F9, Tab switches side,
  F5/F6 copy/move to the other side with an editable target.
- Breadcrumb address bar (`AddressBar` 190), Ctrl+L edits.
- Filter box (Ctrl+F) with `fs_util::name_matcher`.
- More columns (created, accessed, extension, attributes) with chooser and widths
  (`FileTable` 286), saved in the session.

### Stage 4: drag & drop and selection box

- Rubber-band selection (`Marquee` 258): press outside a row's name starts the box; autoscroll;
  Shift adds, Ctrl toggles.
- Internal drag & drop between list, tree, tabs and sides: always asks Move/Copy/Cancel after
  the drop (`MainWindow.drop` → `moveOrCopy`).
- **Risk**: drag & drop to and from other applications. Check what Slint/winit support at that
  point; fallbacks: platform code per OS, or clipboard only. Decide before building on it.

### Stage 5: view modes and thumbnails

- The eight Explorer views (`ViewMode` 134, `FileGrid` 1025): virtualized grid on the same
  rows and selection; size slider, Ctrl+wheel, Ctrl+Shift+1…8.
- Thumbnails (`Thumbnails` 203): `image` crate, LRU bounded by pixels, fixed load sizes.
- Readers to port into core: `PsdReader` 327 (`PsdReaderTest` 10), `RawReader` 550 + `RawImages`
  (`RawReaderTest` 8, `RawFixtures`), `NetpbmReader` 160, JPEG 2000 (`ImageIoReader` 218,
  `ImageIoReaderTest` 9): pick `jpeg2k`/OpenJPEG or a pure-Rust decoder; keep resolution-level
  decoding for speed. Copy the test fixtures from `src/test/resources`.

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
  (rows not measured yet): keep a pending target and re-apply it in `changed visible-height` /
  `changed viewport-height` handlers until the row is in view (`apply-tree-scroll`).
- Special keys arrive as private-use characters: compare with `char::from(Key::…)`.
- Testing backend: init once per process (one `#[test]` with scenarios); compile the UI with
  `with_debug_info(true)` (debug builds only) or element search finds nothing; give the window
  a size or list views create no rows; pump the event loop with a single-shot timer +
  `run_event_loop_until_quit`; clicks via `WindowEvent::Pointer*` at element positions.
- `FILEEM_SNAPSHOT` renders the window for visual checks; on Wayland the new window takes
  keyboard focus, so the user's typing reaches it (it once "went up" by itself in a snapshot).
- The tree must show a hidden folder while it is the current folder or one of its parents.
