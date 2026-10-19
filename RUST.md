# fileEm in Rust (branch `rust`)

A port of the JavaFX app to Rust with a [Slint](https://slint.dev) UI, for a smaller memory
footprint and a single native executable. The Java app stays in `src/` as the reference until the
Rust one has caught up; both build from this tree.

## Why, in numbers

Same machine (Linux, Wayland), resident memory of a running app showing a folder:

| | Memory (RSS) | Shipped as |
|---|---|---|
| Java (jpackage) | ~360 MB | 84 MB folder with a bundled runtime |
| Rust, OpenGL renderer (default) | ~105 MB | one 17 MB executable |
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
- Visual checks (Wayland has no desktop screenshots): `FILEEM_SNAPSHOT=out.ppm cargo run -p fileem
  FOLDER` renders the window after the folder loaded, writes it as PPM and quits. Don't type while
  it runs: the window takes keyboard focus as it opens and your keys reach it.

The test rules of `AGENTS.md` apply here too: every feature gets tests, every bug fix a
regression test, file operations need integrity tests, and nothing touches real user folders,
the real Trash or launches programs (`FILEEM_NO_LAUNCH` records opens in `shell::launched()`).

## Layout

```
Cargo.toml                 workspace (release: LTO, stripped, panic=abort)
crates/fileem-core/        no UI: everything that touches the file system
  src/util/                natural_order, formats, fs_util, os            (Java util)
  src/fs/                  entry (FileEntry), file_types, listing, places,
                           trash                                          (Java fs)
  src/ops/                 file_operation (+ tests.rs: port of FileOperationTest)
crates/fileem/             the app
  ui/app.slint             window layout and drawing only
  src/app.rs               controller: callbacks, keys, models (Java MainWindow)
  src/tab.rs, tree.rs      tab and tree state, toolkit-free and unit-tested
  src/bg.rs                background jobs with results on the UI thread (Java Bg)
  src/shell.rs             open with the default program
  tests/ui.rs              end-to-end UI test on Slint's headless testing backend
```

Notes for the UI test: Slint's testing backend can be initialized once per process, so
`tests/ui.rs` has one `#[test]` that runs each scenario with its own window and folder. Elements
are found by accessible label (debug builds compile the UI with element info for that, see
`build.rs`). The window needs a size, or the list views create no rows. Wait with
`wait_until(..)`, which pumps the event loop; never sleep.

## Differences from the Java version (on purpose)

- Linux batch flushes call `syncfs(2)` directly instead of starting `sync -f`.
- Renames that must not replace use `renameat2(RENAME_NOREPLACE)` (macOS: `renamex_np`), so
  "check, then rename" has no race.
- A failed same-drive rename onto a displaced folder restores the folder (Java left it under its
  temp name).
- Zip names: the `zip` crate decodes CP437 / UTF-8 itself.
- Folder-into-itself checks resolve links in both paths (Java resolved only the target's).
- The tree shows a hidden folder while you are in it.

## Port status

Done:

- [x] Core: natural sort, formats, name helpers, file types, `FileEntry` (lazy owner), listing.
- [x] File operations: copy, move, delete, trash, compress, extract; pause/resume/cancel; every
      integrity rule of `AGENTS.md`, with the Java tests ported (and mutation-checked).
- [x] Window: toolbar (back/forward/up/refresh, address bar, hidden toggle), folder tree (Quick
      access with localized XDG folders, volumes, lazy loading, reveal + boxed current folder +
      bold path, folds what it opened, scrolls to the folder), splitter, details list (icons, sort
      by column with folders first), status line.
- [x] Keys: Enter, Backspace/Alt+Up, Alt+Left/Right, arrows/PgUp/PgDn/Home/End with Shift/Ctrl,
      Ctrl+Space, Ctrl+A, Ctrl+R, Ctrl+H, Ctrl+L, Del/F8 (Trash), type-ahead.

To do (roughly in order):

- [ ] Session (window, tabs, columns, sort, hidden) and autosave; settings in the same folder.
- [ ] Directory watcher (`notify` crate) with coalesced refreshes.
- [ ] Progress window: rows per operation with pause/resume/cancel and inline conflict questions.
- [ ] Copy/cut/paste with the OS clipboard; F2 rename, F7 new folder, Shift+Del with confirmation.
- [ ] Right-click menus; properties dialog.
- [ ] Tabs, dual pane (F9, F5/F6, Tab), breadcrumb address bar, filter (Ctrl+F).
- [ ] Drag & drop (internal first; to/from other apps needs checking in Slint/winit).
- [ ] Rubber-band selection; view modes (icons, list, tiles) with thumbnails.
- [ ] Preview window (text, images, PSD, camera raw, JPEG 2000, Netpbm: port the readers).
- [ ] Search window; bookmarks; eject; volumes from the mount table (Java `Volumes`).
- [ ] Themes (Slint styles: fluent, cupertino, material, cosmic; light/dark).
- [ ] Packaging: `.desktop` install script, icons, Windows/macOS bundles; CI on all three OSes.
