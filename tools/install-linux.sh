#!/usr/bin/env bash
# Installs the jpackage build of fileEm for the current user (no root needed): copies the app to
# ~/.local/share/fileem/app, adds the icon and an app launcher entry, so fileEm shows up in the
# app grid / menu and the dock shows its icon.
#
#   mvn -Pdist -DskipTests package && tools/install-linux.sh
#   tools/install-linux.sh --uninstall
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
app_dir="$data/fileem/app"
# The desktop file is named after the window class (WM_CLASS) JavaFX gives the main window, and
# says so in StartupWMClass, so the shell links running windows to this entry and its icon.
wm_class="com.fileem.FileEmApp"
desktop="$data/applications/$wm_class.desktop"
icons="$data/icons/hicolor"

refresh() {
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$data/applications" || true
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$icons" || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -rf "$data/fileem"
    rm -f "$desktop"
    find "$icons" -path '*/apps/fileem.png' -delete 2>/dev/null || true
    refresh
    echo "fileEm uninstalled (settings in ~/.config/fileem are kept)."
    exit 0
fi

build="$repo/target/dist/app/fileEm"
if [[ ! -x "$build/bin/fileEm" ]]; then
    echo "No app build found at $build" >&2
    echo "Build it first: mvn -Pdist -DskipTests package" >&2
    exit 1
fi

mkdir -p "$data/fileem"
rm -rf "$app_dir.new"
cp -a "$build" "$app_dir.new"
rm -rf "$app_dir"
mv "$app_dir.new" "$app_dir"

for size in 16 24 32 48 64 128 256 512; do
    mkdir -p "$icons/${size}x${size}/apps"
    cp "$repo/src/main/resources/com/fileem/icons/fileem-$size.png" "$icons/${size}x${size}/apps/fileem.png"
done

mkdir -p "$data/applications"
cat > "$desktop" <<EOF
[Desktop Entry]
Type=Application
Name=fileEm
GenericName=File Manager
Comment=Explorer-style file manager with an optional second side
Exec="$app_dir/bin/fileEm"
Icon=fileem
Terminal=false
Categories=System;FileTools;FileManager;
Keywords=files;folders;explorer;commander;
StartupWMClass=$wm_class
StartupNotify=true
EOF

refresh
echo "fileEm installed: $app_dir"
echo "Start it from the app grid / menu (search \"fileEm\"), or run: $app_dir/bin/fileEm"
