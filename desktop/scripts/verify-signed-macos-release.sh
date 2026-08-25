#!/usr/bin/env bash
set -euo pipefail

# Verify the final signed and notarized macOS release surfaces. This script is
# intentionally credential-free and suppresses tool output that can contain a
# signing certificate's display name. The fixed TeamIdentifier is enforced by
# verify-code-git-xpc-signature.sh.

if [[ $# -lt 3 || $# -gt 4 ]]; then
  echo "Usage: $0 <arm64|x86_64> <SchoolX.app> <SchoolX.dmg> [SchoolX.app.tar.gz]" >&2
  exit 2
fi

expected_arch=$1
app_path=$2
dmg_path=$3
updater_archive_path=${4:-}
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
xpc_identifier=io.github.schoolx520.app.schoolx-code-git

case "$expected_arch" in
  arm64 | x86_64) ;;
  *)
    echo "unsupported expected macOS architecture: $expected_arch" >&2
    exit 2
    ;;
esac

fail() {
  echo "$1" >&2
  exit 1
}

require_regular_bundle() {
  local bundle_path=$1 label=$2
  if [[ ! -d "$bundle_path" || -L "$bundle_path" ]]; then
    fail "$label must be a non-symlink bundle: $bundle_path"
  fi
}

binary_architectures() {
  /usr/bin/lipo -archs "$1" 2>/dev/null | /usr/bin/awk '{$1=$1; print}'
}

verify_thin_architecture() {
  local binary_path=$1 label=$2 architectures
  if [[ ! -f "$binary_path" || -L "$binary_path" || ! -x "$binary_path" ]]; then
    fail "$label is missing, linked, or not executable: $binary_path"
  fi
  architectures=$(binary_architectures "$binary_path") ||
    fail "could not inspect $label architecture"
  if [[ "$architectures" != "$expected_arch" ]]; then
    fail "$label must be thin $expected_arch; found: ${architectures:-unknown}"
  fi
}

verify_ticket() {
  local artifact_path=$1 label=$2
  if ! /usr/bin/xcrun stapler validate "$artifact_path" >/dev/null 2>&1; then
    fail "$label has no valid stapled notarization ticket"
  fi
}

verify_app_gatekeeper() {
  local artifact_path=$1 label=$2
  if ! /usr/sbin/spctl --assess --type execute "$artifact_path" >/dev/null 2>&1; then
    fail "Gatekeeper rejected $label"
  fi
}

verify_app() {
  local candidate_app=$1 label=$2 app_info app_executable_name app_executable xpc_path xpc_executable
  require_regular_bundle "$candidate_app" "$label"
  app_info="$candidate_app/Contents/Info.plist"
  [[ -f "$app_info" && ! -L "$app_info" ]] || fail "$label is missing a regular Info.plist"
  app_executable_name=$(
    /usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app_info" 2>/dev/null
  ) || fail "$label has no CFBundleExecutable"
  if [[ ! "$app_executable_name" =~ ^[0-9A-Za-z._+-]+$ ]]; then
    fail "$label has an unsafe CFBundleExecutable"
  fi
  app_executable="$candidate_app/Contents/MacOS/$app_executable_name"
  xpc_path="$candidate_app/Contents/XPCServices/${xpc_identifier}.xpc"
  xpc_executable="$xpc_path/Contents/MacOS/schoolx-code-git"

  verify_thin_architecture "$app_executable" "$label executable"
  require_regular_bundle "$xpc_path" "$label Code Git XPC"
  verify_thin_architecture "$xpc_executable" "$label Code Git XPC executable"

  if ! /usr/bin/codesign --verify --deep --strict --all-architectures \
    "$candidate_app" >/dev/null 2>&1; then
    fail "$label failed strict nested code-signature verification"
  fi
  if ! "$script_dir/verify-code-git-xpc-signature.sh" "$candidate_app" >/dev/null 2>&1; then
    fail "$label app/XPC identifier, TeamIdentifier, or nested signature is invalid"
  fi
  if ! "$script_dir/verify-macos-entitlements.sh" "$candidate_app" >/dev/null 2>&1; then
    fail "$label is missing required entitlements"
  fi
  verify_ticket "$candidate_app" "$label"
  verify_app_gatekeeper "$candidate_app" "$label"
}

verify_updater_archive() (
  local archive_path=$1 extract_dir members_file member saw_app
  if [[ ! -f "$archive_path" || -L "$archive_path" ]]; then
    fail "updater archive must be a regular non-symlink file: $archive_path"
  fi

  extract_dir=$(mktemp -d "${TMPDIR:-/tmp}/schoolx-updater-archive.XXXXXX")
  members_file=$(mktemp "${TMPDIR:-/tmp}/schoolx-updater-members.XXXXXX")
  cleanup_archive() {
    /bin/rm -rf "$extract_dir"
    /bin/rm -f "$members_file"
  }
  trap cleanup_archive EXIT
  trap 'exit 1' HUP INT TERM

  if ! /usr/bin/tar -tzf "$archive_path" >"$members_file" 2>/dev/null; then
    fail "final updater archive is unreadable"
  fi

  saw_app=0
  while IFS= read -r member; do
    case "$member" in
      SchoolX.app | SchoolX.app/*) saw_app=1 ;;
      *) fail "final updater archive contains a path outside SchoolX.app" ;;
    esac
    case "/$member/" in
      */../*) fail "final updater archive contains parent traversal" ;;
    esac
  done <"$members_file"
  [[ "$saw_app" -eq 1 ]] || fail "final updater archive does not contain SchoolX.app"

  if ! /usr/bin/tar -xzf "$archive_path" -C "$extract_dir" >/dev/null 2>&1; then
    fail "could not extract final updater archive"
  fi
  verify_app "$extract_dir/SchoolX.app" "app extracted from final updater archive"
)

if [[ ! -f "$dmg_path" || -L "$dmg_path" ]]; then
  fail "DMG must be a regular non-symlink file: $dmg_path"
fi

verify_app "$app_path" "signed app"

if ! /usr/bin/hdiutil verify "$dmg_path" >/dev/null 2>&1; then
  fail "final DMG checksum or structure validation failed"
fi
if ! /usr/bin/codesign --verify --strict "$dmg_path" >/dev/null 2>&1; then
  fail "final DMG code signature is invalid"
fi
verify_ticket "$dmg_path" "final DMG"
if ! /usr/sbin/spctl --assess --type open \
  --context context:primary-signature "$dmg_path" >/dev/null 2>&1; then
  fail "Gatekeeper rejected final DMG"
fi

mount_dir=$(mktemp -d "${TMPDIR:-/tmp}/schoolx-release-dmg.XXXXXX")
mounted=0
cleanup() {
  if [[ "$mounted" -eq 1 ]]; then
    /usr/bin/hdiutil detach "$mount_dir" >/dev/null 2>&1 || true
  fi
  /bin/rmdir "$mount_dir" >/dev/null 2>&1 || true
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

if ! /usr/bin/hdiutil attach -readonly -nobrowse -noautoopen \
  -mountpoint "$mount_dir" "$dmg_path" >/dev/null 2>&1; then
  fail "could not mount final DMG read-only"
fi
mounted=1

verify_app "$mount_dir/SchoolX.app" "app mounted from final DMG"

if ! /usr/bin/hdiutil detach "$mount_dir" >/dev/null 2>&1; then
  fail "could not detach final DMG"
fi
mounted=0
/bin/rmdir "$mount_dir"
trap - EXIT HUP INT TERM

verified_surface="app, XPC, and final DMG"
if [[ -n "$updater_archive_path" ]]; then
  verify_updater_archive "$updater_archive_path"
  verified_surface="$verified_surface, including the final updater archive"
fi

echo "Verified thin $expected_arch signed and notarized $verified_surface"
