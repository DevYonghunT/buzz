#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
config="$repo_root/desktop/src-tauri/tauri.conf.json"
release_workflow="$repo_root/.github/workflows/release.yml"
canary_workflow="$repo_root/.github/workflows/signed-macos-canary.yml"
verifier='desktop/scripts/verify-code-git-xpc-signature.sh'

node --test "$repo_root/desktop/scripts/stage-code-git-xpc.test.mjs"

node - "$config" "$release_workflow" "$canary_workflow" "$repo_root/$verifier" <<'NODE'
const fs = require("node:fs");
const config = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const releaseWorkflow = fs.readFileSync(process.argv[3], "utf8");
const canaryWorkflow = fs.readFileSync(process.argv[4], "utf8");
const verifierPath = process.argv[5];
const verifier = "desktop/scripts/verify-code-git-xpc-signature.sh";
const identifier = "io.github.schoolx520.app.schoolx-code-git";
const bundle = `${identifier}.xpc`;
const productName = config.productName;
if (typeof productName !== "string" || !/^[0-9A-Za-z._ -]+$/.test(productName)) {
  throw new Error("Tauri productName cannot identify a fixed macOS app bundle");
}
const appBundle = `${productName}.app`;
const hook = config.build?.beforeBundleCommand;
if (
  hook?.script !== "node scripts/stage-code-git-xpc.mjs" ||
  hook?.cwd !== ".."
) {
  throw new Error("Tauri beforeBundleCommand no longer stages the Code Git XPC");
}
const source = config.bundle?.macOS?.files?.[`XPCServices/${bundle}`];
if (source !== `generated/code-git-xpc/${bundle}`) {
  throw new Error("Tauri macOS.files no longer embeds the fixed Code Git XPC bundle");
}

function jobBlock(workflow, jobName) {
  const marker = `  ${jobName}:\n`;
  const start = workflow.indexOf(marker);
  if (start < 0) throw new Error(`workflow is missing ${jobName} job`);
  const rest = workflow.slice(start + marker.length);
  const nextJob = rest.search(/^  [0-9A-Za-z_-]+:\n/m);
  return nextJob < 0 ? rest : rest.slice(0, nextJob);
}

function occurrences(text, needle) {
  return text.split(needle).length - 1;
}

function requireSignedJob({ block, label, layout }) {
  const signingAction = "block/apple-codesign-action@";
  const deepVerify = "codesign --verify --deep --strict";
  if (occurrences(block, signingAction) !== 1) {
    throw new Error(`${label} must invoke the signing action exactly once`);
  }
  if (occurrences(block, deepVerify) !== 1) {
    throw new Error(`${label} must deep-verify the signed app exactly once`);
  }
  if (occurrences(block, verifier) !== 1) {
    throw new Error(`${label} must verify the nested Code Git XPC exactly once`);
  }
  const signingIndex = block.indexOf(signingAction);
  const deepVerifyIndex = block.indexOf(deepVerify);
  const xpcVerifyIndex = block.indexOf(verifier);
  if (!(signingIndex < deepVerifyIndex && deepVerifyIndex < xpcVerifyIndex)) {
    throw new Error(`${label} must sign, deep-verify, then verify the nested XPC`);
  }
  if (!block.includes(appBundle)) {
    throw new Error(`${label} must use Tauri product bundle ${appBundle}`);
  }
  if (block.includes("Buzz.app")) {
    throw new Error(`${label} still uses the stale internal Buzz.app bundle name`);
  }
  if (!block.includes(`SCHOOLX_CODE_GIT_CARGO_LAYOUT: ${layout}`)) {
    throw new Error(`${label} has the wrong Code Git Cargo output layout`);
  }
}

const armRelease = jobBlock(releaseWorkflow, "release");
const intelRelease = jobBlock(releaseWorkflow, "release-macos-x64");
const signedCanary = jobBlock(canaryWorkflow, "build");
requireSignedJob({ block: armRelease, label: "arm64 release", layout: "native" });
requireSignedJob({
  block: intelRelease,
  label: "x86_64 release",
  layout: "target-triple",
});
requireSignedJob({ block: signedCanary, label: "signed canary", layout: "native" });

if (!armRelease.includes("Buzz_${VERSION}_aarch64.app.tar.gz")) {
  throw new Error("arm64 public updater asset basename changed");
}
if (!intelRelease.includes("Buzz_${VERSION}_x64.app.tar.gz")) {
  throw new Error("x86_64 public updater asset basename changed");
}
if (!signedCanary.includes("Buzz_${VERSION}_aarch64-signed.dmg")) {
  throw new Error("signed canary public DMG basename changed");
}

const verifierSource = fs.readFileSync(verifierPath, "utf8");
if (!verifierSource.includes(`app_identifier=${config.identifier}`)) {
  throw new Error("signature verifier app identifier drifted from Tauri config");
}
NODE

grep -Fq 'xpc_identifier=io.github.schoolx520.app.schoolx-code-git' \
  "$repo_root/desktop/scripts/verify-code-git-xpc-signature.sh"
grep -Fq 'app_identifier=io.github.schoolx520.app' \
  "$repo_root/desktop/scripts/verify-code-git-xpc-signature.sh"
grep -Fq 'codesign --verify --strict --verbose=2 "$xpc_path"' \
  "$repo_root/desktop/scripts/verify-code-git-xpc-signature.sh"
grep -Fq 'app_team" != "$xpc_team' \
  "$repo_root/desktop/scripts/verify-code-git-xpc-signature.sh"
grep -Fq 'certificate 1[field.1.2.840.113635.100.6.2.6] exists' \
  "$repo_root/desktop/scripts/verify-code-git-xpc-signature.sh"
grep -Fq 'certificate leaf[field.1.2.840.113635.100.6.1.13] exists' \
  "$repo_root/desktop/scripts/verify-code-git-xpc-signature.sh"
grep -Fq 'codesign --verify --strict --verbose=2 -R="$xpc_requirement" "$xpc_path"' \
  "$repo_root/desktop/scripts/verify-code-git-xpc-signature.sh"

if ! awk '
  /^desktop-release-build / { in_recipe = 1 }
  in_recipe { print }
  in_recipe && /^desktop-ci:/ { exit }
' "$repo_root/justfile" | grep -Fq \
  'export SCHOOLX_CODE_GIT_CARGO_LAYOUT=target-triple'; then
  echo "desktop-release-build must select the target-triple XPC Cargo output" >&2
  exit 1
fi

echo "Code Git XPC packaging contract passed"
