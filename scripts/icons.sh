#!/bin/sh
# Regenerates the raster icons from crates/slopp/assets/linux/org.slopp.Slopp.svg
# (which Linux and macOS use as is). Run it after changing the SVG.
#
# - assets/windows/slopp.ico: build.rs embeds it in slopp.exe (Explorer, the
#   taskbar, the installer's shortcuts).
# - assets/linux/org.slopp.Slopp.png: the X11 window icon (_NET_WM_ICON),
#   which X11 taskbars and window switchers show when a window sets one.
#
# Needs rsvg-convert and Python 3:
#
#   nix shell nixpkgs#librsvg nixpkgs#python3 -c scripts/icons.sh
set -eu

cd "$(dirname "$0")/.."
svg=crates/slopp/assets/linux/org.slopp.Slopp.svg
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

rsvg-convert -w 128 -h 128 "$svg" -o crates/slopp/assets/linux/org.slopp.Slopp.png

# The sizes Windows asks for at 100-200% scaling.
sizes="16 20 24 32 40 48 64 256"
for size in $sizes; do
    rsvg-convert -w "$size" -h "$size" "$svg" -o "$tmp/$size.png"
done

# An ICO of PNG images (fine since Windows Vista), smaller than ImageMagick's
# uncompressed bitmaps.
python3 - "$tmp" crates/slopp/assets/windows/slopp.ico $sizes <<'PY'
import struct, sys
tmp, out, sizes = sys.argv[1], sys.argv[2], [int(s) for s in sys.argv[3:]]
images = [open(f"{tmp}/{s}.png", "rb").read() for s in sizes]
offset = 6 + 16 * len(images)
entries = b""
for size, png in zip(sizes, images):
    side = size % 256  # 0 means 256
    entries += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(png), offset)
    offset += len(png)
with open(out, "wb") as f:
    f.write(struct.pack("<HHH", 0, 1, len(images)) + entries + b"".join(images))
PY
echo "Wrote slopp.ico and org.slopp.Slopp.png in crates/slopp/assets"
