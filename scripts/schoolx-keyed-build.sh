#!/usr/bin/env bash
#
# Build a SchoolX desktop installer with the LLM provider, model, and API key
# baked in, so teammates only have to install the app and create a key.
#
# This is the same mechanism internal Block builds use: `desktop/src-tauri/
# build.rs` reads BUZZ_BUILD_* at compile time and `option_env!` embeds the
# values in the binary (see managed_agents/agent_env.rs). Baked values are the
# lowest precedence layer, so a teammate can still override them in Settings.
#
#   ANTHROPIC_API_KEY=sk-ant-... SCHOOLX_MODEL=<model-id> \
#     ./scripts/schoolx-keyed-build.sh
#
# The key is read from the environment and never written to the repo. Prefer
# feeding it from a password manager over typing it inline, e.g.
#
#   ANTHROPIC_API_KEY="$(op read op://private/anthropic/key)" \
#     SCHOOLX_MODEL=<model-id> ./scripts/schoolx-keyed-build.sh
#
# ┌──────────────────────────────────────────────────────────────────────────┐
# │ THE RESULTING INSTALLER CONTAINS THE KEY.                                │
# │ It is base64 inside the binary — `strings` plus a decode recovers it.    │
# │ Hand the file to teammates directly (USB, AirDrop, private drive).       │
# │ Do NOT upload it to GitHub Actions artifacts, a Release, or any public   │
# │ location: this fork is a PUBLIC repository.                             │
# │ Use a spend-limited key you can revoke.                                  │
# └──────────────────────────────────────────────────────────────────────────┘
#
# Options via environment:
#   ANTHROPIC_API_KEY  (required) the key to bake in
#   SCHOOLX_MODEL      (required) Anthropic model id. buzz-agent errors with
#                      "config: ANTHROPIC_MODEL required" when unset, so there
#                      is deliberately no default to guess wrong.
#   SCHOOLX_RELAY_URL  (optional) pre-point the app at your relay so teammates
#                      never type a relay address
#   SCHOOLX_TARGET     (optional) rust target triple; defaults to this host.
#                      Intel Macs: x86_64-apple-darwin
#   SCHOOLX_VERSION    (optional) version suffix; defaults to <base>-team.local

set -euo pipefail

cd "$(dirname "$0")/.."
repo_root=$(pwd)

die() {
    echo "error: $*" >&2
    exit 1
}

[[ -n "${ANTHROPIC_API_KEY:-}" ]] ||
    die "ANTHROPIC_API_KEY is not set. See the header of this script."
[[ -n "${SCHOOLX_MODEL:-}" ]] ||
    die "SCHOOLX_MODEL is not set (e.g. SCHOOLX_MODEL=claude-sonnet-4-6). buzz-agent requires an explicit Anthropic model."

# Newlines would forge extra KEY=VALUE pairs in the baked env blob.
[[ "$ANTHROPIC_API_KEY" != *$'\n'* ]] || die "ANTHROPIC_API_KEY must not contain a newline"
[[ "$SCHOOLX_MODEL" != *$'\n'* ]] || die "SCHOOLX_MODEL must not contain a newline"

host_target=$(rustc -vV | sed -n 's|host: ||p')
target="${SCHOOLX_TARGET:-$host_target}"

base_version=$(node -p "require('./desktop/package.json').version")
version="${SCHOOLX_VERSION:-${base_version%%-*}-team.local}"

echo "SchoolX keyed build"
echo "  target   : $target"
echo "  version  : $version"
echo "  provider : anthropic"
echo "  model    : $SCHOOLX_MODEL"
echo "  api key  : (set, ${#ANTHROPIC_API_KEY} chars — never printed)"
[[ -n "${SCHOOLX_RELAY_URL:-}" ]] && echo "  relay    : $SCHOOLX_RELAY_URL"
echo

# ── frontend deps and version ────────────────────────────────────────────────
pnpm install --frozen-lockfile
(cd desktop && node scripts/set-version-from-tag.mjs "$version")

# ── sidecars ─────────────────────────────────────────────────────────────────
# Pair rule (same as .github/workflows/schoolx-team-build.yml): a --target build
# lands under target/<triple>/release, so bundle-sidecars.sh gets the triple; a
# host build lands under target/release and it gets no argument.
sidecars=(-p buzz-acp -p buzz-agent -p buzz-dev-mcp -p git-credential-nostr -p buzz-cli)
if [[ "$target" == "$host_target" ]]; then
    cargo build --release "${sidecars[@]}"
    ./scripts/bundle-sidecars.sh
else
    rustup target add "$target"
    cargo build --release --target "$target" "${sidecars[@]}"
    ./scripts/bundle-sidecars.sh "$target"
fi

# ── bundle config: no updater artifacts (there is no endpoint to serve them) ──
cat >desktop/src-tauri/tauri.team.conf.json <<'JSON'
{
  "bundle": {
    "createUpdaterArtifacts": false
  }
}
JSON

# ── the actual bake ──────────────────────────────────────────────────────────
# build.rs base64-encodes BUZZ_BUILD_AGENT_ENV and re-exports it as
# BUZZ_DESKTOP_BUILD_AGENT_ENV for option_env!.
export BUZZ_BUILD_BUZZ_AGENT_PROVIDER="anthropic"
export BUZZ_BUILD_BUZZ_AGENT_MODEL="$SCHOOLX_MODEL"
export BUZZ_BUILD_AGENT_ENV="ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}"
if [[ -n "${SCHOOLX_RELAY_URL:-}" ]]; then
    export BUZZ_BUILD_RELAY_URL="$SCHOOLX_RELAY_URL"
    export BUZZ_BUILD_AUTO_CONNECT_DEFAULT_RELAY="true"
fi

# `cargo:rustc-env` only reaches the binary if the crate actually recompiles.
# A desktop crate left over from an earlier build can be judged fresh, in which
# case build.rs re-runs and emits the new values while tauri bundles the stale
# binary — a keyless installer that looks like a success. Touching the file that
# holds the `option_env!` calls forces the expansion to happen again.
touch desktop/src-tauri/src/managed_agents/agent_env.rs

build_args=(--verbose --no-sign --config src-tauri/tauri.team.conf.json)
[[ "$target" == "$host_target" ]] || build_args+=(--target "$target")
(cd desktop && pnpm tauri build "${build_args[@]}")

# ── locate the installer ─────────────────────────────────────────────────────
if [[ "$target" == "$host_target" ]]; then
    bundle_dir="desktop/src-tauri/target/release/bundle"
else
    bundle_dir="desktop/src-tauri/target/${target}/release/bundle"
fi

installer=$(find "$bundle_dir" \( -name '*.dmg' -o -name '*setup.exe' \) -type f | head -1 || true)
[[ -n "$installer" ]] || die "no installer found under $bundle_dir"

# ── prove the key is actually in the binary ──────────────────────────────────
# Without this the failure is silent: teammates install, open Settings, and find
# an empty API key field with nothing to explain it.
#
# The blob is base64 of "ANTHROPIC_API_KEY=<key>". "ANTHROPIC_API_KEY=" is 18
# bytes — a whole number of 3-byte base64 groups — so its encoding is a stable
# prefix of the encoded blob regardless of the key that follows. Matching the
# prefix proves the bake landed without putting the key on screen.
if [[ "$target" == "$host_target" ]]; then
    built_binary="desktop/src-tauri/target/release/buzz"
else
    built_binary="desktop/src-tauri/target/${target}/release/buzz"
fi
blob_prefix=$(printf 'ANTHROPIC_API_KEY=' | base64 | cut -c1-20)

if [[ ! -f "$built_binary" ]]; then
    die "cannot verify the bake: $built_binary is missing"
fi

if ! grep -aq "$blob_prefix" "$built_binary"; then
    die "the API key is NOT in the built binary — do not ship $installer

build.rs emitted the values but the desktop crate was not recompiled, so the
bundle carries a stale binary. Force a rebuild and run this script again:

    cargo clean -p buzz-desktop --release --manifest-path desktop/src-tauri/Cargo.toml"
fi

echo "verified: the baked provider, model, and key are present in the binary."

echo
echo "Built: ${repo_root}/${installer}"
echo
echo "This file contains the API key. Hand it over directly — do not upload it"
echo "to Actions artifacts, a Release, or anywhere public."
