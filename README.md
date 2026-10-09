# fileEm

A cross-platform (Linux, macOS, Windows) file explorer in JavaFX: Explorer-style folder tree and
detail view, with an optional second side for Norton Commander / Directory Opus style copy and move.

## Run

Requires JDK 21+ and Maven.

```sh
mvn javafx:run
```

## Test

```sh
mvn test                    # unit + end-to-end UI tests, headless (no display needed)
mvn test -Dheadless=false   # watch the UI tests run on screen
```

## Features

- **Folder tree**: Quick access (Home, Desktop, Documents, … including localized XDG folders on Linux)
  and all volumes/mounts with free space. Removable drives show up automatically (polled every 5 s).
- **Bookmarks**: Ctrl+D (or the bookmark button) bookmarks the current folder; bookmarks appear
  in their own tree section on both sides. Right-click a bookmark to rename, reorder or remove it,
  or use *Manage bookmarks* (Ctrl+Shift+B). Bookmarks are saved immediately.
- **File list**: Name, Size, Type, Date modified by default; Date created, Date accessed, Extension and
  Attributes/permissions can be turned on from the `⋮` header menu. Click headers to sort (folders stay
  on top, natural "file2 < file10" order). Column widths, order, visibility and sort are remembered.
- **Tabs** on each side, **dual pane** (F9), breadcrumb address bar (click or Ctrl+L to type a path),
  live filter box with `*` / `?` wildcards, type-ahead find, auto-refresh when folder contents change.
- **Operations** run in the background with a progress window and cancel: copy, move, delete to
  Trash, permanent delete, rename, new folder. Conflicts ask Replace / Skip / Keep both / Cancel
  (optionally for all).
- **Safe file operations**: copies are written to a temp file, size-checked, then renamed into
  place, so a failed or cancelled copy never damages an existing file. Moves between drives are
  flushed to disk before the source is deleted. Links are never followed, special files are
  skipped, and free space is checked up front.
- **Drag & drop** between sides, tabs, the tree and other applications. Inside fileEm the OS modifier
  keys choose copy or move; drops from other applications are always copied.
- **Clipboard** uses the OS clipboard, so files can be copied/pasted to and from Explorer, Finder or
  Nautilus.
- **Properties** dialog with background folder-size calculation.
- Light/dark theme (AtlantaFX), session restore (window, tabs, dual mode, hidden files, columns).

## Keyboard

| Key | Action |
| --- | --- |
| Enter / double-click | Open folder or file (default app) |
| Backspace, Alt+Up | Up one folder |
| Alt+Left / Alt+Right | Back / Forward |
| Tab | Switch side (dual pane) |
| F2 | Rename |
| F5 / F6 | Copy / Move selection to the other side (target is editable) |
| F7, Ctrl+Shift+N | New folder |
| Del, F8 | Move to Trash |
| Shift+Del | Delete permanently |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / Cut / Paste |
| Ctrl+A | Select all |
| Alt+Enter | Properties |
| Ctrl+T / Ctrl+W | New tab / Close tab |
| Ctrl+L | Edit address |
| Ctrl+F | Filter |
| Ctrl+H (Cmd+Shift+. on macOS) | Show hidden files |
| Ctrl+R | Refresh |
| Ctrl+D | Bookmark / un-bookmark the current folder |
| Ctrl+Shift+B | Manage bookmarks |
| F9 | Toggle dual pane |

On macOS, Ctrl is Cmd; Cmd+Backspace deletes and Cmd+Up/Down go up/open.

## Settings location

- Linux: `$XDG_CONFIG_HOME/fileem/session.properties` (default `~/.config/fileem`)
- macOS: `~/Library/Application Support/fileEm/session.properties`
- Windows: `%APPDATA%\fileEm\session.properties`
