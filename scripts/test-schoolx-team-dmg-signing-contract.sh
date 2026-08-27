#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
signer="$repo_root/scripts/schoolx-sign-notarize-team-dmg.sh"

bash -n "$signer"
help_output=$("$signer" --help)
grep -Fq -- '--arch <arm64|x86_64>' <<<"$help_output"
grep -Fq -- '--input <unsigned-team-build.dmg>' <<<"$help_output"
grep -Fq -- '--output <signed-notarized.dmg>' <<<"$help_output"

if "$signer" --arch universal --input /nonexistent/input.dmg \
  --output /nonexistent/output.dmg >/dev/null 2>&1; then
  echo "team DMG signer accepted an unsupported architecture" >&2
  exit 1
fi

node - "$signer" <<'NODE'
const fs = require("node:fs");

const source = fs.readFileSync(process.argv[2], "utf8");
const requireToken = (token) => {
  if (!source.includes(token)) {
    throw new Error(`team DMG signer is missing: ${token}`);
  }
};

for (const token of [
  'DEVELOPER_ID_IDENTITY:?set an authorized Developer ID Application identity',
  'NOTARY_PROFILE:?set an existing notarytool keychain profile name',
  'expected_team=3WPS7QNZV5',
  'sidecar_names=(buzz buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr)',
  'input DMG is already signed; expected an unsigned Team Build artifact',
  'output already exists; refusing to overwrite',
  'staged app contains an unexpected executable',
  'staged app must contain exactly seven executable signing surfaces',
  '/usr/bin/mktemp -d "$secure_tmp_root/schoolx-team-dmg-sign.XXXXXX"',
  'trap cleanup EXIT',
  '/usr/bin/hdiutil attach -readonly -nobrowse -noautoopen',
  '/usr/bin/hdiutil detach "$input_device"',
  '/usr/bin/hdiutil detach "$rw_device"',
  'mounts_detached=1',
  'secure staging was preserved',
  '/usr/bin/codesign --force --options runtime --timestamp',
  '--entitlements "$entitlements_path"',
  'anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists',
  '.status == "Accepted" and (.id | type == "string")',
  '.status == "Accepted" and .statusCode == 0 and ((.issues // []) | length == 0)',
  '/usr/bin/xcrun stapler staple',
  '/usr/bin/xcrun stapler validate',
  '/usr/sbin/spctl --assess --type execute',
  'final DMG is not signed by the expected SchoolX team',
  'final DMG is not signed with the expected Developer ID certificate',
  '"$release_verifier" "$arch" "$app_path" "$candidate_dmg"',
  '/usr/bin/mktemp "$output_dir/.schoolx-team-dmg-output.XXXXXX"',
  '[[ "$publish_sha" == "$candidate_sha" ]]',
  '/bin/mv -n "$publish_temp" "$output_path"',
]) {
  requireToken(token);
}

for (const sidecar of [
  "buzz",
  "buzz-acp",
  "buzz-agent",
  "buzz-dev-mcp",
  "git-credential-nostr",
]) {
  requireToken(sidecar);
}

const positions = [
  ['sidecar signing', 'sign_code "$app_path/Contents/MacOS/$sidecar_name"'],
  ['XPC signing', 'sign_code "$xpc_path" "Code Git XPC"'],
  ['app signing', 'sign_app\n'],
  ['app submission', 'submit_and_assert_accepted "$app_notary_zip" app'],
  ['app stapling', 'staple_and_validate "$app_path" "signed app"'],
  ['DMG signing', '"$candidate_dmg" >/dev/null 2>"$codesign_failure"'],
  ['DMG submission', 'submit_and_assert_accepted "$candidate_dmg" dmg'],
  ['DMG stapling', 'staple_and_validate "$candidate_dmg" "final DMG"'],
  ['final verification', '"$release_verifier" "$arch" "$app_path" "$candidate_dmg"'],
];

let previous = -1;
for (const [label, token] of positions) {
  const position = source.indexOf(token, previous + 1);
  if (position < 0) throw new Error(`could not locate ${label}`);
  if (position <= previous) throw new Error(`${label} is out of order`);
  previous = position;
}

if (/set\s+-[^\n]*x/.test(source)) {
  throw new Error("team DMG signer must never enable shell tracing");
}
if (/echo[^\n]*(DEVELOPER_ID_IDENTITY|NOTARY_PROFILE)/.test(source)) {
  throw new Error("team DMG signer may expose credential selectors in output");
}
if (source.includes("codesign --deep --force")) {
  throw new Error("team DMG signer must keep explicit leaf-first nested signing");
}
NODE

echo "SchoolX Team Build DMG signing contract passed"
