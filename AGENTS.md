# AGENTS.md

Guidance for coding agents working on fileEm, a cross-platform (Linux, macOS, Windows) JavaFX file
explorer with an Explorer-style tree + detail list and an optional Commander-style second side.
User-facing features and shortcuts are in `README.md`.

## Tests are mandatory

The test suite is how this project stays working through changes, refactorings and new features.
Treat it as part of every change, not an afterthought:

- **Run `mvn test` before and after every change** and report the result. A change is not done
  while any test fails; never delete, skip, weaken or `@Disabled` a test to get green. If a test
  is genuinely wrong, fix the test and say why.
- **Every new feature or behavior gets tests** in the same change: unit tests for logic in
  `util`/`fs`/`ops`, and a `MainWindowUiTest` case for anything reachable from the UI (shortcut,
  menu, dialog, drag & drop).
- **Every bug fix gets a regression test** that fails without the fix.
- **Keep tests in sync when refactoring**: update tests that reference renamed or moved code, and
  keep covering the same behavior.
- **File operations (`FileOperation`, `Trash`, rename, new folder) need integrity tests** for
  the failure paths too: originals intact after failure or cancel, no `.fileem-*.part` leftovers,
  links not followed, and data never deleted before it's safely copied.
- Tests use `@TempDir` only. Never touch real user folders or the real Trash (only test Trash
  with paths that don't exist), and never pass a real file system root to a delete or move.

## Build, run, test

- JDK 21, Maven. No module-info: the app runs from the classpath via `com.fileem.Launcher`
  (a non-`Application` main class, which avoids the "JavaFX runtime components are missing" check).
- `mvn -q compile`: must stay warning/error free.
- `mvn javafx:run`: starts the app. It is a GUI; when running it non-interactively wrap it in
  `timeout` so it does not block.
- `mvn test`: about 80 tests, around a minute. UI tests run **headless on Monocle** (configured in
  the pom's `headless-ui-tests` profile), so no display is needed.
  - `mvn test -Dtest=FileOperationTest`: a single class. `-Dtest='!MainWindowUiTest'` skips
    the slow UI tests while iterating, but run the full suite before finishing.
  - `mvn test -Dheadless=false`: watch the UI tests on a real display.
- Visual checks: the dev machine runs Wayland, so desktop screenshots don't work. Render the scene
  instead (`scene.snapshot(null)` → PNG via `ImageIO`) from a scratch harness that builds
  `new MainWindow(stage, Session.load(tempFile))`.

### Test layout & conventions

```
src/test/java/com/fileem/
  FxTestSupport        starts the FX toolkit once; drainFx() waits for queued FX events
  SessionTest, BookmarksTest
  util/                NaturalOrderTest, FormatsTest (fixes Locale.US), FsUtilTest
  fs/                  FileEntryTest, FileTypesTest, VolumesTest, DirWatcherTest
  ops/FileOperationTest  copy/move/delete/trash semantics + integrity on failure/cancel
  ui/MainWindowUiTest  TestFX end-to-end: keyboard, dialogs, dual pane, clipboard, DnD, session
```

- `FileOperation` is a JavaFX `Task`: call `op.run()` synchronously on the test thread, but read
  its properties (`getWorkDone`, `getException`, …) **on the FX thread** after draining events
  (`onFx(...)` helper in the test).
- UI tests: build the window in `@Start` with a temp-file `Session`; read FX state via the `fx(...)`
  helper and wait with `waitUntil(...)`. Never `Thread.sleep` for UI state.
- Monocle quirk: robot key events go to the main window, not to an open dialog. Fill dialogs with
  `answerDialog(robot, text)` (replaces the field's selection, clicks OK) and close them by
  clicking button text.
- Known noise: across UI tests, JavaFX may log `ClassCastException ... while converting value for
  '-fx-background-color'` for tree arrows. It comes from VirtualFlow's detached measuring cell in
  hidden windows left over from earlier tests, not from the app; ignore it, but don't add new
  warnings.
- Platform-specific tests use `assumeTrue(OsUtil.isPosix())` / `isWindows()` so they skip, not
  fail, elsewhere.
- UI test hooks are package-private accessors (`MainWindow.left()/right()/isDual()/activeSide()`,
  `FileTab.table()`). Add more of these rather than reaching in with reflection.

## Dependencies

- `atlantafx-base` (Primer light/dark user-agent stylesheet, `Styles.*` classes, `CustomTextField`).
- `ikonli-javafx` + `ikonli-materialdesign2-pack`. Icons are referenced by **string literal**
  (`"mdi2f-folder"`). An unknown literal throws at runtime, not at compile time, so verify new
  ones exist in the pack jar (e.g. `unzip` it and run `strings` on `MaterialDesign?.class`).
- Test only: JUnit 5, TestFX (`testfx-junit5`), `openjfx-monocle` (headless glass platform).
- Add no other dependencies without a good reason; AWT (`java.desktop`) is used only for
  `Desktop.open` / `moveToTrash`, always off the FX thread.

## Layout

```
com.fileem          Launcher, FileEmApp (stage, session save), Session (properties file),
                    Bookmarks (ordered named folders, saved on every change)
com.fileem.util     OsUtil, Bg (background executor → FX callbacks), Formats, FsUtil, NaturalOrder
com.fileem.fs       FileEntry (row snapshot), FileTypes, Volumes (mount polling), DirWatcher,
                    Trash, Shell (open with OS)
com.fileem.ops      FileOperation (copy/move/delete/trash Task), FileClipboard
com.fileem.ui       MainWindow (toolbar, F-key bar, ALL commands + key handling)
                    └ ExplorerSide ×2 (left/right): FolderTree + TabPane of FileTab
                       └ FileTab: AddressBar + filter + FileTable + status line
                    OperationUi (progress window, conflict dialog), PropertiesDialog, BookmarksDialog,
                    Dialogs,
                    Dnd, Theme, Icons, Views, TreeNode
src/main/resources/com/fileem/app.css   author stylesheet on top of AtlantaFX
```

## File operation integrity (do not regress)

`FileOperation` guarantees these; each one has a test in `FileOperationTest`:

- Files are copied to a hidden temp sibling (`.fileem-<random>.part`), **size-verified**, then
  renamed into place (atomic replace on overwrite). An existing target is only replaced by a
  complete copy; failure or cancel leaves the original intact and removes the temp file.
- Cross-file-system moves **fsync** the copy before deleting the source. A source is deleted only
  after its copy succeeded; skipped or failed children keep their source folder.
- Same-file-system moves are renames (no data copied). A failed rename falls back to copying only
  for errors where that makes sense, and never silently overwrites.
- Symlinks are copied/deleted as links, never followed. Windows junctions are never descended
  into (delete removes only the junction; copy skips it). Special files (FIFOs, devices) are
  skipped with an error instead of blocking.
- "Replace" moves the displaced item aside first and restores it if the replacement fails.
- Moving an item onto itself (through a link, bind mount or hard link) is refused; copying onto
  itself creates "name - Copy". Folders can't be copied or moved into themselves. Drive/file
  system roots are refused for move, delete and trash.
- Free space is checked before a copy starts. Per-item failures are collected in `errors()` and
  processing continues.

## Performance notes (keep it fast)

- Listing reads one attribute call per entry. The owner name (`getpwuid` per file, about 80% of
  listing time) is **lazy**: `FileEntry.attributes()` is built only when the Attributes column or
  Properties needs it. Don't add eager per-file lookups to `FileEntry.read`.
- Directory watcher refreshes go through `FileTab.requestRefresh()`, which coalesces bursts and
  throttles to 2× the last listing time. The tree reloads a folder only when its set of
  subfolders changed.
- `FileOperation` caches `FileStore` lookups (each one parses the mount table on Linux), uses
  `Files.copy` for files ≤ 8 MB and chunked `FileChannel.transferTo` above that (kernel copy,
  cancellable, with progress). Linux trash is batched (one `gio trash` per 100 items).
- Never touch the file system on the FX thread except trivially cheap calls (rename, mkdir).

## Conventions & invariants

- **Threading**: use `Bg.run(work, onSuccess, onError)`; callbacks arrive on the FX thread. Async
  loads use a generation counter (`loadGeneration`, `revealToken`) to drop stale results: keep
  that pattern.
- **Commands live in `MainWindow`** and act on the *active* side's active tab. Keyboard
  shortcuts are a single scene-level `KEY_PRESSED` **filter** (`MainWindow.onKey`), which skips
  most keys while a `TextInputControl` has focus. Add new shortcuts there, add them to the README
  table, and add a UI test for them.
- **Sorting**: `FileTable` items are `SortedList(FilteredList(master))`; the SortedList
  comparator is bound to `FileTable.effectiveComparator()` (folders first → user column →
  natural name) and the table's sort policy is `t -> true`. Don't bind it to
  `table.comparatorProperty()` directly or folders-first breaks.
- **Hidden files** stay in `master` and are filtered by predicate, so toggling them needs no reload.
- **FolderTree**: children load lazily (`FolderItem.load/reload`) and are updated with
  `syncChildren` (minimal diff) so expansion and selection survive refreshes. Any programmatic
  selection/children change must be wrapped in `programmatic++ / --`, otherwise the selection
  listener navigates the active tab.
- **Tree ↔ tab sync**: the tab calls `ExplorerSide.tabNavigated` after a successful load, and the
  tree `reveal`s the path (no-op if the selected item already shows it). Tree clicks call
  `FileTab.navigateTo`.
- **Refreshing**: `DirWatcher` refreshes tabs. After our own operations
  `MainWindow.refreshDirs` refreshes explicitly too, because macOS watching is slow.
- **Drag & drop**: drops from other applications accept only COPY (a MOVE could let the source
  delete originals before our async copy finishes). Keep `Dnd.canDrop` (check) separate from
  `Dnd.accept` (check + accept modes): row handlers must not consume drops they can't handle.
- **Cross-platform**: branch with `OsUtil.isWindows/isMac/isLinux`. Use `isShortcutDown()` for
  Ctrl/Cmd. Paths: compare with `Path.equals`/`startsWith` (case-insensitive on Windows), never
  string ops. Config dir comes from `OsUtil.configDir()`.
- **Styling**: use AtlantaFX looked-up colors (`-color-fg-muted`, `-color-accent-emphasis`, …) in
  `app.css` so light and dark themes both work. Icon colors are CSS classes (`icon-folder`, …)
  applied via `Icons.apply`, not `setIconColor`. Dialogs/stages must get `Theme.APP_CSS`
  (`Dialogs.prepare`).
- **Session**: everything persisted goes through `Session` keys (`window.*`, `left.*`/`right.*`,
  `columns.*`, `sort.*`, `dual`, `showHidden`, `theme.dark`, `bookmarks.*`). Saved on window
  close, except `Bookmarks`, which saves the session file on every change.
- **Bookmarks**: one `Bookmarks` model per `MainWindow`, shared by both trees. The tree's
  Bookmarks section (`TreeNode.Kind.BOOKMARK_HEADER` / `BOOKMARK`) mirrors it via `syncBookmarks`
  and is hidden while empty. Bookmark items are lazy `FolderItem`s like volumes, but `reveal`
  only walks volumes. Mutate through `Bookmarks` (its list is read-only).
- Code style: 4-space indent, braces always, `final` classes, package-private UI internals, short
  Javadoc on non-obvious classes/methods, no wildcard imports.

## Known gaps / ideas

The suite has only run on Linux; the macOS/Windows paths (Trash via AWT, drive labels, junctions,
Cmd shortcuts) are untested there. Possible next steps: CI on all three OSes, jpackage installers,
archive browsing, folder-size column.
