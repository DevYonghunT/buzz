import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Write a tauri.release.conf.json with release-only overrides.
//
// Tauri's --config flag merges the provided JSON on top of the base
// tauri.conf.json, so this file must contain ONLY the delta fields —
// not a copy of the base config.
//
// For OSS release builds this script emits:
// 1. bundle.macOS.minimumSystemVersion = "10.15" for broad compatibility.
// 2. bundle.createUpdaterArtifacts = true so Tauri produces the .tar.gz
//    archive and .sig signature during the build.
// 3. On Linux only, bundle.useLocalToolsDir = true so tauri-bundler consumes
//    the SHA-pinned AppImage tools staged in Cargo's target/.tauri directory.
// 4. plugins.updater with the public key and endpoint from env vars.
//    Both BUZZ_UPDATER_PUBLIC_KEY and BUZZ_UPDATER_ENDPOINT are required -
//    the script fails if either is missing (OSS builds always ship with updater).
//
// Apple code signing and notarization happen post-build via
// block/apple-codesign-action in release.yml, so no signingIdentity is
// emitted here and the Tauri build is invoked with --no-sign.

export function createReleaseConfig({
  updaterPubkey,
  updaterEndpoint,
  platform = process.platform,
}) {
  const bundle = {
    macOS: {
      minimumSystemVersion: "10.15",
    },
    createUpdaterArtifacts: true,
  };

  if (platform === "linux") {
    bundle.useLocalToolsDir = true;
  }

  return {
    bundle,
    plugins: {
      updater: {
        pubkey: updaterPubkey,
        endpoints: [updaterEndpoint],
      },
    },
  };
}

function main() {
  const outputConfigPath = resolve(
    process.cwd(),
    "src-tauri/tauri.release.conf.json",
  );
  const updaterPubkey = process.env.BUZZ_UPDATER_PUBLIC_KEY;
  const updaterEndpoint = process.env.BUZZ_UPDATER_ENDPOINT;

  const missing = [];
  if (!updaterPubkey) missing.push("BUZZ_UPDATER_PUBLIC_KEY");
  if (!updaterEndpoint) missing.push("BUZZ_UPDATER_ENDPOINT");
  if (missing.length > 0) {
    console.error(
      `Error: required environment variable(s) missing: ${missing.join(", ")}`,
    );
    process.exit(1);
  }

  const releaseConfig = createReleaseConfig({
    updaterPubkey,
    updaterEndpoint,
  });

  console.log(`Updater enabled -> ${updaterEndpoint}`);

  writeFileSync(
    outputConfigPath,
    `${JSON.stringify(releaseConfig, null, 2)}\n`,
  );
  console.log(`Wrote ${outputConfigPath}`);
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main();
}
