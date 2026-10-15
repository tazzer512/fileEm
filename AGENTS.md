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
- Tests never start real programs. Surefire sets `-Dfileem.noLaunch=true`, which makes `Shell`
  (open, terminal, reveal) only record requests in `Shell.launched()`; assert on that. Watch for
  accidental double-clicks: a robot click right before a drag's press on the same item opens it
  (this once opened `drag.txt` in the user's editor on every test run).

## Shell commands: use rtk when available

[rtk](https://github.com/rtk-ai/rtk) is a CLI proxy that trims command output before it reaches the
agent's context (fewer tokens, same information). Use it where it helps, and fall back to the plain
command when it isn't installed:

- Check once per session: `command -v rtk`. If it's missing, run every command below without the
  `rtk` prefix. Never install it or change hooks yourself; never make the build or scripts depend on it.
- Where it pays off (noisy output): `rtk err mvn -q compile` (only errors/warnings),
  `rtk git status|diff|log`, `rtk grep`/`rtk find` for searches with many hits, `rtk ls`/`rtk tree`,
  `rtk log <file>`. Short, quiet commands gain nothing; plain is fine.
- Maven tests: **don't** use `rtk test mvn test` (it keeps only the last lines, which are test log
  noise, and drops the summary) or `rtk err` (a green run prints nothing). Filter instead; this
  works with or without rtk:
  `mvn test 2>&1 | grep -E "Tests run:|FAIL|ERROR\]|BUILD" | tail -15`
  and rerun the failing class plainly (`mvn test -Dtest=Name`) to read a failure in full.
- If filtered output hides something you need (a full stack trace, a complete diff before
  committing), rerun the plain command, or `rtk proxy <cmd>` for unfiltered output. `rtk recall` can
  show what a filter left out.
- A Claude Code `PreToolUse` hook (`rtk hook claude`) may already rewrite Bash commands to rtk; then
  plain commands are filtered automatically. `rtk gain` shows what it did.
- Test results are still reported faithfully: the rules under "Tests are mandatory" apply unchanged.

## Build, run, test

- JDK 27 (with JavaFX 27), Maven. If the default JDK is older, point `JAVA_HOME` at a 27 (e.g.
  sdkman's `27.0.0.0+35-tem`); an older JDK fails with "release version 27 not supported". JavaFX
  loads native libraries, so every launch passes `--enable-native-access` (surefire `argLine`,
  `javafx:run` options, jpackage `--java-options` for the `javafx.graphics` module; `PackagingTest`
  checks). `.mvn/jvm.config` silences Maven's own final-field-mutation warning. No module-info: the app runs from the classpath via `com.fileem.Launcher`
  (a non-`Application` main class, which avoids the "JavaFX runtime components are missing" check).
- `mvn -q compile`: must stay warning/error free.
- `mvn javafx:run`: starts the app. It is a GUI; when running it non-interactively wrap it in
  `timeout` so it does not block.
- `mvn -Pdist -DskipTests package`: native app via jpackage in `target/dist/app` (pom `dist`
  profile; it clears `target/dist` first, because jpackage refuses to overwrite an existing image and
  `install-linux.sh` would then install the stale one; `os-*` profiles pick the JavaFX platform jars and icon format). JavaFX is linked into the
  runtime as modules, everything else stays on the classpath. `dist.modules` lists the JDK modules:
  jdeps finds most, but add ones only used reflectively or by name (`jdk.charsets` for IBM437,
  `jdk.localedata`). `tools/install-linux.sh` installs it per user with a `.desktop` entry named
  after the window class (`com.fileem.FileEmApp`; renaming `FileEmApp` breaks the dock icon,
  `PackagingTest` checks). Icons: `tools/make-icon.py` writes `src/packaging/` and the window icons.
- `mvn test`: about 250 tests, a bit over a minute. UI tests run **headless on Monocle** (configured in
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
  SearchHistoryTest    newest first, no duplicates, last 15, survives a restart
  AppInfoTest          version.properties is filtered and matches the pom's app.version
  SessionTest          round trip, atomic + change-only writes, background vs. sync save order
  BookmarksTest
  PackagingTest        dist profile + install-linux.sh stay consistent (window class, icons, modules)
  ui/ThemeTest         theme sheets parse, both modes define all colors, live restyling
  ui/IconsTest         app icon sizes load; dialogs get the icon
  util/                NaturalOrderTest, FormatsTest (fixes Locale.US), FsUtilTest
  fs/                  FileEntryTest, FileTypesTest, DirWatcherTest,
                       PsdReaderTest (modes, depths, RLE/raw, PSB, JPEG fallback, damaged files),
                       RawReaderTest (CR2/NEF/RW2/RAF/CR3 previews, choice by size, orientation,
                       damaged files; RawFixtures builds the files),
                       ImageIoReaderTest (jp2 + bare codestream, resolution levels, shrinking,
                       8/16-bit grey, damaged files; OpenJPEG fixtures; Netpbm P1–P6, 16-bit,
                       hostile headers),
                       FileSearchTest (wildcards, terms and ! exclusions, recursion, hidden, links not followed, unreadable
                       folders, stopping),
                       PreviewTest (kinds by name, text vs binary, truncation, encodings),
                       ShellTest (terminal launch commands per platform), VolumesTest (listing,
                       eject commands, mount table parsing)
  ops/FileOperationTest  copy/move/delete/trash semantics + integrity on failure/cancel, pause/resume
  util/EtaTest         time-left estimate: warm-up, steady rate, speed changes, reset on pause, wording
  util/BgTest          virtual-thread I/O never blocked by stuck jobs, cancel drops results,
                       operations run in parallel on platform threads
  ui/MainWindowUiTest  TestFX end-to-end: keyboard, dialogs, dual pane, clipboard, DnD, session,
                       tree sync (current folder marks, folding branches it opened),
                       autosave, rubber-band selection, stacked progress window (pause/cancel),
                       eject (menu/button, tabs leave first, refused while busy, errors),
                       preview (toggle, image zoom/fit/pan, text, quick selection, PSD, raw upright,
                       JPEG 2000 + PPM preview and thumbnails,
                       docking, session), search (menus, Ctrl+Shift+F, results, open/show, hidden, history),
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
  `FileTab.table()/grid()`, `ProgressWindow.of(owner)/operations()/pauseButton(op)/cancelButton(op)/
  conflictButton(op, text)`). Add more of these
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
- `jai-imageio-jpeg2000` (+ `jai-imageio-core`): an ImageIO plugin for JPEG 2000 (pure Java, JJ2000
  and BSD-3 licenses), registered through `META-INF/services` on the classpath; needs only
  `java.desktop`. Its *writer* produces wrong colours: test files come from OpenJPEG (fixtures in
  `src/test/resources/com/fileem/fs/jpeg2000/`, made with Pillow).
- `ikonli-javafx` + `ikonli-materialdesign2-pack`. Icons are referenced by **string literal**
  (`"mdi2f-folder"`). An unknown literal throws at runtime, not at compile time, so verify new
  ones exist in the pack jar (e.g. `unzip` it and run `strings` on `MaterialDesign?.class`).
- Test only: JUnit 5, TestFX (`testfx-junit5`), `openjfx-monocle` (headless glass platform).
- Add no other dependencies without a good reason; AWT (`java.desktop`) is used only for
  `Desktop.open` / `moveToTrash`, always off the FX thread.

## Layout

```
com.fileem          Launcher, FileEmApp (stage), Session (properties file), Autosave,
                    Bookmarks (ordered named folders, saved on every change),
                    SearchHistory (last 15 search patterns),
                    AppInfo (version + build date from version.properties)
com.fileem.util     OsUtil, Bg (background executor → FX callbacks), Formats, FsUtil, NaturalOrder
com.fileem.fs       FileEntry (row snapshot), FileTypes, Volumes (mount polling), DirWatcher,
                    Trash, Shell (open with OS), Preview (kinds, text reading), PsdReader, RawReader,
                    ImageIoReader (JPEG 2000 via jai-imageio, Netpbm via NetpbmReader),
                    FileSearch (recursive name search with wildcards)
com.fileem.ops      FileOperation (copy/move/delete/trash Task), FileClipboard
com.fileem.ui       MainWindow (toolbar, F-key bar, ALL commands + key handling)
                    └ ExplorerSide ×2 (left/right): FolderTree + TabPane of FileTab
                       └ FileTab: AddressBar + view/sort MenuButtons + filter + FileTable + status line
                       └ FileTable + Marquee (Details view: rubber band, autoscroll)
                       └ FileGrid (the other ViewModes: virtualized grid on the table's models)
                         + Thumbnails (image thumbnail LRU)
                    OperationUi (starts operations, conflict dialog)
                    ProgressWindow (one per main window; a row per running op with Pause/Resume,
                      Cancel and inline conflict questions)
                    PreviewWindow (one per main window: previews the active tab's selection;
                      floats or docks into MainWindow's `outer` SplitPane), RawImages
                    SearchWindow (one per search: pattern, streaming results table)
                    PropertiesDialog, BookmarksDialog, Dialogs, Dnd, Theme, Icons, Views, TreeNode
src/main/resources/com/fileem/app.css   author stylesheet on top of AtlantaFX
src/main/resources-filtered/            resources Maven fills in (${...}): version.properties
                                        (app.version, build date); keep binaries out of it
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
- Cross-file-system moves make the copy **durable** before deleting the source. On Linux this is
  batched: finished copies queue their sources (`pendingSources`), and every 1000 files / 1 GB /
  15 s one `sync -f <target>` (syncfs) flushes the destination, then that batch's sources are
  deleted, folders after their contents (37× faster than an fsync per file for many small files).
  If the sync fails, each copy is fsynced on its own; a copy that can't be flushed keeps its
  source. Completed moves are flushed and their sources removed also when the operation is
  cancelled or fails (`work()`'s finally). Elsewhere (no `sync`): fsync per file. A source is
  deleted only after its copy succeeded; skipped or failed children keep their source folder.
  Tests force copy-style moves with `forceCopyMoves()` and stand in for syncfs with `syncer(..)`.
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
  that would land outside the target (zip-slip: `..`, absolute paths) or have names that aren't valid
  paths are skipped and reported. An entry is never written past its declared size (zip bomb: the
  free-space check trusts those sizes); one failing archive doesn't stop the others.
- Copying or moving a folder into itself is also refused when the target reaches it through a link
  (real-path check), otherwise the copy recurses into its own output.
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
- Never touch the file system on the FX thread except trivially cheap calls (rename, mkdir). That
  includes watcher registration (`DirWatcher.watch` runs in a `Bg` job) and existence checks in cells.
- `Volumes` polls space with a per-volume timeout; a hung mount keeps one stuck lookup (reused by
  later polls), never blocking the poller or piling up threads.
- Restore selections with one `selectIndices` call, never `select(i)` per item: every change re-runs
  the selection listeners (status line, grid), so per-item selects are quadratic.

## Conventions & invariants

- **Threading** (`Bg`): short blocking I/O (listings, tree loads, free space, launching programs)
  goes through `Bg.run(work, onSuccess, onError)` on **virtual threads** (`Bg.IO`, one per task, so
  a hung network mount never delays anything else); callbacks arrive on the FX thread. It returns a
  `Future`: cancel stale work (`FileTab.startLoad` cancels the previous listing) and call
  `Bg.checkInterrupted()` in long loops. Cancelled work gets no callback. File operations run on
  **platform threads** (`Bg.OPERATIONS`, one per operation, all in parallel): they are long and
  sit in blocking file I/O, which holds a virtual thread's carrier for the whole call (the JDK
  only compensates with extra carriers). Since JDK 24 `synchronized`/`wait` no longer pin virtual
  threads, so either style of locking is fine; the existing `java.util.concurrent` locks stay.
  Async loads also use a generation counter (`loadGeneration`, `revealToken`) to drop stale
  results: keep that pattern. A watcher refresh is skipped while a navigation is loading
  (`navigating`), or it would replace the navigation's generation and drop it. A navigation
  cancels a running refresh, which then never calls back, so `go` resets `refreshing` itself.
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
  `FileTab.navigateTo`. `FolderTree.currentItem()` is the item showing the tab's folder; cells mark
  it `:current` (boxed; themes give the box their selection's insets) and it and its parents
  `:on-path` (bold `.tree-name`), also while the tree has no focus. Branches `reveal` expanded are
  remembered (`autoExpanded`) and collapsed once the current folder leaves them; a branch the user
  expands or collapses is theirs and left alone. Reveal scrolls the item 3 rows below the top.
  Scrolling and folding happen only while the tree is `shown()` (window showing; it catches up when
  the window appears): in the hidden/detached windows earlier UI tests leave behind they re-measure
  cells outside a shown scene and log hundreds of CSS lookup warnings.
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
  dialog's own buttons (`answerDrop`): the F-key bar also has "Move" and "Copy". The question is
  asked through `MainWindow.afterDrag`: for our own drags only after DRAG_DONE reaches the source
  (2 s timeout). On Linux JavaFX runs our drags in a nested loop that keeps the mouse grab until then;
  a dialog opened earlier took Enter but no mouse clicks (Move "did nothing"). Keep `Dnd.canDrop` (check) separate from
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
  `columns.*`, `sort.*`, `dual`, `showHidden`, `theme.name`, `theme.dark`, `bookmarks.*`,
  `search.history.*`). `Autosave` collects
  the state every 2 s (`MainWindow.saveSession` + window geometry) and writes it in the background
  only when it changed; close, `Application.stop` and a JVM shutdown hook save too, so a kill or
  logout loses nothing. `Bookmarks` saves on every change. Writes are atomic (temp file + rename),
  and an older background write never replaces a newer save. New persisted state only needs to be
  put into the session from `saveSession` (or on change); don't add other save paths.
- **Operations UI**: start every operation through `OperationUi.run/start` (it registers it in
  `OperationUi.running()`). An operation still running after 350 ms gets a row in its owner's
  `ProgressWindow`; rows are removed when the operation ends and the window closes with the last
  one. Each row's Pause/Resume and Cancel act on that operation only; Escape is the cancel button
  only while one row is shown; closing the window cancels all. Each row's speed and time left come
  from `util/Eta` (sliding 10 s window, nothing before 2 s of measuring), sampled by the window's
  1 s ticker (`ProgressWindow.tick(now)`, driven with a fake clock in tests); the row resets its
  `Eta` while paused or asking, so waiting never reads as slow progress. Pause blocks the worker in
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
  Each tab's bar has a view and a sort `MenuButton` (`FileTab.viewButton()/sortButton()`) whose items
  come from `MainWindow.viewItems(tab)` / `sortItems(table)` (shared with the right-click menu;
  sort columns are `FileTable.SORT_COLUMNS`), rebuilt on every opening; their icons follow the view
  mode and the sort direction.
  Item size (slider, + / −, Ctrl++ / Ctrl+−): only `ViewMode.resizable()` views (icon views, Tiles).
  Sizes are fractional steps; each whole step scales the icon by 1.25, clamped to 16–320 px
  (`ViewMode.iconSize(steps)`, `minSteps/maxSteps`). The slider runs -1…1 with the view's default in
  the middle, each half mapped to its limit (`stepsForSlider` / `sliderForSteps`; snaps to 0 near the
  middle). The cell grows by the icon's change and keeps its text area. `FileTab` keeps steps per view
  (`<side>.tab.<i>.size` = `large-icons:2,tiles:-0.5`) and gives them to `FileGrid.setSizeSteps`,
  which rebuilds its cells only when the pixel size changes (cells take their icon size when created).
  Icons of 48 px and up show thumbnails. `Thumbnails.request(entry, px, callback)` loads at fixed
  sizes (`loadSize`: multiples of 32) so the slider doesn't re-decode per pixel; JavaFX formats use
  background loading, PSD/PSB (`fs/PsdReader`), camera raws (`fs/RawReader`, via `RawImages`) and JPEG 2000 / Netpbm
  (`fs/ImageIoReader`, `Preview.Kind.IMAGEIO`) go on two daemon threads (composite image,
  sampled rows only; falls back to the embedded JPEG; failures remembered per file version). Cells
  drop late callbacks with a request counter. The status line is an
  HBox (`.status-bar`: themes style its background there, text stays on `.status-label`).
- **Preview window** (`PreviewWindow`, F3 / toolbar toggle `#preview-toggle`, session `preview.*`):
  an owned, non-modal Stage. `FileTab` reports selection and focus changes to
  `MainWindow.selectionChanged`, which forwards the active tab's `previewEntry()` (focused item if
  selected, else the first selected); `tabChanged` does too. `select` waits `SETTLE` (120 ms) so arrow
  keys through a folder load only the last item, and every load bumps a generation and cancels the
  previous one (`Bg` future / `Image.cancel`). Kinds come from `fs/Preview` (FX-free:
  `kindOf`, `readText` with binary sniffing, UTF-8 then Latin-1, 256 KB). Images: the header size is
  read first (`imageSize`, ImageIO) because JavaFX *enlarges* to a requested size; decode at natural
  size or bounded to 4096 px with background loading. PSD: `PsdReader` at 2048 px. Camera raws:
  `RawReader` finds the embedded JPEGs (TIFF IFDs, sub-IFDs, RW2 tag 0x2E; CR3 PRVW box; RAF header),
  skips ones a decoder can't show (lossless SOF3 sensor data also starts with FFD8), and picks the
  smallest one at least the asked size (thumbnails) or the largest (preview); `RawImages` decodes and
  turns it upright per EXIF orientation (IFD0, CR3's CMT1, else the JPEG's own EXIF). Zoom = image scale
  or font scale; fit never enlarges; pan buttons move a quarter of the viewport. Showing it hands focus
  back to the main window. Closing it, F3 or Escape switches it off. JPEG 2000 (`ImageIoReader`, at
  2048 px like PSD): the COD marker gives the number of resolution levels; the smallest level at
  least the asked size is decoded (`J2KImageReadParam.setResolution`; ~0.3 s instead of ~4.5 s for a
  thumbnail of 6000×4000), then shrunk by area averaging. Grey images are read from the raster:
  `getRGB` treats grey as linear and would brighten it. Netpbm (`.ppm .pgm .pbm .pnm`) goes to our
  own `NetpbmReader` (P1–P6, 8/16 bits, whole-pixel steps then shrink, sides capped against damaged
  headers): jai-imageio's PNM reader reads 16-bit files as twice as many 8-bit pixels.
  **Docking** (`dockedProperty`, session `preview.docked`, divider `split.preview`): the same content
  node (`pane()`) moves between the window's scene (a placeholder stands in while docked) and the
  main window's `outer` SplitPane through `PreviewWindow.DockHost`; `place()` puts it where
  enabled/docked say. The host adds/removes only the pane: `setAll(sides, pane)` would detach and
  re-add the whole explorer (a full restyle, and stray CSS lookup warnings from detached nodes). The header has the dock toggle and, only when docked, a close button; the
  zoom/pan buttons are a FlowPane at the bottom so they wrap in a narrow pane. Split dividers are
  styled by class (`.sides-split`, `.main-split`), not by nesting. Tests: `preview()` hooks (`kind`,
  `image`, `textContent`, `messageText`, `button(id)`, `scrollPane`, `pane`, `dockedProperty`).
- **Search** (`SearchWindow`, *Search in folder…* in the file/tree/background menus, Ctrl+Shift+F;
  `MainWindow.search(dir)` / `searches()`): `FileSearch.matcher` splits the pattern at spaces/`;`
  (quotes keep spaces); a name matches any plain term and no `!` term. `fs/FileSearch` walks the tree without following links
  (a linked start folder is walked through its target, results keep the asked path), skips hidden
  items and hidden folders' contents unless asked, reports unreadable folders and keeps going, and
  stops when interrupted or `Listener.wantsMore()` says no (the window caps at `MAX_RESULTS`).
  The walk is a `Bg` job; results queue up and reach the table in batches (one pending
  `runLater` at a time); each run is an object, and only the current one may touch the table.
  Search, Stop, Escape and closing the window cancel the job. The pattern field is an editable
  `ComboBox` over `SearchHistory` (one per `MainWindow`, last 15 patterns newest first, session
  `search.history.<i>`, added on every search with something to look for; the list is refreshed on
  opening). Enter (editor key filter) and a click on a list cell search; not the ComboBox's action
  event, which fires while arrowing through the list and not for Enter on an unchanged value. Results are `FileEntry`s shown with
  `FileTable.NameCell`. Result commands (menu + keys in a table key filter) reuse `MainWindow`'s
  owner-taking versions, so dialogs and progress open over the search window:
  `delete(owner, paths, permanent)` returns the operation (results gone when it ends leave the list,
  checked with `Files.exists` in a `Bg` job), `rename(owner, src, onRenamed)` (results under a renamed
  folder are re-read at their new paths), `properties(owner, paths)`, `clipboardPut`. Folders open in the active tab (`FileTab.showItem` opens a result's folder
  with it selected), files with `Shell.open`.
- **Version** (`AppInfo`, About box from the toolbar's `#about-button`, `MainWindow.showAbout`):
  one number, `app.version` in the pom, used by jpackage and filled into `version.properties`.
  Bump it there for a release; `AppInfoTest` checks they agree.
- **Mnemonics**: `Button`/`MenuItem` treat `_` as a mnemonic marker and hide it. Anything showing a
  file name (address bar crumbs, `MainWindow.item`) sets `setMnemonicParsing(false)`.
- **Rubber band** (`Marquee` in Details, built into `FileGrid` in the other views): event filters
  on the table. A press on a row's name (icon + text, plus a few pixels of slack) drags files; anywhere else (empty space, other columns) starts the box. Rows are tracked in
  content coordinates (index × row height) so autoscroll keeps the anchor. Shift adds, Ctrl/Cmd
  toggles.
- **Eject** (`Volumes.eject`, `MainWindow.eject`): `Volume.ejectable` marks drives people plug in
  (Linux: on a USB/MMC bus per sysfs, or mounted by the desktop under `/media`, `/run/media`; not
  internal disks under `/mnt`. Windows: `volume:isRemovable`/`isCdrom`. macOS: `/Volumes`). The
  tree shows an eject button and an *Eject* menu item for them (`TreeNode.ejectable`). The whole
  drive goes, like Windows/macOS: `Volumes.sameDrive` finds its other mounted partitions (sysfs
  parent disk). Ejecting is refused while a running operation uses any of them, moves tabs on them
  to home first and waits for them to leave (open folders and watchers keep a drive busy), then
  runs the blocking eject on `Bg`. Linux: `udisksctl unmount` each partition + `power-off` the
  disk (a failed power-off still counts: the data is safe), else `gio mount --eject`, else
  `umount`. Tests swap the real eject with `MainWindow.setEjector` (`drive` + `eject`) and show
  fake drives via `FolderTree.syncVolumes`; never eject for real.
- **Bookmarks**: one `Bookmarks` model per `MainWindow`, shared by both trees. The tree's
  Bookmarks section (`TreeNode.Kind.BOOKMARK_HEADER` / `BOOKMARK`) mirrors it via `syncBookmarks`
  and is hidden while empty. Bookmark items are lazy `FolderItem`s like volumes, but `reveal`
  only walks volumes. Mutate through `Bookmarks` (its list is read-only).
- Code style: 4-space indent, braces always, `final` classes, package-private UI internals, short
  Javadoc on non-obvious classes/methods, no wildcard imports.

## Diagnosing "stuck" operations

When a copy or move seems to hang, check the device before the app: take a `jstack` of the
running JVM and look at the operation thread's state, its kernel wait channel
(`/proc/<pid>/task/<tid>/wchan`), `/proc/diskstats` progress, and `journalctl -k`. A failing USB
drive or UAS adapter (`uas_eh_abort_handler` / device resets in the kernel log) blocks the thread
inside a kernel read; Pause and Cancel only take effect once that read returns. This happened
once with an ASMedia `174c:55aa` adapter (fix: the `usb-storage quirks=174c:55aa:u` option to disable UAS).

## Known gaps / ideas

The suite has only run on Linux; the macOS/Windows paths (Trash via AWT, drive labels, junctions,
Cmd shortcuts) are untested there. Packaging is only tried on Linux as an app-image (+
`install-linux.sh`); the macOS/Windows images and the `deb`/`rpm`/`dmg`/`msi` installer types are
untested, and on macOS the Dock icon only comes from the packaged `.app`. Possible next steps: CI on
all three OSes (tests + `-Pdist`), archive browsing, folder-size column.
