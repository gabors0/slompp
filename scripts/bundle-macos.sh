#!/bin/sh
# Builds SLOPP.app: the panel as a macOS application for /Applications,
# Launchpad and the Dock, with sloppctl next to it.
#
#   scripts/bundle-macos.sh [output directory, default target/release]
#
# Needs only Rust and what ships with macOS (sips, iconutil, codesign).
set -eu

cd "$(dirname "$0")/.."
out="${1:-target/release}"
app="$out/SLOPP.app"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"

cargo build --release -p slopp -p slopp-core

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/slopp target/release/sloppctl "$app/Contents/MacOS/"

# The icon, at every size macOS asks for, from the same SVG Linux uses.
iconset="$(mktemp -d)/SLOPP.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
    for scale in 1 2; do
        pixels=$((size * scale))
        name="icon_${size}x${size}"
        [ "$scale" = 2 ] && name="${name}@2x"
        sips -s format png -z "$pixels" "$pixels" \
            crates/slopp/assets/linux/org.slopp.Slopp.svg \
            --out "$iconset/$name.png" >/dev/null
    done
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/SLOPP.icns"
rm -rf "$(dirname "$iconset")"

cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>SLOPP</string>
    <key>CFBundleDisplayName</key>
    <string>SLOPP</string>
    <key>CFBundleIdentifier</key>
    <string>org.slopp.Slopp</string>
    <key>CFBundleExecutable</key>
    <string>slopp</string>
    <key>CFBundleIconFile</key>
    <string>SLOPP</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

# An ad-hoc signature: enough to run on the Mac that built it.
codesign --force --sign - "$app" >/dev/null 2>&1

echo "Built $app"
