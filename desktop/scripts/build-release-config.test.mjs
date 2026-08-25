import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { createReleaseConfig } from "./build-release-config.mjs";

const input = {
  updaterPubkey: "public-test-key",
  updaterEndpoint: "https://example.invalid/latest.json",
};

const scriptsDir = dirname(fileURLToPath(import.meta.url));

test("Linux release config selects Cargo target-local Tauri tools", () => {
  const config = createReleaseConfig({ ...input, platform: "linux" });

  assert.equal(config.bundle.useLocalToolsDir, true);
  assert.equal(config.bundle.createUpdaterArtifacts, true);
});

test("non-Linux release configs do not change their tool cache location", () => {
  for (const platform of ["darwin", "win32"]) {
    const config = createReleaseConfig({ ...input, platform });

    assert.equal(
      Object.hasOwn(config.bundle, "useLocalToolsDir"),
      false,
      platform,
    );
  }
});

test("macOS release config preserves the 10.15 application floor", () => {
  const config = createReleaseConfig({ ...input, platform: "darwin" });

  assert.equal(config.bundle.macOS?.minimumSystemVersion, "10.15");
});

test("AppImage tool lock stays coupled to Tauri CLI 2.11.2", () => {
  const pnpmLock = readFileSync(
    resolve(scriptsDir, "../../pnpm-lock.yaml"),
    "utf8",
  );
  const importerMarker = "      '@tauri-apps/cli':\n";
  const importerStart = pnpmLock.indexOf(importerMarker);
  assert.notEqual(importerStart, -1);
  const importerRemainder = pnpmLock.slice(
    importerStart + importerMarker.length,
  );
  const nextImporterEntry = importerRemainder.search(/\n {6}\S/);
  assert.notEqual(nextImporterEntry, -1);
  const importerEntry = importerRemainder.slice(0, nextImporterEntry);
  assert.match(importerEntry, /^ {8}version: 2\.11\.2$/m);

  const toolLock = readFileSync(
    resolve(scriptsDir, "tauri-appimage-tools-x86_64.lock"),
    "utf8",
  );
  assert.match(toolLock, /^# Tauri CLI 2\.11\.2 uses tauri-bundler 2\.9\.2/m);
});
