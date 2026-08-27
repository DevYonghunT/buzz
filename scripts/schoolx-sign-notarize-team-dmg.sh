#!/usr/bin/env bash

# Turn one unsigned SchoolX Team Build DMG into a Developer ID signed,
# notarized, stapled, and repository-verified DMG. Credentials stay in the
# caller's keychain/environment; this script never prints their values.

set -euo pipefail
umask 077

usage() {
  cat <<'EOF'
Usage:
  DEVELOPER_ID_IDENTITY=<existing-identity> \
  NOTARY_PROFILE=<existing-keychain-profile> \
    scripts/schoolx-sign-notarize-team-dmg.sh \
      --arch <arm64|x86_64> \
      --input <unsigned-team-build.dmg> \
      --output <signed-notarized.dmg>

The input is never modified. The output path must not already exist. The
script publishes it only after the nested app, app notarization ticket, final
DMG signature, DMG notarization ticket, Gatekeeper checks, and repository
release verifier all pass.
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

require_regular_file() {
  local path=$1 label=$2
  [[ -f "$path" && ! -L "$path" ]] ||
    die "$label must be a regular non-symlink file: $path"
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || die "required tool is unavailable: $1"
}

arch=
input_path=
output_path=

while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch)
      [[ $# -ge 2 ]] || die "--arch requires a value"
      arch=$2
      shift 2
      ;;
    --input)
      [[ $# -ge 2 ]] || die "--input requires a value"
      input_path=$2
      shift 2
      ;;
    --output)
      [[ $# -ge 2 ]] || die "--output requires a value"
      output_path=$2
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

case "$arch" in
  arm64 | x86_64) ;;
  "")
    die "--arch is required (arm64 or x86_64)"
    ;;
  *)
    die "unsupported architecture: $arch (expected arm64 or x86_64)"
    ;;
esac
[[ -n "$input_path" ]] || die "--input is required"
[[ -n "$output_path" ]] || die "--output is required"
[[ "$(uname -s)" == Darwin ]] || die "this signing workflow requires macOS"

: "${DEVELOPER_ID_IDENTITY:?set an authorized Developer ID Application identity}"
: "${NOTARY_PROFILE:?set an existing notarytool keychain profile name}"

[[ "$DEVELOPER_ID_IDENTITY" != *$'\n'* && "$DEVELOPER_ID_IDENTITY" != *$'\r'* ]] ||
  die "DEVELOPER_ID_IDENTITY must be a single line"
[[ "$NOTARY_PROFILE" =~ ^[0-9A-Za-z._-]+$ ]] ||
  die "NOTARY_PROFILE may contain only letters, numbers, dot, underscore, and hyphen"
[[ "$input_path" != *$'\n'* && "$input_path" != *$'\r'* ]] ||
  die "--input must be a single-line path"
[[ "$output_path" != *$'\n'* && "$output_path" != *$'\r'* ]] ||
  die "--output must be a single-line path"

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd "$script_dir/.." && pwd -P)
runtime_verifier="$repo_root/desktop/scripts/verify-macos-runtime-compatibility.sh"
signature_verifier="$repo_root/desktop/scripts/verify-code-git-xpc-signature.sh"
entitlements_verifier="$repo_root/desktop/scripts/verify-macos-entitlements.sh"
release_verifier="$repo_root/desktop/scripts/verify-signed-macos-release.sh"
entitlements_path="$repo_root/desktop/src-tauri/Entitlements.plist"

for repo_file in \
  "$runtime_verifier" \
  "$signature_verifier" \
  "$entitlements_verifier" \
  "$release_verifier" \
  "$entitlements_path"; do
  require_regular_file "$repo_file" "repository signing dependency"
done

for tool in jq; do
  require_tool "$tool"
done

input_dir=$(cd "$(dirname "$input_path")" 2>/dev/null && pwd -P) ||
  die "input directory does not exist"
input_path="$input_dir/$(basename "$input_path")"
output_dir=$(cd "$(dirname "$output_path")" 2>/dev/null && pwd -P) ||
  die "output directory does not exist"
output_path="$output_dir/$(basename "$output_path")"

require_regular_file "$input_path" "input DMG"
[[ -s "$input_path" ]] || die "input DMG is empty"
[[ "$input_path" == *.dmg ]] || die "--input must name a .dmg file"
[[ "$output_path" == *.dmg ]] || die "--output must name a .dmg file"
[[ "$input_path" != "$output_path" ]] || die "input and output must be different paths"
[[ ! -e "$output_path" && ! -L "$output_path" ]] ||
  die "output already exists; refusing to overwrite: $output_path"
[[ -w "$output_dir" ]] || die "output directory is not writable: $output_dir"

if ! /usr/bin/hdiutil verify "$input_path" >/dev/null 2>&1; then
  die "input DMG checksum or structure validation failed"
fi
if /usr/bin/codesign --verify --strict "$input_path" >/dev/null 2>&1; then
  die "input DMG is already signed; expected an unsigned Team Build artifact"
fi

secure_tmp_root=$(cd "${TMPDIR:-/tmp}" 2>/dev/null && pwd -P) ||
  die "secure temporary directory does not exist"
work_dir=$(/usr/bin/mktemp -d "$secure_tmp_root/schoolx-team-dmg-sign.XXXXXX") ||
  die "could not create a secure staging directory"
/bin/chmod 700 "$work_dir"
input_mount="$work_dir/input-mount"
rw_mount="$work_dir/rw-mount"
app_parent="$work_dir/app"
app_path="$app_parent/SchoolX.app"
notary_dir="$work_dir/notary"
app_notary_zip="$notary_dir/SchoolX.app.zip"
rw_dmg="$work_dir/template-rw.dmg"
candidate_dmg="$work_dir/final.dmg"
input_mounted=0
rw_mounted=0
input_device=
rw_device=
publish_temp=

/bin/mkdir "$input_mount" "$rw_mount" "$app_parent" "$notary_dir"

cleanup() {
  local status=$? mounts_detached=1
  trap - EXIT
  if [[ "$rw_mounted" -eq 1 ]]; then
    if ! /usr/bin/hdiutil detach "${rw_device:-$rw_mount}" >/dev/null 2>&1; then
      mounts_detached=0
    fi
  fi
  if [[ "$input_mounted" -eq 1 ]]; then
    if ! /usr/bin/hdiutil detach "${input_device:-$input_mount}" >/dev/null 2>&1; then
      mounts_detached=0
    fi
  fi
  if [[ -n "${publish_temp:-}" && -f "$publish_temp" && ! -L "$publish_temp" &&
    "$publish_temp" == "$output_dir"/.schoolx-team-dmg-output.* ]]; then
    /bin/rm -f "$publish_temp"
  fi
  if [[ "$mounts_detached" -eq 1 && -n "${work_dir:-}" && -d "$work_dir" &&
    "$work_dir" == "$secure_tmp_root"/schoolx-team-dmg-sign.* ]]; then
    /bin/rm -rf "$work_dir"
  elif [[ "$mounts_detached" -eq 0 ]]; then
    echo "warning: a temporary DMG could not be detached; secure staging was preserved" >&2
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

if ! input_attach_output=$(/usr/bin/hdiutil attach -readonly -nobrowse -noautoopen \
  -mountpoint "$input_mount" "$input_path" 2>/dev/null); then
  die "could not mount input DMG read-only"
fi
input_mounted=1
input_device=$(printf '%s\n' "$input_attach_output" | /usr/bin/awk '$1 ~ /^\/dev\/disk[0-9]+$/ { print $1; exit }')
[[ "$input_device" =~ ^/dev/disk[0-9]+$ ]] ||
  die "could not identify the complete input DMG device"

mounted_app="$input_mount/SchoolX.app"
[[ -d "$mounted_app" && ! -L "$mounted_app" ]] ||
  die "input DMG does not contain a regular top-level SchoolX.app"
if [[ -n "$(/usr/bin/find "$mounted_app" -type l -print -quit)" ]]; then
  die "SchoolX.app contains a symbolic link; refusing an unsafe signing input"
fi
if ! /usr/bin/ditto "$mounted_app" "$app_path" >/dev/null 2>&1; then
  die "could not copy SchoolX.app into secure staging"
fi
if ! /usr/bin/hdiutil detach "$input_device" >/dev/null 2>&1; then
  die "could not detach input DMG"
fi
input_mounted=0
input_device=

app_identifier=io.github.schoolx520.app
xpc_identifier=io.github.schoolx520.app.schoolx-code-git
xpc_path="$app_path/Contents/XPCServices/${xpc_identifier}.xpc"
xpc_executable="$xpc_path/Contents/MacOS/schoolx-code-git"
expected_team=3WPS7QNZV5
sidecar_names=(buzz buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr)

plist_value() {
  local plist_path=$1 key=$2
  /usr/libexec/PlistBuddy -c "Print :$key" "$plist_path" 2>/dev/null
}

verify_thin_binary() {
  local path=$1 label=$2 observed
  [[ -f "$path" && ! -L "$path" && -x "$path" ]] ||
    die "$label is missing, linked, or not executable"
  observed=$(/usr/bin/lipo -archs "$path" 2>/dev/null | /usr/bin/awk '{$1=$1; print}') ||
    die "could not inspect $label architecture"
  [[ "$observed" == "$arch" ]] ||
    die "$label must be thin $arch; found ${observed:-unknown}"
}

verify_unsigned_layout() {
  local app_info="$app_path/Contents/Info.plist"
  local xpc_info="$xpc_path/Contents/Info.plist"
  local name relative executable_count=0

  [[ -d "$app_path" && ! -L "$app_path" ]] || die "staged app is not a regular bundle"
  [[ -d "$xpc_path" && ! -L "$xpc_path" ]] || die "staged app is missing its Code Git XPC"
  [[ -f "$app_info" && ! -L "$app_info" ]] || die "staged app has no regular Info.plist"
  [[ -f "$xpc_info" && ! -L "$xpc_info" ]] || die "Code Git XPC has no regular Info.plist"
  [[ "$(plist_value "$app_info" CFBundleIdentifier)" == "$app_identifier" ]] ||
    die "staged app has an unexpected bundle identifier"
  [[ "$(plist_value "$app_info" CFBundleExecutable)" == buzz-desktop ]] ||
    die "staged app has an unexpected executable"
  [[ "$(plist_value "$xpc_info" CFBundleIdentifier)" == "$xpc_identifier" ]] ||
    die "Code Git XPC has an unexpected bundle identifier"
  [[ "$(plist_value "$xpc_info" CFBundleExecutable)" == schoolx-code-git ]] ||
    die "Code Git XPC has an unexpected executable"

  if [[ -n "$(/usr/bin/find "$app_path" -type l -print -quit)" ]]; then
    die "staged SchoolX.app contains a symbolic link"
  fi

  while IFS= read -r -d '' executable; do
    relative=${executable#"$app_path/"}
    case "$relative" in
      Contents/MacOS/buzz | \
        Contents/MacOS/buzz-acp | \
        Contents/MacOS/buzz-agent | \
        Contents/MacOS/buzz-desktop | \
        Contents/MacOS/buzz-dev-mcp | \
        Contents/MacOS/git-credential-nostr | \
        Contents/XPCServices/io.github.schoolx520.app.schoolx-code-git.xpc/Contents/MacOS/schoolx-code-git)
        ;;
      *)
        die "staged app contains an unexpected executable: $relative"
        ;;
    esac
    executable_count=$((executable_count + 1))
  done < <(/usr/bin/find "$app_path/Contents" -type f -perm -111 -print0)
  [[ "$executable_count" -eq 7 ]] ||
    die "staged app must contain exactly seven executable signing surfaces"

  verify_thin_binary "$app_path/Contents/MacOS/buzz-desktop" "app executable"
  for name in "${sidecar_names[@]}"; do
    verify_thin_binary "$app_path/Contents/MacOS/$name" "bundled $name sidecar"
  done
  verify_thin_binary "$xpc_executable" "Code Git XPC executable"
  if ! "$runtime_verifier" "$app_path" "$arch" >/dev/null 2>&1; then
    die "staged app failed the macOS deployment-target or Swift runtime contract"
  fi
}

codesign_failure="$work_dir/codesign-error.log"

sign_code() {
  local path=$1 label=$2
  if ! /usr/bin/codesign --force --options runtime --timestamp \
    --sign "$DEVELOPER_ID_IDENTITY" "$path" \
    >/dev/null 2>"$codesign_failure"; then
    die "Developer ID signing failed for $label"
  fi
}

sign_app() {
  if ! /usr/bin/codesign --force --options runtime --timestamp \
    --entitlements "$entitlements_path" \
    --sign "$DEVELOPER_ID_IDENTITY" "$app_path" \
    >/dev/null 2>"$codesign_failure"; then
    die "Developer ID signing failed for SchoolX.app"
  fi
}

developer_id_requirement() {
  printf '%s' \
    "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"${expected_team}\""
}

verify_signed_component() {
  local path=$1 label=$2 metadata team count requirement
  if ! /usr/bin/codesign --verify --strict --all-architectures "$path" \
    >/dev/null 2>&1; then
    die "$label failed strict code-signature verification"
  fi
  metadata=$(/usr/bin/codesign --display --verbose=4 "$path" 2>&1) ||
    die "could not inspect $label signature metadata"
  team=$(printf '%s\n' "$metadata" | /usr/bin/awk -F= '$1 == "TeamIdentifier" { print substr($0, index($0, "=") + 1) }')
  count=$(printf '%s\n' "$team" | /usr/bin/awk 'NF { count++ } END { print count + 0 }')
  [[ "$count" -eq 1 && "$team" == "$expected_team" ]] ||
    die "$label is not signed by the expected SchoolX team"
  printf '%s\n' "$metadata" | /usr/bin/grep -Eq '^CodeDirectory .*flags=.*runtime' ||
    die "$label is missing the hardened runtime signing option"
  printf '%s\n' "$metadata" | /usr/bin/grep -Eq '^Timestamp=' ||
    die "$label is missing a secure signing timestamp"
  requirement=$(developer_id_requirement)
  if ! /usr/bin/codesign --verify --strict --all-architectures \
    -R="$requirement" "$path" >/dev/null 2>&1; then
    die "$label is not signed with the expected Developer ID certificate"
  fi
}

verify_signed_dmg() {
  local metadata team count requirement
  if ! /usr/bin/codesign --verify --strict "$candidate_dmg" >/dev/null 2>&1; then
    die "final DMG code-signature verification failed"
  fi
  metadata=$(/usr/bin/codesign --display --verbose=4 "$candidate_dmg" 2>&1) ||
    die "could not inspect final DMG signature metadata"
  team=$(printf '%s\n' "$metadata" | /usr/bin/awk -F= '$1 == "TeamIdentifier" { print substr($0, index($0, "=") + 1) }')
  count=$(printf '%s\n' "$team" | /usr/bin/awk 'NF { count++ } END { print count + 0 }')
  [[ "$count" -eq 1 && "$team" == "$expected_team" ]] ||
    die "final DMG is not signed by the expected SchoolX team"
  printf '%s\n' "$metadata" | /usr/bin/grep -Eq '^Timestamp=' ||
    die "final DMG is missing a secure signing timestamp"
  requirement=$(developer_id_requirement)
  if ! /usr/bin/codesign --verify --strict -R="$requirement" "$candidate_dmg" \
    >/dev/null 2>&1; then
    die "final DMG is not signed with the expected Developer ID certificate"
  fi
}

verify_signed_app() {
  local name
  if ! /usr/bin/codesign --verify --deep --strict --all-architectures "$app_path" \
    >/dev/null 2>&1; then
    die "SchoolX.app failed strict nested code-signature verification"
  fi
  if ! "$signature_verifier" "$app_path" >/dev/null 2>&1; then
    die "SchoolX app/XPC identity or TeamIdentifier verification failed"
  fi
  if ! "$entitlements_verifier" "$app_path" >/dev/null 2>&1; then
    die "SchoolX.app is missing required entitlements"
  fi
  if ! "$runtime_verifier" "$app_path" "$arch" >/dev/null 2>&1; then
    die "signed app failed the macOS runtime contract"
  fi

  for name in "${sidecar_names[@]}"; do
    verify_signed_component "$app_path/Contents/MacOS/$name" "bundled $name sidecar"
  done
  verify_signed_component "$xpc_path" "Code Git XPC"
  verify_signed_component "$app_path" "SchoolX.app"
}

submit_and_assert_accepted() {
  local artifact=$1 label=$2
  local result log stderr_log submission_id
  result="$notary_dir/${label}-submit.json"
  log="$notary_dir/${label}-log.json"
  stderr_log="$notary_dir/${label}-stderr.log"

  if ! /usr/bin/xcrun notarytool submit "$artifact" \
    --keychain-profile "$NOTARY_PROFILE" \
    --wait --output-format json >"$result" 2>"$stderr_log"; then
    die "Apple notarization submission failed for $label"
  fi
  if ! jq -e '.status == "Accepted" and (.id | type == "string")' \
    "$result" >/dev/null 2>&1; then
    die "Apple did not accept the $label notarization submission"
  fi
  submission_id=$(jq -r '.id' "$result")
  [[ "$submission_id" =~ ^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$ ]] ||
    die "Apple returned an invalid $label notarization submission identifier"
  if ! /usr/bin/xcrun notarytool log "$submission_id" "$log" \
    --keychain-profile "$NOTARY_PROFILE" \
    >/dev/null 2>"$stderr_log"; then
    die "could not retrieve the Apple notarization log for $label"
  fi
  if ! jq -e \
    '.status == "Accepted" and .statusCode == 0 and ((.issues // []) | length == 0)' \
    "$log" >/dev/null 2>&1; then
    die "Apple notarization log did not pass the strict $label acceptance contract"
  fi
}

staple_and_validate() {
  local artifact=$1 label=$2 stderr_log="$notary_dir/stapler-error.log"
  if ! /usr/bin/xcrun stapler staple "$artifact" \
    >/dev/null 2>"$stderr_log"; then
    die "could not staple the notarization ticket to $label"
  fi
  if ! /usr/bin/xcrun stapler validate "$artifact" >/dev/null 2>&1; then
    die "$label has no valid stapled notarization ticket"
  fi
}

verify_unsigned_layout

# Nested signing is intentionally leaf-first: five sidecars, the XPC bundle,
# then the containing app. Do not replace this with codesign --deep signing.
for sidecar_name in "${sidecar_names[@]}"; do
  sign_code "$app_path/Contents/MacOS/$sidecar_name" "bundled $sidecar_name sidecar"
done
sign_code "$xpc_path" "Code Git XPC"
sign_app
verify_signed_app

if ! /usr/bin/ditto -c -k --keepParent "$app_path" "$app_notary_zip" \
  >/dev/null 2>&1; then
  die "could not create the app notarization transport archive"
fi
submit_and_assert_accepted "$app_notary_zip" app
staple_and_validate "$app_path" "signed app"
verify_signed_app
if ! /usr/sbin/spctl --assess --type execute "$app_path" >/dev/null 2>&1; then
  die "Gatekeeper rejected the signed and notarized app"
fi

# Preserve the original Team Build DMG layout, replace only its app with the
# signed/stapled copy, and produce a new final byte under secure staging.
if ! /usr/bin/hdiutil convert "$input_path" -format UDRW \
  -o "$rw_dmg" -ov >/dev/null 2>&1; then
  die "could not create a writable DMG template"
fi
if ! rw_attach_output=$(/usr/bin/hdiutil attach -readwrite -nobrowse -noautoopen \
  -mountpoint "$rw_mount" "$rw_dmg" 2>/dev/null); then
  die "could not mount the writable DMG template"
fi
rw_mounted=1
rw_device=$(printf '%s\n' "$rw_attach_output" | /usr/bin/awk '$1 ~ /^\/dev\/disk[0-9]+$/ { print $1; exit }')
[[ "$rw_device" =~ ^/dev/disk[0-9]+$ ]] ||
  die "could not identify the complete writable DMG device"
[[ -d "$rw_mount/SchoolX.app" && ! -L "$rw_mount/SchoolX.app" ]] ||
  die "writable DMG template lost its regular top-level SchoolX.app"
[[ "$rw_mount" == "$work_dir/rw-mount" ]] || die "unsafe writable DMG mount path"
/bin/rm -rf "$rw_mount/SchoolX.app"
if ! /usr/bin/ditto "$app_path" "$rw_mount/SchoolX.app" >/dev/null 2>&1; then
  die "could not place the signed app into the writable DMG"
fi
if ! /usr/bin/hdiutil detach "$rw_device" >/dev/null 2>&1; then
  die "could not detach the writable DMG"
fi
rw_mounted=0
rw_device=
if ! /usr/bin/hdiutil convert "$rw_dmg" -format UDZO -imagekey zlib-level=9 \
  -o "$candidate_dmg" -ov >/dev/null 2>&1; then
  die "could not create the final compressed DMG"
fi

if ! /usr/bin/codesign --force --timestamp --sign "$DEVELOPER_ID_IDENTITY" \
  "$candidate_dmg" >/dev/null 2>"$codesign_failure"; then
  die "Developer ID signing failed for the final DMG"
fi
verify_signed_dmg
submit_and_assert_accepted "$candidate_dmg" dmg
staple_and_validate "$candidate_dmg" "final DMG"
verify_signed_dmg

if ! "$release_verifier" "$arch" "$app_path" "$candidate_dmg" \
  >/dev/null 2>&1; then
  die "final app and DMG failed the repository release verifier"
fi

# Copy the verified byte into a mode-0600 sibling first. BSD mv -n can then
# publish atomically without overwriting a path that appeared during signing.
candidate_sha=$(/usr/bin/shasum -a 256 "$candidate_dmg" | /usr/bin/awk '{print $1}')
[[ "$candidate_sha" =~ ^[0-9a-f]{64}$ ]] || die "could not hash the verified DMG"
publish_temp=$(/usr/bin/mktemp "$output_dir/.schoolx-team-dmg-output.XXXXXX") ||
  die "could not reserve a secure output staging file"
/bin/chmod 600 "$publish_temp"
if ! /usr/bin/ditto "$candidate_dmg" "$publish_temp" >/dev/null 2>&1; then
  die "could not copy the verified DMG to output staging"
fi
require_regular_file "$publish_temp" "output staging DMG"
publish_sha=$(/usr/bin/shasum -a 256 "$publish_temp" | /usr/bin/awk '{print $1}')
[[ "$publish_sha" == "$candidate_sha" ]] ||
  die "output staging copy does not match the verified DMG byte"

/bin/mv -n "$publish_temp" "$output_path"
if [[ -e "$publish_temp" ]]; then
  die "output appeared during signing; refusing to overwrite it"
fi
publish_temp=
require_regular_file "$output_path" "published output DMG"

output_sha=$(/usr/bin/shasum -a 256 "$output_path" | /usr/bin/awk '{print $1}')
[[ "$output_sha" == "$candidate_sha" ]] || die "published output DMG byte changed"
echo "Signed, notarized, stapled, and verified $arch SchoolX DMG: $output_path"
echo "SHA-256: $output_sha"
