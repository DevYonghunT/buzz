#!/usr/bin/env bash
# Build the release-profile desktop lib test, then prove the pinned Linux
# descriptor-bound Git launcher uses vfork semantics as a non-root user.
set -euo pipefail

readonly PINNED_CONTAINER_IMAGE="ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90"
readonly EXPECTED_RUST_RELEASE="1.95.0"
readonly EXPECTED_GLIBC="glibc 2.39"
readonly NONROOT_UID="${SCHOOLX_LAUNCHER_TEST_UID:-10001}"
readonly NONROOT_GID="${SCHOOLX_LAUNCHER_TEST_GID:-10001}"
readonly TEST_NAME="code_workspace::git_launch::tests::renamed_and_replaced_path_still_launches_in_opened_directory"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
verifier="$script_dir/linux_git_launcher_trace.py"
verifier_tests="$script_dir/test_linux_git_launcher_trace.py"

fail() {
  echo "linux Git launcher release gate: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable: $1"
}

require_pinned_tuple() {
  [[ "$(uname -s)" == "Linux" ]] || fail "requires native Linux"
  [[ "$(uname -m)" == "x86_64" ]] || fail "requires native x86_64; got $(uname -m)"
  [[ -r /etc/os-release ]] || fail "/etc/os-release is unavailable"
  # shellcheck disable=SC1091
  source /etc/os-release
  [[ "${ID:-}" == "ubuntu" && "${VERSION_ID:-}" == "24.04" ]] ||
    fail "requires Ubuntu 24.04; got ${ID:-unknown} ${VERSION_ID:-unknown}"
  [[ "$(dpkg --print-architecture)" == "amd64" ]] ||
    fail "requires dpkg architecture amd64"
  [[ "$(getconf GNU_LIBC_VERSION)" == "$EXPECTED_GLIBC" ]] ||
    fail "requires $EXPECTED_GLIBC; got $(getconf GNU_LIBC_VERSION)"
  rustc -vV | grep -Fxq "release: $EXPECTED_RUST_RELEASE" ||
    fail "requires Rust $EXPECTED_RUST_RELEASE"
  rustc -vV | grep -Fxq "host: x86_64-unknown-linux-gnu" ||
    fail "Rust host is not x86_64-unknown-linux-gnu"
  [[ "${SCHOOLX_LAUNCHER_CONTAINER_IMAGE:-}" == "$PINNED_CONTAINER_IMAGE" ]] ||
    fail "SCHOOLX_LAUNCHER_CONTAINER_IMAGE must name the pinned Ubuntu image"
  [[ "$(stat -f -c %T /proc)" == "proc" ]] || fail "/proc is not procfs"
}

record_hash() {
  local label="$1" path="$2" digest
  digest="$(sha256sum "$path" | awk '{print $1}')"
  printf '%s  %s\n' "$digest" "$label" >> "$evidence_dir/sha256sums.txt"
}

run_nonroot_gate() {
  local test_binary="$1"
  evidence_dir="$2"

  [[ "$(id -u)" -ne 0 ]] || fail "runtime/strace gate must not run as root"
  [[ "$(id -g)" -ne 0 ]] || fail "runtime/strace gate must not use root group"
  [[ -w "$evidence_dir" ]] || fail "evidence directory is not writable by the test user"
  [[ -x "$test_binary" ]] || fail "test executable is missing or not executable: $test_binary"

  require_command file
  require_command cargo
  require_command python3
  require_command rustc
  require_command sha256sum
  require_command strace
  require_pinned_tuple

  file -Lb "$test_binary" | grep -Eq 'ELF 64-bit LSB.*x86-64' ||
    fail "release test binary is not native x86-64 ELF"
  file -Lb /usr/bin/git | grep -Eq 'ELF 64-bit LSB.*x86-64' ||
    fail "/usr/bin/git is not native x86-64 ELF"
  [[ "$test_binary" == */release/deps/* ]] ||
    fail "test executable did not come from a release profile: $test_binary"

  : > "$evidence_dir/sha256sums.txt"
  record_hash test-binary.before "$test_binary"
  record_hash system-git.before /usr/bin/git

  {
    printf 'container_image=%s\n' "$SCHOOLX_LAUNCHER_CONTAINER_IMAGE"
    printf 'uid=%s\n' "$(id -u)"
    printf 'gid=%s\n' "$(id -g)"
    printf 'uname=%s\n' "$(uname -srm)"
    printf 'dpkg_architecture=%s\n' "$(dpkg --print-architecture)"
    printf 'glibc=%s\n' "$(getconf GNU_LIBC_VERSION)"
    printf 'proc_filesystem=%s\n' "$(stat -f -c %T /proc)"
    printf 'test_binary_file=%s\n' "$(file -Lb "$test_binary")"
    printf 'system_git_file=%s\n' "$(file -Lb /usr/bin/git)"
    cat /etc/os-release
    rustc -vV
    cargo -V
    printf 'rustc_path=%s\n' "$(command -v rustc)"
    printf 'cargo_path=%s\n' "$(command -v cargo)"
    /usr/bin/git --version
    strace --version | sed -n '1p'
  } > "$evidence_dir/environment.txt"

  local trace_prefix="$evidence_dir/trace"
  # Do not use strace -v here: verbose execve output can expand the inherited
  # environment into the uploaded evidence. The selected syscalls and argv are
  # sufficient for the launcher contract without exposing CI credentials.
  strace -ff -qq -s 4096 \
    -e trace=clone,clone3,fork,vfork,execve,chdir,setpgid \
    -o "$trace_prefix" \
    "$test_binary" --exact "$TEST_NAME" --nocapture --test-threads=1 \
    > "$evidence_dir/test.stdout" 2> "$evidence_dir/test.stderr"

  grep -Fq 'running 1 test' "$evidence_dir/test.stdout" ||
    fail "exact launcher test did not run exactly one test"
  grep -Eq 'test result: ok\. 1 passed; 0 failed; 0 ignored;' "$evidence_dir/test.stdout" ||
    fail "launcher runtime test did not pass exactly once"

  python3 "$verifier" verify "$trace_prefix" --output "$evidence_dir/trace-verdict.json"

  record_hash test-binary.after "$test_binary"
  record_hash system-git.after /usr/bin/git
  record_hash strace "$(command -v strace)"
  record_hash rustc-launcher "$(command -v rustc)"
  record_hash cargo-launcher "$(command -v cargo)"
  local rust_sysroot
  rust_sysroot="$(rustc --print sysroot)"
  record_hash rustc-driver "$rust_sysroot/bin/rustc"
  record_hash cargo-driver "$rust_sysroot/bin/cargo"
  record_hash runtime-gate "$script_dir/verify-linux-git-launcher-runtime.sh"
  record_hash trace-verifier "$verifier"
  record_hash workspace-cargo-lock "$repo_root/Cargo.lock"
  record_hash tauri-cargo-lock "$repo_root/desktop/src-tauri/Cargo.lock"
  record_hash rust-toolchain "$repo_root/rust-toolchain.toml"

  local before after
  before="$(awk '$2 == "test-binary.before" {print $1}' "$evidence_dir/sha256sums.txt")"
  after="$(awk '$2 == "test-binary.after" {print $1}' "$evidence_dir/sha256sums.txt")"
  [[ "$before" == "$after" ]] || fail "release test binary changed during the trace"
  before="$(awk '$2 == "system-git.before" {print $1}' "$evidence_dir/sha256sums.txt")"
  after="$(awk '$2 == "system-git.after" {print $1}' "$evidence_dir/sha256sums.txt")"
  [[ "$before" == "$after" ]] || fail "/usr/bin/git changed during the trace"

  find "$evidence_dir" -maxdepth 1 -type f -name 'trace.*' -print0 |
    sort -z | xargs -0 sha256sum > "$evidence_dir/trace-files.sha256"
  echo "Linux descriptor-bound Git release gate passed as UID $(id -u)"
}

if [[ "${1:-}" == "--run-nonroot" ]]; then
  [[ "$#" -eq 3 ]] || fail "internal non-root mode expects TEST_BINARY EVIDENCE_DIR"
  run_nonroot_gate "$2" "$3"
  exit 0
fi

[[ "$#" -eq 1 ]] || fail "usage: $0 EVIDENCE_DIR"
require_command cargo
require_command dpkg
require_command getconf
require_command just
require_command python3
require_command rustc
require_command sha256sum
require_pinned_tuple

if [[ -e "$1" ]]; then
  fail "evidence directory already exists: $1"
fi
mkdir -p "$1"
evidence_dir="$(cd "$1" && pwd)"
python3 "$verifier_tests" > "$evidence_dir/verifier-tests.txt"

cd "$repo_root"
just _ensure-sidecar-stubs
cargo test \
  --manifest-path desktop/src-tauri/Cargo.toml \
  --release \
  --lib \
  --no-run \
  --message-format=json-render-diagnostics \
  > "$evidence_dir/cargo-build.jsonl"
test_binary="$(python3 "$verifier" cargo-executable "$evidence_dir/cargo-build.jsonl")"
[[ -x "$test_binary" ]] || fail "Cargo-reported test executable is unavailable: $test_binary"

if [[ "$(id -u)" -eq 0 ]]; then
  require_command setpriv
  [[ "$NONROOT_UID" =~ ^[1-9][0-9]*$ ]] || fail "test UID must be a positive integer"
  [[ "$NONROOT_GID" =~ ^[1-9][0-9]*$ ]] || fail "test GID must be a positive integer"
  # Hermit keeps the Rust toolchain under .hermit. Grant traversal (not
  # directory listing) on that single ephemeral workspace directory so the
  # dropped-privilege process can execute and hash the exact rustc/cargo used
  # for the build. run_nonroot_gate proves access again after setpriv.
  if [[ -d "$repo_root/.hermit" ]]; then
    chmod o+x "$repo_root/.hermit"
  fi
  readonly parent_hermit_state_dir="${HERMIT_STATE_DIR:-${XDG_CACHE_HOME:-${HOME}/.cache}/hermit}"
  mkdir -p "$evidence_dir/home"
  chown -R "$NONROOT_UID:$NONROOT_GID" "$evidence_dir"
  setpriv \
    --reuid "$NONROOT_UID" \
    --regid "$NONROOT_GID" \
    --clear-groups \
    --no-new-privs \
    env HOME="$evidence_dir/home" \
      HERMIT_STATE_DIR="$parent_hermit_state_dir" \
      PATH="$PATH" rustc -vV \
    > "$evidence_dir/nonroot-hermit-rustc.txt" 2>&1
  exec setpriv \
    --reuid "$NONROOT_UID" \
    --regid "$NONROOT_GID" \
    --clear-groups \
    --no-new-privs \
    env HOME="$evidence_dir/home" \
      HERMIT_STATE_DIR="$parent_hermit_state_dir" \
      SCHOOLX_LAUNCHER_CONTAINER_IMAGE="$SCHOOLX_LAUNCHER_CONTAINER_IMAGE" \
      PATH="$PATH" \
      "$script_dir/verify-linux-git-launcher-runtime.sh" \
      --run-nonroot "$test_binary" "$evidence_dir"
fi

run_nonroot_gate "$test_binary" "$evidence_dir"
