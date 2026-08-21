# SchoolX Code Phase 2 `thread/fork` 세션 인계

작성일: 2026-08-16

## 1. 현재 결론

Phase 2의 다음 독립 수직 슬라이스였던 pinned Codex 0.145 `thread/fork`는 구현과 검증을
마쳤다.

- Public Tauri input은 exact `{scope, threadId}`다.
- `lastTurnId`는 첫 슬라이스에 노출하지 않으며 source의 전체 persisted history를 fork한다.
- lifecycle-clean stable `active` managed binding, idle thread, clean source worktree만 source가
  될 수 있다.
- source의 current immutable HEAD에서 fresh detached managed destination을 먼저 만든다.
- source와 destination은 execution root와 worktree ID를 공유하지 않는다.
- preparation journal v3는 `operation: start | fork`와 fork의 exact `sourceThreadId`를 저장한다.
- Codex response와 recovery candidate의 ID, ancestry, cwd, source marker, source kind,
  non-ephemeral 상태를 검증한 뒤에만 binding을 commit한다.
- definitely-not-sent만 같은 preparation/destination을 `prepared`로 rollback한다.
- byte admission 이후 response loss와 4 MiB JSONL line 초과는 `starting`으로 sticky하게 남고,
  reload 후 list/read 검증으로 bind한다. `thread/fork`를 다시 보내지 않는다.
- dirty patch 복사와 worktree 자동 삭제는 포함하지 않았다.

기준 설계는 [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)다. 이전 완료 경계는
[`SESSION_HANDOFF_20260816_CODE_PHASE2_ARCHIVE.md`](SESSION_HANDOFF_20260816_CODE_PHASE2_ARCHIVE.md)다.

## 2. 작업 트리 보존 규칙

이 checkout에는 Phase 1/2 구현과 사용자의 다른 변경이 함께 dirty/untracked 상태로 남아 있다.
이번 세션은 기존 파일을 reset/checkout/delete하지 않았고 stage/commit도 하지 않았다.

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

Hermit 활성화 없이 Git/Rust/Node command나 hook을 실행하지 않는다. 새 작업에서도
stage/commit/reset/checkout/clean을 하지 않는다.

## 3. 고정한 Codex 0.145 계약

### 3.1 정확한 binary

기본 Hermit `codex`는 현재 0.147.0일 수 있으므로 계약 재생성에 사용하면 안 된다. 감사한
정확한 binary는 다음이다.

```text
/Users/kim-yonghun/.codex/packages/standalone/releases/0.145.0-aarch64-apple-darwin/bin/codex
codex-cli 0.145.0
sha256 1da3f4e0e96028b8a771814293c3033dafd1971f943f6c7e79b0897fe705f590
```

### 3.2 SchoolX outbound wire

Non-experimental `thread/fork` request는 다음 다섯 필드만 보낸다.

```json
{
  "threadId": "<exact source>",
  "cwd": "<fresh native destination>",
  "approvalPolicy": "on-request",
  "sandbox": "workspace-write",
  "threadSource": "schoolx-code/<preparation UUID>"
}
```

`lastTurnId`, `serviceName`, `model`, `modelProvider`, `config`, instructions, `ephemeral`,
experimental `excludeTurns`는 보내지 않는다. Public input에도 이 필드를 추가하지 않는다.

0.145 fork response는 copied turns 전체를 포함할 수 있다. 별도 `thread/forked` notification은
없으며 success response 뒤 optional `thread/tokenUsage/updated`, 기존 `thread/started`가 올 수
있다. SchoolX는 commit/recovery를 `thread/started` 도착에 의존시키지 않는다.

### 3.3 schema fixture

추가된 leaf와 hash는 다음과 같다.

```text
v2/ThreadForkParams.json
c7378b60b22d5ecd7b1cf421b36bb631751864dbe036e677a96f50f9f8751489

v2/ThreadForkResponse.json
c1abf6ed41cf4f1304ea514b9f636611a475d9eecafb68ca8e1eab9592a3c30f
```

Frozen totals:

```text
selected schema count: 64
supported method count: 13
selected aggregate: a275aa5c5bd96dffcf170601563be44d15661348140e78146113e465fa13cb31
selected leaf aggregate: bc17cb05292f21de7a70e50bca2789761505426911a780923a0fd2c83b0d4e23
full generated count/hash: 273 / 기존 값 유지
notification method count: 23 / 기존 값 유지
Tauri command count: 26
```

Selected schema tar를 macOS에서 다시 만들 때 AppleDouble `._*` entry가 생기지 않도록
`COPYFILE_DISABLE=1`을 사용한다.

## 4. Native 구현

### 4.1 preparation/store

주요 파일:

- `desktop/src-tauri/src/code_workspace/bindings.rs`
- `desktop/src-tauri/src/code_workspace/bindings/lifecycle.rs`

Binding index current schema는 v3다. V1/V2는 strict decode 후 in-memory에서 start preparation의
`operation: start`, `sourceThreadId: null`로 migration하고 load만으로 bytes/mtime을 바꾸지
않는다.

Store가 보장하는 핵심 invariant:

- start preparation은 source를 가질 수 없다.
- fork preparation은 managed worktree와 exact source를 가져야 한다.
- 같은 scope/source에는 unfinished fork preparation이 하나뿐이다.
- fork source binding은 같은 scope/repository의 managed binding이어야 한다.
- source/destination root와 worktree ID는 달라야 한다.
- fork `starting`은 recovery baseline을 반드시 가진다.
- NotSent rollback은 exact claimed snapshot만 복원한다.
- commit 시 source가 사라졌거나 다른 scope/mode가 되면 bind하지 않는다.

### 4.2 guarded fork와 semantic validation

주요 파일:

- `desktop/src-tauri/src/commands/code_thread_fork.rs`
- `desktop/src-tauri/src/commands/code_thread_fork/tests.rs`
- `desktop/src-tauri/src/code_workspace/protocol.rs`
- `desktop/src-tauri/src/code_workspace/runtime.rs`

Fresh fork 순서:

1. exact scope의 stable Active source와 app-wide lifecycle authority를 확인한다.
2. exact clean lifecycle checkpoint를 얻는다.
3. managed source가 clean이고 thread가 idle인지 확인한다.
4. source 소유 PTY를 terminate/drain한다.
5. source HEAD/clean 상태와 lifecycle checkpoint를 다시 확인한다.
6. current HEAD에서 fresh detached clean destination을 만든다.
7. fork preparation을 durable journal에 기록한다.
8. source/destination, idle, baseline, lifecycle proof를 다시 확인하고 `starting`으로 claim한다.
9. runtime/EventBridge/approval lock 아래 최종 source lifecycle/turn/approval 상태를 검사하면서
   `thread/fork` bytes를 admit한다.
10. response를 semantic validation하고 source completion receipt를 다시 검증하면서 preparation
    → binding과 child-clean lifecycle을 atomic commit한다.

Response hard checks:

- child ID != source ID
- `sessionId == child ID`
- `forkedFromId == source ID`
- `parentThreadId == null`
- top-level `cwd`와 `thread.cwd` 모두 canonical destination
- explicit `ephemeral: false`
- exact `threadSource == schoolx-code/<preparation UUID>`
- valid SchoolX source가 물려준 `source: appServer`
- quiescent/idle status

Preparation journal 생성 전 destination 검증이 실패해도 생성된 root는 삭제하지 않으며
structured error의 `executionRoot`로 반환한다.

### 4.3 sticky recovery와 startup reconciliation

주요 파일:

- `desktop/src-tauri/src/code_workspace/thread_lifecycle.rs`
- `desktop/src-tauri/src/commands/code_thread_lifecycle.rs`
- `desktop/src-tauri/src/commands/code_workspace.rs`

`code_thread_binding_recover`는 `(operation, state)`로 dispatch한다.

- Start + Prepared → 기존 `code_thread_start`
- Start + Starting → 기존 start recovery
- Fork + Prepared → exact persisted destination으로 fork를 한 번 계속
- Fork + Starting → candidate discover/read/resume/commit만 수행, 재-fork 금지

Starting recovery는 destination이 preparation base에서 detached/clean인지 다시 확인하고,
baseline 이후의 unbound candidate 중 exact marker/ancestry/root를 만족하는 하나만 선택한다.
Large-history response 복구에서 copied turns를 다시 받지 않도록 authoritative read/list는
`includeTurns:false`를 사용한다.

Startup authoritative graph에는 Fork+Starting preparation expectation을 별도로 전달한다.
따라서 response-lost child가 loaded/list에만 있어도 exact marker/root/ancestry가 맞으면 graph가
authority를 잃지 않으며 source archive는 pending child ancestry 때문에 닫힌다. 일반 binding의
strict parser는 완화하지 않았다.

## 5. Frontend 구현

주요 파일:

- `desktop/src/features/code/api/types.ts`
- `desktop/src/features/code/api/schemas.ts`
- `desktop/src/features/code/api/codeWorkspace.ts`
- `desktop/src/features/code/state/useCodeThreadMutations.ts`
- `desktop/src/features/code/state/useCodeThreadLifecycleSync.ts`
- `desktop/src/features/code/ui/CodeThreadActions.tsx`
- `desktop/src/features/code/ui/CodeThreadSidebar.tsx`
- `desktop/src/features/code/ui/CodeWorkspaceScreen.tsx`
- `desktop/src/testing/e2eBridge.ts`
- `desktop/tests/e2e/schoolx-code.spec.ts`

UI/adapter behavior:

- 기존 task row dropdown에 non-destructive `Fork task` action을 추가했다.
- stable Active, available managed-worktree row에만 활성화한다.
- 같은 source를 가리키는 unfinished fork preparation이 있으면 duplicate fork를 막는다.
- pending/error 동안 source URL selection과 `aria-current`를 유지한다.
- API result는 scope, binding/thread identity, distinct child, exact `forkedFromId`, destination cwd,
  managed worktree, non-ephemeral 상태를 strict 검증한다.
- 성공 후 threads와 preparations를 둘 다 authoritative refetch한 뒤에만 새 child를 선택한다.
- fork child를 자동 resume하거나 PTY를 열지 않는다.
- preparation card는 `Prepared task`, `Recover task`, `Prepared fork`, `Recover fork` 경로를
  구분하고 fork의 Prepared/Starting을 모두 binding recovery로 보낸다.
- motion spinner는 `motion-reduce`에서 비활성화하며 기존 Radix/button primitives를 사용한다.

## 6. Frozen fixture/contract 변경

주요 파일:

- `desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-schema-manifest.json`
- `desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-selected-schemas.tar.gz.base64`
- `desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-wire.json`
- `desktop/src-tauri/src/code_workspace/fixtures/tauri-contract-v1.json`
- `desktop/src-tauri/src/code_workspace/contract_tests.rs`
- `desktop/src/features/code/api/codeWorkspace.contract.test.mjs`

Historical `thread-bindings-v1.json`은 새 preparation 필드 없이 그대로 유지한다. Current-version
fixture가 fork preparation을 고정한다. Wire fixture는 exact five-key params, copied turn response,
distinct child/ancestry/destination/marker와 empty-turn `thread/started`를 고정한다.

## 7. 검증 결과

통과:

```text
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo check --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib -- -D warnings
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib --quiet
  2288 passed, 17 ignored, 0 failed

fork 관련 필터: 9 passed
code_thread_fork module: 5 passed
binding lifecycle module: 14 passed
pinned 0.145 manual schema regeneration audit: passed

pnpm typecheck
Biome focused Code/E2E check
Code contract/view/query tests: 32 passed
pnpm check:px-text
pnpm build:e2e
schoolx-code.spec.ts smoke: 20 passed
git diff --check
```

Fork native tests는 다음을 실제 임시 Git repository/fake pinned server로 검증한다.

- current clean HEAD에서 distinct managed destination 생성과 exact five-key wire
- deterministic NotSent에서 Prepared rollback 후 같은 destination continuation
- 4 MiB 초과 post-write response에서 Starting 유지 후 restart recovery, fork resend 0회
- response/candidate ID, ancestry, root, marker, source, ephemeral strict rejection
- response 뒤 source lifecycle drift 시 atomic commit 거부
- list-absent pending child startup graph admission은 exact expectation에만 허용

## 8. 남아 있는 기존 quality-gate 부채

이번 fork 슬라이스가 새로 만든 실패는 아니다.

- strict `cargo clippy --all-targets -- -D warnings`는 기존 untracked
  `commands/code_terminal.rs:138` test helper의 `too_many_arguments` 한 건에서 멈춘다.
- 같은 command에 `-A clippy::too_many_arguments`를 주면 all-targets가 통과한다.
- `pnpm check:file-sizes`는 Phase 1/2와 다른 기존 dirty 파일 여러 개가 baseline ratchet을
  이미 넘겨 실패한다. 새 `code_thread_fork.rs`(728), fork tests(678),
  `CodeWorkspaceScreen.tsx`(992)는 각각 1000줄 이하이고 실패 목록에 없다.

이 부채를 fork 후속 작업과 섞어 임의로 분할하거나 allowlist/상한을 올리지 않는다.

## 9. 남은 작업과 권장 순서

현재 남은 범위와 권장 순서는 다음과 같다.

1. **다음 세션 — read-only managed-worktree inventory와 보존/제거 eligibility**
   - exact scope의 durable binding과 unfinished start/fork preparation이 예약한 managed root만
     목록화한다.
   - 현재 정책인 `preserved: true`와 제거를 막는 native-derived reason을 보여준다.
   - 실제 remove, directory orphan scan, 자동 채택/정리는 아직 하지 않는다.
2. **후속 Phase 2 — 명시적인 safe remove mutation**
   - 아래 10.2의 네 decision gate를 먼저 해결한 뒤 별도 수직 슬라이스로 구현한다.
3. **후속 Phase 2 — model/reasoning selector**
   - pinned 0.145 `model/list` 계약, strict normalized catalog, start/resume/turn 선택과 최근 선택
     복구가 필요하다. Fork의 five-key wire와 recovery input은 바꾸지 않는다.
4. **남은 platform/runtime residual**
   - native root revalidation과 PTY spawn 사이의 pathname replacement/disappearance,
     `portable-pty` missing-cwd fallback을 닫아야 한다.
   - 별도 app-server `command/exec` 계열은 아직 활성화 대상이 아니다.
5. **Phase 3/4**
   - Git stage/unstage/commit, branch/push/PR 연결, inline diff context, 선택적 Talk 공유,
     `review/start`, dirty-patch fork 정책, 편집기 필요성 평가가 남아 있다.

기존 all-target clippy/file-size 부채는 제품 범위와 섞지 않는다. Automatic orphan cleanup과
frontend가 넘긴 임의 경로를 대상으로 한 remove도 별도 권한 설계 전에는 구현하지 않는다.

## 10. 다음 세션 범위: read-only worktree inventory

### 10.1 고정할 최소 계약

- Public list input은 exact `{scope}`만 받는다. Frontend가 path, worktree ID, descriptor,
  lifecycle 또는 `canRemove`를 주장하게 하지 않는다.
- Authority는 binding store v3의 exact-scope managed binding+lifecycle과 unfinished managed
  preparation이다. `WORKTREES` directory crawl, Git의 임의 linked worktree, local checkout,
  unbound physical orphan은 결과에 자동 채택하지 않는다.
- 한 unavailable/invalid root가 다른 row를 숨기지 않도록 row별 status/error로 반환한다.
- `preserved`는 현재 모든 row에서 native-derived default다. 자동 retention policy가 없으므로
  별도 mutable Keep flag는 이번 슬라이스에 추가하지 않는다.
- 모든 preparation과 Active/Archiving/Unarchiving/Unknown binding은 제거 불가다. Archived
  managed binding도 root, clean, detached HEAD, immutable base를 읽기 전용으로 검사하되,
  merge target authority가 없으므로 이번 슬라이스에서는 제거 가능으로 만들지 않는다.
- Blocker는 free-form frontend 추론이 아니라 native가 만든 closed reason으로 표현한다. 최소한
  lifecycle unsettled, active binding, unfinished preparation, local checkout, unavailable root,
  dirty root, branch-attached/HEAD drift, merge proof unavailable를 서로 구분한다.
- List 호출은 binding index bytes/mtime, Git worktree/admin metadata, filesystem content,
  lifecycle/preparation을 바꾸지 않아야 한다.
- UI는 기존 Code task surface에서 `Preserved` 상태와 정확한 blocker를 읽기 전용으로 보여준다.
  Remove button이나 optimistic row 제거는 이번 슬라이스에 넣지 않는다.

### 10.2 실제 remove 전에 해결할 decision gate

1. **Merged authority** — 단순 clean은 merged가 아니다. 어느 native-resolved ref/commit을 기준으로
   `HEAD` reachability를 증명할지 고정한다. 그 전에는 가장 좁게 `HEAD == baseRef`조차
   eligibility 정보일 뿐 삭제 권한이 아니다.
2. **Crash journal** — Git/filesystem mutation 전에 durable claim을 기록하고 response loss와
   process crash 뒤 exact 상태를 복구하는 store state가 필요하다. Archive lifecycle이나 fork
   preparation을 cleanup journal로 재해석하지 않는다.
3. **Binding semantics** — worktree를 제거한 뒤 Codex transcript와 SchoolX recovery coordinate를
   보존할지, 별도 tombstone/worktree lifecycle을 둘지 먼저 정한다. Binding/preparation을
   filesystem mutation보다 앞서 삭제하지 않는다.
4. **Pinned deletion boundary** — 현재 pinned helper는 존재하는 target 안에서 add/checkout/read를
   수행하고 실행 전후 같은 directory chain을 검증한다. Path 자체가 사라지는 remove에는 별도
   pre/postcondition과 race model이 필요하다.

실제 remove 후속에서도 `--force`, `git clean/reset`, broad `remove_dir_all`, `git worktree prune`,
archive/fork recovery에 의한 implicit cleanup은 금지한다. Dirty patch 복사와 Git write handoff도
섞지 않는다.

### 10.3 다음 세션 검증 기준

- Rust actual-Git/store tests: exact scope 격리, binding/preparation ownership, stable/transition
  lifecycle, dirty/HEAD/branch/missing root, symlink/path escape, repository identity drift,
  한 invalid row의 국소화, 호출 전후 zero mutation.
- Frozen Tauri contract와 frontend strict schema/API transport test.
- Pure view tests: closed blocker label, preserved state, unavailable row, remove action 부재.
- E2E: active/archived/prepared/fork-preparation row, refresh/error recovery, no filesystem mutation.
- Hermit에서 Rust fmt/check/lib clippy/full lib test, frontend typecheck/focused Biome/unit,
  `check:px-text`, E2E build와 `schoolx-code` smoke, `git diff --check`를 실행한다.

## 11. 새 세션 복사용 시작 요청

```text
SCHOOLX_CODE_DESIGN.md와 최신
SESSION_HANDOFF_20260816_CODE_PHASE2_FORK.md, 현재 작업 트리를 먼저 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Phase 1의 열 개 closure와 Phase 2 exact bound-thread PTY terminal, exact-bound 검색/이름
변경, persisted five-state archive/unarchive lifecycle authority, pinned Codex 0.145
thread/fork 수직 슬라이스는 완료됐으므로 다시 구현하지 마.

다음 독립 수직 슬라이스는 read-only exact-scope managed-worktree inventory와
보존/removal-eligibility projection으로 고정해줘. Public input은 `{scope}`만 받고, binding store
v3의 managed binding+lifecycle과 unfinished start/fork preparation이 예약한 root만 native
authority로 사용해줘. Directory orphan scan, local checkout, frontend path/descriptor를 inventory에
자동 채택하지 마. 한 invalid root가 다른 row를 숨기지 않게 row별 status/error를 반환해줘.

모든 row는 현재 보존이 기본이고 이번 슬라이스에는 실제 remove command/button을 넣지 마.
Active/transition/Unknown binding과 모든 unfinished preparation은 hard blocker로, Archived root도
clean/detached/HEAD/base 상태를 read-only 검사하되 merged authority가 없으므로 can-remove로 만들지
마. Native closed blocker reason과 zero-mutation test를 먼저 고정해줘. List 전후 binding index,
Git admin/worktree metadata, filesystem 내용이 바뀌지 않아야 해.

실제 remove는 merged-ref authority, crash-safe durable removal journal, 제거 뒤 binding/transcript
semantics, path 자체를 지우는 pinned deletion boundary 네 가지가 설계될 때까지 미뤄줘.
Model/reasoning selector, Git write handoff, dirty patch fork 복사, automatic orphan cleanup,
`command/exec`, 기존 quality-gate 부채 정리는 이번 범위에 섞지 마.

기존 사용자 변경과 untracked 파일을 보존하고 stage나 commit은 하지 마.
```
