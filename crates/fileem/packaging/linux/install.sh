#!/usr/bin/env bash
# Installs the Rust build of fileEm for the current user (no root needed): copies the executable to
# ~/.local/share/fileem-rust, adds the icons and the launcher entry, so fileEm shows up in the app
# grid / menu and the dock shows its icon. Next to the Java app's install (tools/install-linux.sh),
# not over it.
#
#   cargo build --release -p fileem && crates/fileem/packaging/linux/install.sh
#   crates/fileem/packaging/linux/install.sh --uninstall
#
# FILEEM_BIN installs another executable (tests); XDG_DATA_HOME moves the whole install.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
repo="$(cd "$crate/../.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
app_dir="$data/fileem-rust"
# The entry is named after the app id the windows carry (about::APP_ID): Wayland's app id and X11's
# window class. That links running windows to this entry and its icon.
app_id="com.fileem.FileEm"
desktop="$data/applications/$app_id.desktop"
icons="$data/icons/hicolor"

refresh() {
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$data/applications" || true
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$icons" || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -rf "$app_dir"
    rm -f "$desktop"
    find "$icons" -path "*/apps/$app_id.png" -delete 2>/dev/null || true
    refresh
    echo "fileEm (Rust) uninstalled (settings in ~/.config/fileem are kept)."
    exit 0
fi

bin="${FILEEM_BIN:-$repo/target/release/fileem}"
if [[ ! -x "$bin" ]]; then
    echo "No executable found at $bin" >&2
    echo "Build it first: cargo build --release -p fileem" >&2
    exit 1
fi

mkdir -p "$app_dir"
cp "$bin" "$app_dir/fileem.new"
chmod 755 "$app_dir/fileem.new"
mv -f "$app_dir/fileem.new" "$app_dir/fileem"

for size in 16 24 32 48 64 128 256 512; do
    mkdir -p "$icons/${size}x${size}/apps"
    cp "$crate/icons/fileem-$size.png" "$icons/${size}x${size}/apps/$app_id.png"
done

mkdir -p "$data/applications"
sed "s|^Exec=fileem |Exec=\"$app_dir/fileem\" |" "$here/$app_id.desktop" > "$desktop.new"
mv -f "$desktop.new" "$desktop"

refresh
echo "fileEm (Rust) installed: $app_dir/fileem"
echo "Start it from the app grid / menu (search \"fileEm\"), or run: $app_dir/fileem"
