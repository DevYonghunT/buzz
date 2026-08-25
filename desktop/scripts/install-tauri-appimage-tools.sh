#!/usr/bin/env bash
# Install the exact AppImage helper inputs expected by Tauri CLI 2.11.2.
#
# The effective Tauri config MUST also contain:
#   { "bundle": { "useLocalToolsDir": true } }
# That makes tauri-bundler use <cargo-target-dir>/.tauri instead of the user's
# platform cache. This script resolves that directory from Cargo metadata so
# custom target directories stay aligned with Tauri.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DESKTOP_DIR=$(cd "$SCRIPT_DIR/.." && pwd)
DEFAULT_MANIFEST="$SCRIPT_DIR/tauri-appimage-tools-x86_64.lock"
MANIFEST="$DEFAULT_MANIFEST"
TOOLS_DIR=
MODE=install
TOOLS_DIR_SET=0

usage() {
  cat <<'EOF'
Usage: install-tauri-appimage-tools.sh [--verify-only] [--manifest PATH] [TOOLS_DIR]

Downloads all tools to a temporary staging directory, verifies their pinned
SHA-256 digests, then installs them into Tauri's local tools cache. The
--verify-only mode performs no network access and accepts only the downloaded
form or Tauri's documented deterministic cache form.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --verify-only)
      MODE=verify
      shift
      ;;
    --manifest)
      [[ $# -ge 2 ]] || {
        echo "Error: --manifest requires a path" >&2
        exit 1
      }
      MANIFEST=$2
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    -*)
      echo "Error: unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
    *)
      [[ $TOOLS_DIR_SET -eq 0 ]] || {
        echo "Error: tools directory was specified more than once" >&2
        exit 1
      }
      TOOLS_DIR=$1
      TOOLS_DIR_SET=1
      shift
      ;;
  esac
done

[[ $(uname -s) == Linux ]] || {
  echo "Error: AppImage tools may only be installed or verified on Linux" >&2
  exit 1
}
[[ $(uname -m) == x86_64 ]] || {
  echo "Error: only the release workflow's pinned x86_64 AppImage tools are approved" >&2
  exit 1
}
[[ -f "$MANIFEST" ]] || {
  echo "Error: manifest not found: $MANIFEST" >&2
  exit 1
}

if [[ $TOOLS_DIR_SET -eq 0 ]]; then
  command -v cargo >/dev/null 2>&1 || {
    echo "Error: cargo is required to resolve Tauri's target directory" >&2
    exit 1
  }
  command -v node >/dev/null 2>&1 || {
    echo "Error: node is required to parse cargo metadata" >&2
    exit 1
  }
  CARGO_TARGET_PATH=$(
    cargo metadata \
      --manifest-path "$DESKTOP_DIR/src-tauri/Cargo.toml" \
      --no-deps \
      --format-version 1 |
      node -e '
        let input = "";
        process.stdin.setEncoding("utf8");
        process.stdin.on("data", (chunk) => { input += chunk; });
        process.stdin.on("end", () => {
          const metadata = JSON.parse(input);
          if (typeof metadata.target_directory !== "string" || metadata.target_directory.length === 0) {
            process.exit(1);
          }
          process.stdout.write(metadata.target_directory);
        });
      '
  )
  [[ -n "$CARGO_TARGET_PATH" ]] || {
    echo "Error: cargo metadata did not return a target directory" >&2
    exit 1
  }
  TOOLS_DIR="$CARGO_TARGET_PATH/.tauri"
fi

[[ ! -L "$TOOLS_DIR" ]] || {
  echo "Error: refusing symlinked tools directory: $TOOLS_DIR" >&2
  exit 1
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "Error: sha256sum or shasum is required" >&2
    return 1
  fi
}

REQUIRED_NAMES=(
  AppRun-x86_64
  linuxdeploy-x86_64.AppImage
  linuxdeploy-plugin-gtk.sh
  linuxdeploy-plugin-gstreamer.sh
  linuxdeploy-plugin-appimage.AppImage
)
DOWNLOAD_HASHES=()
CACHE_HASHES=()
NAMES=()
URLS=()

while IFS=$' \t' read -r download_hash cache_hash name url extra; do
  if [[ -z "${download_hash:-}" || "$download_hash" == \#* ]]; then
    continue
  fi
  [[ -z "${extra:-}" ]] || {
    echo "Error: malformed manifest row for ${name:-unknown}" >&2
    exit 1
  }
  [[ "$download_hash" =~ ^[0-9a-f]{64}$ ]] || {
    echo "Error: invalid download SHA-256 for ${name:-unknown}" >&2
    exit 1
  }
  [[ "$cache_hash" =~ ^[0-9a-f]{64}$ ]] || {
    echo "Error: invalid cache SHA-256 for ${name:-unknown}" >&2
    exit 1
  }
  case "$name" in
    AppRun-x86_64 | linuxdeploy-x86_64.AppImage | linuxdeploy-plugin-gtk.sh | linuxdeploy-plugin-gstreamer.sh | linuxdeploy-plugin-appimage.AppImage) ;;
    *)
      echo "Error: unexpected tool name in manifest: $name" >&2
      exit 1
      ;;
  esac
  case "$url" in
    https://* | file://*) ;;
    *)
      echo "Error: tool URL must use https (or file for an explicit test manifest): $name" >&2
      exit 1
      ;;
  esac
  for existing_name in "${NAMES[@]:-}"; do
    [[ "$existing_name" != "$name" ]] || {
      echo "Error: duplicate manifest entry: $name" >&2
      exit 1
    }
  done
  DOWNLOAD_HASHES+=("$download_hash")
  CACHE_HASHES+=("$cache_hash")
  NAMES+=("$name")
  URLS+=("$url")
done <"$MANIFEST"

[[ ${#NAMES[@]} -eq ${#REQUIRED_NAMES[@]} ]] || {
  echo "Error: manifest must contain exactly ${#REQUIRED_NAMES[@]} tools" >&2
  exit 1
}
for required_name in "${REQUIRED_NAMES[@]}"; do
  found=0
  for name in "${NAMES[@]}"; do
    if [[ "$name" == "$required_name" ]]; then
      found=$((found + 1))
    fi
  done
  [[ $found -eq 1 ]] || {
    echo "Error: manifest must contain exactly one $required_name" >&2
    exit 1
  }
done

verify_cache() {
  local i path actual
  for ((i = 0; i < ${#NAMES[@]}; i++)); do
    path="$TOOLS_DIR/${NAMES[$i]}"
    [[ -f "$path" && ! -L "$path" ]] || {
      echo "Error: missing or symlinked Tauri tool: ${NAMES[$i]}" >&2
      return 1
    }
    actual=$(sha256_file "$path")
    if [[ "$actual" != "${DOWNLOAD_HASHES[$i]}" && "$actual" != "${CACHE_HASHES[$i]}" ]]; then
      echo "Error: Tauri tool SHA-256 mismatch: ${NAMES[$i]}" >&2
      return 1
    fi
    [[ -x "$path" ]] || {
      echo "Error: Tauri tool is not executable: ${NAMES[$i]}" >&2
      return 1
    }
  done
}

if [[ "$MODE" == verify ]]; then
  verify_cache
  echo "Verified ${#NAMES[@]} pinned Tauri AppImage tools in $TOOLS_DIR"
  exit 0
fi

mkdir -p "$TOOLS_DIR"
STAGE_DIR=$(mktemp -d "${TMPDIR:-/tmp}/schoolx-tauri-appimage-tools.XXXXXX")
trap 'rm -rf "$STAGE_DIR"' EXIT

# Stage and validate every input before changing the cache. This prevents a
# late download failure from leaving a mixed old/new tool set.
for ((i = 0; i < ${#NAMES[@]}; i++)); do
  staged="$STAGE_DIR/${NAMES[$i]}"
  echo "Downloading pinned Tauri tool: ${NAMES[$i]}"
  curl \
    --fail \
    --location \
    --silent \
    --show-error \
    --retry 3 \
    --connect-timeout 30 \
    --output "$staged" \
    --url "${URLS[$i]}"
  actual=$(sha256_file "$staged")
  [[ "$actual" == "${DOWNLOAD_HASHES[$i]}" ]] || {
    echo "Error: downloaded Tauri tool SHA-256 mismatch: ${NAMES[$i]}" >&2
    exit 1
  }
done

for ((i = 0; i < ${#NAMES[@]}; i++)); do
  target="$TOOLS_DIR/${NAMES[$i]}"
  [[ ! -L "$target" ]] || {
    echo "Error: refusing to replace symlinked Tauri tool: ${NAMES[$i]}" >&2
    exit 1
  }
  install -m 0755 "$STAGE_DIR/${NAMES[$i]}" "$target"
done

verify_cache
echo "Installed ${#NAMES[@]} pinned Tauri AppImage tools in $TOOLS_DIR"
