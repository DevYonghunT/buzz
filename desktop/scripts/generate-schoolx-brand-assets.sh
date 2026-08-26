#!/usr/bin/env bash
# Regenerate checked-in SchoolX assets from the canonical brand SVGs.

set -euo pipefail

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
icon_source="$repo_root/brand/schoolx-mark.svg"
dmg_source="$repo_root/brand/schoolx-dmg-background.svg"
icons_dir="$repo_root/desktop/src-tauri/icons"
public_dir="$repo_root/desktop/public"
public_brand_dir="$public_dir/brand"
work_dir=$(mktemp -d -t schoolx-brand-assets)
trap 'rm -rf "$work_dir"' EXIT

mode=write
if [[ ${1:-} == "--check" ]]; then
    mode=check
    shift
fi
if (( $# != 0 )); then
    echo "Usage: $0 [--check]" >&2
    exit 2
fi

for source in "$icon_source" "$dmg_source"; do
    if [[ ! -f "$source" ]]; then
        echo "Missing SchoolX brand source: $source" >&2
        exit 1
    fi
done

if [[ $(uname -s) != Darwin ]]; then
    echo "SchoolX asset generation requires macOS sips and iconutil." >&2
    exit 1
fi

cd "$repo_root"
. ./bin/activate-hermit

generated_icons_dir="$work_dir/icons"
generated_public_dir="$work_dir/public"
generated_public_brand_dir="$generated_public_dir/brand"
mkdir -p "$generated_icons_dir" "$generated_public_brand_dir"

pnpm --dir desktop tauri icon "$icon_source" --output "$generated_icons_dir"

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
    -o "$generated_icons_dir/icon.icns"

sips -s format png -s dpiWidth 144 -s dpiHeight 144 "$dmg_source" \
    --out "$generated_icons_dir/schoolx-dmg-background.png" >/dev/null

dmg_metadata=$(sips -g pixelWidth -g pixelHeight -g dpiWidth -g dpiHeight \
    "$generated_icons_dir/schoolx-dmg-background.png" 2>/dev/null)
if [[ "$dmg_metadata" != *"pixelWidth: 1320"* \
    || "$dmg_metadata" != *"pixelHeight: 1000"* \
    || "$dmg_metadata" != *"dpiWidth: 144"* \
    || "$dmg_metadata" != *"dpiHeight: 144"* ]]; then
    echo "SchoolX DMG background must render at 1320x1000 and 144 dpi." >&2
    exit 1
fi

# The public SVG is deliberately a byte copy, not a separately maintained
# WebView drawing. QR center images need an opaque plate because the QR renderer
# paints its foreground behind the image; the canonical SVG already includes
# the intended mark padding inside its 256x256 view box.
install -m 0644 "$icon_source" \
    "$generated_public_brand_dir/schoolx-mark.svg"
webview_icon_source="$work_dir/schoolx-app-icon.svg"
node --input-type=module - "$icon_source" "$webview_icon_source" <<'NODE'
import { readFileSync, writeFileSync } from "node:fs";

const [, , sourcePath, outputPath] = process.argv;
const source = readFileSync(sourcePath, "utf8");
const match = source.match(/<svg\b[^>]*>([\s\S]*)<\/svg>\s*$/);
if (!match) {
  throw new Error(`Invalid canonical SchoolX mark SVG: ${sourcePath}`);
}

const plate = [
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">',
  '  <rect width="256" height="256" fill="#F4EDDD"/>',
  match[1].trim(),
  "</svg>",
  "",
].join("\n");
writeFileSync(outputPath, plate);
NODE

pnpm --dir desktop tauri icon "$webview_icon_source" \
    --output "$work_dir/webview-icons" --png 112,168
install -m 0644 "$work_dir/webview-icons/112x112.png" \
    "$generated_public_dir/app-icon@2x.png"
install -m 0644 "$work_dir/webview-icons/168x168.png" \
    "$generated_public_dir/app-icon@3x.png"

for size_and_file in \
    "112:$generated_public_dir/app-icon@2x.png" \
    "168:$generated_public_dir/app-icon@3x.png"; do
    expected_size=${size_and_file%%:*}
    generated_file=${size_and_file#*:}
    metadata=$(sips -g pixelWidth -g pixelHeight "$generated_file" 2>/dev/null)
    if [[ "$metadata" != *"pixelWidth: $expected_size"* \
        || "$metadata" != *"pixelHeight: $expected_size"* ]]; then
        echo "SchoolX WebView app icon must be ${expected_size}x${expected_size}: $generated_file" >&2
        exit 1
    fi
done

node --input-type=module - \
    "$icon_source" "$generated_public_dir" \
    "$generated_public_brand_dir/manifest.json" <<'NODE'
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [, , sourcePath, publicDirectory, manifestPath] = process.argv;
const hash = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
const bytes = (path) => readFileSync(path).byteLength;
const sourceHash = hash(sourcePath);
const publicMarkPath = join(publicDirectory, "brand", "schoolx-mark.svg");
const icon2xPath = join(publicDirectory, "app-icon@2x.png");
const icon3xPath = join(publicDirectory, "app-icon@3x.png");

const manifest = {
  schemaVersion: 1,
  generatedBy: "desktop/scripts/generate-schoolx-brand-assets.sh",
  source: {
    path: "brand/schoolx-mark.svg",
    sha256: sourceHash,
    bytes: bytes(sourcePath),
  },
  assets: {
    "brand/schoolx-mark.svg": {
      mediaType: "image/svg+xml",
      sha256: hash(publicMarkPath),
      bytes: bytes(publicMarkPath),
    },
    "app-icon@2x.png": {
      mediaType: "image/png",
      sha256: hash(icon2xPath),
      bytes: bytes(icon2xPath),
      width: 112,
      height: 112,
      opaque: true,
    },
    "app-icon@3x.png": {
      mediaType: "image/png",
      sha256: hash(icon3xPath),
      bytes: bytes(icon3xPath),
      width: 168,
      height: 168,
      opaque: true,
    },
  },
};

writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
NODE

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
    schoolx-dmg-background.png
)

generated_files=()
target_files=()
for icon in "${desktop_icons[@]}"; do
    generated_files+=("$generated_icons_dir/$icon")
    target_files+=("$icons_dir/$icon")
done
generated_files+=(
    "$generated_public_brand_dir/schoolx-mark.svg"
    "$generated_public_brand_dir/manifest.json"
    "$generated_public_dir/app-icon@2x.png"
    "$generated_public_dir/app-icon@3x.png"
)
target_files+=(
    "$public_brand_dir/schoolx-mark.svg"
    "$public_brand_dir/manifest.json"
    "$public_dir/app-icon@2x.png"
    "$public_dir/app-icon@3x.png"
)

if [[ "$mode" == check ]]; then
    drift=0
    for index in "${!generated_files[@]}"; do
        generated_file=${generated_files[$index]}
        target_file=${target_files[$index]}
        if [[ ! -f "$target_file" ]] || ! cmp -s "$generated_file" "$target_file"; then
            echo "SchoolX generated asset is missing or stale: ${target_file#"$repo_root/"}" >&2
            drift=1
        fi
    done
    if (( drift != 0 )); then
        echo "Run desktop/scripts/generate-schoolx-brand-assets.sh to regenerate assets." >&2
        exit 1
    fi
    echo "SchoolX generated assets are current."
    exit 0
fi

mkdir -p "$icons_dir" "$public_brand_dir"
for index in "${!generated_files[@]}"; do
    install -m 0644 "${generated_files[$index]}" "${target_files[$index]}"
done

echo "Generated SchoolX desktop, DMG, and WebView assets."
