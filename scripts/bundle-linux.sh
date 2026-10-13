#!/bin/sh
# Builds slopp-<version>-linux-x86_64.AppImage: the panel and sloppctl as
# one file that runs on most distributions (those with the build machine's
# glibc or newer, so build on the oldest one it should support).
#
#   scripts/bundle-linux.sh [output directory, default target/release]
#
# Needs Rust, GPUI's build dependencies (see the release workflow), curl,
# and a network connection the first time: linuxdeploy (copies in the
# libraries that aren't part of every desktop) and appimagetool (packs the
# AppDir with the static runtime, which needs no libfuse2) are downloaded
# into target/appimage-tools.
#
# The AppImage runs the panel; a symlink to it named sloppctl runs sloppctl.
set -eu

cd "$(dirname "$0")/.."
out="${1:-target/release}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
arch=x86_64
tools=target/appimage-tools
appdir=target/release/SLOPP.AppDir

cargo build --release --locked -p slopp -p slopp-core

mkdir -p "$tools"
for tool in linuxdeploy/linuxdeploy/linuxdeploy AppImage/appimagetool/appimagetool; do
    name="${tool##*/}"
    if [ ! -x "$tools/$name" ]; then
        curl -fsSL -o "$tools/$name" \
            "https://github.com/${tool%/*}/releases/download/continuous/$name-$arch.AppImage"
        chmod +x "$tools/$name"
    fi
done
# The tools are AppImages themselves; this runs them without FUSE (CI has
# none).
export APPIMAGE_EXTRACT_AND_RUN=1

rm -rf "$appdir"

# linuxdeploy copies the binaries and the libraries they link that aren't on
# AppImage's exclude list (glibc, fontconfig, graphics drivers and the like
# come from the host), and points the binaries' rpath at them; Vulkan and
# Wayland are loaded at run time from the host.
"$tools/linuxdeploy" --appdir "$appdir" \
    --executable target/release/slopp \
    --executable target/release/sloppctl \
    --desktop-file crates/slopp/assets/linux/org.slopp.Slopp.desktop \
    --icon-file crates/slopp/assets/linux/org.slopp.Slopp.svg \
    --icon-file crates/slopp/assets/linux/org.slopp.Slopp.png \
    --custom-apprun crates/slopp/assets/linux/AppRun
# File managers' AppImage thumbnailers read .DirIcon, and not all take SVG.
ln -sf usr/share/icons/hicolor/128x128/apps/org.slopp.Slopp.png "$appdir/.DirIcon"

mkdir -p "$out"
image="$out/slopp-$version-linux-$arch.AppImage"
ARCH="$arch" VERSION="$version" "$tools/appimagetool" --no-appstream "$appdir" "$image"
echo "Built $image"
