#!/usr/bin/env bash
# Regenerate desktop bundle icons and the macOS DMG background from brand SVGs.

set -euo pipefail

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
icon_source="$repo_root/brand/schoolx-mark.svg"
dmg_source="$repo_root/brand/schoolx-dmg-background.svg"
icons_dir="$repo_root/desktop/src-tauri/icons"
work_dir=$(mktemp -d -t schoolx-brand-assets)
trap 'rm -rf "$work_dir"' EXIT

for source in "$icon_source" "$dmg_source"; do
    if [[ ! -f "$source" ]]; then
        echo "Missing SchoolX brand source: $source" >&2
        exit 1
    fi
done

if [[ $(uname -s) != Darwin ]]; then
    echo "DMG background generation requires macOS sips." >&2
    exit 1
fi

cd "$repo_root"
. ./bin/activate-hermit
pnpm --dir desktop tauri icon "$icon_source" --output "$work_dir/icons"

# Tauri's ICNS encoder can reorder chunks between runs. Build the canonical
# Apple iconset explicitly so committed output is byte-for-byte reproducible.
pnpm --dir desktop tauri icon "$icon_source" --output "$work_dir/icns-png" \
    --png 16,32,64,128,256,512,1024
mkdir -p "$work_dir/SchoolX.iconset"
install -m 0644 "$work_dir/icns-png/16x16.png" \
    "$work_dir/SchoolX.iconset/icon_16x16.png"
install -m 0644 "$work_dir/icns-png/32x32.png" \
    "$work_dir/SchoolX.iconset/icon_16x16@2x.png"
install -m 0644 "$work_dir/icns-png/32x32.png" \
    "$work_dir/SchoolX.iconset/icon_32x32.png"
install -m 0644 "$work_dir/icns-png/64x64.png" \
    "$work_dir/SchoolX.iconset/icon_32x32@2x.png"
install -m 0644 "$work_dir/icns-png/128x128.png" \
    "$work_dir/SchoolX.iconset/icon_128x128.png"
install -m 0644 "$work_dir/icns-png/256x256.png" \
    "$work_dir/SchoolX.iconset/icon_128x128@2x.png"
install -m 0644 "$work_dir/icns-png/256x256.png" \
    "$work_dir/SchoolX.iconset/icon_256x256.png"
install -m 0644 "$work_dir/icns-png/512x512.png" \
    "$work_dir/SchoolX.iconset/icon_256x256@2x.png"
install -m 0644 "$work_dir/icns-png/512x512.png" \
    "$work_dir/SchoolX.iconset/icon_512x512.png"
install -m 0644 "$work_dir/icns-png/1024x1024.png" \
    "$work_dir/SchoolX.iconset/icon_512x512@2x.png"
/usr/bin/iconutil -c icns "$work_dir/SchoolX.iconset" \
    -o "$work_dir/icons/icon.icns"

desktop_icons=(
    32x32.png
    64x64.png
    128x128.png
    128x128@2x.png
    icon.png
    icon.icns
    icon.ico
    StoreLogo.png
    Square30x30Logo.png
    Square44x44Logo.png
    Square71x71Logo.png
    Square89x89Logo.png
    Square107x107Logo.png
    Square142x142Logo.png
    Square150x150Logo.png
    Square284x284Logo.png
    Square310x310Logo.png
)

for icon in "${desktop_icons[@]}"; do
    install -m 0644 "$work_dir/icons/$icon" "$icons_dir/$icon"
done

sips -s format png -s dpiWidth 144 -s dpiHeight 144 "$dmg_source" \
    --out "$icons_dir/schoolx-dmg-background.png" >/dev/null

metadata=$(sips -g pixelWidth -g pixelHeight -g dpiWidth -g dpiHeight \
    "$icons_dir/schoolx-dmg-background.png" 2>/dev/null)
if [[ "$metadata" != *"pixelWidth: 1320"* || "$metadata" != *"pixelHeight: 1000"* \
    || "$metadata" != *"dpiWidth: 144"* || "$metadata" != *"dpiHeight: 144"* ]]; then
    echo "SchoolX DMG background must render at 1320x1000 and 144 dpi." >&2
    exit 1
fi

echo "Generated SchoolX desktop icons and DMG background."
