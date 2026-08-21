# SchoolX Code Phase 2 인계

> 후속 최신 인계는
> [`SESSION_HANDOFF_20260816_CODE_PHASE2_ARCHIVE.md`](SESSION_HANDOFF_20260816_CODE_PHASE2_ARCHIVE.md)다.
> Archive/unarchive lifecycle authority는 완료됐으므로 새 세션은 후속 인계를 우선한다.

작성일: 2026-08-16

상태: Phase 1의 열 개 closure, Phase 2 exact bound-thread PTY terminal,
exact-bound 로컬 검색과 pinned Codex 0.145 이름 변경까지의 archive 전 기준선.
Archive/unarchive 구현 결과와 다음 순서는 위 후속 최신 인계를 따른다.

이 문서는 archive 구현 전의 역사적 기준선이다. 현재 작업 트리와
`SCHOOLX_CODE_DESIGN.md`, 위 후속 최신 인계를 우선한다.

## 1. 새 세션 시작 순서

첫 명령은 반드시 다음과 같다.

```bash
. ./bin/activate-hermit && git status --short
```

그 뒤 아래 순서로 문맥을 복구한다.

1. [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
2. 이 문서
3. Phase 1의 상세 hardening 근거가 필요한 경우에만
   [`SESSION_HANDOFF_20260814_CODE_PHASE1F.md`](SESSION_HANDOFF_20260814_CODE_PHASE1F.md)

모든 Git, Rust, Node 검증 명령 전에 Hermit 환경을 활성화한다. stage, commit, reset,
clean, 기존 사용자 변경 재포맷은 하지 않는다. UI E2E는 반드시 `pnpm build:e2e`로
artifact를 다시 만든 뒤 실행한다.

## 2. 다시 구현하지 않을 완료 범위

Phase 1의 다음 열 개 closure는 모두 끝났다.

1. Changes freshness exact call-count E2E
2. E2E start/resume exact-binding fail-closed
3. pinned Codex 0.145 permission actual boundary
4. Git replacement-object 차단
5. runtime diagnostic egress redaction
6. cross-platform app-server descendant cleanup
7. permission display/authority 분리
8. authoritative checkpoint와 listener-first recovery
9. generation Changes/prompt reconciliation
10. Changes completeness/status, bounded manifest와 drift retry

Phase 2에서도 아래 두 수직 슬라이스가 끝났다.

- exact bound-thread native PTY ownership → typed resize/stdin/terminate → `Cmd/Ctrl+J`
  bottom drawer
- exact-scope bound-result 로컬 검색 → pinned 0.145 `thread/name/set` 이름 변경

이 기능을 재구현하거나 app-server `command/exec`와 통합하지 않는다.

## 3. 완료한 exact bound-thread PTY terminal

### 3.1 소유권 경계

- `commands/project_terminal.rs`는 외부 OS Terminal 앱을 여는 기존 escape hatch다.
  child, PTY, stdin, output, resize, terminate handle을 소유하지 않으며 그대로 유지했다.
- `CodeRuntime`은 `codex app-server` JSONL child만 소유한다. 사용자 OS shell PTY와 process,
  session, output event, shutdown authority를 공유하지 않는다.
- 별도 `CodeTerminalManager`가 `AppState`에서 native PTY master/writer/child와 session map을
  소유한다.
- frontend는 cwd, shell path, argv, env를 보내지 않는다. open은 `{scope, threadId, cols,
  rows}`만 받고 persisted binding의 `executionRoot`를 native에서 lookup/revalidate한다.
- native는 platform 기본 사용자 shell을 선택하되 managed-agent `RESERVED_ENV_KEYS`를 child
  environment에서 제거하고 `TERM=xterm-256color`, `COLORTERM=truecolor`만 보완한다.
- stdin/resize/terminate는 `{scope, threadId, sessionId}` 전체 owner를 매번 검증한다.
  stale session 또는 다른 scope/thread의 session ID는 거부한다.
- raw output은 global `CodeWorkspaceEvent`, replay ring, transcript, Nostr, diagnostic log에
  넣지 않는다. open 시 전달한 전용 Tauri `Channel<CodeTerminalEvent>`로만 보낸다.

현재 terminal wire는 다음과 같다.

```text
open      { scope, threadId, cols, rows }
  ->      { scope, threadId, sessionId, cols, rows }

resize    { scope, threadId, sessionId, cols, rows }
stdin     { scope, threadId, sessionId, data: byte[] }
terminate { scope, threadId, sessionId }

output    { type, scope, threadId, sessionId, sequence, data: byte[] }
exit      { type, scope, threadId, sessionId, sequence, exitCode, signal }
```

제한은 active session 8개, stdin 64 KiB, dimension 1..1000, output chunk 16 KiB,
bounded control/output/writer queue다. sequence는 session별 1부터 단조 증가한다.

### 3.2 lifecycle과 process cleanup

- 동일 exact owner의 재-open은 이전 session을 종료·reap하고 새 opaque UUID를 등록한다.
- manager는 open-vs-drain race를 막는 `Accepting | Draining | Shutdown` gate를 가진다.
- 일반 `terminate_all` 뒤에는 다시 열 수 있지만 app shutdown은 permanent shutdown gate다.
- app shutdown과 macOS main-window `CloseRequested` tray-hide 경로에서 terminal manager를
  app-server와 별도로 drain한다.
- Unix child는 새 POSIX session leader여야 start가 성공한다. explicit terminate와 natural
  leader exit 모두 같은 SID의 관찰 가능한 member를 정리하고 leader를 reap한다.
- Windows는 kill-on-close Job Object에 child를 즉시 배치한다. validated `cmd.exe /D`로
  AutoRun을 억제하고 Job 배치 실패 시 tree cleanup 뒤 start를 fail closed한다.
- pre-24H2 `ClosePseudoConsole`의 blocking 동작 때문에 destructive close는 Job을 먼저
  drop하고 reader를 discard/drain mode로 전환한다. natural exit는 별도 closer가 master를
  닫는 동안 actor가 output을 drain한다.
- PTY 생성 뒤 실패하는 모든 Windows rollback 경로는 `SpawnMasterGuard`가 master를 closer로
  넘긴다. helper 전송 자체가 불가능하면 앱을 동기 deadlock시키는 drop 대신 master를 leak한다.

남아 있는 platform residual은 정확히 다음과 같다.

- binding root의 canonical revalidation/`is_dir` 검사와 `portable-pty`가 cwd를 resolve해
  child를 spawn하는 사이에는 pathname replacement/disappearance TOCTOU가 남는다. frontend가
  cwd authority를 갖지는 않지만, 특히 local checkout 또는 같은 사용자가 바꿀 수 있는 managed
  root가 이 짧은 구간에 사라지면 library의 missing-cwd fallback을 native가 원자적으로 막지 못한다.
- `portable-pty 0.9`는 Windows suspended spawn/atomic Job assignment hook을 제공하지 않는다.
  running spawn과 즉시 Job 배치 사이의 작은 gap은 제거하지 못했다.
- 구형 Windows에서 `ClosePseudoConsole`이 영구 block하면 Job cleanup 뒤 detached helper와
  ConPTY handle 하나가 남을 수 있지만 app shutdown 자체를 block하지 않는다.
- Unix descendant가 의도적으로 새 `setsid()`로 원래 session을 탈출하면 cleanup 범위 밖이다.
- Windows target/runner는 없었다. macOS에서 실제 PTY lifecycle tests를 수행했고 Windows
  경로는 source/structural review만 했다. Windows cfg body의 compile/runtime은 검증하지 못했다.

### 3.3 drawer UX

- renderer는 `@xterm/xterm 6.0.0`과 `@xterm/addon-fit 0.11.0`을 사용한다.
- `Cmd/Ctrl+J`는 Code route의 window capture listener로 drawer를 여닫는다. xterm focus 중에도
  동작하지만 Escape는 shell/readline/vim 입력이므로 drawer를 닫지 않고 PTY로 전달한다.
- drawer hide는 session과 scrollback을 보존한다. explicit terminate action만 session을
  종료하며 AlertDialog 확인을 거친다.
- scope/thread route가 바뀌거나 component lifetime이 실제 종료되면 exact old session을
  terminate한다. React StrictMode probe unmount와 실제 unmount를 구분한다.
- open 시 xterm으로 focus하고 hide 시 이전 focus를 복원한다.
- `screenReaderMode`, semantic label, theme token, rem-derived font size, ResizeObserver/FitAddon,
  dark mode와 reduced motion을 반영했다.
- terminal availability는 app-server interaction generation이 아니라 route의 exact
  `selectedThreadId`와 native durable binding 검증에 의해 결정된다.

## 4. 완료한 검색과 이름 변경

### 4.1 로컬 검색

- `code_threads_list`는 먼저 exact scope binding store를 조회하고 각 bound thread를
  `thread/read`와 canonical cwd로 검증한다.
- 검색은 이 결과 array에서만 name, preview, full thread ID, short ID를 case-insensitive로
  필터링한다. app-server `thread/list.searchTerm`이나 foreign/unbound thread listing을
  추가하지 않았다.
- query가 선택된 row를 숨겨도 route/selection/opened thread는 바꾸지 않는다. Clear search는
  기존 selection을 그대로 복원하고 unfinished preparation은 검색 대상에서 제외하지 않는다.

### 4.2 이름 변경

- exact pinned Codex 0.145 schema에서 `thread/name/set {threadId,name} -> {}`를 frozen method,
  selected schema archive, manifest, wire fixture에 추가했다.
- public Tauri input은 `{scope, threadId, name}`뿐이다. cwd, path, model, argv나 다른 authority를
  받지 않는다.
- name은 non-empty, exact trimmed, control character 없음, 최대 128 Unicode scalar와
  512 UTF-8 byte로 Rust와 Zod 양쪽에서 제한한다.
- native facade는 exact persisted binding을 lookup하고 execution root를 revalidate한 뒤
  `thread/name/set`을 호출한다. 성공 직후 authoritative `thread/read`를 수행해 returned
  thread ID, canonical cwd, exact name을 모두 다시 검증한다.
- rename은 desired value가 명시된 idempotent metadata mutation이므로 별도 journal을 만들지
  않았다. 응답 유실 뒤 재시도해도 같은 이름을 다시 설정한다.
- UI는 row와 열린 thread header를 즉시 갱신하되 기존 opened `turns`를 보존하고 exact scoped
  query를 invalidate한다. 실패는 dialog-local alert로 남기고 기존 row/header/route를 보존한다.
- semantic `<ul>/<li>`, row별 Radix menu/dialog, pending state와 focus primitive를 사용했다.

## 5. 현재 contract 기준

- SchoolX Code public Tauri command: 23개
- pinned Codex 0.145 curated method: 10개
- 새 terminal command:
  `code_terminal_open`, `code_terminal_resize`, `code_terminal_stdin`,
  `code_terminal_terminate`
- 새 thread metadata command: `code_thread_rename`
- app-server `thread/archive`, `thread/unarchive`, `thread/fork`, `command/exec`은 아직 curated
  support에 포함하지 않았다.

현재 Hermit shell의 기본 `codex --version`은 `0.147.0`이지만 SchoolX runtime과 frozen
contract는 계속 `0.145.x`만 지원한다. exact 0.145 binary는 아래에 있다.

```text
/Users/kim-yonghun/.codex/packages/standalone/releases/0.145.0-aarch64-apple-darwin/bin/codex
```

Schema를 다시 생성하거나 actual-boundary probe를 실행할 때 기본 PATH의 0.147을 사용하지
않는다. 새 Codex version 지원으로 범위를 넓히지 않는다.

계약 변경 시 아래를 함께 갱신해야 한다.

```text
desktop/src-tauri/src/code_workspace/fixtures/tauri-contract-v1.json
desktop/src-tauri/src/code_workspace/contract_tests.rs
desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-schema-manifest.json
desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-selected-schemas.tar.gz.base64
desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-wire.json
desktop/src/features/code/api/{types.ts,schemas.ts,codeWorkspace.ts}
desktop/src/features/code/api/codeWorkspace.contract.test.mjs
desktop/src/testing/e2eBridge.ts
```

## 6. 주요 변경 위치

```text
desktop/src-tauri/Cargo.toml
desktop/src-tauri/Cargo.lock
desktop/src-tauri/src/app_state.rs
desktop/src-tauri/src/shutdown.rs
desktop/src-tauri/src/lib.rs
desktop/src-tauri/src/code_workspace/terminal.rs
desktop/src-tauri/src/code_workspace/terminal/{process.rs,tests.rs}
desktop/src-tauri/src/commands/code_terminal.rs
desktop/src-tauri/src/commands/code_thread_management.rs
desktop/src-tauri/src/code_workspace/{mod.rs,protocol.rs,runtime.rs,contract_tests.rs}
desktop/src-tauri/src/code_workspace/fixtures/{tauri-contract-v1.json,codex-0.145.0-*}

desktop/package.json
pnpm-lock.yaml
desktop/src/features/code/api/{types.ts,schemas.ts,codeWorkspace.ts,codeWorkspace.contract.test.mjs}
desktop/src/features/code/lib/{codeWorkspaceView.ts,codeWorkspaceView.test.mjs}
desktop/src/features/code/ui/{CodeWorkspaceScreen.tsx,CodeThreadSidebar.tsx}
desktop/src/features/code/ui/{CodeTerminalDrawer.tsx,CodeThreadRenameAction.tsx}
desktop/src/shared/lib/keyboard-shortcuts.ts
desktop/src/testing/e2eBridge.ts
desktop/tests/e2e/schoolx-code.spec.ts
docs/schoolx-2/SCHOOLX_CODE_DESIGN.md
```

새 native Phase 2 파일은 모두 1,000줄 미만이다.

```text
terminal.rs                 963
terminal/process.rs         794
terminal/tests.rs           502
commands/code_terminal.rs   127
commands/code_thread_management.rs 178
```

frontend의 `CodeWorkspaceScreen.tsx`는 970줄, `CodeThreadSidebar.tsx`는 271줄,
`CodeThreadRenameAction.tsx`는 170줄이다. 다음 작업에서 1,000줄에 가까운 Screen이나 이미
oversized인 native owner를 더 키우지 말고 새 leaf module/facade로 분리한다.

## 7. 검증 기준선

모든 명령은 Hermit 활성화 뒤 실행했다.

```text
[x] cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
[x] cargo check --manifest-path desktop/src-tauri/Cargo.toml --locked
[x] cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --locked -- -D warnings
[x] terminal namespace: 13 passed
[x] rename facade: 2 passed
[x] rename runtime request/readback sequence: 1 passed
[x] native contract: 7 passed, pinned-manual 1 ignored
[x] focused frontend API/view contract: 23 passed
[x] pnpm typecheck
[x] targeted Biome check
[x] pnpm check:px-text
[x] full desktop unit: 3,993 passed
[x] pnpm build:e2e
[x] SchoolX Code smoke E2E: 15 passed
[x] git diff --check
[x] production 경로에 새 unsafe/unwrap()/expect() 없음
```

E2E 명령은 다음과 같다.

```bash
. ./bin/activate-hermit
pnpm --dir desktop build:e2e
pnpm --dir desktop exec playwright test tests/e2e/schoolx-code.spec.ts --project=smoke
```

`pnpm check:file-sizes`는 누적 Phase 1/Phase 2 dirty tree의 기존 oversized/ratchet 파일들 때문에
계속 실패한다. 새 terminal/rename 파일은 지목되지 않지만 `app_state.rs`, `lib.rs`, 기존
`code_workspace` 대형 파일과 Git diff/exec owner 등이 현재 gate에 남아 있다. ratchet을
올리거나 allowlist를 추가해 숨기지 않는다. 다음 feature가 이미 oversized인 파일을 더
키워야 한다면 먼저 leaf module로 추출한다.

## 8. 작업 트리 보존

SchoolX Code 작업과 무관한 기존 사용자 변경은 그대로 보존한다. 특히 다음 항목을 reset,
clean, stage, 전역 포맷하거나 SchoolX 변경으로 간주하지 않는다.

```text
.dockerignore
.gitignore
crates/buzz-core/src/relay.rs
deploy/compose/README.md
deploy/compose/Dockerfile.local
brand/
supabase/
desktop/src-tauri/src/managed_agents/restore.rs
desktop/src-tauri/src/managed_agents/runtime.rs
desktop/src-tauri/src/managed_agents/runtime/tests.rs
desktop/src-tauri/src/managed_agents/runtime_commands.rs
```

Phase 1/2의 SchoolX Code 변경도 아직 전부 unstaged이며 여러 새 파일과 인계 문서는 untracked다.
일반 `git diff`만으로는 보이지 않으므로 항상 `git status --short`를 함께 확인한다. 이 단계에서
stage, commit, push, PR 생성은 하지 않았다.

## 9. 다음 세션의 exact 구현 순서

다음 수직 슬라이스는 **archive/unarchive lifecycle authority**다. UI action이나 RPC facade부터
붙이지 말고 persisted lifecycle gate부터 구현한다.

### 9.1 먼저 고정할 persisted state

현재 binding v1의 public 8-field binding은 유지한다. binding index의 새 schema version에
각 exact binding의 lifecycle와 operation journal을 함께 저장한다.

```text
active
archiving   { operationId, ... }
archived
unarchiving { operationId, ... }
unknown     { operationId, target, ... }
```

- v1 load는 in-memory `active`로 migration하되 load만으로 파일을 쓰지 않는다.
- future/unknown schema와 invalid transition은 fail closed한다.
- lifecycle와 binding을 별도 파일로 나눠 atomic join을 잃지 않는다.
- transition을 app-server RPC 전에 durable save한다.
- definitely-not-sent만 이전 stable state로 rollback한다.
- timeout, partial write, response loss처럼 delivery가 불확실하면 `unknown`을 유지한다.
- 실행 권한은 오직 stable `active`에만 준다. `archiving`, `archived`, `unarchiving`,
  `unknown`은 crash/reload와 in-flight 중에도 auto-resume, composer, turn start, 새 PTY open을
  모두 fail closed한다.
- startup/refetch reconciliation은 uncertain operation뿐 아니라 stable state도 검증한다.
  persisted `active`가 archived membership에만 있거나 persisted `archived`가 active membership에만
  있는 out-of-band drift는 실행 가능 상태로 노출하지 않고 먼저 `unknown`으로 durable 전환한다.

### 9.2 공유 serialization 경계

archive/unarchive, terminal open, thread resume, turn start, 이후 fork는 같은 exact-binding
lifecycle lock/barrier를 사용해야 한다.

```text
binding/lifecycle -> terminal manager -> CodeRuntime
```

- notification dispatcher가 이 lock을 잡고 store write를 하면 notification-before-response
  deadlock이 가능하므로 notification은 refetch/reconcile 신호로만 취급한다.
- `turn/start` RPC 성공은 delayed `turn/started` notification을 기다리지 않고 같은 lifecycle
  lock 안에서 native in-flight/active gate에 동기 반영한 뒤 unlock한다. archive는 frontend
  replay/checkpoint만 믿지 않고 이 native authoritative gate를 검사하며, restart/resume 뒤 상태를
  확정하지 못하면 pinned `thread/read` 등 authoritative read로 증명하거나 RPC 전에 거부한다.
- archive 전 active turn과 pending approval이 있으면 명확히 거부한다.
- pinned archive는 spawned descendant까지 연쇄 archive할 수 있다. 첫 수직 슬라이스는 native가
  authoritative thread graph에서 exact target이 leaf임을 증명한 경우에만 허용한다. 이 증명은
  cwd filter 없이 active와 archived를 각각 조회하고, pinned 0.145의 모든 `ThreadSourceKind`
  (`cli`, `vscode`, `exec`, `appServer`, `subAgent`, `subAgentReview`, `subAgentCompact`,
  `subAgentThreadSpawn`, `subAgentOther`, `unknown`)을 `sourceKinds`에 명시해 cursor 끝까지
  exhaustive pagination한 뒤 `parentThreadId`/`forkedFromId` ancestry를 검사해야 한다. 현재
  recovery list builder의 `sourceKinds: ["appServer"]` 또는 default interactive-source 조회를
  이 증명에 재사용하지 않는다.
- graph가 불완전하거나 cursor cycle/page bound에 걸리거나 duplicate/conflicting ancestry가 있거나,
  결과가 `unknown` source인 경우, 또는 bound/unbound, 같은/다른 scope를 막론하고 descendant가
  하나라도 있으면 RPC 전에 fail closed한다. 새/미지원 source kind도 strict parse 단계에서 같은
  zero-RPC 결과로 귀결한다. descendant binding을 target lifecycle과 무관하게 active로 남기거나
  암묵적으로 함께 archive하지 않는다.
- archive는 SessionEnd hook을 실행할 수 있으므로 frontend cwd를 받지 않고, RPC 직전에 exact
  persisted binding의 execution root를 다시 revalidate한다.
- archive는 exact owner terminal을 먼저 terminate/drain/reap한 뒤 RPC를 보낸다.
- stable `active`만 composer, auto-resume, turn start, 새 terminal open을 허용한다.
- archived thread의 binding, worktree reservation, dirty file은 삭제하거나 clean/reset하지 않는다.
- stable `archived`는 Changes read, rename, explicit unarchive를 허용할 수 있다.
  `archiving`/`unarchiving`/`unknown`은 read-only inspection과 explicit reconcile만 허용하고 rename을
  포함한 새 mutation은 막는다.
- selected archived route가 다른 row를 자동 선택하거나 그 row의 PTY를 자동 open하지 않게 한다.

### 9.3 pinned 0.145 protocol 범위

정확한 0.145 generated schema에서 아래 method와 response/notification leaf만 새로 curate한다.

```text
thread/archive   { threadId } -> {}
thread/unarchive { threadId } -> { thread }
thread/archived
thread/unarchived
```

Frontend가 cwd, archive path, shell, worktree ID를 보내지 않게 한다. unarchive 응답의 thread ID와
cwd를 exact persisted binding과 검증한다. active/archived list membership reconciliation에서
target을 확정할 수 없거나 양쪽/어느 쪽에도 모호하면 `unknown`을 유지한다. stable `active`와
`archived`도 각각 기대한 membership과 정확히 일치하지 않으면 `unknown`으로 durable 전환한 뒤
explicit reconcile/unarchive 외 실행을 막는다.

`thread/archive`의 descendant cascade는 exact single-binding command 권한을 넓히지 않는다.
향후 cascade archive를 제품 기능으로 원하면 affected descendant의 전체 exact binding set을
RPC 전에 durable transition하는 별도 multi-binding transaction과 unbound/foreign descendant
거부 정책을 먼저 설계해야 한다. 현재 첫 슬라이스에서는 leaf-only가 closed semantics다.

### 9.4 archive 필수 테스트

- v1→v2 migration, no-write-on-load, strict future/unknown rejection
- wrong scope/thread가 store/RPC/terminal에 side effect 0회
- archive/unarchive success, definitely-not-sent rollback, uncertain delivery, save failure, reload
  reconciliation
- stable active/archived membership drift를 startup/refetch에서 `unknown`으로 durable 전환하고,
  archiving/unarchiving/unknown reload가 auto-resume/composer/turn/PTY를 모두 차단하는 검증
- archive vs terminal open, archive vs resume/turn start barrier race
- `turn/start` response 뒤 `turn/started` notification이 지연된 seam과 restart/resume active-turn
  seam에서 archive RPC 0회
- active+archived를 cwd filter 없이 모든 pinned source kind로 cursor 끝까지 열거한 authoritative
  graph의 no-descendant leaf success
- default/`appServer`-only source filter가 숨기는 subagent descendant, bound/unbound/foreign
  descendant, unknown/new source, cursor cycle/page bound, duplicate/conflicting ancestry의 zero-RPC
  fail-closed
- archive 전 terminal descendant cleanup과 exact terminate ack
- dirty worktree, binding, global worktree reservation 보존
- archiving/archived/unarchiving/unknown route가 auto-resume/composer/turn/PTY를 열지 않는 UI/E2E
- archived rename과 explicit unarchive 뒤 정상 복귀
- just-started empty pinned 0.145 thread의 recoverable archive behavior

## 10. archive 뒤의 fork 경계

Fork는 archive lifecycle gate 뒤 별도 수직 슬라이스다.

- managed source와 fork가 같은 managed root를 공유하도록 uniqueness invariant를 완화하지 않는다.
- native가 source의 current immutable HEAD에서 fresh destination worktree를 먼저 준비한다.
- 첫 의미론은 clean managed source만 허용한다. dirty patch 복사는 Git handoff 전 범위 밖이다.
- frontend input은 source scope/thread와 필요한 bounded option만 허용하고 cwd/base/worktree는
  native가 도출한다.
- preparation journal에 operation kind `start | fork`와 exact `sourceThreadId`를 저장한다.
- `thread/fork` 응답은 new ID != source, exact destination cwd, exact `forkedFromId`, preparation
  threadSource를 검증한 뒤 binding을 atomic commit한다.
- response loss는 sticky uncertain preparation으로 남기고 reload recovery가 exact ancestry와
  destination root를 확인한다. 실패 시 source/destination worktree를 자동 삭제하지 않는다.

## 11. 계속 범위 밖인 항목

- worktree remove/cleanup
- model/reasoning selector
- stage/unstage/commit/push/PR Git handoff
- SchoolX Talk/Nostr sharing
- review/model API
- app-server `command/exec` terminal 통합
- 새 Codex version/schema 지원
- Phase 1 Changes를 atomic filesystem snapshot이라고 표현하는 변경

## 12. 새 세션에 전달할 시작 요청

다음 문장을 새 세션의 첫 요청으로 사용할 수 있다.

```text
SCHOOLX_CODE_DESIGN.md와 최신
SESSION_HANDOFF_20260816_CODE_PHASE2.md, 현재 작업 트리를 먼저 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Phase 1의 열 개 closure와 Phase 2 PTY terminal, exact-bound 검색/이름 변경은
완료됐으므로 다시 구현하지 마.

다음 순서인 archive/unarchive lifecycle authority를 진행해줘. UI/RPC부터 붙이지 말고
binding index v2의 persisted active/archiving/archived/unarchiving/unknown gate와
delivery-aware journal을 먼저 구현한 뒤, terminal open/resume/turn start와 공유하는
exact-binding serialization과 turn-start response 시점의 native authoritative active-turn gate,
active+archived를 cwd filter 없이 pinned 0.145의 모든 source kind로 완전 pagination해 archive
descendant-cascade를 막는 authoritative leaf-only gate, pinned 0.145 archive/unarchive contract,
마지막으로 UI/E2E 순서의 최소 수직 슬라이스로 진행해줘. Stable
`active`만 실행을 허용하고 나머지 네 lifecycle state와 membership drift는 fail closed해줘.
Fork는 이번 슬라이스에 섞지 마.

기존 사용자 변경과 untracked 파일을 보존하고 stage나 commit은 하지 마.
```
