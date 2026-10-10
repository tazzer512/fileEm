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
- `mvn -Pdist -DskipTests package`: native app via jpackage in `target/dist/app` (pom `dist`
  profile; `os-*` profiles pick the JavaFX platform jars and icon format). JavaFX is linked into the
  runtime as modules, everything else stays on the classpath. `dist.modules` lists the JDK modules:
  jdeps finds most, but add ones only used reflectively or by name (`jdk.charsets` for IBM437,
  `jdk.localedata`). `tools/install-linux.sh` installs it per user with a `.desktop` entry named
  after the window class (`com.fileem.FileEmApp`; renaming `FileEmApp` breaks the dock icon,
  `PackagingTest` checks). Icons: `tools/make-icon.py` writes `src/packaging/` and the window icons.
- `mvn test`: about 160 tests, a bit over a minute. UI tests run **headless on Monocle** (configured in
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
  SessionTest          round trip, atomic + change-only writes, background vs. sync save order
  BookmarksTest
  PackagingTest        dist profile + install-linux.sh stay consistent (window class, icons, modules)
  ui/ThemeTest         theme sheets parse, both modes define all colors, live restyling
  ui/IconsTest         app icon sizes load; dialogs get the icon
  util/                NaturalOrderTest, FormatsTest (fixes Locale.US), FsUtilTest
  fs/                  FileEntryTest, FileTypesTest, VolumesTest, DirWatcherTest
  ops/FileOperationTest  copy/move/delete/trash semantics + integrity on failure/cancel, pause/resume
  util/BgTest          virtual-thread I/O never blocked by stuck jobs, cancel drops results,
                       operations run in parallel on platform threads
  ui/MainWindowUiTest  TestFX end-to-end: keyboard, dialogs, dual pane, clipboard, DnD, session,
                       autosave, rubber-band selection, stacked progress window (pause/cancel),
                       view modes (menu, keys, wheel, grid mouse/keys/box/DnD/menus, thumbnails)
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
- Context menus: Monocle never turns a robot right-click into a menu request. Open menus in tests
  with `requestContextMenu(robot, node)` (fires `CONTEXT_MENU_REQUESTED`) and check
  `getContextMenu().isShowing()`; test menu contents via `fileMenuItems` / `treeMenuItems` +
  `findMenuItem`/`hasItem`/`hasCommand`.
- Platform-specific tests use `assumeTrue(OsUtil.isPosix())` / `isWindows()` so they skip, not
  fail, elsewhere.
- UI test hooks are package-private accessors (`MainWindow.left()/right()/isDual()/activeSide()`,
  `FileTab.table()`, `ProgressWindow.of(owner)/operations()/cancelButton(op)`). Add more of these
  rather than reaching in with reflection.
- Long-running operations in UI tests: build a `FileOperation` whose `ConflictHandler` blocks on a
  latch (copy onto an existing name) and start it with `OperationUi.start(...)`; it stays running
  until the latch opens or it is cancelled. No big files or sleeps needed.
- Monocle quirk: a mouse drag that isn't a drag-and-drop (rubber band) leaves Monocle's
  `MouseInput.dragView` set and breaks the next test's DnD; `MainWindowUiTest`'s `@AfterEach`
  resets it. Keep that reset.
- Known flake: `rightClickInTheTreeShowsAMenuWithoutNavigating` has failed once on the outline
  check and passed on rerun. Rerun before digging in; fix it if it fails again.
- Grid UI tests: switch with `tab.setViewMode(..)`, click items at `FileGrid.screenCenterOf(i)`,
  read cells via `cellFor(i)` / `visibleIndices()` / `lanes()`.
- `Autosave` and the shutdown hook are only started by `FileEmApp`; tests drive
  `Autosave.saveInBackground()` directly with a temp `Session`. Never let a test write
  `OsUtil.configDir()`.

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
com.fileem          Launcher, FileEmApp (stage), Session (properties file), Autosave,
                    Bookmarks (ordered named folders, saved on every change)
com.fileem.util     OsUtil, Bg (background executor → FX callbacks), Formats, FsUtil, NaturalOrder
com.fileem.fs       FileEntry (row snapshot), FileTypes, Volumes (mount polling), DirWatcher,
                    Trash, Shell (open with OS)
com.fileem.ops      FileOperation (copy/move/delete/trash Task), FileClipboard
com.fileem.ui       MainWindow (toolbar, F-key bar, ALL commands + key handling)
                    └ ExplorerSide ×2 (left/right): FolderTree + TabPane of FileTab
                       └ FileTab: AddressBar + filter + FileTable + status line
                       └ FileTable + Marquee (Details view: rubber band, autoscroll)
                       └ FileGrid (the other ViewModes: virtualized grid on the table's models)
                         + Thumbnails (image thumbnail LRU)
                    OperationUi (starts operations, conflict dialog)
                    ProgressWindow (one per main window; a row with its own Cancel per running op)
                    PropertiesDialog, BookmarksDialog, Dialogs, Dnd, Theme, Icons, Views, TreeNode
src/main/resources/com/fileem/app.css   author stylesheet on top of AtlantaFX
src/main/resources/com/fileem/themes/   theme overlays: <id>.css shapes, <id>-light/-dark.css colors
src/main/resources/com/fileem/icons/    app icon PNGs (16–512 px); regenerate with tools/make-icon.py (Pillow)
src/packaging/                          jpackage icons (png Linux, ico Windows, icns macOS)
tools/                                  make-icon.py, install-linux.sh (per-user install + .desktop)
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
- COMPRESS writes the archive to a temp file and renames it into place; links, junctions and
  special files are left out (reported). EXTRACT unpacks into a temp folder, then renames it into
  place (never merging into an existing folder; a single top-level item is not nested). Entries
  that would land outside the target (zip-slip: `..`, absolute paths) are skipped and reported.
- Free space is checked before a copy or extract starts. Per-item failures are collected in `errors()` and
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

- **Threading** (`Bg`): short blocking I/O (listings, tree loads, free space, launching programs)
  goes through `Bg.run(work, onSuccess, onError)` on **virtual threads** (`Bg.IO`, one per task, so
  a hung network mount never delays anything else); callbacks arrive on the FX thread. It returns a
  `Future`: cancel stale work (`FileTab.startLoad` cancels the previous listing) and call
  `Bg.checkInterrupted()` in long loops. Cancelled work gets no callback. File operations run on
  **platform threads** (`Bg.OPERATIONS`, one per operation, all in parallel): they are long and
  sit in blocking file I/O, which pins virtual threads in JDK 21. Inside code that may run on a
  virtual thread, block with `java.util.concurrent` locks, not `synchronized`/`wait` (pinning).
  Async loads also use a generation counter (`loadGeneration`, `revealToken`) to drop stale
  results: keep that pattern. A watcher refresh is skipped while a navigation is loading
  (`navigating`), or it would replace the navigation's generation and drop it.
- **No file system calls on the FX thread** in navigation paths, not even `Files.isRegularFile`:
  on a dead network mount they hang the UI. Do them in the `Bg.run` job (see `FileTab.go`).
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
- **Context menus** are built per request with `Views.dynamicContextMenu(control, supplier)`. Never
  fill a `ContextMenu` in `onShowing`: `ContextMenu.show` silently ignores a menu that is still
  empty, so it would never open (this bug shipped once). Menu contents live in
  `MainWindow.fileMenuItems` / `treeMenuItems`; commands there take explicit paths so the tree and
  the list share them. Quick access, bookmark and volume tree items are shortcuts: no cut, rename,
  delete or duplicate. Right-click in the tree must not select (selection navigates).
- **Drag & drop**: every drop goes through `MainWindow.drop` → `moveOrCopy`, which asks Move
  (default) / Copy / Cancel after the drag ended (never show a dialog inside the drop event).
  Internal drags are accepted as MOVE; drops from other applications are accepted as COPY even
  when the user picks Move (our MOVE deletes sources itself, after copying; reporting MOVE could
  let the source delete originals before our async copy finishes). In tests, answer with the
  dialog's own buttons (`answerDrop`): the F-key bar also has "Move" and "Copy". Keep `Dnd.canDrop` (check) separate from
  `Dnd.accept` (check + accept modes): row handlers must not consume drops they can't handle.
- **Cross-platform**: branch with `OsUtil.isWindows/isMac/isLinux`. Use `isShortcutDown()` for
  Ctrl/Cmd. Paths: compare with `Path.equals`/`startsWith` (case-insensitive on Windows), never
  string ops. Config dir comes from `OsUtil.configDir()`.
- **Styling**: use AtlantaFX looked-up colors (`-color-fg-muted`, `-color-accent-emphasis`, …) in
  `app.css` so every theme and mode works; never hard-code colors there. Icon colors are CSS
  classes (`icon-folder`, …) applied via `Icons.apply`, not `setIconColor`; their colors are
  looked-up colors (`icon-folder: …` on `.root`) that themes may retint. New scenes and dialogs
  must call `Theme.style(stylesheets)` (`Dialogs.prepare` does), never add `Theme.APP_CSS` alone.
- **Themes** (`Theme.Family`: Classic, Windows 11, macOS, Adwaita, Aurora; each light + dark): an
  AtlantaFX user-agent stylesheet plus, except Classic, `themes/<id>.css` (shapes) and
  `themes/<id>-light.css` / `-dark.css` (colors: AtlantaFX `-color-*` plus the theme's own `-fe-*`
  looked-up colors). `Theme.apply` swaps the sheets in every open window; they always sit right
  after `app.css`. Every `-fe-*` color a shape file uses must be defined in both mode files
  (`ThemeTest` checks). Keep effects (shadows, blur) off rows and cells: they are re-rendered while
  scrolling. Check new styling with the scene-snapshot harness in light and dark.
- **Session**: everything persisted goes through `Session` keys (`window.*`, `left.*`/`right.*`,
  `columns.*`, `sort.*`, `dual`, `showHidden`, `theme.name`, `theme.dark`, `bookmarks.*`). `Autosave` collects
  the state every 2 s (`MainWindow.saveSession` + window geometry) and writes it in the background
  only when it changed; close, `Application.stop` and a JVM shutdown hook save too, so a kill or
  logout loses nothing. `Bookmarks` saves on every change. Writes are atomic (temp file + rename),
  and an older background write never replaces a newer save. New persisted state only needs to be
  put into the session from `saveSession` (or on change); don't add other save paths.
- **Operations UI**: start every operation through `OperationUi.run/start` (it registers it in
  `OperationUi.running()`). An operation still running after 350 ms gets a row in its owner's
  `ProgressWindow`; rows are removed when the operation ends and the window closes with the last
  one. Each row's Pause/Resume and Cancel act on that operation only; Escape is the cancel button
  only while one row is shown; closing the window cancels all. Pause blocks the worker in
  `FileOperation.checkCancel` (every item and chunk), so new loops must call `checkCancel()`.
  **Conflicts are asked inline** in the operation's row (`OperationUi.askInProgressRow` →
  `ProgressWindow.ask`), never with a modal dialog: a modal `showAndWait` blocks the whole app, and
  with several operations the nested event loops can only return last-opened-first, so answering
  the first question appears to hang. The operation's thread reads the file details and waits on a
  `CompletableFuture`; the FX thread never blocks. Don't add modal dialogs to operation flows.
  Closing the main window with operations running asks, then `OperationUi.cancelAllAndWait`
  (waits on `FileOperation.awaitStopped`, i.e. until cleanup is done) before exiting; a shutdown
  hook does the same for 3 s on SIGTERM/logout.
- **View modes** (`ViewMode`, Explorer's eight, per tab, saved as `<side>.tab.<i>.view`): Details is
  `FileTable`; the others are one `FileGrid` per tab (created on first use). The grid has no model
  of its own: it draws `table.getItems()` and uses the table's selection and focus models, so
  commands, status, restoreSelection and sorting need no view-specific code. Go through
  `FileTab.view()`, `ensureVisible`, `focusTable` and `typeAhead` rather than the table directly.
  `FileGrid` is virtualized (fixed slots, cells reused, unmanaged so the layer pane doesn't resize
  them); keep per-layout work O(visible items) (e.g. List's column width is cached per items change).
  Thumbnails (`Thumbnails`) load with JavaFX background loading into a pixel-bounded LRU cache.
- **Rubber band** (`Marquee`): event filters on the table. A press on a row's name (icon + text)
  drags files; anywhere else (empty space, other columns) starts the box. Rows are tracked in
  content coordinates (index × row height) so autoscroll keeps the anchor. Shift adds, Ctrl/Cmd
  toggles.
- **Bookmarks**: one `Bookmarks` model per `MainWindow`, shared by both trees. The tree's
  Bookmarks section (`TreeNode.Kind.BOOKMARK_HEADER` / `BOOKMARK`) mirrors it via `syncBookmarks`
  and is hidden while empty. Bookmark items are lazy `FolderItem`s like volumes, but `reveal`
  only walks volumes. Mutate through `Bookmarks` (its list is read-only).
- Code style: 4-space indent, braces always, `final` classes, package-private UI internals, short
  Javadoc on non-obvious classes/methods, no wildcard imports.

## Known gaps / ideas

The suite has only run on Linux; the macOS/Windows paths (Trash via AWT, drive labels, junctions,
Cmd shortcuts) are untested there. Packaging is only tried on Linux as an app-image (+
`install-linux.sh`); the macOS/Windows images and the `deb`/`rpm`/`dmg`/`msi` installer types are
untested, and on macOS the Dock icon only comes from the packaged `.app`. Possible next steps: CI on
all three OSes (tests + `-Pdist`), archive browsing, folder-size column.
