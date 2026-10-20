#!/usr/bin/env bash
# Builds a .deb of the Rust app from the release executable: /usr/bin/fileem, the launcher entry
# and the icons. Written to target/dist/fileem_<version>_<arch>.deb.
#
#   cargo build --release -p fileem && crates/fileem/packaging/linux/make-deb.sh
#
# FILEEM_BIN packages another executable and FILEEM_DIST writes elsewhere (tests).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
repo="$(cd "$crate/../.." && pwd)"
app_id="com.fileem.FileEm"
bin="${FILEEM_BIN:-$repo/target/release/fileem}"
dist="${FILEEM_DIST:-$repo/target/dist}"
if [[ ! -x "$bin" ]]; then
    echo "No executable found at $bin" >&2
    echo "Build it first: cargo build --release -p fileem" >&2
    exit 1
fi
command -v dpkg-deb >/dev/null || { echo "dpkg-deb is missing" >&2; exit 1; }

# the workspace version (one number with the pom's app.version)
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$repo/Cargo.toml")"
arch="$(dpkg --print-architecture 2>/dev/null || echo amd64)"

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
install -Dm755 "$bin" "$root/usr/bin/fileem"
install -Dm644 "$here/$app_id.desktop" "$root/usr/share/applications/$app_id.desktop"
for size in 16 24 32 48 64 128 256 512; do
    install -Dm644 "$crate/icons/fileem-$size.png" "$root/usr/share/icons/hicolor/${size}x${size}/apps/$app_id.png"
done
mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<EOF
Package: fileem
Version: $version
Architecture: $arch
Maintainer: fileEm <noreply@fileem.invalid>
Section: utils
Priority: optional
Depends: libc6, libfontconfig1, libfreetype6
Recommends: udisks2, libglib2.0-bin, xdg-utils
Description: Explorer-style file manager with an optional second side
 A file manager with a folder tree, tabs, eight views with thumbnails,
 a preview, search and bookmarks, and a Commander-style second side.
EOF

mkdir -p "$dist"
out="$dist/fileem_${version}_${arch}.deb"
dpkg-deb --root-owner-group --build "$root" "$out" >/dev/null
echo "$out"
