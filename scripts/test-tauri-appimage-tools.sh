#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
installer="$repo_root/desktop/scripts/install-tauri-appimage-tools.sh"
production_manifest="$repo_root/desktop/scripts/tauri-appimage-tools-x86_64.lock"
release_workflow="$repo_root/.github/workflows/release.yml"
canary_workflow="$repo_root/.github/workflows/linux-canary.yml"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

bash -n "$installer"

# The plugin scripts must come from the exact Tauri CLI source commit, never a
# mutable branch. Binary release assets are checksum-pinned in the same lock.
grep -Fq '499df79be65ef8c0670abc0207cd9e37b55d8491/crates/tauri-bundler/' "$production_manifest"
if grep -Eq 'raw\.githubusercontent\.com/.+/(master|main)/' "$production_manifest"; then
  echo "AppImage tool manifest contains a mutable raw GitHub branch" >&2
  exit 1
fi

assert_workflow_wiring() {
  local workflow=$1 job=$2 label=$3 block=$4 install_line build_line verify_line
  awk -v marker="  ${job}:" '
    $0 == marker { in_job = 1 }
    in_job && $0 != marker && /^  [0-9A-Za-z_-]+:$/ { exit }
    in_job { print }
  ' "$workflow" >"$block"
  [[ -s "$block" ]] || {
    echo "$label workflow job was not found" >&2
    exit 1
  }
  [[ "$(grep -Fc 'desktop/scripts/install-tauri-appimage-tools.sh' "$block")" -eq 2 ]] || {
    echo "$label must install and then reverify pinned AppImage tools" >&2
    exit 1
  }
  install_line=$(grep -nE '^[[:space:]]+run: desktop/scripts/install-tauri-appimage-tools\.sh$' "$block" | cut -d: -f1)
  build_line=$(grep -nF 'pnpm tauri build' "$block" | cut -d: -f1)
  verify_line=$(grep -nF 'run: desktop/scripts/install-tauri-appimage-tools.sh --verify-only' "$block" | cut -d: -f1)
  [[ "$install_line" =~ ^[0-9]+$ && "$build_line" =~ ^[0-9]+$ && "$verify_line" =~ ^[0-9]+$ ]]
  [[ "$install_line" -lt "$build_line" && "$build_line" -lt "$verify_line" ]] || {
    echo "$label AppImage tool verification must bracket the Tauri build" >&2
    exit 1
  }
}

release_job_block="$tmp/release-linux.job"
canary_job_block="$tmp/linux-canary.job"
assert_workflow_wiring "$release_workflow" release-linux "Linux release" "$release_job_block"
assert_workflow_wiring "$canary_workflow" build "Linux canary" "$canary_job_block"
grep -Fq '"useLocalToolsDir": true' "$canary_job_block" || {
  echo "Linux canary must select Cargo target-local Tauri tools" >&2
  exit 1
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

mkdir -p "$tmp/bin" "$tmp/sources" "$tmp/cache"
cat >"$tmp/bin/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) printf '%s\n' Linux ;;
  -m) printf '%s\n' x86_64 ;;
  *) printf '%s\n' Linux ;;
esac
EOF
chmod +x "$tmp/bin/uname"

# Keep the Linux platform shim isolated from the host's Hermit launchers.
# Hermit also consults uname, so invoking the real cargo/node here would make
# a macOS test try to bootstrap Linux executables. These fakes prove the
# metadata contract without changing any host tool cache.
cat >"$tmp/bin/cargo" <<'EOF'
#!/bin/sh
set -eu
[ "$#" -eq 6 ]
[ "$1" = metadata ]
[ "$2" = --manifest-path ]
case "$3" in
  */desktop/src-tauri/Cargo.toml) ;;
  *) exit 2 ;;
esac
[ "$4" = --no-deps ]
[ "$5" = --format-version ]
[ "$6" = 1 ]
[ -n "${CARGO_TARGET_DIR:-}" ]
python3 -c 'import json, os; print(json.dumps({"target_directory": os.environ["CARGO_TARGET_DIR"]}))'
EOF
cat >"$tmp/bin/node" <<'EOF'
#!/bin/sh
set -eu
[ "$#" -eq 2 ]
[ "$1" = -e ]
python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"], end="")'
EOF
chmod +x "$tmp/bin/cargo" "$tmp/bin/node"

names=(
  AppRun-x86_64
  linuxdeploy-x86_64.AppImage
  linuxdeploy-plugin-gtk.sh
  linuxdeploy-plugin-gstreamer.sh
  linuxdeploy-plugin-appimage.AppImage
)

test_manifest="$tmp/tools.lock"
: >"$test_manifest"
for name in "${names[@]}"; do
  printf 'fixture for %s\n' "$name" >"$tmp/sources/$name"
  hash=$(sha256_file "$tmp/sources/$name")
  printf '%s %s %s file://%s\n' \
    "$hash" "$hash" "$name" "$tmp/sources/$name" >>"$test_manifest"
done

PATH="$tmp/bin:$PATH" "$installer" --manifest "$test_manifest" "$tmp/cache"
PATH="$tmp/bin:$PATH" "$installer" --verify-only --manifest "$test_manifest" "$tmp/cache"
for name in "${names[@]}"; do
  [[ -x "$tmp/cache/$name" ]]
done

# With no explicit cache argument, the installer must resolve the same Cargo
# target directory that Tauri uses rather than assume the repository default.
metadata_target="$tmp/cargo-target"
CARGO_TARGET_DIR="$metadata_target" PATH="$tmp/bin:$PATH" \
  "$installer" --manifest "$test_manifest"
CARGO_TARGET_DIR="$metadata_target" PATH="$tmp/bin:$PATH" \
  "$installer" --verify-only --manifest "$test_manifest"
for name in "${names[@]}"; do
  [[ -x "$metadata_target/.tauri/$name" ]]
done

printf 'tampered cache\n' >"$tmp/cache/AppRun-x86_64"
if PATH="$tmp/bin:$PATH" "$installer" --verify-only --manifest "$test_manifest" "$tmp/cache"; then
  echo "tampered AppImage tool cache was accepted" >&2
  exit 1
fi

# Restore the trusted cache, then prove a bad late download cannot partially
# replace it: all downloads are staged and verified before installation.
PATH="$tmp/bin:$PATH" "$installer" --manifest "$test_manifest" "$tmp/cache"
before=$(sha256_file "$tmp/cache/AppRun-x86_64")
printf 'tampered source\n' >"$tmp/sources/linuxdeploy-plugin-appimage.AppImage"
if PATH="$tmp/bin:$PATH" "$installer" --manifest "$test_manifest" "$tmp/cache"; then
  echo "tampered AppImage tool download was accepted" >&2
  exit 1
fi
after=$(sha256_file "$tmp/cache/AppRun-x86_64")
[[ "$before" == "$after" ]] || {
  echo "cache changed before every staged tool passed integrity validation" >&2
  exit 1
}

if [[ "${SCHOOLX_TEST_TAURI_APPIMAGE_DOWNLOADS:-0}" == 1 ]]; then
  production_cache="$tmp/production-cache"
  PATH="$tmp/bin:$PATH" "$installer" "$production_cache"
  PATH="$tmp/bin:$PATH" "$installer" --verify-only "$production_cache"

  # Mirror tauri-bundler 2.9.2 prepare_tools(): bytes 8..10 are zeroed to
  # prevent desktop AppImage integration from discovering linuxdeploy itself.
  # The exact transformed hash is locked and must remain verifiable.
  printf '\0\0\0' | dd \
    of="$production_cache/linuxdeploy-x86_64.AppImage" \
    bs=1 seek=8 conv=notrunc 2>/dev/null
  PATH="$tmp/bin:$PATH" "$installer" --verify-only "$production_cache"
fi

echo "Tauri AppImage tool pinning contract passed"
