# SchoolX Code Phase 1B 인계

작성일: 2026-08-14
상태: Phase 1B native worktree/thread binding persistence 완료, Phase 1C 시작 전 세션 경계

## 1. 먼저 읽을 문서

새 세션은 아래 순서로 문맥을 복구한다.

1. [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
2. 이 문서
3. 역사적 경계가 필요할 때만
   [`SESSION_HANDOFF_20260813_CODE_PHASE1A.md`](SESSION_HANDOFF_20260813_CODE_PHASE1A.md)와
   [`SESSION_HANDOFF_20260813_CODE_PHASE0.md`](SESSION_HANDOFF_20260813_CODE_PHASE0.md)

Phase 1A 인계의 “caller가 `workspaceRoot`를 보내는 과도기 계약”과 “Phase 1B가
다음 범위”라는 설명은 당시의 역사적 상태다. 현재 command는 저장된 binding의
execution root를 권위 있게 사용한다.

## 2. 이번 단계에서 완료한 것

- Tauri app data의 `code/thread-bindings.json`에 schema version 1인 binding index와
  미완료 preparation journal을 저장한다.
- binding은 설계에서 고정한 8개 필드만 가진다.
  - `community_id`
  - `project_dtag`
  - `repository_identity`
  - `codex_thread_id`
  - `execution_mode`
  - canonical `execution_root`
  - resolved commit인 `base_ref`
  - managed worktree에만 있는 `worktree_id`
- community + project dtag + repository identity scope를 정확히 격리한다.
- Codex thread ID는 전체 index에서 유일하며, managed worktree ID와 execution root도
  binding/preparation 전체에서 중복 예약할 수 없다. Local checkout은 여러 thread가
  공유할 수 있다.
- index는 4 MiB, binding/preparation 각각 4,096개 상한, strict JSON/schema
  validation, deterministic ordering, sibling temporary file 기반 atomic commit을
  사용한다. Unix에서는 `code` directory를 0700, index를 0600으로 제한한다.
- stale local checkout이나 외부에서 제거된 managed worktree는 index load 자체를
  막지 않는다. 실제 command가 해당 binding을 사용할 때만 filesystem/Git 상태를
  다시 검증하며 `code_threads_list`는 항목별 `unavailable`로 내린다.
- Git common-dir의 canonical path로 domain-separated SHA-256 repository identity를
  만든다.
- 활성 SchoolX nest의
  `WORKTREES/<repository-identity>/<worktree-uuid>` 아래에만 detached managed
  worktree를 만든다. Local mode는 선택한 checkout을 바꾸지 않는다.
- prepare/status 응답은 `headCommit`, `branch`와 `dirty`를 반환한다. Detached
  managed worktree의 `branch`는 `null`이다.
- destructive worktree remove, branch switch, reset, clean은 구현하지 않았다.
- Codex CLI는 엄격한 `codex-cli 0.145.<numeric patch>` 출력만 지원한다. 지원하지
  않는 버전은 app-server process를 만들기 전에 `Failed`로 차단하면서 probe
  executable/version은 진단 상태에 보존한다.
- app-server start/stop, binding claim/start/commit/recovery와 앱 종료가 같은
  application-level mutex를 사용해 runtime 전환과 durable transition이 겹치지 않는다.
- protocol error와 event payload의 secret-shaped 값은 frontend/diagnostic 경계 전에
  redaction한다.
- frontend UI, route, TypeScript adapter는 아직 만들지 않았다.

## 3. Durable start와 recovery 계약

Worktree/local checkout 준비는 thread를 만들기 전에 native-issued UUID와 함께
`Prepared`로 journal된다.

```text
code_worktree_prepare
  → execution root 준비 및 재검증
  → preparation(Prepared) atomic save

code_thread_start
  → scope/preparation/root/runtime/params 검증
  → exact-root recovery baseline 수집
  → preparation(Starting) atomic claim
  → Codex thread/start
  → thread id + cwd + threadSource 검증
  → preparation을 binding으로 atomic 교체
```

`threadSource`는 preparation별 `schoolx-code/<preparation-uuid>`다. 성공 응답과
recovery 결과 모두 이 값을 정확히 유지해야 한다.

`Starting` 이후의 오류는 다음 규칙을 지킨다.

- request bytes가 전혀 쓰이지 않았음이 transport에서 증명된 경우에만 exact claimed
  snapshot compare-and-set으로 `Prepared`로 되돌린다.
- partial/full write, timeout, EOF, server error, malformed success, 응답 후 identity/root
  검증 실패는 thread 생성 여부가 불확실하므로 `Starting`을 유지한다.
- 앱이 durable claim과 첫 write 사이에 종료된 경우도 미전송을 증명할 수 없으므로
  재시작 후 `Starting`을 유지한다.
- binding commit 실패 때 생성된 thread나 worktree를 삭제하지 않는다.
- stale/forged snapshot rollback은 거부한다.

`code_thread_binding_recover`는 `Starting` preparation에 대해서만 다음을 수행한다.

1. 저장된 execution root를 다시 검증한다.
2. Codex의 exact-root persisted thread와 loaded thread를 bounded pagination으로 읽는다.
3. claim 전에 존재한 baseline과 이미 bound된 thread를 제외한다.
4. exact preparation `threadSource`를 가진 후보가 정확히 하나인지 확인한다.
5. 후보를 다시 read하고 source/id/root를 검증한다.
6. resume 결과도 같은 source/id/root인지 다시 검증한다.
7. 마지막으로 preparation을 binding으로 atomic 교체한다.

Codex 0.145가 새 empty thread를 app-server 재시작 뒤 보존하지 않으면 후보를 찾지
못할 수 있다. 이 경우 자동으로 새 thread를 만들거나 preparation을 해제하지 않고
`Starting`을 그대로 보존한다.

## 4. 현재 native command 계약

```text
code_runtime_probe
code_runtime_start
code_runtime_stop
code_runtime_status
code_runtime_events

code_worktree_prepare
code_worktree_status

code_thread_preparations_list
code_threads_list
code_thread_start
code_thread_binding_recover
code_thread_resume

code_turn_start
code_turn_steer
code_turn_interrupt
code_approval_respond
```

- start는 caller path가 아니라 native preparation ID를 받는다.
- list는 complete binding scope를 받고, resume/turn/approval은 scope와 thread ID로
  저장된 root를 조회한다.
- turn start의 `cwd`와 `sandboxPolicy.writableRoots`는 재검증한 root 하나로 고정된다.
- 현재 Codex 0.145 계약에 없는 top-level `runtimeWorkspaceRoots`는 보내지 않는다.
- live event와 replay backlog는 durable binding이 있는 thread만 exact scope를 붙여
  frontend 경계로 내보낸다. Unbound thread event는 버린다.
- event envelope는 `scope`, `runtimeGeneration`, `sequence`, optional
  thread/turn/item ID, `kind`, redacted `payload`를 가진다.
- approval은 runtime generation + request ID + thread ID + turn ID와 binding scope에
  묶인다. 승인 응답은 요청된 permission 범위를 넓힐 수 없다.

## 5. Worktree와 Git 안전 경계

- 모든 persisted/use-time execution root는 absolute, canonical, real directory여야 한다.
- managed root는 활성 nest의 exact `WORKTREES/repository/uuid` chain과 Git common-dir
  identity를 다시 확인한다.
- Unix의 mutating Git은 전체 디렉터리 chain을 열린 handle로 유지하고 target clone을
  private helper의 stdin으로 넘긴다. Helper는 target device/inode를 검사하고
  `fchdir`한 뒤 pathname cwd 없이 Git으로 `exec`한다.
- helper request는 schema v1, unknown-field 거부, 64 KiB cap, commit/path/filter
  key count와 길이 제한을 가진다.
- Git environment를 clear한 뒤 prompt, credentials, hooks, fsmonitor, external protocol,
  repository filter command를 차단한다. Worktree-local/conditional filter key도 checkout
  전에 열거해 비활성화한다.
- process group, 60초 timeout, stdout/stderr cap을 유지한다.
- Git 성공 후 named path와 열린 device/inode chain을 다시 검사한다. 경로가 바뀌면
  descriptor나 binding persistence로 진행하지 않는다.

Portable POSIX API는 동일 UID 프로세스가 실행 중 pinned inode나 조상을 rename하는
행위를 완전히 막지 못한다. 이 경우 symlink decoy로 실행 경로가 redirect되지는 않고
사후 검증은 실패하지만, 이미 pinned inode에 생긴 Git side effect는 자동으로
되돌리지 않는다. Canonical Git executable/common-dir도 동일 UID 신뢰 경계 안의 named
path다. 현재 desktop은 별도 권한을 가진 helper가 아니므로 private helper marker 자체는
권한 상승 경계로 취급하지 않는다.

Descriptor-bound helper는 Unix에만 적용된다. Windows/non-Unix 경로도 canonical path와
containment를 검사하지만 Unix의 directory-FD race hardening을 제공하지 않는다.

## 6. 주요 파일

```text
desktop/src-tauri/src/code_workspace/
├── mod.rs
├── approvals.rs
├── bindings.rs
├── discovery.rs
├── jsonrpc.rs
├── paths.rs
├── protocol.rs
├── runtime.rs
└── worktrees.rs

desktop/src-tauri/src/commands/code_workspace.rs
desktop/src-tauri/src/app_state.rs
desktop/src-tauri/src/commands/mod.rs
desktop/src-tauri/src/lib.rs
desktop/src-tauri/src/main.rs
desktop/src-tauri/src/shutdown.rs
desktop/src-tauri/Cargo.toml
desktop/src-tauri/Cargo.lock
```

`rustix`의 `fs`/`process` feature를 direct Unix dependency로 추가했다. 안전한
directory-FD `fchdir`에 사용한다.

## 7. 검증 결과

2026-08-14 현재 작업 트리에서 다음을 통과했다.

```text
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib
→ 2166 passed, 0 failed, 15 ignored

cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  code_workspace::bindings::tests::
→ 29 passed, 0 failed

cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  code_workspace::worktrees::tests::
→ 14 passed, 0 failed, 1 ignored helper subprocess entry

cargo clippy --manifest-path desktop/src-tauri/Cargo.toml \
  --all-targets -- -D warnings
→ passed

cargo build --manifest-path desktop/src-tauri/Cargo.toml --bin buzz-desktop
→ passed

cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
git diff --check
→ passed
```

검증에는 store reload/scope isolation/duplicate rejection, corrupt/newer schema,
symlink와 stale root, exact rollback, ambiguous recovery, thread source 재검증, Codex
version gate, detached worktree, filter/hook hardening, pre/post-validation pathname 교체와
descriptor-bound helper 실행이 포함된다.

## 8. 유지해야 할 불변식

- `buzz-acp` 관리형 Talk 봇과 SchoolX Code runtime을 합치지 않는다.
- frontend가 raw app-server JSON-RPC, child stdin, 임의 execution root, Git argv를
  직접 소유하지 않는다.
- caller가 보낸 path로 persisted thread의 root를 대체하지 않는다.
- binding의 8개 필드와 schema v1을 임의로 확장하지 않는다. 변경이 필요하면 먼저
  명시적 schema migration과 backward/forward failure tests를 설계한다.
- scope 없는 thread event를 frontend로 내보내지 않는다.
- uncertain start를 자동 재시도하거나 `Starting` preparation을 수동 추측으로
  `Prepared`로 바꾸지 않는다.
- 생성/복구 실패 시 기존 checkout, thread, worktree를 remove/reset/clean하지 않는다.
- Code transcript를 사용자 action 없이 relay/Nostr에 게시하지 않는다.
- production path에 새 `unsafe`, `unwrap()`, `expect()`를 추가하지 않는다.
- 실제 Codex wire 계약을 바꿀 때는 0.145 compatibility gate와 fixtures를 함께
  갱신한다.

## 9. 아직 구현하지 않은 것

- React/TypeScript invoke adapter, event subscriber, reducer/query state
- SchoolX Code route와 화면
- 작업 사이드바, timeline/composer, approval card, Changes/Files inspector
- PTY terminal drawer
- worktree rename/archive/remove와 safe cleanup UI
- branch/stage/commit/push/PR handoff UI
- Talk 공유
- Codex 설치/업데이트 UX

Phase 1B에서 만들어진 orphan worktree나 persistent empty thread가 자동 제거되지 않는
것은 의도된 안전 동작이다. Cleanup은 Phase 2의 별도 사용자 action과 검증을 거쳐야
한다.

추가로 현재 경계를 정확히 이해해야 한다.

- app-server crash를 자동 재시작하거나 thread를 자동 resume하지 않는다. Health check는
  runtime을 `Failed`로 바꾸며 명시적 start/resume이 필요하다.
- normalized event backlog는 process memory에 최대 512개만 유지한다. Durable transcript가
  아니며 runtime restart/generation 교체나 overflow 시 replay gap/truncation이 발생한다.
- `threadSource`의 UUID에는 community/path가 없지만 Codex metadata이므로 Codex 쪽
  telemetry에 나타날 수 있다.
- checked-in generated Codex schema snapshot/version/hash artifact는 아직 없다. 현재
  호환성 보장은 좁은 DTO 테스트, fake shell app-server fixture와 엄격한 0.145 numeric
  patch version gate에 기반한다. 모든 0.145 patch의 upstream wire 호환성을 증명했다고
  간주하지 않는다.

## 10. 권장 다음 범위: Phase 1C native contract/fixture freeze

다음 세션은 UI에 들어가기 전에 native/public wire 계약과 재시작 복구 시나리오를 먼저
고정하는 것을 권장한다.

1. Tauri command 이름, camelCase input/output DTO와 event envelope를 curated JSON fixture로
   고정한다.
2. 실제 사용하는 Codex 0.145 method의 generated/curated schema snapshot과 source version,
   hash를 checked-in artifact로 남긴다.
3. snapshot과 좁은 native DTO/fixture가 어긋나면 실패하는 compatibility test를 추가한다.
4. fake app-server와 임시 app-data/Git repository를 사용해 prepare → start → bind →
   runtime restart → list/resume → turn의 native 시나리오를 한 테스트로 고정한다.
5. 별도 시나리오에서 uncertain start → store reload → exact recovery → binding commit을
   검증한다.
6. public capability는 늘리지 않는다. 테스트 구성을 위해 꼭 필요할 때만 read-only
   binding/preparation snapshot helper를 검토한다.
7. React/TypeScript adapter/reducer, route, 화면/styling, terminal, worktree remove,
   Talk/Nostr 공유, Git handoff, 새 Codex version 지원은 넣지 않는다.

완료 기준은 public Tauri/Codex 계약 변경이 fixture test에서 드러나고, process/store
재시작을 포함한 binding과 recovery 흐름이 실제 UI 없이 deterministic하게 통과하는
것이다. 그 다음 Phase 1D에서 frontend invoke/listen adapter와 pure reducer/query state를
구현한다.

## 11. 작업 트리 주의

SchoolX Code와 무관한 기존 사용자 변경이 같은 working tree에 있다. 다음 항목을
되돌리거나 정리하거나 일괄 stage하지 않는다.

- `.dockerignore`, `.gitignore`
- `crates/buzz-core/src/relay.rs`
- `deploy/compose/README.md`, `deploy/compose/Dockerfile.local`
- `desktop/src-tauri/src/managed_agents/restore.rs`
- `desktop/src-tauri/src/managed_agents/runtime.rs`
- `desktop/src-tauri/src/managed_agents/runtime/tests.rs`
- `desktop/src-tauri/src/managed_agents/runtime_commands.rs`
- `brand/`, `supabase/`

Git이나 hook을 실행하기 전에 `. ./bin/activate-hermit`를 사용한다. Desktop Tauri는
root workspace에서 제외되므로 native test는 항상
`--manifest-path desktop/src-tauri/Cargo.toml`을 명시한다.

## 12. 새 세션 시작 문구

```text
docs/schoolx-2/SCHOOLX_CODE_DESIGN.md와
docs/schoolx-2/SESSION_HANDOFF_20260814_CODE_PHASE1B.md, 현재 작업 트리를 확인하고
SchoolX Code Phase 1C native contract/fixture freeze를 구현해줘.

Phase 1B native binding schema와 recovery/Git 안전 계약을 유지하고 기존 사용자
변경을 보존해줘. 먼저 현재 Tauri DTO와 Codex 0.145 wire fixture 범위를 조사해 계약을
고정한 뒤, fake app-server를 사용한 process/store restart 및 uncertain-start recovery
시나리오를 검증해줘. 이번 단계에서는 React/TypeScript adapter/reducer, 화면/route/
styling, terminal, worktree remove, Talk/Nostr, Git handoff, 새 Codex version 지원까지
넓히지 말아줘.
```
