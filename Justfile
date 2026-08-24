# Buzz — development task runner

set dotenv-load := true

desktop_dir := "desktop"
desktop_tauri_manifest := "desktop/src-tauri/Cargo.toml"
web_dir := "web"

# Opt-in mesh-llm. Off by default so `just dev`/`just staging`/`just production`
# skip ~420 extra crates + the llama.cpp native runtime build and stay fast to
# iterate on. Turn on to test mesh compute features: `just mesh=1 dev` /
# `just mesh=1 staging` / `just mesh=1 production`.
mesh := ""

# Reset only the current standalone desktop instance before launch.
# Usage: `just fresh=1 desktop-standalone`.
fresh := ""

# List all available tasks
default:
    @just --list

# ─── Dev Environment ─────────────────────────────────────────────────────────

# Install required dev tools via Hermit and create .env (safe to re-run)
bootstrap:
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    # Hermit's bin/ symlinks auto-download pinned tool versions on first use.
    # Running each tool once triggers the download if not already cached.
    echo "Ensuring toolchain via Hermit..."
    cargo --version &
    node --version &
    pnpm --version &
    wait
    if ! command -v docker &>/dev/null; then
        echo "Error: Docker is required but not installed."
        echo "Install it from https://docs.docker.com/get-docker/"
        exit 1
    fi
    if [[ ! -f .env ]]; then
        cp .env.example .env
        echo "Created .env from .env.example — review it before running just dev."
    fi

# Start Docker services, run migrations, install desktop deps
setup: bootstrap
    ./scripts/dev-setup.sh

# Install git hooks via lefthook (dispatches from the shared .git/hooks dir so all
# linked worktrees inherit the same hooks without a worktree-relative .hooks path)
hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    # Use the Hermit-pinned lefthook (bin/lefthook self-downloads on first use):
    # works with no pre-installed lefthook and guarantees the pinned version
    # rather than whatever happens to be on PATH.
    export PATH="{{justfile_directory()}}/bin:$PATH"
    # --path-format=absolute guarantees an absolute path from every invocation context:
    # without it, --git-common-dir returns ".git" from the main checkout and a
    # relative hooksPath would break linked-worktree dispatch just like .hooks did.
    HOOKS_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/hooks"
    git config --local core.hooksPath "$HOOKS_DIR"
    lefthook install --force

# Wipe development state and recreate a clean environment. Installed Buzz is preserved.
[confirm("This will DELETE all development data and preserve installed Buzz. Continue? (y/N)")]
reset:
    ./scripts/dev-reset.sh --yes

# Stop all dev services (keep data)
down:
    docker compose down

# Show dev service status
ps:
    docker compose ps

# Tail all service logs
logs *ARGS:
    docker compose logs -f {{ARGS}}

# ─── Build & Check ───────────────────────────────────────────────────────────

# Build the Rust workspace
build:
    cargo build --workspace

# Build the Rust workspace in release mode
build-release:
    cargo build --workspace --release

# Run repo lint, formatting, and repository policy checks
check: fmt-check clippy desktop-check desktop-tauri-fmt-check desktop-tauri-clippy web-check mobile-check file-size-check

# Run the repository-wide differential file-size ratchet and its policy tests.
# The ratchet inspects only files changed from the merge base, so this stays
# cheap enough to run unconditionally without duplicating path filters.
file-size-check:
    node --test scripts/check-file-sizes-core.test.mjs
    node desktop/scripts/check-file-sizes.mjs
    node web/scripts/check-file-sizes.mjs
    node mobile/scripts/check-file-sizes.mjs

# Format all Rust code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Run clippy with warnings as errors
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Install JS dependencies (pnpm workspace — installs all packages from root)
desktop-install:
    pnpm install

# Install JS dependencies reproducibly for CI (pnpm workspace)
desktop-install-ci:
    pnpm install --frozen-lockfile

# Run desktop lint and format checks
desktop-check:
    cd {{desktop_dir}} && pnpm check

# Fix desktop lint and format issues
desktop-fix:
    cd {{desktop_dir}} && pnpm exec biome check --write .

# Run desktop TS helper unit tests
desktop-test:
    cd {{desktop_dir}} && pnpm test

# Run desktop TypeScript checks
desktop-typecheck:
    cd {{desktop_dir}} && pnpm typecheck

# Build desktop frontend assets
desktop-build:
    cd {{desktop_dir}} && pnpm build

# Format desktop Tauri Rust code
desktop-tauri-fmt:
    cargo fmt --manifest-path {{desktop_tauri_manifest}} --all

# Check desktop Tauri Rust formatting
desktop-tauri-fmt-check:
    cargo fmt --manifest-path {{desktop_tauri_manifest}} --all -- --check

# Format all code (Rust + Tauri Rust + Dart)
fmt-all: fmt desktop-tauri-fmt mobile-fmt

# Fix all formatting and lint issues
fix-all: fmt desktop-tauri-fmt desktop-fix web-fix mobile-fix

# Ensure sidecar placeholder binaries exist (Tauri validates externalBin at compile time)
# Sidecar binary list must stay in sync with desktop-release-build below.
_ensure-sidecar-stubs:
    #!/usr/bin/env bash
    set -euo pipefail
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    mkdir -p desktop/src-tauri/binaries
    SIDECARS=(buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz)
    if [[ "$TARGET" != *windows* ]]; then
        SIDECARS+=(buzz-backend-kubernetes)
    fi
    for bin in "${SIDECARS[@]}"; do
        touch "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done

# Ensure Docker dev services (Postgres, Redis, etc.) are running and healthy
_ensure-services:
    #!/usr/bin/env bash
    set -euo pipefail
    pg=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' buzz-postgres 2>/dev/null || echo "not_found")
    redis=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' buzz-redis 2>/dev/null || echo "not_found")
    if [[ "$pg" == "healthy" && "$redis" == "healthy" ]]; then
        echo "Services already healthy"
        exit 0
    fi
    echo "Starting services..."
    docker compose up -d || true
    echo -n "Waiting for services"
    for i in $(seq 1 40); do
        pg=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' buzz-postgres 2>/dev/null || echo "not_found")
        redis=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' buzz-redis 2>/dev/null || echo "not_found")
        if [[ "$pg" == "healthy" && "$redis" == "healthy" ]]; then
            echo " ready"
            exit 0
        fi
        echo -n "."
        sleep 3
    done
    echo " timed out"
    exit 1

# Apply database migrations and seed the local dev community if the dev database is running
_ensure-migrations: _ensure-services
    cargo run -p buzz-admin -- migrate
    ./scripts/seed-local-community.sh

# Run clippy on the desktop Tauri Rust crate
desktop-tauri-clippy: _ensure-sidecar-stubs
    cargo clippy --manifest-path {{desktop_tauri_manifest}} --workspace --all-targets -- -D warnings

# Check the desktop Tauri Rust crate compiles
desktop-tauri-check: _ensure-sidecar-stubs
    cargo check --manifest-path {{desktop_tauri_manifest}}

# Run desktop Tauri Rust unit tests
desktop-tauri-test: _ensure-sidecar-stubs
    cd desktop/src-tauri && cargo test --workspace

# Run the native terminal latency gate explicitly on a known-idle host.
# This is intentionally excluded from shared CI: scheduler contention makes a
# wall-clock assertion flaky, and the release profile is the shipped shape.
desktop-terminal-performance-test:
    cargo test --manifest-path desktop/src-tauri/crates/buzz-terminal/Cargo.toml --release --test latency g3_renderer_acquire_stays_within_frame_budget -- --ignored --exact --nocapture

# Verify compiled-flag behavior under both compile states (clean + capability set).
# Runs the auto-connect and owner-only access focused tests twice with
# independently supplied expected values; build.rs rerun-if-env-changed
# triggers recompilation.
desktop-tauri-test-compiled-flags: _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    cd desktop/src-tauri
    echo "=== Clean build (no flag) → expect false ==="
    env -u BUZZ_BUILD_AUTO_CONNECT_DEFAULT_RELAY \
      BUZZ_TEST_EXPECTED_AUTO_CONNECT_DEFAULT_RELAY=false \
      cargo test compiled_flag_matches_expected -- --ignored --nocapture
    env -u BUZZ_BUILD_AGENT_ACCESS_OWNER_ONLY \
      BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY=false \
      cargo test --lib
    env -u BUZZ_BUILD_AGENT_ACCESS_OWNER_ONLY \
      BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY=false \
      cargo test compiled_policy_matches_expected -- --ignored --nocapture
    echo "=== Internal build (flags set) → expect true ==="
    BUZZ_BUILD_AUTO_CONNECT_DEFAULT_RELAY=1 \
      BUZZ_TEST_EXPECTED_AUTO_CONNECT_DEFAULT_RELAY=true \
      cargo test compiled_flag_matches_expected -- --ignored --nocapture
    BUZZ_BUILD_AGENT_ACCESS_OWNER_ONLY=1 \
      BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY=true \
      cargo test --lib
    BUZZ_BUILD_AGENT_ACCESS_OWNER_ONLY=1 \
      BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY=true \
      cargo test compiled_policy_matches_expected -- --ignored --nocapture
    echo "Both compiled states verified."

# Build the full desktop Tauri app locally (unsigned, for testing)
# Sidecar binary list must stay in sync with _ensure-sidecar-stubs above.
# pnpm install is unconditional here: release builds must start from a clean dep tree.
desktop-release-build target="aarch64-apple-darwin":
    #!/usr/bin/env bash
    set -euo pipefail
    TARGET={{target}}
    # This recipe always passes --target, so the XPC staging hook must select
    # the target-triple Cargo output even when a native release binary exists.
    export SCHOOLX_CODE_GIT_CARGO_LAYOUT=target-triple
    mkdir -p desktop/src-tauri/binaries
    touch "desktop/src-tauri/binaries/buzz-acp-$TARGET"
    touch "desktop/src-tauri/binaries/buzz-agent-$TARGET"
    if [[ "$TARGET" != *windows* ]]; then
        touch "desktop/src-tauri/binaries/buzz-backend-kubernetes-$TARGET"
    fi
    touch "desktop/src-tauri/binaries/buzz-dev-mcp-$TARGET"
    touch "desktop/src-tauri/binaries/git-credential-nostr-$TARGET"
    touch "desktop/src-tauri/binaries/buzz-$TARGET"
    pnpm install
    cd {{desktop_dir}} && pnpm tauri build --features mesh-llm --target {{target}}

# Run desktop checks suitable for CI / pre-push
desktop-ci: desktop-check desktop-test desktop-tauri-fmt-check desktop-build desktop-tauri-check desktop-tauri-test

# Seed deterministic channel data for desktop Playwright tests
desktop-e2e-seed: _ensure-migrations
    ./scripts/setup-desktop-test-data.sh

# Run desktop browser smoke tests
desktop-e2e-smoke:
    cd {{desktop_dir}} && pnpm test:e2e:smoke

# Run desktop relay-backed e2e tests
desktop-e2e-integration: _ensure-migrations
    cd {{desktop_dir}} && pnpm test:e2e:integration

# Run the deterministic desktop correctness smoke against an isolated local relay
desktop-release-smoke:
    ./scripts/run-desktop-release-smoke.sh

# Run only the e2e specs changed vs origin/main (both projects) before pushing
desktop-e2e-pre-push: _ensure-migrations
    git fetch origin main
    cd {{desktop_dir}} && pnpm build:e2e && pnpm exec playwright test --only-changed=origin/main

# Run all checks suitable for CI / pre-push (no infra needed)
ci: check test-unit desktop-test desktop-build desktop-tauri-check desktop-tauri-test web-build mobile-test

# ─── Test ─────────────────────────────────────────────────────────────────────

# Run all tests (unit + integration)
test:
    ./scripts/run-tests.sh all

# Run the relay-backed e2e suites (the #[ignore]d tests in buzz-test-client).
#
# `just test` runs `cargo test --test '*'` without `--ignored`, so these never
# execute there. They need a live relay, which needs MinIO in addition to
# Postgres and Redis — the relay's git object-store conformance probe aborts
# startup without an S3 backend. Boots the relay, waits for /health, runs the
# suites, and always tears the relay back down.
#
# Pass a suite name to narrow: `just test-e2e e2e_access_matrix`.
test-e2e suite="": _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    docker compose up -d minio minio-init
    cargo build -p buzz-relay
    # Run the built binary, not `cargo run`: killing cargo leaves the relay it
    # spawned running, which then holds port 3000 against the next invocation.
    relay_log="$(mktemp -t buzz-relay-e2e)"
    echo "relay log: ${relay_log}"
    ./target/debug/buzz-relay >"${relay_log}" 2>&1 &
    relay_pid=$!
    trap 'kill "${relay_pid}" 2>/dev/null || true; wait "${relay_pid}" 2>/dev/null || true' EXIT
    echo -n "Waiting for relay"
    for _ in $(seq 1 60); do
        if curl -fsS http://localhost:3000/health >/dev/null 2>&1; then
            echo " ready"
            break
        fi
        if ! kill -0 "${relay_pid}" 2>/dev/null; then
            echo " relay exited during startup:" >&2
            tail -20 "${relay_log}" >&2
            exit 1
        fi
        echo -n "."
        sleep 2
    done
    curl -fsS http://localhost:3000/health >/dev/null
    # --test-threads=1 keeps the "nothing was delivered" assertions honest:
    # concurrent suites share one relay and one community.
    if [ -n "{{suite}}" ]; then
        RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client \
            --test {{suite}} -- --ignored --test-threads=1
    else
        RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client \
            --tests -- --ignored --test-threads=1
    fi

# Run unit tests only (no infra needed)
test-unit:
    #!/usr/bin/env bash
    # 이 파일의 다른 셰방 레시피 33개와 같다. 이게 없으면 마지막 명령의 종료
    # 코드만 전파되어 앞선 그룹의 실패가 조용히 묻힌다 — 그룹을 하나 덧붙일
    # 때마다 그전까지 게이트 역할을 하던 그룹이 가려진다.
    set -euo pipefail
    if command -v cargo-nextest &>/dev/null; then
        cargo nextest run -p buzz-core -p buzz-auth --lib
        cargo nextest run -p buzz-voice --lib
        cargo nextest run -p buzz-cli
        # buzz-db migrator/lint tests: pure SQL-parsing unit tests (no infra).
        # They guard the embedded-migrator invariant (exactly the consolidated
        # 0001; cutover/backfill stays an operator script, not startup state)
        # and the tenant-scoping lints. The Postgres-backed buzz-db tests are
        # #[ignore]d, so --lib runs only the infra-free set. Without this gate a
        # stray file in migrations/ or a broken lint ships green.
        cargo nextest run -p buzz-db --lib
        # Multi-tenant conformance gate (buzz-conformance): the independent
        # replay checker + golden fixtures. No infra — pure in-process trace
        # replay — so it belongs in the unit job. Run all targets (lib + the
        # tests/replay_fixtures.rs integration test), not just --lib.
        cargo nextest run -p buzz-conformance
        # Gateway unit and black-box HTTP tests are infra-free. Postgres-backed
        # contract/race tests run in the dedicated CI job below.
        cargo nextest run -p buzz-push-gateway
        # schoolx-catalog: catalog/channel-id/provenance unit tests, including
        # the compile-time assert that guards against KIND_WORKSPACE_PROVENANCE
        # drifting out of sync with buzz-core's independently declared copy.
        # Pure Rust, no Postgres/Redis, so it belongs in the infra-free unit job.
        cargo nextest run -p schoolx-catalog --lib
        # Kubernetes backend provider: the decision layers (state machine, GC
        # planner, env precedence, naming, wire) are pure functions with a fake
        # substrate, so they belong in the unit job. Enumerated explicitly
        # because nothing in CI runs `cargo test --workspace` — workspace
        # membership alone buys clippy/check, not a single executed test.
        cargo nextest run -p buzz-backend-kubernetes
        # buzz-agent model-capabilities corpus: the Rust half of the
        # cross-language drift guard. `model_capabilities.rs` embeds
        # scripts/model-capabilities.json + scripts/normative-corpus.json via
        # include_str! and replays the full locked corpus as pure in-process tests (no
        # infra). Enumerated explicitly because nothing in CI runs
        # `cargo test --workspace`; without this step a manifest edit that
        # diverges Rust from the corpus ships green.
        cargo nextest run -p buzz-agent --lib
    else
        ./scripts/run-tests.sh unit
    fi

# Run integration tests only (starts services if needed)
test-integration:
    ./scripts/run-tests.sh integration

# Regenerate the model-capability normative corpus from the production Rust
# resolver. The corpus is a golden snapshot, never hand-edited: this runs the
# `#[ignore]`d writer test in buzz-agent, which serializes `resolve()` over the
# inputs-only question table to scripts/normative-corpus.json. Run this after
# any model-capabilities.json edit, then commit the regenerated file. The
# `corpus_matches_generated_snapshot` gate fails CI if the committed file drifts.
regen-model-corpus:
    cargo test -p buzz-agent --lib model_capabilities::tests::regen_corpus_file -- --ignored --exact

# Buzz shared compute e2e: current desktop discovery/admission logic and
# Playwright UI coverage.
mesh-e2e:
    cargo test --manifest-path {{desktop_dir}}/src-tauri/Cargo.toml --features mesh-llm mesh_llm --lib
    cd {{desktop_dir}} && pnpm test:e2e:smoke -- mesh-compute.spec.ts

# Reset only development state, seed deterministic local channels, and launch
# the mesh-enabled desktop with the repository's public Tyler test identity.
# This is for local verification only; never point this identity at staging/prod.
[confirm("This will reset development data, preserve installed Buzz, then launch a seeded mesh dev app. Continue? (y/N)")]
mesh-dev-fresh:
    #!/usr/bin/env bash
    set -euo pipefail
    ./scripts/dev-reset.sh --yes
    ./scripts/setup-desktop-test-data.sh
    export BUZZ_PRIVATE_KEY="3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03"
    export BUZZ_REQUIRE_RELAY_MEMBERSHIP=true
    export BUZZ_ALLOW_NIP_OA_AUTH=true
    export RELAY_OWNER_PUBKEY="e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34"
    export BUZZ_RELAY_PRIVATE_KEY="0000000000000000000000000000000000000000000000000000000000000001"
    export BUZZ_RECONCILE_CHANNELS=true
    export BUZZ_RESET_WEBVIEW_STATE=1
    exec just mesh=1 dev

# Real serve->client->inference on this machine (not CI).
mesh-e2e-hardware:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -p buzz-relay --example mesh_serve_client_smoke

# Three isolated node processes: trusted member joins and infers; stranger is rejected.
# Uses temp homes and explicit mesh owner keystores. Never reads the Buzz Keychain.
mesh-e2e-admission:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -p buzz-relay --example mesh_admission_smoke

# Full hardware confidence suite: routing, owner admission, and real agent inference.
mesh-e2e-confidence:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p buzz-agent -p buzz-dev-mcp
    cargo run -p buzz-relay --example mesh_serve_client_smoke
    cargo run -p buzz-relay --example mesh_admission_smoke
    cargo run -p buzz-relay --example mesh_agent_e2e

# Take desktop screenshots using the mock bridge
desktop-screenshot *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    pnpm -C {{desktop_dir}} build:e2e
    cd {{desktop_dir}}
    if ! curl -sf http://127.0.0.1:4173/ >/dev/null 2>&1; then
        python3 -m http.server 4173 -d dist >/dev/null 2>&1 &
        trap "kill $! 2>/dev/null || true" EXIT
        for i in $(seq 1 20); do curl -sf http://127.0.0.1:4173/ >/dev/null && break; sleep 0.5; done
    fi
    node tests/helpers/screenshot.mjs {{ARGS}}

# ─── Run ──────────────────────────────────────────────────────────────────────

# Start the relay server (auto-starts Docker services if needed)
relay: bootstrap _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    cargo run -p buzz-relay

# Start the relay with the built web UI served from it
relay-web: bootstrap _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    [[ -d node_modules ]] || pnpm install
    pnpm -C web build
    BUZZ_WEB_DIR=./web/dist cargo run -p buzz-relay

# Build and run the private read-only admin dashboard
admin: bootstrap _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    [[ -d node_modules ]] || pnpm install
    pnpm -C admin-web build
    export BUZZ_ADMIN_HOST="${BUZZ_ADMIN_HOST:-admin.localhost:3000}"
    export BUZZ_ADMIN_WEB_DIR="${BUZZ_ADMIN_WEB_DIR:-{{justfile_directory()}}/admin-web/dist}"
    echo "Admin dashboard: http://${BUZZ_ADMIN_HOST}/reports"
    cargo run -p buzz-relay

# Seed deterministic reports and product feedback for local admin dashboard review
admin-seed: _ensure-migrations
    ./scripts/seed-admin-dashboard.sh

# Run focused relay and browser checks for the read-only admin dashboard
admin-check: fmt-check
    cargo check -p buzz-relay --all-targets
    cargo test -p buzz-relay api::admin
    cargo test -p buzz-relay router::tests
    pnpm -C admin-web check
    pnpm -C admin-web exec playwright test

# Start the relay server in release mode
relay-release: _ensure-migrations
    cargo run -p buzz-relay --release


# Run the desktop Tauri app in dev mode with a local relay (ports and identity derived from worktree)
dev *ARGS: bootstrap _ensure-sidecar-stubs _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    bind_addr="${BUZZ_BIND_ADDR:-0.0.0.0:3000}"
    relay_port="${bind_addr##*:}"; [[ -n "$relay_port" ]] || relay_port=3000
    health_port="${BUZZ_HEALTH_PORT:-8080}"
    metrics_port="${BUZZ_METRICS_PORT:-9102}"
    if command -v lsof >/dev/null 2>&1; then
        for spec in "relay:$relay_port" "health:$health_port" "metrics:$metrics_port"; do
            name="${spec%%:*}"; port="${spec##*:}"
            if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
                echo "Error: $name port $port is already in use; refusing to launch desktop against a stale relay." >&2
                lsof -nP -iTCP:"$port" -sTCP:LISTEN >&2 || true
                echo "Stop the process above (often a stale buzz-relay) and rerun: just dev" >&2
                exit 1
            fi
        done
    fi
    cargo build -p buzz-acp -p buzz-agent -p buzz-backend-kubernetes -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr -p buzz-relay
    # Docker Desktop's forwarded MinIO port can stall under the deployment
    # probe's 32 concurrent writers. Keep the gate enabled in local dev, using
    # the bounded profile already used by the relay test launcher.
    export BUZZ_GIT_PROBE_WRITERS="${BUZZ_GIT_PROBE_WRITERS:-8}"
    export BUZZ_GIT_PROBE_ROUNDS="${BUZZ_GIT_PROBE_ROUNDS:-2}"
    ./target/debug/buzz-relay &
    RELAY_PID=$!
    cleanup() {
        [[ -n "${INSTANCE_ID:-}" ]] && ../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true
        kill "$RELAY_PID" 2>/dev/null || true
    }
    trap cleanup EXIT
    relay_ready=false
    for _ in $(seq 1 120); do
        if ! kill -0 "$RELAY_PID" 2>/dev/null; then
            echo "Error: buzz-relay exited during startup; refusing to launch desktop." >&2
            wait "$RELAY_PID" || true
            exit 1
        fi
        if curl --silent --fail --max-time 1 "http://127.0.0.1:${health_port}/_readiness" >/dev/null; then
            relay_ready=true
            break
        fi
        sleep 0.5
    done
    if [[ "$relay_ready" != true ]]; then
        echo "Error: buzz-relay did not become healthy within 60 seconds; refusing to launch desktop." >&2
        exit 1
    fi
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    source ../scripts/instance-env.sh
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.BUZZ_TAURI_CONFIG).identifier)")
    echo "Starting on Vite port ${BUZZ_VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    FEATURES=(); [[ -n "{{mesh}}" ]] && FEATURES=(--features mesh-llm)
    pnpm exec tauri dev ${FEATURES[@]+"${FEATURES[@]}"} --config "$BUZZ_TAURI_CONFIG" {{ARGS}}

# Run only the desktop app. No relay, database, Docker, migrations, or .env are needed.
# The app opens normally and asks for a community before making a relay connection.
desktop-standalone *ARGS: _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    cargo build -p buzz-acp -p buzz-agent -p buzz-backend-kubernetes -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")
    for bin in buzz-acp buzz-agent buzz-backend-kubernetes buzz-dev-mcp git-credential-nostr buzz; do
        cp "${TARGET_DIR}/debug/${bin}" "desktop/src-tauri/binaries/${bin}-${TARGET}"
        chmod +x "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    unset BUZZ_PRIVATE_KEY BUZZ_SHARE_IDENTITY
    if [[ -n "{{fresh}}" ]]; then
        export BUZZ_RESET_WEBVIEW_STATE=1
    fi
    source ../scripts/instance-env.sh
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.BUZZ_TAURI_CONFIG).identifier)")
    export BUZZ_DEV_KEYRING_SERVICE="buzz-desktop-dev.${BUZZ_INSTANCE_SLUG:-main}"
    if [[ -n "{{fresh}}" ]]; then
        ../scripts/reset-desktop-standalone-state.sh "$INSTANCE_ID" "$BUZZ_DEV_KEYRING_SERVICE"
    fi
    trap '../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true' EXIT
    echo "Starting standalone desktop on Vite port ${BUZZ_VITE_PORT}; no relay services were started"
    pnpm exec tauri dev --config "$BUZZ_TAURI_CONFIG" {{ARGS}}

# Run the desktop app against the internal staging relay (installs deps + builds agent tools automatically)
staging *ARGS: bootstrap _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    pnpm install  # unconditional: staging must always start with a clean dep tree
    cargo build --release -p buzz-acp -p buzz-agent -p buzz-backend-kubernetes -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr
    FEATURES=()
    if [[ -n "{{mesh}}" ]]; then
        FEATURES=(--features mesh-llm)
    fi
    # Replace 0-byte sidecar stubs with real binaries so tauri dev picks them up.
    # buzz: the CLI sidecar. buzz-backend-kubernetes: provider discovery scans the
    # exe dir for executable buzz-backend-* files, so the non-executable stub that
    # tauri dev copies next to the exe would hide the provider from "Run on".
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")
    STAGING_SIDECARS=(buzz)
    if [[ "$TARGET" != *windows* ]]; then
        STAGING_SIDECARS+=(buzz-backend-kubernetes)
    fi
    for bin in "${STAGING_SIDECARS[@]}"; do
        cp "${TARGET_DIR}/release/${bin}" "desktop/src-tauri/binaries/${bin}-${TARGET}"
        chmod +x "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done
    cd {{desktop_dir}}
    export BUZZ_RELAY_URL="wss://sprout-oss.stage.blox.sqprod.co"
    source ../scripts/instance-env.sh
    # Ctrl+C kills the Tauri app before its in-process sweep finishes, leaking
    # agent workers. Reap this instance's agents on exit as a backstop.
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.BUZZ_TAURI_CONFIG).identifier)")
    trap '../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true' EXIT
    echo "Starting staging on Vite port ${BUZZ_VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    pnpm exec tauri dev ${FEATURES[@]+"${FEATURES[@]}"} --config "$BUZZ_TAURI_CONFIG" {{ARGS}}

# Run the desktop app against the production relay (installs deps + builds agent tools automatically)
production *ARGS: bootstrap _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    pnpm install  # unconditional: production must always start with a clean dep tree
    cargo build --release -p buzz-acp -p buzz-agent -p buzz-backend-kubernetes -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr
    FEATURES=()
    if [[ -n "{{mesh}}" ]]; then
        FEATURES=(--features mesh-llm)
    fi
    # Replace 0-byte sidecar stubs with real binaries so tauri dev picks them up.
    # buzz: the CLI sidecar. buzz-backend-kubernetes: provider discovery scans the
    # exe dir for executable buzz-backend-* files, so the non-executable stub that
    # tauri dev copies next to the exe would hide the provider from "Run on".
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")
    PRODUCTION_SIDECARS=(buzz)
    if [[ "$TARGET" != *windows* ]]; then
        PRODUCTION_SIDECARS+=(buzz-backend-kubernetes)
    fi
    for bin in "${PRODUCTION_SIDECARS[@]}"; do
        cp "${TARGET_DIR}/release/${bin}" "desktop/src-tauri/binaries/${bin}-${TARGET}"
        chmod +x "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done
    cd {{desktop_dir}}
    export BUZZ_RELAY_URL="wss://buzz.block.builderlab.xyz"
    source ../scripts/instance-env.sh
    # Ctrl+C kills the Tauri app before its in-process sweep finishes, leaking
    # agent workers. Reap this instance's agents on exit as a backstop.
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.BUZZ_TAURI_CONFIG).identifier)")
    trap '../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true' EXIT
    echo "Starting production on Vite port ${BUZZ_VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    pnpm exec tauri dev ${FEATURES[@]+"${FEATURES[@]}"} --config "$BUZZ_TAURI_CONFIG" {{ARGS}}

# Run the desktop frontend dev server (port derived from worktree)
desktop-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    source ../scripts/instance-env.sh
    echo "Starting frontend dev server on Vite port ${BUZZ_VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    pnpm exec vite --port "${BUZZ_VITE_PORT}" --strictPort

# ─── Web ─────────────────────────────────────────────────────────────────────

# Run the web frontend dev server (port derived from worktree to avoid collisions)
web:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ -d node_modules ]] || pnpm install
    source scripts/instance-env.sh
    export VITE_PORT=$((BUZZ_VITE_PORT + 100))
    export VITE_RELAY_URL="${BUZZ_RELAY_URL}"
    echo "Starting web dev server on port ${VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    cd {{web_dir}}
    pnpm exec vite --port "${VITE_PORT}" --strictPort

# Run web lint and format checks
web-check:
    cd {{web_dir}} && pnpm check

# Fix web lint and format issues
web-fix:
    cd {{web_dir}} && pnpm exec biome check --write .

# Run web TypeScript checks
web-typecheck:
    cd {{web_dir}} && pnpm typecheck

# Build web frontend assets
web-build:
    cd {{web_dir}} && pnpm build

# Run web browser smoke tests
web-e2e-smoke:
    cd {{web_dir}} && pnpm test:e2e:smoke

# ─── Mobile ──────────────────────────────────────────────────────────────────

mobile_dir := "mobile"

# Install mobile Flutter dependencies
mobile-install:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && flutter pub get

# Format all Dart code
mobile-fmt:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && dart format .

# Fix mobile formatting and run analysis
mobile-fix:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && dart format . && flutter analyze

# Run mobile lint and format checks
mobile-check:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && dart format --output=none --set-exit-if-changed . && flutter analyze

# Run mobile tests
mobile-test:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && flutter test

# Regenerate the emoji dataset asset from desktop's emoji-mart install.
# Output is committed — rerun after bumping @emoji-mart/data.
mobile-emoji-data:
    node {{mobile_dir}}/scripts/generate-emoji-data.mjs

# Compile an unsigned Android debug APK (worktree-aware debug identity)
mobile-build-android:
    ./scripts/mobile-worktree-overrides.sh
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && flutter build apk --debug --no-pub

# Run the mobile app on iOS simulator (worktree-aware debug identity)
mobile-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! pgrep -x Simulator &>/dev/null; then
        open -a Simulator
        sleep 3
    fi
    ./scripts/mobile-worktree-overrides.sh
    cd {{mobile_dir}}
    unset GIT_DIR GIT_WORK_TREE
    flutter run

# Uninstall stale worktree-suffixed Buzz debug installs (production apps kept)
mobile-clean:
    ./scripts/mobile-worktree-clean.sh

# ─── Database ─────────────────────────────────────────────────────────────────

# Apply database migrations
migrate: _ensure-migrations

# ─── Utilities ────────────────────────────────────────────────────────────────

# Remove build artifacts
clean:
    cargo clean
    cargo clean --manifest-path desktop/src-tauri/Cargo.toml

# Check the Rust workspace compiles without producing binaries
check-compile:
    cargo check --workspace --all-targets

# ─── Release ─────────────────────────────────────────────────────────────────

# Read the current desktop version from package.json
get-current-version:
    @node -p "require('./desktop/package.json').version"

# Read the current relay version from its crate manifest
get-current-relay-version:
    @grep -m1 '^version = ' crates/buzz-relay/Cargo.toml | sed -E 's/version = "(.*)"/\1/'

# Compute next minor version (e.g., 0.3.0 → 0.4.0)
get-next-minor-version:
    @python3 -c "v='$(just get-current-version)'.split('.'); print(f'{v[0]}.{int(v[1])+1}.0')"

# Compute next patch version (e.g., 0.3.0 → 0.3.1)
get-next-patch-version:
    @python3 -c "v='$(just get-current-version)'.split('.'); print(f'{v[0]}.{v[1]}.{int(v[2])+1}')"

# Compute next relay patch version (e.g., 0.3.0 → 0.3.1)
get-next-relay-patch-version:
    @python3 -c "v='$(just get-current-relay-version)'.split('.'); print(f'{v[0]}.{v[1]}.{int(v[2])+1}')"

# Update version in desktop package manifests and regenerate lockfiles
bump-desktop-version version:
    #!/usr/bin/env bash
    set -euo pipefail
    # desktop/package.json
    cd desktop && npm pkg set "version={{ version }}" && cd ..
    # desktop/src-tauri/tauri.conf.json
    node -e "
        const fs = require('fs');
        const p = 'desktop/src-tauri/tauri.conf.json';
        const c = JSON.parse(fs.readFileSync(p, 'utf8'));
        c.version = '{{ version }}';
        fs.writeFileSync(p, JSON.stringify(c, null, 2) + '\n');
    "
    # JSON.stringify expands arrays/objects in a way biome rejects; reformat to match.
    (cd desktop && pnpm exec biome format --write src-tauri/tauri.conf.json)
    # desktop/src-tauri/Cargo.toml — only first version line (under [package])
    node -e "
        const fs = require('fs');
        const p = 'desktop/src-tauri/Cargo.toml';
        let t = fs.readFileSync(p, 'utf8');
        t = t.replace(/^version = \".*\"/m, 'version = \"{{ version }}\"');
        fs.writeFileSync(p, t);
    "
    # Regenerate lockfiles
    pnpm install --lockfile-only
    cargo update -p buzz-desktop --manifest-path desktop/src-tauri/Cargo.toml
    echo "Bumped desktop manifests to {{ version }} and regenerated lockfiles"

# Bump the relay crate version and regenerate the lockfile
bump-relay-version version:
    #!/usr/bin/env bash
    set -euo pipefail
    # buzz-relay carries its own `version =` (not version.workspace), so the
    # replace targets the package version line only.
    perl -i -pe 's/^version = ".*"/version = "{{ version }}"/' crates/buzz-relay/Cargo.toml
    cargo update -p buzz-relay
    echo "Bumped buzz-relay to {{ version }} and regenerated Cargo.lock"

# Open or update the desktop release PR from an immutable origin/main snapshot
release-desktop *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    ARG="{{ ARGS }}"
    if [[ -z "$ARG" || "$ARG" == "patch" ]]; then
        VERSION=$(just get-next-patch-version)
    else
        VERSION="$ARG"
    fi
    scripts/prepare-desktop-release.sh "$VERSION"

# Open or update the relay release PR (ghcr.io/block/buzz image)
release-relay *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    ARG="{{ ARGS }}"
    if [[ -z "$ARG" || "$ARG" == "patch" ]]; then
        VERSION=$(just get-next-relay-patch-version)
    else
        VERSION="$ARG"
    fi
    just _release-pr relay "$VERSION"

# Shared release-PR engine for desktop and relay. Mobile publishes immutable
# candidate tags directly from remote main instead of using metadata-only PRs.
_release-pr lane version:
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION="{{ version }}"
    if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
        echo "Error: '$VERSION' is not valid semver (expected X.Y.Z)"
        exit 1
    fi
    # Lane-specific identifiers. The bump command runs after the branch switch.
    case "{{ lane }}" in
        desktop)
            BRANCH_PREFIX="version-bump"
            TAG_FETCH='v*'
            TAG_MATCH='v[0-9]*'
            TAG_EXCLUDE='*-*'
            TAG_PREFIX="v"
            CHANGELOG="CHANGELOG.md"
            ADD_FILES=(desktop/package.json desktop/src-tauri/tauri.conf.json desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock pnpm-lock.yaml CHANGELOG.md)
            LOG_PATHS=(desktop/ crates/buzz-core/ crates/buzz-persona/ crates/buzz-sdk/ crates/buzz-agent/)
            ARTIFACT="Buzz Desktop" ;;
        relay)
            BRANCH_PREFIX="relay-release"
            TAG_FETCH='relay-v*'
            TAG_MATCH='relay-v[0-9]*'
            TAG_EXCLUDE='relay-v*-*'
            TAG_PREFIX="relay-v"
            CHANGELOG="crates/buzz-relay/CHANGELOG.md"
            ADD_FILES=(crates/buzz-relay/Cargo.toml Cargo.lock crates/buzz-relay/CHANGELOG.md)
            LOG_PATHS=(crates/buzz-relay/ crates/buzz-core/ crates/buzz-db/ crates/buzz-auth/ crates/buzz-pubsub/ crates/buzz-search/ crates/buzz-audit/ crates/buzz-media/ crates/buzz-sdk/ crates/buzz-workflow/ crates/buzz-conformance/ migrations/)
            ARTIFACT="Buzz Relay" ;;
        *)
            echo "Error: unknown release lane '{{ lane }}'"
            exit 1 ;;
    esac
    echo "Preparing ${ARTIFACT} release v${VERSION}..."
    # Must run on main with a clean, up-to-date tree.
    CURRENT_BRANCH=$(git symbolic-ref --short HEAD)
    if [[ "$CURRENT_BRANCH" != "main" ]]; then
        echo "Error: must be on main branch (currently on '$CURRENT_BRANCH')"
        exit 1
    fi
    git fetch origin refs/heads/main:refs/remotes/origin/main --no-tags
    # Release tags are remote-owned state; sync only this lane's tags so stale
    # local tags from older histories do not make release preflight fail.
    git fetch origin "+refs/tags/${TAG_FETCH}:refs/tags/${TAG_FETCH}"
    if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
        echo "Error: local main is not up-to-date with origin/main. Run 'git pull' first."
        exit 1
    fi
    if ! git diff --quiet || ! git diff --cached --quiet; then
        echo "Error: working tree is dirty. Commit or stash changes first."
        exit 1
    fi
    # Switch to the release branch (create, or reset to main if it exists).
    BRANCH="${BRANCH_PREFIX}/${VERSION}"
    if git rev-parse --verify "refs/heads/$BRANCH" >/dev/null 2>&1; then
        echo "Branch '$BRANCH' already exists — resetting to origin/main..."
        git switch "$BRANCH"
        git reset --hard origin/main
    elif git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1; then
        echo "Branch '$BRANCH' exists on remote — checking out and resetting to origin/main..."
        git switch -c "$BRANCH" --track "origin/$BRANCH"
        git reset --hard origin/main
    else
        git switch -c "$BRANCH"
    fi
    # Lane-specific bump (the one diverging step).
    case "{{ lane }}" in
        desktop) just bump-desktop-version "$VERSION" ;;
        relay)   just bump-relay-version "$VERSION" ;;
    esac
    # Generate the changelog from commits since this lane's last release tag.
    LAST_TAG=$(git describe --tags --abbrev=0 --match "$TAG_MATCH" --exclude "$TAG_EXCLUDE" 2>/dev/null || echo "")
    REPO=$(git remote get-url origin | sed -E 's|.*github\.com[:/]||; s|\.git$||')
    format_log() {
        local range="$1"
        git log "$range" --format="%h %H %s" --no-merges -- "${LOG_PATHS[@]}" | while IFS=' ' read -r short full rest; do
            local pr subject
            pr=$(printf '%s' "$rest" | grep -oE '\(#[0-9]+\)$' | grep -oE '[0-9]+' || true)
            if [[ -n "$pr" ]]; then
                subject=$(printf '%s' "$rest" | sed -E 's/ \(#[0-9]+\)$//')
                printf -- '- %s ([#%s](https://github.com/%s/pull/%s)) ([`%s`](https://github.com/%s/commit/%s))\n' \
                    "$subject" "$pr" "$REPO" "$pr" "$short" "$REPO" "$full"
            else
                printf -- '- %s ([`%s`](https://github.com/%s/commit/%s))\n' \
                    "$rest" "$short" "$REPO" "$full"
            fi
        done
    }
    TMPFILE=$(mktemp)
    {
        echo "# Changelog"
        echo ""
        echo "## ${TAG_PREFIX}${VERSION}"
        echo ""
        if [[ -n "$LAST_TAG" ]]; then
            format_log "${LAST_TAG}..HEAD"
        else
            echo "- Initial release"
        fi
        echo ""
        if [[ -f "$CHANGELOG" ]]; then
            tail -n +2 "$CHANGELOG"
        fi
    } > "$TMPFILE"
    mkdir -p "$(dirname "$CHANGELOG")"
    mv "$TMPFILE" "$CHANGELOG"
    # Commit.
    git add "${ADD_FILES[@]}"
    RELEASE_MSG="chore(release): release ${ARTIFACT} version ${VERSION}"
    if [[ "$(git log -1 --format='%s' 2>/dev/null)" == "$RELEASE_MSG" ]]; then
        git commit --amend --no-edit
    else
        git commit -m "$RELEASE_MSG"
    fi
    # Push and open/update the PR.
    git push --force-with-lease -u origin "$BRANCH"
    PR_BODY="## ${ARTIFACT} release v${VERSION}"$'\n\n'
    if [[ -n "$LAST_TAG" ]]; then
        PR_BODY+="### Changes since ${LAST_TAG}:"$'\n\n'
        CHANGELOG_BODY=$(format_log "${LAST_TAG}..HEAD~1")
        MAX_LOG=62000
        if (( ${#CHANGELOG_BODY} > MAX_LOG )); then
            TRUNCATED=$(printf '%s' "$CHANGELOG_BODY" | awk -v max="$MAX_LOG" \
                'BEGIN{n=0} {line_len=length($0)+1; if(n+line_len>max) exit; n+=line_len; print}')
            SHOWN=$(printf '%s\n' "$TRUNCATED" | grep -c '^-' || true)
            TOTAL=$(printf '%s\n' "$CHANGELOG_BODY" | grep -c '^-' || true)
            SKIPPED=$(( TOTAL - SHOWN ))
            CHANGELOG_BODY="${TRUNCATED}"$'\n'"_… and ${SKIPPED} more commits — [compare ${LAST_TAG}…${TAG_PREFIX}${VERSION}](https://github.com/${REPO}/compare/${LAST_TAG}...${TAG_PREFIX}${VERSION})_"
        fi
        PR_BODY+="${CHANGELOG_BODY}"$'\n\n'
    else
        PR_BODY+="Initial release."$'\n\n'
    fi
    PR_BODY+="**To release:** merge this PR. The tag and build will happen automatically."
    PR_TITLE="chore(release): release ${ARTIFACT} version ${VERSION}"
    EXISTING_PR=$(gh pr list --head "$BRANCH" --json url --jq '.[0].url' 2>/dev/null || true)
    if [[ -n "$EXISTING_PR" ]]; then
        gh pr edit "$BRANCH" --title "$PR_TITLE" --body "$PR_BODY"
        PR_URL="$EXISTING_PR"
        echo ""
        echo "Updated existing release PR: ${PR_URL}"
    else
        PR_URL=$(gh pr create --title "$PR_TITLE" --body "$PR_BODY")
        echo ""
        echo "Release PR opened: ${PR_URL}"
    fi
    echo "Merge it to trigger the release build."

# ─── Agent Harness ────────────────────────────────────────────────────────────

# Run a goose agent connected to a Buzz relay (foreground)
goose relay="ws://localhost:3000" agents="1" heartbeat="0" prompt="" key="$BUZZ_PRIVATE_KEY":
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    source ./scripts/_goose-env.sh "{{relay}}" "{{key}}" "{{agents}}" "{{heartbeat}}" "{{prompt}}"
    exec env "${env_args[@]}" ./target/release/buzz-acp

# Run a goose agent in the background (screen session named 'goose-agent-N')
goose-bg relay="ws://localhost:3000" agents="1" heartbeat="0" prompt="" key="$BUZZ_PRIVATE_KEY":
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    source ./scripts/_goose-env.sh "{{relay}}" "{{key}}" "{{agents}}" "{{heartbeat}}" "{{prompt}}"
    screen -dmS goose-agent-{{agents}} bash -c "$(printf '%q ' env "${env_args[@]}") ./target/release/buzz-acp"
    echo "Agent running in screen session 'goose-agent-{{agents}}'. Attach with: screen -r goose-agent-{{agents}}"

# ─── Benchmarking ─────────────────────────────────────────────────────────────

# Run the Buzz orchestra benchmark — leaderboard-eligible by default (TB 2.1, k=5, Sonnet+Haiku). Stands up its own Docker stack; --gui opens a live spectator desktop app; other flags pass to benchmark.py (--dataset/--path, --include-task, --attempts, --manifest, --dry-run, ...)
benchmark *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    uv run --project benchmarks/harbor-buzz-orchestra/testbed \
        benchmarks/harbor-buzz-orchestra/scripts/benchmark.py {{ARGS}}

# Run the benchmark adapter + testbed gate exactly as CI does (pytest + ruff, pinned ruff from pyproject)
benchmark-check:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}/benchmarks/harbor-buzz-orchestra"
    # CI installs the dev extra with pip, so pyproject — not uv.lock — decides
    # which ruff lints. Read the pin from there so this recipe cannot drift
    # from the workflow (a floating specifier once meant CI failed on RUF100
    # while the locked local ruff passed).
    ruff_pin="$(grep -oE 'ruff==[0-9.]+' pyproject.toml | head -1 | cut -d= -f3)"
    for project in . testbed; do
        (
            cd "$project"
            echo "── harbor-buzz-orchestra/$project (ruff $ruff_pin)"
            uv run --frozen pytest -q
            uvx "ruff@$ruff_pin" check .
            uvx "ruff@$ruff_pin" format --check .
        )
    done
    # The task verifiers live in the sibling benchmarks/buzz-dataset, so they
    # need the harness config passed explicitly to stay linted.
    echo "── buzz-dataset (ruff $ruff_pin)"
    uvx "ruff@$ruff_pin" check --config pyproject.toml ../buzz-dataset
    uvx "ruff@$ruff_pin" format --check --config pyproject.toml ../buzz-dataset

# Stop the benchmark Docker stack (state and channels are kept)
benchmark-down:
    docker compose --project-name buzz-benchmark down

# ─── SchoolX fork maintenance ─────────────────────────────────────────────────

# Read-only: creates no branch and merges nothing.
# Fetch upstream, report divergence, and list files both sides touched
schoolx-upstream-preflight:
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    branch="$(git rev-parse --abbrev-ref HEAD)"
    if [[ -n "$(git status --porcelain)" ]]; then
        echo "✗ working tree is dirty — commit or stash before merging upstream" >&2
        exit 1
    fi
    git fetch upstream
    base="$(git merge-base "${branch}" upstream/main)"
    read -r ours theirs <<<"$(git rev-list --left-right --count "${branch}...upstream/main")"
    echo
    echo "branch:            ${branch}"
    echo "merge-base:        ${base}"
    echo "upstream tip:      $(git log -1 --format='%H %ad %s' --date=short upstream/main)"
    echo "SchoolX commits:   ${ours}"
    echo "upstream commits:  ${theirs}"
    if [[ "${theirs}" == "0" ]]; then
        echo
        echo "✓ already up to date with upstream/main"
        exit 0
    fi
    echo
    echo "── files both sides touched (conflict candidates) ──"
    comm -12 \
        <(git diff --name-only "${base}" "${branch}" | sort) \
        <(git diff --name-only "${base}" upstream/main | sort)
    echo
    echo "Next: just schoolx-upstream-merge"

# Saves a dated rollback branch first; stops at the merge on conflict.
# Merge upstream/main, then run the silent-conflict checks
schoolx-upstream-merge:
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    branch="$(git rev-parse --abbrev-ref HEAD)"
    if [[ -n "$(git status --porcelain)" ]]; then
        echo "✗ working tree is dirty — commit or stash first" >&2
        exit 1
    fi
    git fetch upstream
    # Minute-stamped, not date-stamped: two syncs in one day used to collide on
    # the name, and the old recipe kept the FIRST one — so the printed rollback
    # command pointed at a tip from before the day's earlier work and would have
    # discarded it. A stale rollback point is worse than none, so a name that is
    # taken by a different commit is a hard failure now. The suffix still sorts
    # lexicographically, which is how `schoolx-upstream-check` picks the newest.
    safety="schoolx-pre-upstream-sync-$(date +%Y%m%d-%H%M)"
    if git rev-parse --verify --quiet "${safety}" >/dev/null; then
        if [[ "$(git rev-parse "${safety}")" != "$(git rev-parse HEAD)" ]]; then
            echo "✗ rollback branch ${safety} exists but does not point at HEAD" >&2
            echo "  it is $(git rev-parse --short "${safety}"), HEAD is $(git rev-parse --short HEAD)" >&2
            echo "  delete or rename it before merging" >&2
            exit 1
        fi
        echo "rollback branch ${safety} already at HEAD, reusing it"
    else
        git branch "${safety}" "${branch}"
    fi
    echo "rollback branch: ${safety}  (git reset --hard ${safety})"
    if git merge upstream/main --no-edit; then
        echo
        echo "✓ merged cleanly"
    else
        echo
        echo "✗ merge stopped on conflicts. Resolve, then run:"
        echo "    just schoolx-upstream-check"
        echo
        echo "  pnpm-lock.yaml: do not hand-edit. Take upstream's and regenerate —"
        echo "    git checkout --theirs pnpm-lock.yaml && pnpm install --lockfile-only"
        exit 1
    fi
    just schoolx-upstream-check

# Each of these has merged without a textual conflict before, so a clean
# merge proves nothing about them. Check 3 scans only what changed since
# `since` (default: the newest schoolx-pre-upstream-sync-* branch, i.e. what
# the last merge brought in). Pass a ref, or "all" to scan the whole tree.
# Check the three things a clean upstream merge does NOT give you
schoolx-upstream-check since="":
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    fail=0

    # 1. Migration version collision. sqlx keys `_sqlx_migrations` by version
    # but does NOT reject duplicates at compile time: a collision builds green
    # and strands one of the two migrations forever. SchoolX uses 9001+.
    echo "── 1/3 migration version collisions ──"
    dupes="$(ls migrations/ | cut -c1-4 | sort | uniq -d || true)"
    if [[ -n "${dupes}" ]]; then
        echo "✗ duplicate migration versions: ${dupes}"
        echo "  Move the SchoolX-owned file into the reserved 9001+ range and"
        echo "  update the guard in crates/buzz-db/src/migration.rs."
        fail=1
    else
        echo "✓ no duplicate version prefixes"
    fi

    # 2. Security contract. A file that resolves readable channels via the
    # open-inclusive lookup must also consult the member-only one — that pair
    # is how the managed-agent gate is applied. A file with only the
    # open-inclusive call is a read path that skips the gate entirely.
    # `state.rs` is excluded: it defines both. See SECURITY_CONTRACT.md §2.
    echo "── 2/3 managed-agent membership gate ──"
    unpaired=""
    while read -r f; do
        [[ "${f}" == *"/state.rs" ]] && continue
        grep -q "get_member_channel_ids_cached" "${f}" || unpaired+="  ${f}"$'\n'
    done < <(grep -rl "get_accessible_channel_ids_cached" crates/buzz-relay/src --include="*.rs" || true)
    if [[ -n "${unpaired}" ]]; then
        echo "✗ these resolve readable channels without the member-only lookup:"
        printf '%s' "${unpaired}"
        echo "  Upstream may have added a read path that skips the gate."
        fail=1
    else
        echo "✓ every open-inclusive lookup site also uses the member-only one"
    fi

    # 3. Product identity. Upstream code hardcoding a Buzz identifier in a new
    # path would silently share a data directory, keychain, URL scheme, or
    # process name with a co-installed Buzz. Matches identity literals only —
    # not the `buzz-desktop:` log prefix, not `buzz-agent`/`buzz-cli` binary
    # names, and not comment lines. See PRODUCT_IDENTITY.md §3.
    #
    # Scoped to changed files by default: a whole-tree scan is dominated by
    # test fixtures and deliberate negative assertions, and "what upstream
    # just changed" is the question that actually matters after a merge.
    since="{{since}}"
    if [[ -z "${since}" ]]; then
        since="$(git for-each-ref --sort=-refname --format='%(refname:short)' \
            'refs/heads/schoolx-pre-upstream-sync-*' | head -1)"
    fi
    roots=(desktop/src-tauri/src desktop/src crates/buzz-cli/src crates/schoolx-catalog/src web/src)
    # No `mapfile`: macOS ships bash 3.2.
    files=()
    if [[ -z "${since}" || "${since}" == "all" ]]; then
        echo "── 3/3 Buzz product identifiers (whole tree) ──"
        while IFS= read -r f; do files+=("${f}"); done < <(
            git ls-files "${roots[@]}" | grep -E '\.(rs|ts|tsx)$' || true)
    else
        echo "── 3/3 Buzz product identifiers (changed since ${since}) ──"
        while IFS= read -r f; do files+=("${f}"); done < <(
            git diff --name-only --diff-filter=d "${since}...HEAD" -- "${roots[@]}" \
                | grep -E '\.(rs|ts|tsx)$' || true)
    fi
    if [[ "${#files[@]}" -eq 0 ]]; then
        echo "✓ no source files in scope"
    else
        # Deliberate occurrences opt out in the source with a trailing
        # `schoolx:buzz-name-ok` comment, so "is this product identity or a
        # technical name?" is answered where the reader can see the code
        # rather than guessed at by a regex here.
        hits="$(grep -nH -E 'xyz\.block\.(buzz|sprout)\.app|"buzz-desktop(-dev)?"|"\.buzz(-dev)?"|"Buzz"|buzz://' \
            "${files[@]}" \
            | grep -v -E ':[0-9]+: *(//|///|\*|/\*)' \
            | grep -v -E '/tests?/|/tests?\.rs:|_tests\.rs:|\.test\.|\.spec\.|\.example' \
            | grep -v -E 'schoolx:buzz-name-ok|assert' || true)"
        if [[ -n "${hits}" ]]; then
            echo "✗ review these — route product strings through the product layer:"
            echo "${hits}"
            fail=1
        else
            echo "✓ no Buzz identity literals in ${#files[@]} file(s) in scope"
        fi
    fi

    echo
    if [[ "${fail}" -ne 0 ]]; then
        echo "✗ checks failed — see above before running the test suites"
        exit 1
    fi
    echo "✓ all three checks passed"
    echo
    echo "Now verify, then record the sync in docs/schoolx-2/BASELINE.md"
    echo "and the table in DEVELOPMENT_PLAN.md §10:"
    echo "    just desktop-tauri-test"
    echo "    pnpm --dir desktop typecheck && pnpm --dir desktop check && pnpm --dir desktop test"
    echo "    just test-e2e e2e_access_matrix     # security contract gate"
