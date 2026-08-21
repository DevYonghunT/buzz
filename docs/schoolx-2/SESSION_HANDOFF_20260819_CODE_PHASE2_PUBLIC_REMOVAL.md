# SchoolX Code Phase 2 public safe worktree removal 세션 인계

기준일: **2026-08-19**

## 1. 이번 slice 결과

Managed-worktree safe remove의 public 수직 슬라이스를 열었다. 이전 세션에서 닫은 merge authority/proof,
durable `claimed -> removing -> removed` journal, transcript tombstone과 pinned physical engine 위에 다음을
한 계약으로 연결했다.

- Exact public input `{scope, threadId}`
- Native-derived exact 9-field removal receipt
- Tauri `code_worktree_remove`
- Native positive proof에 결박된 inventory eligibility
- Eligible Archived row의 explicit destructive confirmation
- Same-input retry와 native commit 뒤 response-loss recovery
- Optimistic row deletion 없는 authoritative inventory+thread reconciliation
- Permanent `transcriptDisposition: "preserved"`

Machine-readable removal fixture의 current status는
`authorityProofJournalPhysicalRemovalImplementedPublicSurfaceOpen`이다. Fixture version은 계속 1이고 호환성을
위해 `futureSurface`/`futureReceipt` key 이름도 유지한다. 해당 surface의 `registered`,
`frontendMethodExposed`, `buttonRendered`는 모두 `true`다. 네 gate의 `implementedClosed`는 각 invariant가
구현되고 우회 없이 닫혔다는 뜻이며 public surface가 닫혔다는 뜻이 아니다.

Binding store는 계속 v4이고 public `CodeThreadBinding` exact 8 fields는 바꾸지 않았다. Public merge-proof
command, caller-supplied path/ref/OID/proof/removal ID, transcript mutation과 Git write는 열지 않았다.

## 2. Public contract

Tauri command는 다음 top-level argument 하나만 받는다.

```text
code_worktree_remove({
  input: {
    scope: {
      communityId,
      projectDtag,
      repositoryIdentity
    },
    threadId
  }
})
```

Unknown input field를 거부한다. `executionRoot`, descriptor/worktree ID, base/HEAD/target OID, target ref,
merge proof, blocker/`canRemove`, lifecycle, force/request/removal ID는 caller authority가 아니다.

성공 output은 native tombstone에서 파생한 다음 exact 9 fields다.

```text
removalId
scope
threadId
worktreeId
headCommit
mergedIntoRef
mergedIntoCommit
transcriptDisposition: "preserved"
executionDisposition: "removed"
```

Native와 frontend 모두 strict shape를 검사한다. `removalId`는 native-issued canonical lowercase UUID v4다.
Worktree UUID, Git OID 길이/형식과 `refs/heads/*` direct-local ref도 contract validator를 통과해야 한다.

## 3. Native admission과 retry

`code_worktree_remove`는 app data/nest root와 app-owned runtime, terminal, lifecycle-ready, shutdown state를
native entrypoint에 전달한다. Native는 input을 검증하고 binding mutex를 획득한 뒤 다음을 지킨다.

1. Existing exact removal record/tombstone이 있으면 새 proof, target이나 removal ID를 만들지 않고 같은
   journal에 합류한다.
2. 새 claim은 lifecycle authority ready와 non-shutdown 상태에서 exact stable Archived managed binding,
   runtime idle, PTY-owner absence, approval/activity/preparation clearance를 증명해야 한다.
3. Persisted direct-local merge target을 사용해 bounded native ancestry proof를 다시 수행한다.
4. Manifest-derived authority와 verified-absence capability는 sealed removal module만 만든다.
5. Previous session의 sidecar/journal/proof-ref/quarantine/no-follow physical ordering을 그대로 실행한다.
6. Verified absence 뒤 live binding+lifecycle+merge target을 permanent removed tombstone으로 atomic retire한
   receipt만 반환한다.

Public operation budget은 native-owned 120초다. 같은 `(scope, threadId)`의 concurrent call/retry와 response
loss는 persisted journal/tombstone을 통해 동일 `removalId`와 exact receipt로 수렴한다. Removed tombstone은
resume/turn/PTY/Changes/rename/unarchive/fork authority가 아니며 worktree identity도 재사용할 수 없다.

## 4. Inventory eligibility

Inventory public input은 계속 exact `{scope}`이고 모든 row는 `preserved: true`다. Native는 stable Archived
binding에만 persisted merge target으로 bounded proof를 시도한다.

- Positive proof는 committed `headDrift`를 removal 관점에서 해소하고
  `mergeProofUnavailable`를 추가하지 않는다.
- Not-merged/unavailable proof 또는 proof 뒤 binding/lifecycle/authority/removal join drift는
  `mergeProofUnavailable`다.
- Active/transition/Unknown binding, unfinished preparation, unavailable/dirty/branch-attached root와 남은
  다른 blocker는 그대로 닫힌다.
- Blocker가 하나도 없는 stable Archived binding만 `canRemove: true`다.

Inventory proof와 `canRemove`는 action eligibility일 뿐 deletion authority나 receipt가 아니다. Public
command가 store/Git/physical state를 다시 검증하므로 inventory와 command 사이의 external drift는 fail
closed한다. Removed tombstone은 live inventory나 thread list에 다시 투영하지 않는다.

## 5. Frontend confirmation과 reconciliation

Strict frontend adapter는 `code_worktree_remove`에 `{input: {scope, threadId}}`만 보내고 exact receipt를 다시
검증한다. Inventory/thread list adapter도 반환 row의 scope가 요청 scope와 정확히 같은지 확인한다.

UI 동작은 다음과 같다.

1. `canRemove: true`인 Archived binding row만 thread-specific `Remove worktree` action을 렌더링한다.
2. Dialog는 execution path를 정보로 표시하고 transcript 보존/실행 root 영구 제거를 명시한다. Cancel은
   command를 호출하지 않고 원래 trigger로 focus를 돌린다.
3. Confirm 직전에 사용자 확인을 받은 exact thread coordinate와 native inventory row를 scope별 cache에
   보존하고 same-tick duplicate submission을 막는다. 이것은 deletion authority가 아니며 command는 native
   authority를 다시 검증한다. Command가 pending이거나 response가 유실돼도 기존 inventory/thread row를
   optimistic하게 지우지 않는다.
4. Receipt 없는 outcome-unknown attempt는 sidebar unmount/remount와 target row refetch-absence 뒤에도 남는다.
   같은 input의 destructive retry를 다시 확인받으며, retry는 removal을 완료하거나 existing tombstone
   receipt를 회수할 수 있다.
5. Receipt를 받은 뒤 exact inventory와 thread query를 cancel하고 QueryClient request de-duplication 밖에서
   두 native list를 새로 읽는다.
6. 두 authoritative 결과 모두 target thread absence를 보일 때만 두 cache를 교체하고 receipt state를
   해제한다. Peer rows/threads는 그대로 유지한다.
7. Receipt 뒤 list read 또는 absence verification이 실패하면 이를 removal 실패로 표현하지 않는다.
   Exact-scope committed receipt를 보존하고 다른 removal을 막은 채 list reconciliation만 retry한다.
   Sidebar unmount/remount 뒤에도 완료 배너와 refresh action이 남는다.
8. Dialog close는 connected trigger 또는 refresh fallback으로 focus를 돌린다. Header reconciliation은 사용자가
   다른 곳으로 이동한 뒤 focus를 빼앗지 않으며, 기존 control에 남은 focus는 pending 중에도 보존한다. 완료는
   polite live status로 transcript preserved 결과를 알린다.

이 경계 때문에 irreversible native command는 receipt가 없는 outcome-unknown recovery에서만 같은 exact
input으로 다시 호출된다. Receipt를 확보한 뒤의 UI 복구는 destructive command가 아니라 authoritative
read 두 개만 반복한다.

## 6. 계속 금지되는 우회

- Caller path/descriptor/worktree ID/ref/OID/proof/removal ID/force
- Public merge-proof command
- `--force`, `git clean`, `git reset`
- `git worktree remove`, `git worktree prune`
- Broad/pathname-recursive `remove_dir_all`
- Fetch/network/credential/PR proof
- Inventory row나 frontend state를 receipt/deletion authority로 재사용
- Optimistic inventory/thread row deletion
- Codex transcript/thread archive/delete/move
- Implicit orphan/archive/fork/start cleanup

## 7. 주요 변경 파일

Native/public contract:

- `desktop/src-tauri/src/code_workspace/bindings/removal.rs`
- `desktop/src-tauri/src/code_workspace/worktree_inventory.rs`
- `desktop/src-tauri/src/commands/code_workspace.rs`
- `desktop/src-tauri/src/code_workspace/mod.rs`
- `desktop/src-tauri/src/commands/mod.rs`
- `desktop/src-tauri/src/lib.rs`
- `desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`
- `desktop/src-tauri/src/code_workspace/fixtures/tauri-contract-v1.json`
- `desktop/src-tauri/src/code_workspace/contract_tests.rs`

Frontend/product surface:

- `desktop/src/features/code/api/types.ts`
- `desktop/src/features/code/api/schemas.ts`
- `desktop/src/features/code/api/codeWorkspace.ts`
- `desktop/src/features/code/state/codeSessionQueries.ts`
- `desktop/src/features/code/ui/CodeWorktreeInventorySection.tsx`
- `desktop/src/features/code/ui/CodeThreadSidebar.tsx`
- `desktop/src/testing/e2eBridge.ts`
- `desktop/tests/e2e/schoolx-code.spec.ts`
- 관련 contract/query/component tests

Normative docs:

- `docs/schoolx-2/SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`
- `docs/schoolx-2/SCHOOLX_CODE_DESIGN.md`
- 이 handoff

이전 `SESSION_HANDOFF_20260817_CODE_PHASE2_WORKTREE_PHYSICAL_REMOVAL.md`는 private engine 완료 시점의 역사적
기록이므로 수정하지 않았다.

## 8. 검증

이번 수직 슬라이스에서 Hermit 활성화 뒤 다음 범위를 검증했다.

```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo check --manifest-path desktop/src-tauri/Cargo.toml --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib -- -D warnings

cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  code_workspace::bindings::removal::tests:: -- --test-threads=1
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  code_workspace::worktree_inventory::tests:: -- --test-threads=1
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  code_workspace::contract_tests::worktree_removal_decision_gates_are_frozen_with_the_public_surface_open \
  -- --exact
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  code_workspace::contract_tests::tauri_command_input_enum_and_event_contract_is_exact \
  -- --exact
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  code_workspace::bindings::removal::physical::tests::public_scope_thread_removal_returns_only_the_native_derived_receipt \
  -- --exact --test-threads=1
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::bindings::removal::physical::tests:: -- --test-threads=1

pnpm --dir desktop typecheck
pnpm --dir desktop check:px-text
pnpm --dir desktop exec biome check \
  src/features/code/api/codeWorkspace.ts \
  src/features/code/api/schemas.ts \
  src/features/code/api/types.ts \
  src/features/code/api/codeWorkspace.contract.test.mjs \
  src/features/code/state/codeSessionQueries.ts \
  src/features/code/state/codeSessionQueries.test.mjs \
  src/features/code/ui/CodeWorktreeInventorySection.tsx \
  src/features/code/ui/CodeWorktreeInventorySection.test.mjs \
  src/testing/e2eBridge.ts \
  tests/e2e/schoolx-code.spec.ts
pnpm --dir desktop exec node --import ./test-loader.mjs \
  --experimental-strip-types --test \
  src/features/code/api/codeWorkspace.contract.test.mjs \
  src/features/code/state/codeSessionQueries.test.mjs \
  src/features/code/ui/CodeWorktreeInventorySection.test.mjs
pnpm --dir desktop build:e2e
pnpm --dir desktop exec playwright test tests/e2e/schoolx-code.spec.ts \
  --project=smoke --grep 'lists managed-root removal eligibility|confirms exact worktree removal'

jq empty desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json \
  desktop/src-tauri/src/code_workspace/fixtures/tauri-contract-v1.json
git diff --check
```

Native removal journal tests 10개, inventory tests 11개, removal/Tauri contract tests, public physical
entrypoint와 full physical module 24개(2개 subprocess entry는 ignored)가 통과했다. Frontend targeted
contract/query/component suites, typecheck, E2E build와 두 removal inventory/E2E scenario도 통과했다. 전체
`just ci`/infrastructure integration suite는 이 targeted handoff의 실행 범위가 아니다.

## 9. 작업 트리 주의

이 repository는 시작부터 다수의 tracked/untracked 사용자 변경을 포함한 dirty worktree였다. 이 slice는
관련 파일만 수정했고 stage/commit/reset/clean하지 않았다. 다음 세션도 첫 명령을
`. ./bin/activate-hermit && git status --short`로 실행하고 기존 변경을 보존해야 한다.

## 10. 다음 독립 slice

Phase 2에서 다음으로 남은 제품 slice는 **model/reasoning selector**다. 이 handoff에는 selector UI,
app-server model/reasoning mutation이나 persistence를 구현하지 않았다.

Stage/unstage/commit, branch/push/PR 연결을 포함한 **Git write handoff는 Phase 3**이며 역시 아직 구현하지
않았다. Safe removal을 Git write의 implicit cleanup으로 재사용하거나 scope를 넓히지 않는다.

## 11. 다음 세션 복사용 시작 요청

```text
SCHOOLX_CODE_DESIGN.md, SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md와 최신
SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md를 먼저 읽고 현재 작업 트리를 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Public safe-remove는 완료 상태로 보존해줘. Exact {scope, threadId}, native-derived 9-field receipt,
same-removal retry, no optimistic deletion, inventory+thread authoritative reconciliation과 transcript
preserved invariant를 바꾸지 마.

다음 독립 Phase 2 slice로 model/reasoning selector를 설계·구현해줘. Git write handoff는 Phase 3 범위로
유지하고 safe removal이나 caller Git authority와 결합하지 마. 기존 사용자 변경/untracked 파일을
보존하고 stage/commit/reset/clean하지 마.
```
