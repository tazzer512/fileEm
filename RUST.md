# fileEm in Rust (branch `rust`)

A port of the JavaFX app to Rust with a [Slint](https://slint.dev) UI, for a smaller memory
footprint and a single native executable. The Java app stays in `src/` as the complete version
and the reference until the Rust one has caught up; both build from this tree. This file is the
build/test/layout reference; the migration plan and status are in `MIGRATION.md`.

## Why, in numbers

Same machine (Linux, Wayland), resident memory of a running app showing a folder:

| | Memory (RSS) | Shipped as |
|---|---|---|
| Java (jpackage) | ~360 MB | 84 MB folder with a bundled runtime |
| Rust, OpenGL renderer (default) | ~105 MB | one 20 MB executable |
| Rust, software renderer (`SLINT_BACKEND=winit-software`) | ~32 MB | same executable |

Most of the OpenGL figure is the GPU driver. Whether to make the software renderer the default
is still open (it uses the CPU to draw; fine for lists, to be checked on big 4K windows).

## Build, run, test

- Rust stable via [rustup](https://rustup.rs) (`~/.cargo/bin` on the PATH). Linux also needs
  `libfontconfig1-dev` and `pkg-config` to build (not to run).
- `cargo run -p fileem [FOLDER]`: starts the app (debug build).
- `cargo build --release -p fileem`: the executable is `target/release/fileem`.
- `cargo test --workspace`: core tests, UI logic tests and the headless UI test, a few seconds.
- `cargo clippy --workspace --all-targets`: must stay warning free.
- Visual checks without a display: `cargo test -p fileem --test snapshots -- --ignored` renders
  the main window (also in the grid views), a dialog, a right-click menu, the progress window and a Properties window headless to `target/snapshots/*.ppm`
  (convert with Pillow to look at them). Extend it when adding windows or dialogs.
- Visual checks of the real app (Wayland has no desktop screenshots): `FILEEM_SNAPSHOT=out.ppm cargo run -p fileem
  FOLDER` renders the window after the folder loaded, writes it as PPM and quits. Don't type while
  it runs: the window takes keyboard focus as it opens and your keys reach it.

The test rules of `AGENTS.md` apply here too: every feature gets tests, every bug fix a
regression test, file operations need integrity tests, and nothing touches real user folders,
the real Trash or launches programs (`FILEEM_NO_LAUNCH` records opens in `shell::launched()`).

## Layout

```
Cargo.toml                 workspace (release: LTO, stripped, panics unwind: a decoder
                           that panics on a damaged file costs one thumbnail)
crates/fileem-core/        no UI: everything that touches the file system
  src/util/                natural_order, formats, fs_util, os, eta       (Java util)
  src/fs/                  entry (FileEntry), file_types, listing, places,
                           trash, watcher, measure (folder sizes)         (Java fs)
  src/ops/                 file_operation (+ tests.rs: port of FileOperationTest), names
                           (rename / create, never replacing)
  src/session.rs           session.properties shared with the Java app (Java Session)
  src/images/              image decoding for thumbnails/preview: mod (formats, decode, shrink,
                           EXIF orientation), psd, raw, jpeg2000 (hayro-jpeg2000), netpbm;
                           fixtures (test files; also for the UI tests via feature `fixtures`)
                           (Java PsdReader, RawReader, ImageIoReader, NetpbmReader, RawImages)
  testdata/jpeg2000/       JPEG 2000 test files (made with OpenJPEG; same as the Java tests')
crates/fileem/             the app
  ui/app.slint             main window: toolbar, one or two sides, menu, dialogs (layout only)
  ui/side.slint            one side: tree, tab bar, address + view/sort buttons + filter,
                           columns, list or grid, status with item size
  ui/grid.slint            one item of the grid views (draws only)
  ui/icons.slint           shared icons and the tool button
  src/app.rs               controller: state, session, wiring, models (Java MainWindow)
  src/app/nav.rs           listings per tab, watching, trees, tabs, sides (Java ExplorerSide/FileTab)
  src/app/commands.rs      operations, clipboard, dialogs, menus, properties
  src/app/keys.rs          the keyboard handler (Java MainWindow.onKey)
  src/app/dnd.rs           selection box, drag & drop inside the window, Move/Copy question
                           (Java Marquee, Dnd, MainWindow.drop)
  src/app/views.rs         view modes, item sizes, grid geometry, keys, thumbnails (Java FileTab views)
  src/view_mode.rs         the eight views, sizes and steps, session format (Java ViewMode)
  src/grid.rs              grid layout: slots, lanes, hit tests, box hits, key moves (Java FileGrid)
  src/thumbs.rs            thumbnail threads and LRU cache over fileem_core::images (Java Thumbnails)
  src/tab.rs, tree.rs      tab (rows, filter, selection) and tree state, toolkit-free, unit-tested
  src/columns.rs           details columns: visible, widths, session (Java FileTable)
  src/address.rs           breadcrumbs and typed paths (Java AddressBar)
  src/bg.rs                background jobs with results on the UI thread (Java Bg)
  src/progress.rs          progress rows: detail line, estimate, conflict question (toolkit-free)
  src/ops_ui.rs            runs operations with a progress window row (Java OperationUi)
  ui/progress.slint        the progress window
  src/menu.rs              right-click menus per request (toolkit-free; Java fileMenuItems/treeMenuItems)
  src/properties.rs        what the Properties window shows (toolkit-free) + ui/properties.slint
  src/clipboard.rs         files and text on the OS clipboard (Java FileClipboard)
  src/shell.rs             open, terminal, show in file manager (Java Shell)
  tests/ui.rs              end-to-end UI test on Slint's headless testing backend
```

Notes for the UI test: Slint's testing backend can be initialized once per process, so
`tests/ui.rs` has one `#[test]` that runs each scenario with its own window and folder. Elements
are found by accessible label (debug builds compile the UI with element info for that, see
`build.rs`). The window needs a size, or the list views create no rows. Wait with
`wait_until(..)`, which pumps the event loop; never sleep. The window's state is read through
`Fixture::view()` (the active side's `SideView`) and `app::tabs`, `app::active_side`,
`app::is_dual`; items inside an open menu are clicked with `Fixture::pick` (it adds the popup's
offset). Mouse drags: `Fixture::drag_to(from, to, modifiers)` (waits 120 ms after the press, as
a list holds presses back 100 ms) + `release`, or `drag_drop`; `on_name` / `on_size` give a
row's name (drags files) or Size cell (starts a box). The tree scrolls to the current folder:
`scroll_tree_to_top` before clicking rows above it, after waiting for the reveal. Grid items
are list items too: `list_row(name)` finds them; `set_view`, `view`, `icon_px`, `grid_names`
and `open_folder` help the view scenarios (`app::view_of`, `app::grid`, `app::tab_views`).
`menu_item` takes the last match (the popup comes after a column header of the same name).

## Differences from the Java version (on purpose)

- Linux batch flushes call `syncfs(2)` directly instead of starting `sync -f`.
- Renames that must not replace use `renameat2(RENAME_NOREPLACE)` (macOS: `renamex_np`), so
  "check, then rename" has no race.
- A failed same-drive rename onto a displaced folder restores the folder (Java left it under its
  temp name).
- Zip names: the `zip` crate decodes CP437 / UTF-8 itself.
- Folder-into-itself checks resolve links in both paths (Java resolved only the target's).
- The tree shows a hidden folder while you are in it.
- A session without `<side>.tabs` reads the consecutive `<side>.tab.N` keys.
- Thumbnails of JPEGs (and the other common formats) are turned upright per EXIF, like raws.
- JPEG 2000 is decoded with the pure-Rust `hayro-jpeg2000` (Java: jai-imageio); it needs the
  whole file in memory, so thumbnails skip files over 256 MB.

## Port status

Done, next stages, decisions and lessons learned: see `MIGRATION.md`.
