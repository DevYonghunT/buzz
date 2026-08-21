# SchoolX Code Phase 2 private physical worktree removal 세션 인계

작성일: 2026-08-17

## 1. 현재 결론

Managed-worktree safe remove의 세 번째 구현 slice인 **private pinned quarantine/deletion engine**과 그
다음 **portability/acceptance closure**를 완료했다. Public remove surface는 의도적으로 열지 않았다.

- Binding store current/future schema는 v4이고 public `CodeThreadBinding`은 exact 8 fields를 유지한다.
- Linux/macOS private engine은 exact merge proof, pinned physical manifest, reciprocal Git-admin authority,
  proof ref, no-replace quarantine와 handle-relative no-follow deletion을 사용한다.
- Claim input과 activity clearance는 native sealed authority이고, finalization은 pinned inspector만 만드는
  opaque single-use verified-absence capability를 소비한다.
- Startup sticky recovery는 emitter/runtime start 및 lifecycle/start/fork reconciliation보다 먼저 실행된다.
- Ready runtime의 반복 start는 emitter만 교체하고 startup-only recovery/reconciliation을 다시 실행하지 않는다.
- 실제 aarch64 Ubuntu에서 release cfg compile, positive birth-time crash/reopen recovery와 mount-id가 다른
  same-filesystem self-bind 공격 7종을 실행했다. Git 2.43과 virtualized ARM의 pre-main ONNX 진단도 strict
  helper stderr 계약을 넓히지 않는 exact compatibility path로 닫았다.
- Unsupported production dispatcher의 pending-removal zero-mutation fixture는 unsupported host의 Tauri test에
  cfg-gate되어 있고, 같은 exact predicate를 지원 host에서도 Claimed/Removing 각각 실행한다.
- Startup recovery ordering과 sealed idle/no-PTY/no-approval admission은 actual store/runtime/PTY concurrency
  fixture로 고정됐다.
- Archived rename과 raw-binding turn interrupt는 pending `claimed/removing` ownership 동안 RPC 전에 닫힌다.
- Removed tombstone과 Codex transcript는 보존하며 root/admin execution bytes만 exact manifest authority로
  제거한다.
- `code_worktree_remove`, public proof/remove DTO, frontend adapter/button, `canRemove: true`와 optimistic UI는
  계속 없다.

Machine contract status는
`authorityProofJournalPhysicalRemovalImplementedPublicSurfaceAbsent`다. 네 gate 모두
`implementedClosed`이며 public surface flags는 계속 false다. Normative 계약은
[`SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`](SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md), machine mirror는
`desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`이다.

## 2. 작업 트리 보존 규칙

이 checkout은 이전 Phase 1/2와 사용자 변경, 많은 untracked 파일이 함께 있는 dirty worktree다. 기존
변경을 reset/checkout/delete하지 않았고 stage/commit도 하지 않았다. 다음 세션도 모든 Git/Rust/Node
command 전에 Hermit을 활성화한다.

```bash
. ./bin/activate-hermit && git status --short
```

## 3. Private authority와 manifest

Public caller는 path/ref/OID/proof/removal ID를 제출하지 않는다. Private claim path는 exact archived
lookup에 대해 runtime idle과 PTY-owner absence를 먼저 증명한 sealed clearance만 받는다. Store의
manifest-derived claim type은 removal module 밖에서 구성할 수 없다. Clearance는 binding mutex와 exact
runtime generation/activity/approval admission guard를 claim/execute가 끝날 때까지 보유한다.

Claim inspector는 active nest부터 `WORKTREES/<repositoryIdentity>/<worktreeId>` 및 Git common-dir의
`worktrees/<admin-id>`까지 no-follow handles로 pin하고 다음을 확인한다.

- Exact managed coordinate와 repository identity
- Linked-worktree `.git` ↔ Git-admin `gitdir`/`commondir` reciprocal relationship
- Git-admin `HEAD`와 proven head commit, required `HEAD | commondir | gitdir | index` files
- Clean detached worktree, stage-0 index와 proven HEAD tree의 exact path/mode/OID join, 실제 tracked
  regular-file/symlink bytes의 exact Git blob OID
- Loose-files ref backend(`extensions.refStorage`, 미지정 시 Git 기본 `files`), 모든 HEAD blob의 local
  existence/type, no-follow primary object storage와
  alternate/shared object database 부재
- Root의 `.git`, tracked entries/ancestor directory 외 entry 부재
- Git-admin top-level `locked` marker와 어느 depth의 `*.lock` 부재
- Root/admin entry identity, file/symlink content digest, parent sibling identity snapshot

Canonical strict v1 manifest bytes는 SHA-256 digest로 주소 지정한
`code/removal-manifests-v1/<digest>.json` sidecar에 atomic write+sync한다. Journal의
`physicalManifestDigest`와 sidecar bytes/digest가 exact 일치하지 않으면 recovery를 진행하지 않는다.
Sidecar directory/file도 code data directory에서 same-mount handle-relative로만 만들고 읽고 정리한다.
Persist/load/remove와 이미 absent인 cleanup 경로도 named directory/file identity를 재검증하며 replacement를
success로 해석하지 않는다.

Ignored/untracked file, untracked empty directory, special/cross-device entry, gitlink/submodule, admin
symlink/special/lock entry와 manifest 이후 replacement/content/type drift는 삭제 전에 fail closed한다.

## 4. Frozen physical ordering

```text
1. stable Archived binding/native merge authority load
2. quiescence, pinned reciprocal identity와 physical manifest proof
3. digest-addressed strict manifest sidecar atomic write+sync
4. claimed atomic write+sync
5. proof/identity/manifest 재검증 후 removing atomic write+sync
6. refs/schoolx/removal-claims/<removalId> -> targetCommit exact compare-create와 재검증
7. parent-relative atomic no-replace root -> quarantine rename
8. quarantine의 frozen manifest만 handle-relative no-follow delete
9. reciprocal exact Git-admin manifest/entry delete
10. original/quarantine/admin absence와 sibling snapshot 재검증
11. opaque absence capability로 live trio를 permanent removed tombstone으로 atomic retire
12. expected targetCommit OID로 exact loose proof-ref compare-delete와 ref directory sync
13. digest-bound manifest sidecar unlink와 directory sync
```

Sidecar-first commit 뒤 journal claim 전에 실패한 content-addressed orphan은 inert하며 broad GC/adoption하지 않는다.

Proof ref가 다른 OID로 대체되면 engine은 그 ref를 삭제하지 않는다. Finalization 뒤 cleanup crash는
removed tombstone의 exact coordinate만 다시 읽고 broad ref scan/prune 없이 compare-delete를 재시도한다.
Proof ref는 symbolic/ambiguous raw authority를 거부하고 `--no-deref`로 갱신하며, exact loose regular file을
no-follow로 확인한다. Git reference fsync와 ref file/parent directory fsync를 모두 완료해야 durable하다.
Proof-ref absence가 durable해진 뒤에만 sidecar absence를 cleanup-complete marker로 만든다. Original common-dir가
일시적으로 없으면 startup을 막지 않고 sidecar를 보존해 coordinate가 돌아온 뒤 cleanup을 재시도한다.

## 5. Recovery와 serialization

Startup은 binding-store mutex 아래 removed tombstone cleanup을 먼저, pending journal recovery를 다음으로
실행하고 나서 emitter/runtime start와 lifecycle reconciliation을 연다. `claimed`는 original expected,
quarantine absent, admin expected, proof ref absent라는 definitely-not-started 상태가 재증명될 때만 exact
CAS cancellation할 수 있다. `removing`은 항상 sticky하며 rollback/retarget하지 않는다.

Original/quarantine/admin은 각각 `absent | expected | replacement`로 해석한다. Known prefix만 resume하며
replacement나 impossible combination은 preserved `removing` error다. Raw fd는 process restart 뒤 authority가
아니므로 durable birth-time/generation identity, manifest와 deterministic coordinates를 다시 pin한다.

Linux/macOS 외 platform은 pending journal이 없으면 startup을 계속할 수 있지만 pending record가 있으면
zero-mutation `unsupported`로 fail closed한다.

Removal clearance는 authoritative idle proof 뒤 binding/runtime/activity/approval guard를 실제 private engine
완료까지 보유한다. 경쟁 turn-start, approval insert와 production PTY open은 그 구간에 admission byte/session을
만들 수 없고, tombstone finalization 뒤 PTY open은 removed binding에서 fail closed한다.

## 6. 계속 닫힌 public/product surface

- `code_worktree_remove` registration과 public input/receipt DTO
- Public merge-proof command
- Frontend API adapter, mutation hook, confirmation/button
- Inventory `canRemove`, blocker 또는 archived row projection 변경
- Optimistic row removal
- Codex transcript/thread archive/delete/move RPC

Frozen Tauri command list와 frontend schemas는 그대로이며 inventory는 모든 row에
`preserved: true`, `canRemove: false`를 반환한다.

## 7. 금지된 우회

- `--force`, `git clean`, `git reset`
- `git worktree remove`, `git worktree prune`
- Broad/pathname-recursive `remove_dir_all`
- Fetch/network/credential/PR proof
- Implicit archive/fork/start/orphan cleanup
- Replacement, symlink target 또는 sibling deletion

## 8. 주요 구현/계약 파일

- `desktop/src-tauri/src/code_workspace/bindings/removal.rs`
- `desktop/src-tauri/src/code_workspace/bindings/removal/physical.rs`
- `desktop/src-tauri/src/code_workspace/bindings/removal/physical/unix.rs`
- `desktop/src-tauri/src/code_workspace/bindings/removal/physical/tests.rs`
- `desktop/src-tauri/src/code_workspace/runtime.rs`
- `desktop/src-tauri/src/code_workspace/worktrees.rs`
- `desktop/src-tauri/src/code_workspace/terminal.rs`
- `desktop/src-tauri/src/commands/code_terminal.rs`
- `desktop/src-tauri/src/commands/code_workspace.rs`
- `desktop/src-tauri/src/commands/code_thread_management.rs`
- `desktop/src-tauri/src/lib.rs`
- `desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`
- `desktop/src-tauri/src/code_workspace/contract_tests.rs`
- `desktop/src/features/code/api/codeWorkspace.contract.test.mjs`
- `docs/schoolx-2/SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`
- `docs/schoolx-2/SCHOOLX_CODE_DESIGN.md`
- `.github/workflows/ci.yml`
- 이 handoff 문서

## 9. 실행한 검증

모든 command는 Hermit 활성화 뒤 실행했다.

```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo check --manifest-path desktop/src-tauri/Cargo.toml --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib -- -D warnings
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  code_workspace::bindings::removal::tests:: -- --test-threads=1
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::bindings::removal::physical::tests::
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests::worktree_removal_decision_gates_are_frozen_while_the_surface_is_absent \
  -- --exact
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  commands::code_thread_management::tests:: -- --test-threads=1
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  commands::code_workspace::tests:: -- --test-threads=1

# isolated privileged aarch64 Ubuntu fixture
cargo check --release --manifest-path desktop/src-tauri/Cargo.toml --lib
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  linux_removing_crash_reopen_recovers_positive_birth_time_identities -- --test-threads=1
SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE=direct cargo test \
  --manifest-path desktop/src-tauri/Cargo.toml --lib \
  linux_privileged_same_filesystem_bind_ -- --ignored --test-threads=1

cd desktop
pnpm exec biome check src/features/code/api/codeWorkspace.contract.test.mjs
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/code/api/codeWorkspace.contract.test.mjs
```

최종 결과는 다음과 같다.

- macOS physical actual-Git: 23 passed, helper entry 2 ignored. 그중 11개 durable/mutation 경계는 child
  process hard exit 뒤 reopen/recovery로 같은 removal ID와 tombstone에 수렴한다.
- aarch64 Ubuntu 24.04 / Rust 1.95 / Git 2.43에서 release lib cfg compile과 positive birth-time
  Removing crash/reopen 1개가 통과했다. Self-bind 전후 dev/inode/birth-time 동일성과 mount-id 차이를 먼저
  증명한 뒤 managed root, tracked entry, Git-admin entry, primary objects, manifest sidecar directory/file,
  sticky Removing root 공격 7개가 privileged actual mount로 모두 통과했다.
- Adversarial fixture는 ignored/untracked/empty/FIFO, Git lock, missing local blob, shared alternate ODB,
  non-prefix partial deletion, original/quarantine/admin/sidecar/proof-ref replacement, external symlink target,
  offline common-dir defer/restore cleanup을 고정한다.
- Startup ordering failure/Ready-idempotence 2개와 sealed clearance concurrency 1개가 actual runtime/store/PTY
  경로에서 통과했다. Replacement/incarnation ambiguity는 exact journal/store/tree bytes를 유지한 sticky
  `Removing`으로 남는다.
- Pure journal 8 passed. Unsupported helper fixture는 Claimed/Removing 각각 store bytes/mtime, full fixture와
  exact pending record 불변을 확인하며, actual unsupported dispatcher variant는 Windows Tauri CI에서 실행된다.
  Rust frozen contract 1 passed, frontend frozen contract 25 passed.
- Cargo fmt/check, production lib clippy, targeted Biome, JSON parse와 diff whitespace check가 통과했다.

## 10. 다음 slice 경계

Portability/acceptance closure가 끝났으므로 다음 독립 slice는 public input을 exact `{scope, threadId}`로만
여는 수직 변경이다. Path/ref/OID/proof/removal ID는 native가 파생한다. Command registration, same-removal
receipt DTO, frontend adapter/confirmation UI와 inventory eligibility를 함께 추가하며 optimistic removal은
사용하지 않는다.

## 11. 다음 세션 복사용 시작 요청

```text
SCHOOLX_CODE_DESIGN.md, SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md와 최신
SESSION_HANDOFF_20260817_CODE_PHASE2_WORKTREE_PHYSICAL_REMOVAL.md, 현재 작업 트리를 먼저 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Binding schema v4와 public exact 8-field binding을 유지하고 기존 사용자 변경/untracked 파일을 보존해줘.
Portability/acceptance closure 결과를 먼저 확인한 뒤 public input은 exact {scope, threadId}만 허용해줘.

Native-derived same-removal retry receipt, command registration, frontend confirmation UI와 inventory eligibility를
하나의 수직 변경으로 열어줘. Caller path/ref/OID/proof/removal ID, optimistic UI, transcript mutation,
force/clean/reset/worktree remove/prune/remove_dir_all/network proof는 금지해줘.
```
