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
- Visual checks without a display: `cargo test -p fileem --test snapshots -- --ignored` renders
  the main window, a dialog and the progress window headless to `target/snapshots/*.ppm`
  (convert with Pillow to look at them). Extend it when adding windows or dialogs.
- Visual checks of the real app (Wayland has no desktop screenshots): `FILEEM_SNAPSHOT=out.ppm cargo run -p fileem
  FOLDER` renders the window after the folder loaded, writes it as PPM and quits. Don't type while
  it runs: the window takes keyboard focus as it opens and your keys reach it.

The test rules of `AGENTS.md` apply here too: every feature gets tests, every bug fix a
regression test, file operations need integrity tests, and nothing touches real user folders,
the real Trash or launches programs (`FILEEM_NO_LAUNCH` records opens in `shell::launched()`).

## Layout

```
Cargo.toml                 workspace (release: LTO, stripped, panic=abort)
crates/fileem-core/        no UI: everything that touches the file system
  src/util/                natural_order, formats, fs_util, os, eta       (Java util)
  src/fs/                  entry (FileEntry), file_types, listing, places,
                           trash, watcher                                 (Java fs)
  src/ops/                 file_operation (+ tests.rs: port of FileOperationTest), names
                           (rename / create, never replacing)
  src/session.rs           session.properties shared with the Java app (Java Session)
crates/fileem/             the app
  ui/app.slint             window layout and drawing only
  src/app.rs               controller: callbacks, keys, models (Java MainWindow)
  src/tab.rs, tree.rs      tab and tree state, toolkit-free and unit-tested
  src/bg.rs                background jobs with results on the UI thread (Java Bg)
  src/progress.rs          progress rows: detail line, estimate, conflict question (toolkit-free)
  src/ops_ui.rs            runs operations with a progress window row (Java OperationUi)
  ui/progress.slint        the progress window
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

Done, next stages, decisions and lessons learned: see `MIGRATION.md`.
