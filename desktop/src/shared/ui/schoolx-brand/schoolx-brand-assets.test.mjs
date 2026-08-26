import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import UPNG from "upng-js";

import { SCHOOLX_MARK_ASSET_PATH, SchoolXMark } from "./SchoolXMark.tsx";

const testDirectory = dirname(fileURLToPath(import.meta.url));
const desktopRoot = resolve(testDirectory, "../../../..");
const repositoryRoot = resolve(desktopRoot, "..");
const publicRoot = join(desktopRoot, "public");
const canonicalMarkPath = join(repositoryRoot, "brand", "schoolx-mark.svg");
const publicMarkPath = join(publicRoot, "brand", "schoolx-mark.svg");
const manifestPath = join(publicRoot, "brand", "manifest.json");

const CANONICAL_MARK_SHA256 =
  "3e12926eb2e7525589b7dd5bfbc61348f8e739ca73f697f46898d6a060ea3d80";
const PARCHMENT_RGBA = [244, 237, 221, 255];
const REQUIRED_MARK_COLORS = [
  [53, 86, 73, 255],
  [127, 150, 122, 255],
  [184, 90, 60, 255],
  [215, 169, 75, 255],
  PARCHMENT_RGBA,
  [31, 41, 55, 255],
];
const PNG_CONTRACT = {
  "app-icon@2x.png": {
    bytes: 2049,
    height: 112,
    sha256: "687eeb41d1df9cd94993afebeb7902adb9416d261d50954f17f0f80312d8b0da",
    width: 112,
  },
  "app-icon@3x.png": {
    bytes: 2694,
    height: 168,
    sha256: "92f6752d74104968b1386346f26d2cad6f9a822ee55812cefc28c0d40f533c6c",
    width: 168,
  },
};

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function decodePng(bytes) {
  const arrayBuffer = bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  );
  const png = UPNG.decode(arrayBuffer);
  const frames = UPNG.toRGBA8(png);
  assert.equal(frames.length, 1, "brand app icons must be static PNGs");
  return { png, rgba: new Uint8Array(frames[0]) };
}

function pixelAt(rgba, pixelIndex) {
  const offset = pixelIndex * 4;
  return Array.from(rgba.subarray(offset, offset + 4));
}

test("public mark is a byte-exact copy of the pinned canonical brand source", () => {
  const canonicalBytes = readFileSync(canonicalMarkPath);
  const publicBytes = readFileSync(publicMarkPath);
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

  assert.equal(sha256(canonicalBytes), CANONICAL_MARK_SHA256);
  assert.deepEqual(publicBytes, canonicalBytes);
  assert.equal(manifest.schemaVersion, 1);
  assert.equal(
    manifest.generatedBy,
    "desktop/scripts/generate-schoolx-brand-assets.sh",
  );
  assert.deepEqual(Object.keys(manifest.assets), [
    "brand/schoolx-mark.svg",
    "app-icon@2x.png",
    "app-icon@3x.png",
  ]);
  assert.deepEqual(manifest.source, {
    path: "brand/schoolx-mark.svg",
    sha256: CANONICAL_MARK_SHA256,
    bytes: canonicalBytes.byteLength,
  });
  assert.deepEqual(manifest.assets["brand/schoolx-mark.svg"], {
    mediaType: "image/svg+xml",
    sha256: CANONICAL_MARK_SHA256,
    bytes: publicBytes.byteLength,
  });
});

for (const [filename, contract] of Object.entries(PNG_CONTRACT)) {
  test(`${filename} is the exact opaque SchoolX QR icon`, () => {
    const bytes = readFileSync(join(publicRoot, filename));
    const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
    const { png, rgba } = decodePng(bytes);

    assert.equal(bytes.byteLength, contract.bytes);
    assert.equal(sha256(bytes), contract.sha256);
    assert.equal(png.width, contract.width);
    assert.equal(png.height, contract.height);
    assert.deepEqual(manifest.assets[filename], {
      mediaType: "image/png",
      sha256: contract.sha256,
      bytes: contract.bytes,
      width: contract.width,
      height: contract.height,
      opaque: true,
    });

    for (let offset = 3; offset < rgba.length; offset += 4) {
      assert.equal(
        rgba[offset],
        255,
        `${filename} has a non-opaque pixel at index ${(offset - 3) / 4}`,
      );
    }

    const lastPixel = contract.width * contract.height - 1;
    const corners = [
      0,
      contract.width - 1,
      lastPixel - contract.width + 1,
      lastPixel,
    ];
    for (const corner of corners) {
      assert.deepEqual(pixelAt(rgba, corner), PARCHMENT_RGBA);
    }

    const renderedColors = new Set();
    for (let offset = 0; offset < rgba.length; offset += 4) {
      renderedColors.add(
        Array.from(rgba.subarray(offset, offset + 4)).join(","),
      );
    }
    for (const color of REQUIRED_MARK_COLORS) {
      assert.ok(
        renderedColors.has(color.join(",")),
        `${filename} is missing canonical mark color ${color.join(",")}`,
      );
    }
  });
}

test("SchoolXMark uses the local asset with meaningful and decorative alternatives", () => {
  const meaningful = renderToStaticMarkup(
    React.createElement(SchoolXMark, {
      ariaLabel: "Localized SchoolX mark",
      className: "size-12",
    }),
  );
  assert.equal(SCHOOLX_MARK_ASSET_PATH, "/brand/schoolx-mark.svg");
  assert.match(meaningful, /src="\/brand\/schoolx-mark\.svg"/);
  assert.match(meaningful, /alt="Localized SchoolX mark"/);
  assert.match(meaningful, /data-testid="schoolx-mark"/);
  assert.doesNotMatch(meaningful, /aria-hidden/);

  const decorative = renderToStaticMarkup(
    React.createElement(SchoolXMark, { decorative: true }),
  );
  assert.match(decorative, /alt=""/);
  assert.match(decorative, /aria-hidden="true"/);
  assert.match(decorative, /data-testid="schoolx-mark"/);
});
