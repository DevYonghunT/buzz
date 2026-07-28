---
name: schoolx-upstream-sync
description: >
  Sync block/buzz upstream into the SchoolX fork. Use whenever the user asks to
  fetch, pull, merge, or update from upstream ("업스트림 받아", "upstream 동기화",
  "sync upstream", "pull upstream", "merge upstream"), and at the start of any
  SchoolX development session. Covers the merge policy, the silent conflicts a
  clean merge does not reveal, and where the sync must be recorded.
version: 1
---

# SchoolX ← upstream sync

This repo is **`DevYonghunT/buzz`, a fork of `block/buzz` rebranded as
SchoolX.** Upstream is active — 57 commits in two days, then 85 in three. The
fork carries security, i18n, and product-identity changes that upstream knows
nothing about, so a merge here is never just a merge.

Read `docs/schoolx-2/PRODUCT_IDENTITY.md` and `docs/schoolx-2/SECURITY_CONTRACT.md`
before resolving anything non-obvious.

## Policy (from `DEVELOPMENT_PLAN.md` §10)

- **merge, never rebase.** Rebase rewrites SchoolX SHAs, which forces a push
  over a public fork and invalidates every SHA `BASELINE.md` records. Rebase is
  only for local branches that have not been pushed.
- **Weekly, and before every development session.**
- Work on the main checkout (`/Users/kim-yonghun/Development/schoolX_v2.0`),
  where the work branch `codex/schoolx-2-foundation` is checked out.

## Procedure

Activate the toolchain first — hooks and `just` need it.

```bash
. ./bin/activate-hermit
```

Then:

```bash
just schoolx-upstream-preflight   # fetch, divergence, conflict candidates
just schoolx-upstream-merge       # rollback branch, merge, then the 3 checks
```

`schoolx-upstream-merge` saves `schoolx-pre-upstream-sync-<YYYYMMDD>` before
merging. Rollback is `git reset --hard schoolx-pre-upstream-sync-<date>`.

### If the merge conflicts

Resolve, then run `just schoolx-upstream-check` yourself.

**`pnpm-lock.yaml`: never hand-edit.** Take upstream's and regenerate — the
merged `package.json` already carries both sides' dependencies.

```bash
git checkout --theirs pnpm-lock.yaml && pnpm install --lockfile-only
```

**Translation catalogs** (`desktop/src/shared/i18n/locales/*.ts`): when upstream
changes the *meaning* of a string, keep the i18n key and update the ko/en text
to the new meaning. Never delete a key or revert a `t()` call to a hardcoded
string.

## The part that actually matters: silent conflicts

**A clean merge proves nothing.** No textual conflict only means git did not see
two edits to the same line. Every problem below has merged cleanly, built green,
and passed unit tests.

`just schoolx-upstream-check` covers three. Run it even after a clean merge —
`schoolx-upstream-merge` does this for you.

1. **Migration version collision.** Upstream and SchoolX can add different
   files under the same `NNNN_` prefix. sqlx keys `_sqlx_migrations` by version
   but does **not** reject duplicates at compile time, so one of the two
   migrations is silently stranded forever. This happened on 2026-07-28.
   SchoolX migrations live in the reserved **`9001+`** range.

2. **Managed-agent membership gate.** The SchoolX security contract narrows
   what a classified agent can reach. Any new upstream read path that resolves
   channels without the member-only lookup breaks it silently.

3. **Product identity literals.** Upstream code hardcoding `xyz.block.buzz.app`,
   `"buzz-desktop"`, `~/.buzz`, `buzz://`, or a `"Buzz"` directory in a new path
   makes SchoolX share a data directory, keychain, URL scheme, or process name
   with a co-installed Buzz. Deliberate occurrences opt out with a trailing
   `// schoolx:buzz-name-ok` comment — add one only when the string is genuinely
   a technical name (Cargo package, agent runtime), never to silence a real hit.

Check 3 scans what changed since the last sync. To sweep everything:
`just schoolx-upstream-check all`.

## Verify

```bash
just desktop-tauri-test
```

```bash
pnpm --dir desktop typecheck && pnpm --dir desktop check && pnpm --dir desktop test && pnpm --dir desktop build
```

The security contract has **no CI job** — it only runs when someone runs it.

```bash
just test-e2e e2e_access_matrix
```

`just test` failures in `crates/buzz-agent/tests/fake_llm.rs` are pre-existing
and load-dependent; the parent snapshot fails them at least as often. Failures
outside that file need investigation. See `BASELINE.md`.

## Record it

Not optional — the next session's rollback point and flaky-test classification
depend on it.

- `docs/schoolx-2/BASELINE.md` — upstream sync table: SHA, date, commit count,
  rollback branch, conflicted files.
- `docs/schoolx-2/DEVELOPMENT_PLAN.md` §10 — one row in the sync log, noting
  anything a clean merge hid.

Then commit the merge and push both the branch and the rollback branch.

## Gotchas

- `cargo run -p buzz-admin -- migrate` does **not** read `.env`. Export
  `DATABASE_URL` explicitly or it hits the host's Homebrew postgres on 5432 and
  dies with `role "buzz" does not exist`. The container is on **5433**.
- If a migration renumber leaves the dev DB with a stale ledger row, delete that
  one row rather than resetting the database — SchoolX migrations are idempotent
  `UPDATE`s. Recipe in `BASELINE.md`.
- Live-relay e2e needs MinIO as well as Postgres and Redis; the relay aborts
  startup without an S3 backend. `just test-e2e` starts it.
- `just desktop-tauri-fmt` fails inside a git worktree. Run it from the main
  checkout.
