# SchoolX Code Phase 2 worktree removal journal/tombstone 세션 인계

작성일: 2026-08-17

> 이 문서는 pure journal slice 완료 시점의 기록이다. 후속 private physical-removal 구현 상태는
> [`SESSION_HANDOFF_20260817_CODE_PHASE2_WORKTREE_PHYSICAL_REMOVAL.md`](SESSION_HANDOFF_20260817_CODE_PHASE2_WORKTREE_PHYSICAL_REMOVAL.md)를
> 기준으로 한다.

## 1. 현재 결론

Managed-worktree safe remove의 두 번째 구현 slice인 **pure v4 removal journal/tombstone**을 완료했다.
실제 Git/filesystem deletion과 public remove surface는 추가하지 않았다.

- Binding store current/future schema는 계속 v4다.
- Public `CodeThreadBinding`은 기존 exact 8 fields를 유지한다.
- Empty-only `removals` decoder를 strict flat tagged `claimed | removing | removed` records로 교체했다.
- Exact `(scope, threadId)` retry, native canonical UUID v4, whole-record CAS, sticky `removing`, permanent
  `removed` tombstone을 구현했다.
- Pending removal은 exact stable Archived live binding/lifecycle/merge-target에 join한다.
- Final swap은 binding+lifecycle+merge-target을 한 atomic store write에서 tombstone으로 retire한다.
- 모든 removal state가 Codex thread ID, worktree ID와 execution root를 영구 예약한다.
- Removed tombstone은 executable authority가 아니며 public inventory row로 다시 투영되지 않는다.
- Codex transcript와 외부 worktree/Git-admin/sibling bytes는 이 slice에서 변경하지 않는다.
- `code_worktree_remove`, public proof/remove DTO, frontend adapter/button과 physical mutation은 여전히 없다.

Machine contract status는
`authorityProofJournalImplementedPhysicalRemovalAbsent`다. `mergedAuthority`,
`durableRemovalJournal`, `bindingTranscriptSemantics`는 `implementedClosed`이고
`pinnedDeletionBoundary`는 `designedClosed`다.

Normative 계약은
[`SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`](SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md), machine mirror는
`desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`이다.

## 2. 작업 트리 보존 규칙

이 checkout은 이전 Phase 1/2, 사용자 변경과 많은 untracked 파일이 함께 있는 dirty worktree다. 기존
변경을 reset/checkout/delete하지 않았고 stage/commit도 하지 않았다.

다음 세션의 첫 명령도 다음과 같이 실행한다.

```bash
. ./bin/activate-hermit && git status --short
```

Hermit 활성화 없이 Git/Rust/Node command나 hook을 실행하지 않는다. `.dockerignore`, `.gitignore`,
`crates/`, `deploy/`, `brand/`, `supabase/`, 기존 SchoolX Code 변경과 이전 handoff 문서를 임의로
정리하지 않는다.

## 3. Strict v4 removal wire

V4 top-level collection은 required `removals`를 계속 사용하고 schema version을 올리지 않는다.

각 record는 `state`와 immutable authority가 같은 strict object에 놓인다.

```text
state
removalId
binding
threadLifecycleAtClaim
mergeProof
physicalManifestDigest
physical
transcriptDisposition
executionDisposition
```

- `state`: exact `claimed | removing | removed`
- `removalId`: native-issued canonical lowercase hyphenated UUID v4
- `binding`: original exact 8-field managed-worktree binding
- `threadLifecycleAtClaim`: literal `archived`
- `mergeProof`: exact
  `{repositoryIdentity, worktreeId, headCommit, targetRef, targetCommit}`
- `physicalManifestDigest`: lowercase SHA-256
- `physical`: exact
  `{managedRootParent, managedRoot, quarantineName, gitAdminParent, gitAdminEntry}`
- `transcriptDisposition`: literal `preserved`
- `executionDisposition`: literal `removed`

`quarantineName`은 caller input이 아니라
`.schoolx-removing-<native-removal-id>`로 파생한다. Managed root는 original execution root 및
`managedRootParent/<worktreeId>`와 exact join한다. Proof repository/worktree/ref는 binding과 persisted
merge-target sibling에 join한다. Proof head/target OID 길이는 original binding base OID와 일치해야 하므로
한 repository record에서 SHA-1/SHA-256 형식을 섞을 수 없다.

V4 removal raw probe가 실제 record type으로 original bytes를 decode한다. 따라서
`serde_json::Value`가 last-wins로 숨길 수 있는 duplicate `removals`, `state`, authority/proof/physical
member도 fail closed한다. Unknown/missing field, invalid literal/UUID/digest/ref/OID/path와 join drift는
원본 bytes를 rewrite하지 않고 거부한다.

Probe는 v4 removal에만 적용한다. V1/V2/V3는 load만으로 file bytes/mtime을 바꾸지 않고 in-memory
`removals=[]`로 계속 migration한다.

## 4. State machine, retry와 fault semantics

```text
claimed -> removing -> removed
```

- 첫 claim만 native UUID를 발급한다. 같은 `(scope, threadId)`의 response-loss/retry는 persisted
  claimed/removing/removed record와 동일 `removalId`를 byte-stable하게 반환한다.
- Existing record가 있으면 caller가 새 proof, target, manifest나 coordinate를 제안해도 retarget하지 않는다.
- `claimed` cancellation은 exact whole-record CAS와 definitely-not-started 조건에서만 가능하다.
- Cancellation commit 뒤 response가 사라진 경우 exact absence retry는 idempotent하다. 같은 key에 새 record가
  생긴 ABA 상황은 stale CAS로 거부한다.
- `claimed -> removing`은 exact CAS이고 response loss 뒤 already-removing/removed retry는 같은 authority를
  반환한다.
- `removing`은 cancel/rollback할 수 없는 sticky state다.
- Final store swap은 exact removing authority의 live binding, Archived lifecycle과 merge-target을 제거하고
  동일 authority의 permanent removed tombstone을 한 save로 남긴다.
- Save 전 failure는 prior durable bytes를 유지한다. Save commit 뒤 response loss는 reopen/retry로 같은
  state/receipt에 수렴한다.
- Journal capacity는 4,096 records이고 기존 4 MiB store cap을 그대로 지킨다. Tombstone은 prune하지 않는다.

Finalization primitive는 physical absence engine이 없으므로 removal module private다. Test-only save seam은
atomic retirement 및 failure semantics를 검증하기 위해 verified absence를 시뮬레이션할 뿐 production
deletion authority가 아니다.

## 5. Join, reservation과 non-executable semantics

`claimed/removing`은 다음 세 record와 exact 1:1 join한다.

- byte-equal original managed `CodeThreadBinding`
- exact scope/thread의 stable `Archived {}` lifecycle
- proof worktree/ref와 같은 `mergeTargets` sibling

`removed`는 live binding/lifecycle/merge-target 어느 것과도 공존할 수 없다. Duplicate removal ID,
retry key, thread ID, worktree ID 또는 execution root도 index load/save에서 거부한다.

모든 state의 identity reservation을 다음 admission에 연결했다.

- start/local/managed preparation root 및 worktree reservation
- fork destination과 removal-owned source thread
- `ensure_thread_unbound`
- preparation commit/recovery candidate thread adoption
- availability/upsert test backstops
- start/fork recovery candidate filtering

Start recovery는 reserved candidate를 cwd canonicalization보다 먼저 제외하므로 malformed tombstone candidate가
unrelated recovery를 poison하지 않는다. Pending removal의 Archived lifecycle을 unarchive/reconcile하려는
store save도 exact join validation 전에 fail closed한다.

Removed tombstone은 normal binding lookup, active binding admission, resume/turn/PTY/Changes/rename/unarchive/fork
authority가 아니다. Inventory는 tombstone을 missing root row로 재생성하지 않는다. Codex transcript가
외부에서 사라져도 tombstone을 자동 삭제하거나 root를 재생성하지 않는다.

## 6. Pure-store acceptance와 외부 상태 보존

Focused tests는 다음을 고정한다.

- exact wire key sets와 tagged states
- unknown/missing/duplicate field, malformed UUID/literal/digest/ref/path와 mixed OID rejection
- pending join matrix와 proof/target drift rejection
- native-issued canonical UUID v4와 retry non-retargeting
- pre-save failure와 commit-then-error response loss at claim/removing/final boundaries
- crash/reopen 후 same removal ID/state
- exact cancellation, absent retry, ABA rejection와 sticky removing
- final failure 시 removing+live trio 유지
- final success 시 tombstone만 남는 atomic swap
- removed identity non-reuse across binding/preparation/fork/recovery
- removed tombstone의 executable lookup 및 public inventory 부재
- V1/V2/V3 byte/mtime-preserving migration

Tests는 managed root, Git-admin entry, transcript와 sibling sentinel tree를 따로 만들고 모든 pure-store
transition 전후 snapshot equality를 확인한다. 실제 root/admin이 남아 있는데 tombstone을 만드는 private
test seam은 production path가 아니라 atomic store behavior만 시험한다.

## 7. 계속 부재하는 surface와 mutation

- `code_worktree_remove`
- Public merge-proof/remove command 또는 DTO
- Frontend path/ref/OID/proof claim
- Remove adapter, mutation hook, button/confirmation과 optimistic inventory removal
- `canRemove: true` 또는 inventory blocker 변경
- Git ref creation/deletion
- Worktree/root/quarantine/Git-admin filesystem mutation
- Codex transcript/thread RPC 또는 delete
- `--force`, `git clean/reset`, `git worktree remove/prune`, broad `remove_dir_all`
- Fetch/network/credential/PR merge claim
- Implicit archive/fork/orphan cleanup
- Startup physical-removal recovery dispatcher

Frozen Tauri command list는 기존 27개다. Public binding, inventory DTO와 frontend schema도 그대로다.

## 8. 이번 slice의 주요 변경 파일

- `desktop/src-tauri/src/code_workspace/bindings.rs`
- `desktop/src-tauri/src/code_workspace/bindings/lifecycle.rs`
- `desktop/src-tauri/src/code_workspace/bindings/removal.rs`
- `desktop/src-tauri/src/code_workspace/bindings/removal/tests.rs`
- `desktop/src-tauri/src/code_workspace/worktrees.rs`
- `desktop/src-tauri/src/commands/code_workspace.rs`
- `desktop/src-tauri/src/commands/code_thread_fork.rs`
- `desktop/src-tauri/src/code_workspace/contract_tests.rs`
- `desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`
- `desktop/src/features/code/api/codeWorkspace.contract.test.mjs`
- `docs/schoolx-2/SCHOOLX_CODE_DESIGN.md`
- `docs/schoolx-2/SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`
- 이 handoff 문서

## 9. 실행한 검증

모든 command는 Hermit 활성화 뒤 실행했다.

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml code_workspace::bindings::
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::bindings::removal::tests::
cargo test --manifest-path desktop/src-tauri/Cargo.toml commands::code_workspace::
cargo test --manifest-path desktop/src-tauri/Cargo.toml commands::code_thread_fork::
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests::worktree_removal_decision_gates_are_frozen_while_the_surface_is_absent \
  -- --exact

cargo check --manifest-path desktop/src-tauri/Cargo.toml
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib -- -D warnings
git diff --check

cd desktop
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/code/api/codeWorkspace.contract.test.mjs \
  src/features/code/ui/CodeWorktreeInventorySection.test.mjs
pnpm exec biome check src/features/code/api/codeWorkspace.contract.test.mjs
```

최종 결과:

- Binding/lifecycle/removal: 53 passed
- Focused removal journal: 6 passed
- Workspace command/recovery: 22 passed
- Fork command/recovery: 5 passed
- Frozen Rust removal contract: 1 passed
- Frontend contract/component: 29 passed
- Cargo check/fmt, production lib clippy, diff check와 targeted Biome: passed

## 10. 다음 physical slice가 먼저 닫아야 할 boundary

현재 journal은 physical metadata를 strict하게 저장하지만 provenance를 생성하는 inspector는 아니다.
다음 engine은 mutation 또는 public entrypoint 전에 다음을 닫아야 한다.

1. Pinned parent/common-dir/worktree handles로 manifest와 Git-admin reciprocal identity를 재검증한다.
2. 임의 crate caller가 만들 수 없는 native claim authority/factory에서만 journal claim input을 만든다.
3. Proof object graph를 `refs/schoolx/removal-claims/<removalId>`에 pin하되 claimed sync 뒤, removing sync
   뒤의 첫 Git mutation으로 수행한다.
4. Root를 exact deterministic quarantine name으로 parent-relative atomic no-replace rename한다.
5. Frozen manifest entry만 handle-relative/no-follow로 삭제하고 replacement/new entry는 절대 삭제하지 않는다.
6. Reciprocal proof를 가진 exact Git-admin entry만 제거한다.
7. Original/quarantine/admin absence를 재검증한 opaque capability를 private finalization이 소비하게 한다.
8. Binding-store mutex 아래 claim/recovery를 직렬화하고 Archived rename 및 raw binding 기반 turn interrupt를
   claimed/removing 동안 gate한다.
9. Startup pending-removal recovery를 lifecycle/start/fork reconciliation보다 먼저 실행한다.
10. Unsupported platform은 zero-mutation fail closed하고 response loss/crash boundary를 실제 FS/Git fixture로
    검증한다.

Public remove command/UI와 inventory eligibility는 이 private engine이 완성된 뒤 별도 slice로 둔다.

## 11. 다음 세션 복사용 시작 요청

```text
SCHOOLX_CODE_DESIGN.md와 최신
SESSION_HANDOFF_20260817_CODE_PHASE2_WORKTREE_REMOVAL_JOURNAL.md, 현재 작업 트리를 먼저 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Phase 1/2 terminal/search/rename/archive/fork/inventory, removal decision contract, v4 direct-local merge
authority/hardened ancestry proof와 pure v4 claimed/removing/removed journal/tombstone은 완료됐으므로 다시
구현하지 마. Store schema v4와 public exact 8-field binding을 유지해줘.

다음 독립 slice는 private pinned quarantine/deletion engine으로 제한해줘. Pinned manifest와 reciprocal
Git-admin authority, unforgeable native claim/verified-absence capability, claimed/removing sync ordering,
proof-ref pin, no-replace quarantine, handle-relative no-follow manifest deletion, exact admin removal,
startup sticky recovery와 rename/interrupt serialization을 구현하고 실제 crash/response-loss/replacement
fault matrix로 검증해줘.

아직 code_worktree_remove/public proof command, frontend adapter/button, inventory canRemove/blocker 변경,
Codex transcript mutation이나 optimistic UI를 추가하지 마. force/clean/reset/worktree remove/prune,
broad remove_dir_all, fetch/network/credential/PR proof와 implicit archive/fork/orphan cleanup도 금지해줘.

Model/reasoning selector, Git write handoff, dirty patch fork, command/exec와 기존 quality-gate 부채를 섞지
마. 기존 사용자 변경과 untracked 파일을 보존하고 stage나 commit은 하지 마.
```
