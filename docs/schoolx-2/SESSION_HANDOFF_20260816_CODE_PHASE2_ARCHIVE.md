# SchoolX Code Phase 2 archive/unarchive 완료 인계

작성일: 2026-08-16

상태: Phase 1의 열 개 closure와 Phase 2 exact bound-thread PTY terminal,
exact-bound 검색/이름 변경, persisted archive/unarchive lifecycle authority까지
구현·검증 완료. 다음 독립 수직 슬라이스는 `thread/fork`다.

이 문서는 현재 작업 트리의 최신 SchoolX Code 인계 기준선이다. 이전 Phase 2 인계의
“archive/unarchive를 다음에 구현”한다는 내용은 역사적 계획이며, 새 세션은 이 문서와
현재 작업 트리를 우선한다.

## 1. 새 세션 시작 순서

첫 명령은 반드시 다음과 같다.

```bash
. ./bin/activate-hermit && git status --short
```

그 뒤 아래 순서로 문맥을 복구한다.

1. [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
2. 이 문서
3. archive 설계 전 기준선이 필요할 때만
   [`SESSION_HANDOFF_20260816_CODE_PHASE2.md`](SESSION_HANDOFF_20260816_CODE_PHASE2.md)
4. Phase 1 hardening 근거가 필요할 때만
   [`SESSION_HANDOFF_20260814_CODE_PHASE1F.md`](SESSION_HANDOFF_20260814_CODE_PHASE1F.md)

모든 Git, Rust, Node 명령 전에 Hermit을 활성화한다. 기존 사용자 변경과 untracked
파일을 보존하고 stage, commit, reset, clean, broad rewrite를 하지 않는다. UI E2E는
반드시 `pnpm build:e2e`로 artifact를 다시 만든 뒤 실행한다.

## 2. 다시 구현하지 않을 완료 범위

Phase 1의 다음 열 개 closure는 이미 끝났다.

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

Phase 2에서도 다음 세 수직 슬라이스가 끝났다.

- exact bound-thread native PTY ownership, typed resize/stdin/terminate,
  `Cmd/Ctrl+J` terminal drawer
- exact-scope bound-result 검색과 pinned 0.145 `thread/name/set` 이름 변경
- persisted five-state lifecycle, strict authoritative graph, delivery-aware journal을
  사용하는 leaf-only archive/unarchive와 read-only archived UI

이 기능을 다시 만들거나 app-server `command/exec`와 PTY를 합치지 않는다.

## 3. persisted lifecycle과 journal

단일 app-data 파일 `code/thread-bindings.json`을 index v2로 확장했다.

```text
active
  -> archiving
  -> archived
  -> unarchiving
  -> active

어느 stable/in-flight 상태든 증거가 불완전하거나 결과가 모호하면 unknown
```

- public `CodeThreadBinding`은 기존 정확한 8개 field 그대로다. Lifecycle은 binding
  내부가 아니라 persisted sibling record와 public `CodeBoundThreadSummary.lifecycle`에 있다.
- v2는 모든 binding과 lifecycle record가 정확히 1:1이어야 한다. orphan, duplicate,
  unknown field, future version은 읽기 단계에서 fail closed한다.
- v1은 메모리에서 stable `active`로 승격해 읽되, load만으로 파일 byte나 mtime을
  바꾸지 않는다. 다음 실제 mutation에서만 v2를 atomic write한다.
- 저장은 기존 `AtomicWriteFile` 경계를 그대로 사용하며 validate, deterministic sort,
  owner-only mode, write, commit을 한 index transaction으로 수행한다.
- archive/unarchive는 RPC 전에 exact operation claim을 durable 기록한다.
- definitely-not-sent만 claim의 exact pre-RPC snapshot으로 rollback한다. response loss나
  delivery ambiguity는 재시도하지 않고 durable `unknown`으로 전환한다.
- RPC 성공 뒤 stable commit이 실패하면 stable 상태를 꾸며내지 않는다. 가능한 경우
  `unknown`을 저장하고, 그것도 실패하면 global authority latch를 닫은 채 오류를 반환한다.
- archived binding과 managed-worktree reservation은 계속 보존한다. dirty file, branch,
  worktree를 archive 과정에서 삭제·clean·reset하지 않는다.

## 4. native lifecycle authority

### 4.1 app-wide latch와 exact dirty gate

`AppState`에는 모든 persisted lifecycle이 현재 Codex generation의 authoritative graph와
durably reconcile됐음을 나타내는 fail-closed latch가 있다.

- runtime start/stop, reconcile 시작, graph/persistence 실패에서 latch를 먼저 닫는다.
- empty store 또는 전체 store reconcile과 durable save, dirty checkpoint clear가 모두
  성공한 뒤에만 latch를 연다.
- partial save 뒤 후속 binding 처리에 실패해도 latch가 닫혀 있으므로 아직 stable로
  남은 record가 resume/turn/PTY를 허가하지 못한다.
- `thread/archived`와 `thread/unarchived` notification은 binding store를 직접 수정하지
  않는다. runtime의 exact thread revision과 global graph revision만 dirty로 만든다.
- dirty checkpoint는 generation, runtime boundary, global graph revision, exact thread
  revision과 마지막 `Archived | Unarchived` signal을 함께 보존한다.
- graph 조회와 save 사이에 target 또는 다른 descendant의 lifecycle notification이 오면
  checkpoint clear가 실패한다. graph가 찢어진 두 membership snapshot을 권한 증거로 쓰지 않는다.
- `thread/started`와 `thread/closed`는 clean-thread set을 전역으로 지우지 않고 topology
  epoch만 전진시킨다. Graph scan에서 발급한 proof는 이 epoch를 그대로 RPC byte write까지
  운반하므로 scan 뒤 생긴 descendant를 새 checkpoint로 흡수할 수 없다.
- archive 응답 뒤 `Unarchived`, unarchive 응답 뒤 `Archived`가 먼저 관측되면 stable commit을
  하지 않고 `unknown`으로 닫는다. 응답 뒤에는 EventBridge lock 안에서 expected signal 검증,
  durable stable commit closure, native clean을 한 원자 경계로 수행한다. 같은 방향
  notification 또는 clean checkpoint만 성공 completion 증거로 인정한다.
- 새로 start/recover한 exact binding은 current ready generation, lifecycle notification 0회,
  boundary revision 0을 확인한 뒤 같은 EventBridge lock 안에서 durable binding commit과 clean을
  수행하는 creation-specific closure로만 확정한다. Recovery resume의 RPC write도 이 exact
  creation checkpoint로 보호한다.

Global latch와 exact dirty gate는 다음 active-only native 경계에 적용된다.

- runtime start/stop과 thread start/recover
- thread resume
- turn start/steer와 approval response
- 새 PTY open과 기존 PTY stdin
- rename
- archive/unarchive

Interrupt와 explicit terminal terminate/resize 같은 cleanup/control 경로는 stale 작업을
정리할 수 있어야 하므로 새 실행과 동일하게 막지 않는다. Stdin은 shell 실행 write이므로
cleanup 예외가 아니며 exact lifecycle admission을 반드시 통과한다.

### 4.2 authoritative graph와 leaf proof

Archive는 pinned Codex가 descendant까지 연쇄 archive할 수 있으므로 exact target이 leaf임을
native에서 먼저 증명한다.

- active와 archived `thread/list`를 cwd filter 없이 pinned 0.145의 모든 source kind로
  cursor 끝까지 읽는다.
- pagination은 membership별 64 page, 전체 4,096 ID로 제한한다. cursor cycle, duplicate ID,
  active/archived 중복, page cap 초과는 RPC 0회로 fail closed한다.
- `parentThreadId`와 `forkedFromId`를 strict ancestry로 정규화한다. conflict, cycle,
  missing sub-agent parent, unknown/new source shape를 거부한다.
- bound/unbound, 다른 scope, foreign descendant를 구분하지 않고 target 아래 descendant가
  하나라도 있으면 single-binding archive 권한으로 cascade하지 않는다.
- pinned 0.145에서 just-started empty appServer root가 membership list보다
  `thread/loaded/list`에 먼저 나타나는 예외만 보완한다. caller allowlist는 durable stable
  `active` binding ID로 제한하고, `thread/read(includeTurns:true)`가 exact appServer source,
  idle/notLoaded, zero turns, no ancestry일 때만 Active membership에 합친다.
- archived/unknown/transitional binding과 unbound loaded ID는 이 예외에 들어가지 않는다.
- Graph proof는 idle 확인과 terminal drain 뒤에 새로 수집한다. Proof 획득 뒤 target 또는
  foreign descendant의 topology가 바뀌면 guarded archive/unarchive admission이 NotSent로
  거부하고 RPC byte를 쓰지 않는다.

Active turn은 `turn/started` notification만 기다리지 않는다. `turn/start` 응답에서 즉시
native in-flight authority를 만들며, terminal response/close ordering과 bounded eviction 뒤에도
active proof가 부활하거나 사라지지 않게 token별 terminal proof를 보존한다. Pending 또는
reserved approval도 exact thread gate에 포함한다.

### 4.3 archive/unarchive command ordering

Public input은 두 command 모두 exact `{scope, threadId}`뿐이다. Frontend가 cwd, path,
worktree ID, lifecycle state, operation ID를 보내지 않는다.

Archive의 중요한 순서는 다음과 같다.

1. exact scope/binding lookup; wrong scope는 graph/RPC/terminal/store side effect 0회
2. global latch와 persisted execution root 확인
3. strict `thread/read(includeTurns:true)` idle proof와 pending/reserved approval 0 확인
4. exact terminal owner terminate/drain/reap ack
5. exact lifecycle checkpoint를 잡고 fresh exhaustive graph와 topology proof 수집
6. durable membership reconciliation, checkpoint clear, target Active leaf/root 검증
7. operation journal claim과 execution root 재검증
8. 같은 graph proof를 runtime/event/approval lock 아래 다시 확인하고 active/in-flight/uncertain
   turn과 approval 0을 확인한 즉시 `thread/archive` JSON-RPC byte를 1회 admit
9. response까지 관측된 topology가 exact expected notification만 포함하는지 확인
10. EventBridge lock 안에서 completion proof 재검증, stable Archived durable commit, native
    dirty clean을 한 closure로 수행

Unarchive도 fresh graph proof로 stable Archived leaf membership과 exact root를 증명하고 claim
뒤 동일 proof를 guarded admission까지 운반해 `thread/unarchive` byte를 한 번 쓴다. 응답 thread
ID, cwd, root를 persisted binding과 검증한 뒤 EventBridge-lock completion closure 안에서만
stable Active를 commit한다.

Resume, turn start/steer, approval response, rename, PTY open/stdin도 facade가 얻은 exact clean
checkpoint를 실제 RPC/PTY write admission까지 그대로 전달한다. Notification dispatcher의
approval insert/resolve와 turn completion clear도 같은 event barrier를 사용한다. 따라서
사전 검사 직후 out-of-band archive가 들어오는 check-to-write 틈이 없다.

Runtime process teardown도 fail closed로 보강했다. Stop 실패 시 handle을 버리지 않고
`Failed` phase에 보존하며, 같은 process의 verified teardown이 성공하기 전에는 generation을
올리거나 새 app-server를 spawn하지 않는다.

## 5. contract와 public wire

Exact Codex 0.145 archive contract는 다음과 같다.

```text
thread/archive   { threadId } -> {}
thread/unarchive { threadId } -> { thread }
thread/archived  { threadId }
thread/unarchived { threadId }
```

SchoolX public command는 다음 두 개를 추가했다.

```text
code_thread_archive   { input: { scope, threadId } }
code_thread_unarchive { input: { scope, threadId } }
```

결과는 exact binding, lifecycle, optional authoritative thread summary를 가진 strict
`CodeThreadLifecycleMutationResult`다. Adapter는 response의 scope와 Codex thread ID가
요청과 정확히 일치하는지 검증한다.

Frozen contract 기준선은 다음과 같다.

- SchoolX Code Tauri command 25개
- curated Codex method 12개
- notification 23개
- selected schema 62개
- selected schema aggregate SHA-256:
  `e3337beb535208fe8a47bc8906eafe2869b0bb444cdeb0dd9cc314a181f751f8`
- selected leaf aggregate SHA-256:
  `992f5c4b650ec1a42ed6a35ff3ebb0b1e5979cf4f3c3e64ca437c49d94950eb3`

## 6. frontend 동작

Lifecycle capability는 pure five-state matrix 한 곳에서 결정한다.

- `active`: composer, auto-resume, turn/steer/interrupt, approval response, new PTY,
  Changes, rename, archive
- `archived`: timeline/list metadata, Changes, rename, explicit unarchive
- `archiving | unarchiving | unknown`: list/timeline inspect와 Refresh/reconcile만

Archive mutation을 시작하면 exact row에 local pending gate를 즉시 적용하지만 persisted
`archiving`을 optimistic authority로 꾸미지 않는다. 성공과 오류 모두 exact list를 바로
refetch한다. Refetch가 실패하면 unreconciled gate를 유지해 오래된 Active UI로 돌아가지 않는다.

`thread/archived`와 `thread/unarchived` frontend notification도 state를 직접 뒤집지 않고
exact query invalidate/refetch 신호로만 사용한다.

Selected row를 archive해도 URL, `aria-current`, header, timeline, Changes와 binding row를
그대로 유지한다. 다른 active row를 자동 선택하지 않는다. Composer, retry, approval write,
terminal toggle은 사라지고 archived notice와 Rename/Unarchive만 남는다. Unarchive 성공 뒤
같은 URL에서 auto-resume은 허용하지만 PTY는 닫힌 상태를 유지해 사용자가 다시
`Cmd/Ctrl+J`를 눌러야 한다.

Terminal visibility는 boolean 하나가 아니라 exact owner key에 묶였다. Selection/lifecycle
변경이나 unarchive가 다른 row의 PTY를 자동 open하지 않는다.

## 7. 검증 기준선

현재 공유 트리에서 다음을 통과했다.

- `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check`
- `cargo check --manifest-path desktop/src-tauri/Cargo.toml --lib --locked`
- `cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib --locked -- -D warnings`
- Tauri 전체 lib: 2,279 passed, 17 ignored, 0 failed
- SchoolX `code_workspace::`: 177 passed, 3 ignored
- runtime: 30 passed, 1 manual ignored
- thread lifecycle graph: 6/6
- terminal: 15/15
- command lifecycle facade: 11/11
- approvals: 12/12
- protocol: 14/14
- lifecycle persistence focused tests와 binding namespace 통과
- desktop JS unit: 3,996/3,996
- focused Code contract/view unit: 26/26
- `pnpm --dir desktop typecheck`
- `pnpm --dir desktop build:e2e`
- SchoolX Code E2E: 18/18
- changed frontend files Biome, `pnpm --dir desktop check:px-text`
- `git diff --check`
- 신규 production `unsafe`, `unwrap()`, `expect()`, panic 경로 없음
- stale graph proof는 archive RPC 0회, durable completion 중 signal은 commit 직후 dirty 유지,
  실제 PTY open/archive 동시 경합은 binding lock에서 안전하게 직렬화되는 deterministic race
  회귀 통과
- 최종 독립 integration review에서 HIGH/MED correctness, race, authority 누락 없음

`pnpm --dir desktop check:file-sizes`는 현재 누적 dirty tree의 기존 ratchet/oversized
파일 15개 때문에 실패한다. `app_state.rs`, 기존 Phase 1 SchoolX 대형 모듈, 두 project-git
모듈, `lib.rs`, managed-agent 파일과 기존 project panel이 목록에 남는다. 이번에 분리한
`terminal.rs` 966줄, `thread_lifecycle.rs` 742줄, `code_thread_lifecycle.rs` 614줄,
`CodeWorkspaceScreen.tsx` 970줄과 새 frontend leaf는 목록에 없다. Ratchet이나 allowlist를
늘려 숨기지 않았다.

병렬 PTY 검증에서 readiness timeout 두 건이 한 번 관측됐지만 terminal 단독 15/15와
전체 `code_workspace::` 재실행은 모두 통과했다. Windows runner는 없으므로 기존 PTY
Windows residual은 아래처럼 남는다.

## 8. 현재 작업 트리와 잔여 위험

- staged 파일 0개이며 commit하지 않았다.
- 기존 사용자 수정과 untracked `brand/`, `supabase/`, compose 파일, SchoolX Phase 1/2
  파일을 모두 보존했다.
- binding root revalidation과 `portable-pty` cwd resolve/spawn 사이 pathname TOCTOU가 남는다.
- Windows는 `portable-pty 0.9`의 suspended spawn/atomic Job assignment hook이 없어 spawn과
  Job 배치 사이 gap이 남는다.
- pre-24H2 `ClosePseudoConsole` 영구 block 시 app shutdown을 막지 않는 대신 detached helper와
  ConPTY handle 하나가 남을 수 있다.
- Unix descendant가 새 `setsid()`로 원래 session을 탈출하면 PTY/app-server cleanup 범위 밖이다.
- Codex contract는 0.145 patch line에 고정돼 있다. 새 version/schema를 암묵 허용하지 않는다.
- Archive는 leaf-only다. Cascade archive, worktree remove, Git write handoff는 구현하지 않았다.

## 9. 다음 순서: fork 수직 슬라이스

Fork는 archive lifecycle authority 뒤의 별도 작업이다. Archive를 다시 설계하거나 fork와
operation을 합치지 않는다.

권장 구현 순서는 다음과 같다.

1. source binding이 stable Active이고 lifecycle authority가 clean하며 managed source worktree도
   clean인지 native에서 증명한다.
2. source의 current immutable HEAD에서 fresh destination managed worktree를 먼저 준비한다.
3. source와 destination이 같은 managed root를 공유하도록 uniqueness invariant를 완화하지 않는다.
4. preparation journal에 operation kind `start | fork`와 exact `sourceThreadId`를 추가한다.
5. frontend input은 source scope/thread와 bounded option만 받는다. cwd, base commit,
   destination worktree ID는 native가 도출한다.
6. `thread/fork` response의 new ID != source, exact destination cwd, exact `forkedFromId`,
   preparation source를 검증한 뒤에만 binding을 atomic commit한다.
7. definitely-not-sent만 exact preparation rollback한다. Response loss는 sticky uncertain
   preparation으로 남기고 reload recovery가 exact ancestry와 destination root를 확인한다.
8. 실패나 ambiguity에서 source 또는 destination worktree를 자동 삭제하지 않는다.
9. dirty patch 복사는 Git handoff 전 범위 밖으로 유지한다.

계속 범위 밖인 항목은 worktree remove/cleanup, model/reasoning selector,
stage/unstage/commit/push/PR, Talk/Nostr 공유, review/model API, app-server `command/exec`
terminal 통합과 새 Codex version 지원이다.

## 10. 새 세션에 전달할 시작 요청

다음 문장을 새 세션의 첫 요청으로 사용할 수 있다.

```text
SCHOOLX_CODE_DESIGN.md와 최신
SESSION_HANDOFF_20260816_CODE_PHASE2_ARCHIVE.md, 현재 작업 트리를 먼저 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Phase 1의 열 개 closure와 Phase 2 exact bound-thread PTY terminal, exact-bound 검색/이름
변경, persisted five-state archive/unarchive lifecycle authority는 완료됐으므로 다시
구현하지 마.

다음 독립 수직 슬라이스인 thread/fork를 진행해줘. Source는 lifecycle authority가 clean한
stable Active managed binding과 clean worktree만 허용하고, current immutable HEAD에서 fresh
destination managed worktree를 먼저 준비해 source root를 공유하지 않게 해줘. Preparation
journal에는 operation kind start|fork와 exact sourceThreadId를 저장하고, thread/fork 응답의
new ID != source, exact destination cwd, forkedFromId, source를 검증한 뒤에만 binding을 atomic
commit해줘. Definitely-not-sent만 exact rollback하고 response loss는 sticky uncertain recovery로
남겨 reload에서 ancestry/root를 검증해줘. Dirty patch 복사와 worktree 자동 삭제는 섞지 마.

기존 사용자 변경과 untracked 파일을 보존하고 stage나 commit은 하지 마.
```
