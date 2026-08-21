# SchoolX Code Phase 2 managed-worktree inventory 세션 인계

작성일: 2026-08-16

## 1. 현재 결론

Phase 2의 다음 독립 수직 슬라이스였던 exact-scope read-only managed-worktree inventory와
보존/removal-eligibility projection은 구현을 마쳤다.

- Public Tauri command는 `code_worktrees_list`, top-level argument는 exact `input` 하나다.
- Public input은 `{scope}`만 받는다. Path, worktree ID, descriptor, lifecycle, retention 또는
  removal claim은 frontend가 보낼 수 없다.
- Binding store v3를 한 번 읽어 exact scope의 managed binding+lifecycle과 unfinished managed
  start/fork preparation이 예약한 root만 row authority로 채택한다.
- Local checkout, `WORKTREES` directory orphan, 임의 linked worktree, unbound physical root는
  inventory에 자동 채택하지 않는다.
- 모든 row는 `preserved: true`, `canRemove: false`다. 실제 remove command/button은 없다.
- 한 root의 missing/symlink escape/repository drift/Git failure는 그 row의 tagged
  `unavailable` inspection과 closed blocker로 국소화한다.
- 전체 inventory의 Git 검사는 한 shared 30초 deadline을 사용한다. 남은 budget이 없으면 해당
  row를 unavailable로 닫고, native row error는 UTF-8 경계를 지키며 최대 512 bytes다.
- Read-only store는 exact app-data path만 열고 directory 생성, chmod repair, migration write를
  하지 않는다. Unix의 private owner/mode 계약이 다르면 수정하지 않고 fail closed한다.
- Binding snapshot을 얻은 뒤 Git read 동안 app-wide binding-store lock을 잡고 있지 않는다.
- List 전후 binding index, lifecycle/preparation, Git admin metadata, managed-root content가
  바뀌지 않는 zero-mutation 계약을 actual Git/store test로 고정했다.

기준 설계는 [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)다. 이전 완료 경계는
[`SESSION_HANDOFF_20260816_CODE_PHASE2_FORK.md`](SESSION_HANDOFF_20260816_CODE_PHASE2_FORK.md)다.

## 2. 작업 트리 보존 규칙

이 checkout에는 Phase 1/2 구현, 이번 inventory 구현, 사용자의 다른 변경이 함께
dirty/untracked 상태로 남아 있다. 기존 파일을 reset/checkout/delete하지 않았고 stage/commit도
하지 않았다.

다음 세션의 첫 명령도 반드시 아래와 같다.

```bash
. ./bin/activate-hermit && git status --short
```

특히 아래 범주는 기존 작업이므로 임의로 정리하지 않는다.

- `.dockerignore`, `.gitignore`, `deploy/`, `brand/`, `supabase/`
- `crates/buzz-core/src/relay.rs`
- SchoolX Code Phase 1/2 전체 native/frontend 변경
- project Git diff/exec 변경
- 기존 `SESSION_HANDOFF_*.md`
- 이번 inventory의 새 native/frontend/test 파일

Hermit 활성화 없이 Git/Rust/Node command나 hook을 실행하지 않는다. 새 작업에서도
stage/commit/reset/checkout/clean을 하지 않는다.

## 3. 고정한 public 계약

### 3.1 command와 input

Frozen command는 다음 하나가 추가됐다.

```text
code_worktrees_list
topLevelArgs: ["input"]
```

Input DTO는 exact shape다.

```text
CodeWorktreesListInput {
  scope: CodeThreadBindingScope
}
```

`executionRoot`, `worktreeId`, descriptor, lifecycle, authority, `preserved`, `canRemove`, blocker,
retention flag는 input에 없다. Strict native/TypeScript schema가 unknown field를 거부한다.

### 3.2 row DTO

각 row는 다음 tagged projection이다.

```text
CodeWorktreeInventoryRow {
  scope
  authority:
    | { type: "binding", threadId, lifecycle }
    | {
        type: "preparation",
        preparationId,
        operation: "start" | "fork",
        state: "prepared" | "starting",
        sourceThreadId
      }
  descriptor: {
    executionMode: "worktree",
    repositoryIdentity,
    executionRoot,
    baseRef,
    worktreeId: string
  }
  inspection:
    | { status: "available", headCommit, branch, dirty }
    | { status: "unavailable", error }
  preserved: true
  canRemove: false
  blockers: CodeWorktreeInventoryBlocker[]
}
```

Public row의 descriptor는 항상 managed worktree이며 `worktreeId`가 non-null이다. Preparation의
`sourceThreadId`는 start에서 null, fork에서 exact source다. Private starting recovery baseline은
store snapshot 경계에서 scrub해 Tauri로 내보내지 않는다.

Frontend adapter는 다음을 추가로 strict 검증한다.

- row scope가 요청한 exact scope와 같음
- descriptor repository identity가 row scope와 같음
- descriptor가 managed worktree이고 worktree ID가 non-empty임
- available/unavailable tagged shape에 서로의 필드가 섞이지 않음
- `preserved === true`, `canRemove === false`
- blocker가 closed enum의 non-empty, unique, native-order array임
- authority, lifecycle, inspection, blocker 조합이 native projection과 정확히 일치함
- managed public row에 방어용 `localCheckout` blocker가 나타나지 않음

### 3.3 closed blocker 순서

Frozen 순서는 다음과 같다.

```text
activeBinding
lifecycleUnsettled
unfinishedPreparation
localCheckout
unavailableRoot
dirtyRoot
branchAttached
headDrift
mergeProofUnavailable
```

Projection 규칙:

- stable Active binding → `activeBinding`
- Archiving/Unarchiving/Unknown binding → `lifecycleUnsettled`
- 모든 Prepared/Starting start/fork preparation → `unfinishedPreparation`
- local checkout → 방어용 `localCheckout`; managed inventory source에서는 제외됨
- unavailable inspection → `unavailableRoot`
- available dirty root → `dirtyRoot`
- available branch-attached root → `branchAttached`
- available `HEAD != baseRef` → `headDrift`
- 모든 Archived binding → 다른 blocker와 별개로 `mergeProofUnavailable`

따라서 Archived root가 clean/detached/base-matching이어도 can-remove가 아니다. 반대로 dirty,
branch-attached, HEAD drift, unavailable Archived row도 각각의 blocker 뒤에
`mergeProofUnavailable`를 함께 가진다.

## 4. Native 구현

### 4.1 authority snapshot

주요 파일:

- `desktop/src-tauri/src/code_workspace/worktree_inventory.rs`
- `desktop/src-tauri/src/code_workspace/worktree_inventory/tests.rs`
- `desktop/src-tauri/src/code_workspace/bindings.rs`
- `desktop/src-tauri/src/code_workspace/bindings/lifecycle.rs`
- `desktop/src-tauri/src/commands/code_worktree_inventory.rs`
- `desktop/src-tauri/src/code_workspace/worktrees.rs`

`CodeThreadBindingStore::list_managed_inventory_authority`는 current v3 index를 한 번 strict load한다.
그 한 snapshot에서 exact scope와 `executionMode: worktree`가 일치하는 binding+lifecycle 및
unfinished preparation만 복사한다. Binding row와 preparation row는 store의 deterministic order를
유지한다.

다음 항목은 authority가 아니다.

- `WORKTREES` directory crawl 결과
- Git이 알고 있는 임의 linked worktree
- persisted record가 없는 physical orphan
- local checkout binding/preparation
- frontend가 표시 중인 path 또는 task descriptor
- 기존 thread list/app-server membership만으로 추론한 root

Store-level index decode/path/private-permission 오류는 trustworthy authority snapshot 자체가 없으므로
list 전체를 fail closed한다. Snapshot을 얻은 뒤 개별 root inspection 오류만 row-local로 국소화한다.

### 4.2 exact-path read-only store

`CodeThreadBindingStore::for_app_data_read_only`는 mutation용 constructor와 분리되어 있다.

- app-data는 absolute, existing, non-symlink directory여야 한다.
- `code`는 canonical app-data 바로 아래의 exact child여야 하며 symlink/escape를 거부한다.
- `code`가 없으면 빈 inventory로 처리하고 directory를 생성하지 않는다.
- index target은 exact `code/thread-bindings.json` regular file만 허용한다.
- index가 없으면 empty current index를 in-memory로 읽고 파일을 만들지 않는다.
- open file handle을 얻은 뒤 named parent/target을 다시 검사하고 그 handle만 parse한다.
- V1/V2 strict in-memory migration을 포함한 load는 원본 bytes/mtime을 쓰지 않는다.
- read-only store에서 `save`를 호출하면 거부한다.

Unix에서는 app-data owner를 기준으로 `code` owner와 exact `0700`, binding index owner와 exact
`0600`을 검사한다. Mode가 넓거나 owner가 다르면 chmod/chown으로 복구하지 않고 list를 거부한다.
Non-Unix에서도 exact path, canonical child, symlink, regular-file 검증은 유지한다.

### 4.3 row-local Git inspection

각 native-derived descriptor는 read-only로 다음을 검사한다.

1. managed nest containment와 exact repository/worktree ID path
2. canonical Git top-level
3. Git common-dir 기반 repository identity
4. persisted immutable base commit의 의미와 OID
5. current HEAD commit
6. attached branch 여부
7. tracked/untracked dirty 상태

Git environment는 system/global config, credential helper, hooks, fsmonitor, external protocol,
replacement object와 repository filter execution을 차단하고 read에는 `GIT_OPTIONAL_LOCKS=0`을 쓴다.
Available inspection은 상태를 정보로 반환하고, 실패는 sibling row를 숨기지 않는 Unavailable
inspection이 된다.

### 4.4 shared deadline과 bounded error

Inventory call 하나가 `Instant::now() + 30s` deadline을 한 번 만든다. 모든 row와 그 안의 모든 Git
subprocess가 이 deadline을 공유한다. 각 subprocess timeout은 기본 Git timeout과 남은 inventory
budget 중 작은 값이다.

- 한 row가 budget을 모두 소비해도 list 전체를 top-level error로 버리지 않는다.
- timeout Git child는 terminate/kill/reap하고 그 row를 unavailable로 닫는다.
- 이미 deadline이 끝난 뒤의 row도 `inspection budget was exhausted` unavailable 상태를 받는다.
- Native inspection error는 UTF-8 code-point를 자르지 않고 `[truncated]` suffix를 포함해 최대
  512 bytes로 제한한다.

이는 느리거나 악의적인 repository가 무한히 list를 붙잡거나 한 row error로 response를 비대하게
만드는 것을 막는다.

### 4.5 zero mutation과 lock 경계

List는 lifecycle reconciliation, app-server read/list/resume, preparation recovery, Git mutation,
directory creation, permission repair를 호출하지 않는다. Actual-Git test가 호출 전후 다음을 비교한다.

- binding index bytes와 mtime
- app-data/code tree의 file bytes, symlink target, mode, mtime
- Git common-dir/admin tree의 같은 recursive snapshot
- binding이 가리키는 managed root 전체 snapshot
- persisted five-state lifecycle projection
- unfinished preparation projection

Command는 blocking filesystem/Git work를 async runtime의 `spawn_blocking`으로 넘긴다. App-wide
binding-store mutation mutex를 Git inspection 동안 유지하지 않는다. Validated index snapshot 뒤에
다른 app command가 binding을 갱신할 수 있으므로 inventory는 refresh 가능한 read-only projection일
뿐 mutation serialization receipt가 아니다.

다른 local process도 여러 Git read 사이에 pathname, repository metadata, HEAD 또는 worktree content를
바꿀 수 있다. Per-command validation과 timeout은 redirect/hang 범위를 줄이지만 whole-list atomic
filesystem snapshot을 만들지는 않는다. 이 multi-process pathname inspection은 정보성 residual이며
현재도, 후속 설계에서도 그 자체로 deletion authority가 될 수 없다.

## 5. Frontend 구현

주요 파일:

- `desktop/src/features/code/api/types.ts`
- `desktop/src/features/code/api/schemas.ts`
- `desktop/src/features/code/api/codeWorkspace.ts`
- `desktop/src/features/code/api/codeWorkspace.contract.test.mjs`
- `desktop/src/features/code/state/codeSessionQueries.ts`
- `desktop/src/features/code/state/codeSessionQueries.test.mjs`
- `desktop/src/features/code/state/useCodeThreadMutations.ts`
- `desktop/src/features/code/ui/CodeWorktreeInventorySection.tsx`
- `desktop/src/features/code/ui/CodeWorktreeInventorySection.test.mjs`
- `desktop/src/features/code/ui/CodeThreadSidebar.tsx`
- `desktop/src/features/code/ui/CodeWorkspaceScreen.tsx`

UI/adapter behavior:

- Query key는 community/project/repository scalar를 모두 포함한 exact scope key다.
- Standalone `CodeWorktreeInventorySection`이 query를 직접 소유한다.
- Existing Code task sidebar에 `Managed worktrees` section을 추가했다.
- 각 row는 `Preserved`, authority 상태, exact closed blocker label, execution root를 보여준다.
- Unavailable row는 native error를 그 row 안에 표시한다.
- Header `Refresh managed worktrees`와 inline `Retry inventory`가 error recovery를 제공한다.
- 기존 task refresh와 create/recover, archive/unarchive/fork mutation 뒤에도 inventory query를
  invalidate한다.
- Remove/delete/clean/prune action, optimistic row removal, mutable Keep flag는 없다.
- Icon-only refresh는 accessible label을 가지며 loading/error는 inline이고 새 motion을 추가하지 않았다.
- `CodeWorkspaceScreen.tsx`는 scope 전달 한 줄을 포함해 996줄로 1000줄 ceiling 아래다.

## 6. Frozen fixture/contract 변경

주요 파일:

- `desktop/src-tauri/src/code_workspace/fixtures/tauri-contract-v1.json`
- `desktop/src-tauri/src/code_workspace/contract_tests.rs`
- `desktop/src/features/code/api/codeWorkspace.contract.test.mjs`

Frozen Tauri 변경:

```text
command: code_worktrees_list
top-level args: input
strict input key: worktreesList
output key: worktreeInventory
enum key: worktreeInventoryBlocker
Tauri command count: 27
```

Pinned Codex 0.145 app-server method/schema/wire fixture에는 변화가 없다. Inventory는 binding store와
read-only Git authority만 사용하며 app-server RPC를 추가하지 않는다.

## 7. 검증 결과

현재 확인된 결과:

```text
focused Rust actual-Git/store inventory tests: 10 passed
focused worktrees tests: 15 passed, 1 ignored
frozen Tauri contract test: passed
final full Rust lib tests (serial): 2298 passed, 17 ignored, 0 failed
cargo clippy --lib -- -D warnings: passed
cargo fmt --all -- --check: passed
focused frontend contract/query/view tests: 31 passed
full desktop frontend unit tests: 4005 passed, 0 failed
focused inventory E2E: 1 passed

pnpm typecheck: passed
focused Biome: passed
pnpm check:px-text: passed
pnpm build:e2e: passed
git diff --check: passed
```

Final Rust run은 병렬 test의 shared-global 간섭을 배제하기 위해 `--test-threads=1`로 실행했다.
앞선 default-parallel 재실행 하나는 inventory와 무관한
`relay_admission::tests::concurrent_429_extends_the_window_for_parked_waiters`가 전역 시간 상태와
충돌했지만 exact 단독 재실행은 통과했고, 다른 parallel run은 unrelated readiness test에서 멈췄다.
Serial full run은 shared deadline와 512-byte final bound를 포함한 최종 tree에서 전부 통과했다.

Focused Rust 10개는 다음을 고정한다.

- closed blocker order와 `preserved: true`/`canRemove: false`
- exact-scope managed-only binding/start/fork authority와 local/orphan/linked-worktree 제외
- persisted Active/Archiving/Archived/Unarchiving/Unknown 및 Starting preparation
- missing root, symlink escape, repository identity drift의 row-local unavailable
- binding index/app-data/Git admin/managed roots/lifecycle/preparation zero mutation
- actual dirty, attached branch, HEAD drift와 Archived merge-proof blocker
- absent store directory를 생성하지 않는 read-only open
- insecure Unix `code`/index permission을 repair하지 않는 fail-closed open
- start/fork preparation authority
- exhausted shared budget의 row-local unavailable

Frontend/E2E는 exact `{input:{scope}}`, foreign-scope rejection, managed/non-null descriptor,
available/unavailable shape, literal preservation/removal state, unique native-order blocker,
active/archived/unavailable/start/fork rows, local checkout 부재, refresh failure와 retry, remove action 부재를
검증한다.

## 8. 남아 있는 기존 quality-gate 부채

이번 inventory 슬라이스의 제품 범위와 섞지 않은 기존 부채가 남아 있다.

- strict all-target clippy의 기존 `code_terminal.rs` test helper `too_many_arguments`
- Phase 1/2 및 다른 dirty 파일의 기존 desktop file-size ratchet 초과

새 inventory production/test 파일은 1000줄 이하이고 `CodeWorkspaceScreen.tsx`도 996줄이다. 기존
부채를 inventory 후속 작업에서 임의 분할하거나 allowlist/상한을 올려 숨기지 않는다.

## 9. 남은 작업과 권장 순서

1. **후속 Phase 2 — safe remove의 네 decision gate 설계**
   - 아래 10절의 authority/journal/semantics/deletion boundary를 먼저 고정한다.
   - Gate가 닫히기 전에는 remove mutation이나 button을 추가하지 않는다.
2. **후속 Phase 2 — 명시적인 safe remove mutation**
   - 네 gate와 crash/retry contract가 고정된 뒤 별도 수직 슬라이스로 구현한다.
3. **후속 Phase 2 — model/reasoning selector**
   - pinned 0.145 `model/list`, strict catalog, start/resume/turn selection과 최근 선택 복구가
     필요하다. Fork five-key wire와 inventory input은 바꾸지 않는다.
4. **남은 platform/runtime residual**
   - native PTY root revalidation과 spawn 사이 pathname replacement/disappearance,
     `portable-pty` missing-cwd fallback을 별도로 닫아야 한다.
   - app-server `command/exec` 계열은 아직 활성화하지 않는다.
5. **Phase 3/4**
   - Git stage/unstage/commit, branch/push/PR, inline diff context, 선택적 Talk 공유,
     `review/start`, dirty-patch fork 정책과 편집기 평가가 남아 있다.

Automatic orphan cleanup, directory scan 결과의 implicit adoption, frontend path를 대상으로 한 remove,
Git write handoff, dirty patch 복사, 기존 quality-gate 부채 정리는 별도 권한 설계 전에는 섞지 않는다.

## 10. 실제 remove 전에 해결할 decision gate

### 10.1 Merged authority

Clean/detached/base-matching은 merged proof가 아니다. Native가 어느 resolved ref/commit을 기준으로
worktree HEAD reachability를 증명하는지 고정해야 한다. 그 전에는 `HEAD == baseRef`도 eligibility
정보일 뿐 삭제 권한이 아니다.

### 10.2 Crash-safe durable removal journal

Git/filesystem mutation 전에 durable claim을 기록하고 response loss/process crash 뒤 exact 상태를
복구해야 한다. Archive lifecycle이나 start/fork preparation을 cleanup journal로 재해석하지 않는다.

### 10.3 Binding/transcript semantics

Worktree 제거 뒤 Codex transcript와 SchoolX recovery coordinate를 보존할지, tombstone 또는 별도
worktree lifecycle을 둘지 먼저 정해야 한다. Binding/preparation을 filesystem mutation보다 앞서
삭제하지 않는다.

### 10.4 Pinned deletion boundary

현재 helper와 inventory는 존재하는 target 안에서 read/add/checkout을 수행하거나 상태를 검사한다.
Path 자체가 사라지는 remove에는 별도 pre/postcondition, parent-handle pinning, race model과 crash
recovery가 필요하다. Multi-process pathname inspection 결과를 이 deletion receipt로 재사용하지 않는다.

후속 remove에서도 `--force`, `git clean/reset`, broad `remove_dir_all`, `git worktree prune`,
archive/fork recovery에 의한 implicit cleanup은 금지한다.

## 11. 새 세션 복사용 시작 요청

```text
SCHOOLX_CODE_DESIGN.md와 최신
SESSION_HANDOFF_20260816_CODE_PHASE2_WORKTREE_INVENTORY.md, 현재 작업 트리를 먼저 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Phase 1의 열 개 closure와 Phase 2 exact bound-thread PTY terminal, exact-bound 검색/이름 변경,
persisted five-state archive/unarchive lifecycle authority, pinned Codex 0.145 thread/fork,
exact-scope read-only managed-worktree inventory는 완료됐으므로 다시 구현하지 마.

Actual worktree remove는 merged-ref authority, crash-safe durable removal journal, 제거 뒤
binding/transcript semantics, path 자체를 지우는 pinned deletion boundary 네 gate가 모두 설계될
때까지 구현하지 마. Inventory의 clean/detached/HEAD/base 결과와 multi-process pathname inspection은
정보일 뿐 deletion authority가 아니야.

다음 작업은 네 gate를 먼저 문서와 contract/test로 고정하거나, 별도로 합의된 다른 독립 수직
슬라이스로 제한해줘. Model/reasoning selector, Git write handoff, dirty patch fork 복사, automatic
orphan cleanup, command/exec, 기존 quality-gate 부채 정리를 섞지 마.

기존 사용자 변경과 untracked 파일을 보존하고 stage나 commit은 하지 마.
```
