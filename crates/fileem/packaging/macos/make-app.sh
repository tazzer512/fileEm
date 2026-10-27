#!/usr/bin/env bash
# Builds the macOS app bundle of the Rust app from the release executable:
# target/dist/fileEm.app (executable, icns icon, Info.plist with the app id as bundle id). Only
# shell tools, so it also runs elsewhere (tests); unsigned: the first start needs a right-click
# > Open, or `xattr -dr com.apple.quarantine fileEm.app`.
#
# --dmg also packs it into target/dist/fileEm-<version>.dmg (the app next to a link to
# /Applications, to drag it there): one file to copy to a Mac. That needs macOS's hdiutil.
#
#   cargo build --release -p fileem && crates/fileem/packaging/macos/make-app.sh [--dmg]
#
# FILEEM_BIN bundles another executable and FILEEM_DIST writes elsewhere (tests).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
repo="$(cd "$crate/../.." && pwd)"
app_id="com.fileem.FileEm"
bin="${FILEEM_BIN:-$repo/target/release/fileem}"
dist="${FILEEM_DIST:-$repo/target/dist}"
dmg=false
if [[ "${1:-}" == "--dmg" ]]; then
    dmg=true
    if ! command -v hdiutil >/dev/null; then
        echo "--dmg needs hdiutil (macOS)" >&2
        exit 1
    fi
fi
if [[ ! -x "$bin" ]]; then
    echo "No executable found at $bin" >&2
    echo "Build it first: cargo build --release -p fileem" >&2
    exit 1
fi

# the workspace version (one number with the pom's app.version)
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$repo/Cargo.toml")"

app="$dist/fileEm.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/fileem"
chmod 755 "$app/Contents/MacOS/fileem"
cp "$crate/icons/fileem.icns" "$app/Contents/Resources/fileem.icns"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>fileEm</string>
    <key>CFBundleDisplayName</key><string>fileEm</string>
    <key>CFBundleIdentifier</key><string>$app_id</string>
    <key>CFBundleExecutable</key><string>fileem</string>
    <key>CFBundleIconFile</key><string>fileem.icns</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
echo "$app"

if $dmg; then
    image="$dist/fileEm-$version.dmg"
    stage="$dist/dmg"
    rm -rf "$stage" "$image"
    mkdir -p "$stage"
    cp -R "$app" "$stage/"
    ln -s /Applications "$stage/Applications"
    hdiutil create -quiet -volname fileEm -srcfolder "$stage" -fs HFS+ -format UDZO "$image"
    rm -rf "$stage"
    echo "$image"
fi
