# SchoolX Code Phase 1F 인계

> 후속 최신 인계는
> [`SESSION_HANDOFF_20260816_CODE_PHASE2_ARCHIVE.md`](SESSION_HANDOFF_20260816_CODE_PHASE2_ARCHIVE.md)다.
> 새 세션은 Phase 2 인계를 우선한다.

작성일: 2026-08-14
최종 갱신: 2026-08-16
상태: Phase 1F, Phase 1F.1, 기존 후속 closure, C–F와 Changes
completeness/status closure 구현·검증 완료; 다음 순서는 Phase 2 첫 PTY 수직 슬라이스

이 문서는 최초 Phase 1F 완료 뒤 수행한 freshness, permission, native Changes,
UI lifecycle, E2E bridge, pinned Codex 0.145 actual-boundary, Git replacement-object 차단,
native diagnostic egress redaction, cross-platform app-server descendant cleanup, permission
display/authority 분리, authoritative runtime checkpoint, generation Changes/prompt 정합성
및 Changes completeness/status 후속 작업까지 반영한 현재 인계 기준선이다. 이전 대화
요약보다 현재 작업 트리와 이 문서를 우선한다.

## 1. 새 세션에서 먼저 읽을 문서

아래 순서로 문맥을 복구한다.

1. [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
2. 이 문서
3. Phase 1E UI 수직 슬라이스와 기존 race 방어의 상세 근거가 필요할 때만
   [`SESSION_HANDOFF_20260814_CODE_PHASE1E.md`](SESSION_HANDOFF_20260814_CODE_PHASE1E.md)
4. frontend state/native binding의 더 이전 근거가 필요할 때만 Phase 1D~1B 인계를 읽는다.

Phase 1E 인계의 “17개 Tauri command”와 “Phase 1F 다음 범위”는 당시의 역사적 상태다.
현재는 exact bound-thread Changes read가 추가되어 SchoolX Code public command가 18개다.

## 2. 이번 단계에서 완료한 것

### Normalized event와 recovery E2E

- normalized plan, command/output, file-change, approval, turn-complete 이벤트가 semantic
  timeline으로 투영되는 흐름을 실제 reducer와 typed adapter 경로로 검증했다.
- 숫자와 문자열 approval request ID를 구분하고, accept/decline이 generation, exact scope,
  thread, turn, item, sequence를 보존한 pending identity로만 전송되는 것을 고정했다.
- ring에서 원래 request event가 밀려난 truncated replay도 native authoritative checkpoint가
  active turn과 pending approval을 복원한다. response reservation 중인 approval은
  non-respondable로 전달된다.
- runtime crash 뒤 이전 generation의 pending approval을 제거하고, retry로 시작한 새
  generation을 sequence 0부터 full replay한 뒤 exact thread를 다시 resume하는 흐름을
  검증했다.
- 800×500, dark mode, reduced motion에서 keyboard-only로 Changes inspector를 열고 닫으며
  sidebar와 approval action에 접근할 수 있는 회귀를 추가했다.

E2E mock은 production과 같은 Tauri event channel을 사용한다. test-only seam은 연속 replay,
live event, crash/retry generation, Changes fixture를 제공하지만 UI가 raw app-server payload나
별도 approval state를 직접 읽게 만들지 않는다.

### 읽기 전용 Changes inspector

- typed `code_thread_changes({ scope, threadId })` command와 strict Zod/Rust DTO, contract
  fixture, exact query key를 추가했다.
- native는 caller가 넘긴 경로를 신뢰하지 않고 persisted binding에서 선택 thread의 execution
  root를 가져온다. immutable base와 repository identity를 재검증한 동일 snapshot에서만
  diff를 반환한다.
- 기존 project Git diff 표현과 `ProjectDiffFilesPanel`을 재사용해 changed file, status,
  additions/deletions, patch를 표시한다.
- inspector는 refresh와 close만 제공한다. stage, commit, push, PR 또는 파일 쓰기 action은
  추가하지 않았다.
- 넓은 화면에서는 오른쪽 pane, 좁은 화면에서는 semantic overlay로 표시하며 Escape,
  명시적 accessible label, dark mode와 reduced-motion을 지원한다.
- thread event가 바뀌면 현재 inspector query를 invalidate해 에이전트 변경을 다시 읽는다.

### Phase 1F.1 Changes freshness closure

- selected thread의 `turn/diff/updated`, `item/fileChange/patchUpdated`, `turn/completed`가
  열린 Changes inspector를 정확히 한 번씩 다시 읽게 하는 것을 command call-count로
  고정했다.
- 다른 thread event와 selected-thread non-change event는 refresh하지 않는다.
- inspector가 닫힌 동안에는 refresh하지 않고, 다시 열 때 정확히 한 번 읽는다.
- runtime generation이 바뀌고 sequence 0 full replay가 적용되면 새 generation identity로
  정확히 한 번 refresh한다. 이후 도착한 이전 generation event는 무시한다.

### Codex 0.145 permission approval closure

- frontend는 permission 응답에 raw authority나 review flag를 넣지 않고
  `{ type: "permissions", intent, scope }`만 보낸다.
- native는 pending request의 원본 authority를 보관하며 grant 시 그 전체 원본을 복원한다.
- permission decline은 native가 `permissions: {}`, `scope: "turn"`,
  `strictAutoReview: false`로 canonical 직렬화한다.
- empty permissions는 실행 승인으로 취급하지 않는다.
- non-empty turn grant는 native가 `strictAutoReview: true`, session grant는
  `strictAutoReview: false`로 canonical 직렬화한다.
- 숫자/문자열 request ID와 generation/scope/thread/turn/item identity 검증은 그대로
  유지한다.
- frozen Codex 0.145 wire fixture, native approval tests, frontend contract/E2E를 함께
  갱신했다.

### 실제 managed-worktree Changes native regression

- production prepare 뒤 claim/commit하고 persisted binding store를 reload하는 실제
  managed-worktree 경로를 사용한다.
- diff 기준은 persisted immutable base commit이며, managed checkout 변경만 포함하고
  source checkout decoy는 제외한다.
- read 전후 source/managed checkout status가 바뀌지 않는 것을 검증한다.
- wrong project, repository 또는 thread identity는 fail closed한다.

### Git replacement-object immutable-base closure

- SchoolX Code 전용 Git environment에 `GIT_NO_REPLACE_OBJECTS=1`을 강제해 persisted base
  OID의 tree 의미를 `refs/replace/*`가 바꾸지 못하게 한다.
- 실제 `git replace A B`가 보호되지 않은 `git diff A`를 false-clean으로 만드는 fixture에서
  `thread_changes_native`는 원래 A 대비 변경을 계속 보고한다.
- 이 회귀는 tempfile repository만 사용하고 read 전후 porcelain status가 동일함을 검증한다.
  일반 project Git API 동작은 변경하지 않았다.

### Runtime diagnostic egress redaction closure

- raw canonical executable과 raw version은 native 내부 spawn/0.145 compatibility authority로만
  유지하고, public probe/status clone의 executable, version, error는 redaction한다.
- initialize 성공 metadata와 실패 RPC error/stderr, runtime `lastError`를 Tauri egress 전에
  동일한 protocol redactor로 정리한다.
- `sk-` 형태와 임의 sensitive-env canary를 모두 검증하되 process-global environment는
  테스트에서 변경하지 않는다.
- public display redaction이 raw supported-version 판정을 바꾸지 않는 회귀를 고정했다.

### C — cross-platform app-server descendant cleanup

- Unix app-server는 별도 process group으로 시작한다. 종료 시 group 전체에 TERM을 보내고
  짧은 grace 뒤 남은 descendant를 KILL하며 leader를 반드시 reap한다.
- leader가 먼저 종료돼도 `waitid(..., WNOWAIT)`로 상태를 관찰한 뒤 reap하고, TERM/HUP를
  무시하는 descendant까지 group KILL로 정리한다.
- Windows app-server는 kill-on-close Job Object에 즉시 배치한다. Job 생성이나 child 배치가
  실패하면 runtime start를 fail closed하고 child를 남기지 않는다.
- Unix 회귀는 leader가 먼저 끝나고 resistant descendant가 남는 실제 process tree를 사용한다.
  Windows 동작은 현재 macOS 검증 환경에서 실행하지 않았으며 기존 shared Job Object 경계를
  재사용한다.

### D — permission display/authority 분리

- native pending store만 raw `permissions`를 보유한다. Tauri event에는 deterministic typed
  `permissionDisplay`와 일반 진단 필드만 포함하며 raw authority는 노출하지 않는다.
- frontend approval card는 network와 filesystem path/glob/special scope를 표시하지만 응답은
  grant/decline intent와 turn/session lifetime뿐이다.
- empty, malformed, redaction으로 정확성을 잃은 display는 grant 불가능하다. decline은 display가
  불완전해도 canonical empty response로 처리할 수 있다.
- reserved approval checkpoint는 표시할 수 있지만 `respondable: false`라 중복 응답할 수 없다.

### E — authoritative runtime checkpoint와 listener-first recovery

- native replay는 full replay, generation 변경 또는 ring truncation 때 exact generation과
  sequence watermark의 `activeTurns`와 `pendingApprovals` checkpoint를 제공한다.
- checkpoint approval event는 watermark에 합성되고 command boundary에서 현재 exact binding
  scope의 thread만 통과한다.
- frontend는 listener를 먼저 등록해 live event를 buffer하고 replay event를 적용한 다음
  checkpoint로 transient state를 authoritative overwrite하며 watermark 이후 buffered event만
  순서대로 적용한다.
- checkpoint가 evicted approval/active turn을 복구해도 잘린 transcript는 selected thread의
  exact `thread/read` resume 한 번으로 보완한다. refresh 중 approval/interrupt/composer를
  차단하고, generation/epoch/selection drift에는 completion을 거부한다.
- 선택 thread가 없으면 즉시 완료한다. resume 실패는 자동 loop를 만들지 않고 기존 Retry
  action으로만 다시 시도한다.

### F — generation Changes/prompt 정합성

- Changes query cache identity에 runtime generation을 포함하되 native input은 기존 exact
  `{ scope, threadId }` 그대로 유지한다.
- generation replay가 synchronized되기 전에는 Changes read를 시작하지 않는다. 첫 synchronized
  snapshot은 invalidation baseline이므로 새 generation에서 한 번만 읽고, 이후 같은 generation의
  diff/file-patch/turn-complete live event만 exact query를 invalidate한다.
- 선택/재오픈 시 inspector가 현재 snapshot을 한 번 읽고, 닫힌 동안 들어온 변경은 재오픈
  fetch에 반영한다. 이전 generation의 늦은 event는 무시한다.
- optimistic prompt는 authoritative thread snapshot의 persisted user row와 exact
  `[turnId, text]`별 개수로 reconciliation해 같은 문구의 중복 입력도 하나씩만 상쇄한다.

### Changes completeness/status closure

- native는 tracked와 untracked 전체 manifest를 먼저 정렬한 뒤 반환 목록만 250개로 제한한다.
  응답은 `totalFiles`, `filesTruncated`를 포함하고, 최상위 additions/deletions는 반환된 파일
  subset만 합산한다.
- 각 파일은 closed status
  (`added`, `modified`, `deleted`, `typeChanged`, `unmerged`, `untracked`)와 `binary`를 가진다.
  실제 `UU`가 base-relative name-status에서 `M`으로 보이는 Git 동작 때문에 unmerged path는
  별도 `diff-filter=U` read로 판정한다.
- tracked numstat/status/unmerged와 untracked path inventory를 patch 수집 전후에 비교한다.
  중간 변경이 감지되면 전체 Changes read를 한 번 다시 시도하고, 두 번째 drift는 fail closed한다.
- untracked 파일은 검증된 root-relative `openat`/`O_NOFOLLOW` descriptor를 그대로 Git stdin에
  연결해 patch-only wire를 읽는다. numstat와 patch를 한 명령에 합치면 `/dev/fd/0` shared
  offset 때문에 macOS Git에서 binary 판정이 깨지므로 strict patch header/binary marker를
  해석하고 별도 size metadata와 교차 검증한다.
- filename과 Git output은 strict UTF-8로 처리하고, 파일별 patch는 정확히 2,000줄과
  256 KiB에서 제한한다. binary는 additions/deletions 0/0으로 반환한다.
- frontend strict contract는 빈/중복 path, binary non-zero count, subset 합계와
  `filesTruncated` 모순을 거부한다. UI는 partial file list와 patch truncation을 구분해
  표시하고, synchronization 전에는 Refresh와 최초 read를 차단한다.
- 같은 generation의 authoritative refresh가 완료되면 exact Changes query만 한 번
  invalidate한다. initial replay와 truncated checkpoint recovery를 포함한 call-count를 E2E로
  고정했다.

### UI lifecycle와 exact E2E bridge closure

- New task의 exact prepare → start serialization과 start 결과 선택을 검증한다.
- 새 thread start 결과 때문에 불필요한 resume가 발생하지 않는다.
- provisional turn에서는 Stop이 disabled이고, normalized `turn/started` 뒤에만 exact
  thread/turn interrupt를 허용한다.
- managed thread 생성 뒤 앱 reload 시 exact thread를 resume하고 selection, timeline,
  composer를 복원한다.
- E2E bridge의 `code_thread_start`는 exact scope preparation을 한 번만 소비한다.
  `code_thread_resume`은 exact 기존 binding 또는 exact 새 binding만 반환하며 wrong
  project/repository/thread는 모두 fail closed한다.

### pinned Codex 0.145 actual app-server boundary

- 설치된 exact `codex-cli 0.145.0`을 SchoolX `CodeRuntime`이 실제로
  `app-server --listen stdio://`로 시작하는 ignored manual regression을 추가했다.
- 임시 `CODEX_HOME`, managed config 차단, update check 비활성화, dummy provider key와
  `127.0.0.1` Responses SSE mock만 사용한다. 외부 API와 사용자 인증은 사용하지 않는다.
- 실제 `item/permissions/requestApproval`의 generation/thread/turn/item identity를 받고,
  `session + strictAutoReview=false`를 응답한 뒤 `serverRequest/resolved` →
  `turn/completed` 순서(approval < resolved < completed)와 두 번째 local model request의
  permission tool output을 검증한다.
- 이 테스트는 로컬 exact binary에 의존하므로 기본 test run에서는 ignored다. fixture/mock을
  대체하지 않고 pinned historical boundary를 수동 감사하는 최소 회귀다.

## 3. 보존한 안전·상태 계약

- Tauri 호출은 typed adapter 하나를 통하며 input/output을 strict decode한다.
- community + project dtag + native repository identity의 exact scope를 유지한다.
- listener 등록 → live buffer → replay 적용 순서와 subscription epoch 검증을 유지한다.
- generation 전환은 sequence 0 full replay로만 복구하며 이전 generation의 늦은 결과를 버린다.
- runtime checkpoint는 transient active turn/approval만 보완하고 transcript authority를
  대신하지 않는다.
- timeline state는 pure reducer가 소유하며 mutable replay 객체를 공유하지 않는다.
- pending approval identity와 approval commit generation race 방어를 유지한다.
- thread 전환/resume 직렬화와 provisional turn race 방어를 유지한다.

Changes reader는 추가로 다음 경계를 지킨다.

- 허용된 read-only Git operation만 closed enum으로 실행한다.
- repository root, `.git` entry, common dir를 fd/inode로 고정하고 snapshot 전후 identity를
  fail-closed 재검증한다.
- literal pathspec을 사용하며 clean/smudge/process filter 설정을 실제 read command에서
  무효화한다.
- Git replacement objects를 모든 SchoolX direct/pinned operation에서 비활성화한다.
- untracked 파일은 root-relative `openat`과 `O_NOFOLLOW`로 열고 regular file만 Git stdin으로
  전달한다.
- command 수, 시간, stdout/stderr와 snapshot 합산 출력에 상한을 둔다. timeout이나 예산
  초과 시 descendant process group까지 종료한다.
- Unix가 아닌 플랫폼에서는 이 SchoolX Changes read를 fail closed한다.

현재 Changes 결과는 filesystem의 atomic snapshot이 아니다. initial/final
status/numstat/unmerged/path inventory와 각 patch의 count 일치 여부로 관찰 가능한 drift를
찾아 한 번 재시도하지만, 동일 count의 tracked content 변경이나 untracked 파일의 content-only
race는 서로 다른 시점의 patch를 섞을 수 있다. `git hash-object --no-filters -- <path>`는
symlink를 repository 밖까지 따라가고 mode/type/gitlink를 보존하지 않으므로 이 경계를 해결하는
수단으로 사용하지 않는다. 더 강한 보장이 필요하면 helper 내부의 tracked full-diff streaming
digest와 untracked regular-file `openat`/`O_NOFOLLOW` streaming hash, 또는 immutable filesystem
snapshot을 별도 hardening 범위로 설계한다.

External Git은 named `.git`과 config를 내부에서 다시 연다. 따라서 동일 UID의 악성
프로세스가 수행하는 정밀한 in-place mutation 또는 검사 사이 swap-and-restore ABA를 완전히
제거하려면 in-process Git이나 immutable repository snapshot이 필요하다. 현재 “기존 project
Git read API 재사용” 범위에서는 fd/inode 전후 검증으로 fail closed하는 것을 threat boundary로
둔다.

## 4. 주요 변경 위치

```text
desktop/src-tauri/src/commands/code_workspace.rs
desktop/src-tauri/src/commands/project_git_exec.rs
desktop/src-tauri/src/commands/project_git_diff.rs
desktop/src-tauri/src/managed_agents/process_lifecycle.rs
desktop/src-tauri/src/code_workspace/{approvals.rs,discovery.rs,protocol.rs,runtime.rs,worktrees.rs,contract_tests.rs}
desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-wire.json
desktop/src/features/code/api/{codeWorkspace.ts,schemas.ts,types.ts}
desktop/src/features/code/lib/{codeTimeline.ts,codeTimeline.test.mjs}
desktop/src/features/code/state/{codeSessionQueries.ts,codeSessionReducer.ts,codeSessionStore.ts}
desktop/src/features/code/ui/{CodeWorkspaceScreen.tsx,CodeChangesPanel.tsx,CodeApprovalCard.tsx}
desktop/src/features/projects/ui/ProjectPullRequestFilesChangedPanel.tsx
desktop/src/shared/api/projectGitTypes.ts
desktop/src/testing/e2eBridge.ts
desktop/tests/{helpers/bridge.ts,e2e/schoolx-code.spec.ts}
```

## 5. E2E 시나리오

`desktop/tests/e2e/schoolx-code.spec.ts`는 다음 13개 시나리오를 검증한다.

1. exact scope/bound thread Changes와 Git write action 부재
2. 3/5 partial file list의 subset totals, status/binary와 list/patch truncation 표시
3. initial replay synchronization 전 Refresh/read 차단과 완료 뒤 정확히 한 번 read
4. 같은 generation의 malformed live → full retry → truncated authoritative checkpoint 뒤 stale
   Changes를 정확히 한 번 invalidate하고 fresh 결과 표시
5. managed-worktree New task의 exact prepare → start와 start-result no-resume
6. E2E bridge의 exact existing/new binding 및 wrong scope/thread fail-closed
7. provisional turn Stop 방어와 normalized active turn exact interrupt
8. managed thread 생성 뒤 앱 reload, exact resume와 selection/composer 복원
9. Changes event 종류, open/closed inspector, synchronized generation baseline과 이후 live
   event의 exact call-count freshness
10. normalized plan/command/output/file/approval/turn-complete와 opaque permission intent
11. ring에서 밀려난 approval을 authoritative checkpoint로 자동 복원
12. runtime crash, 새 generation retry, sequence 0 full replay와 thread re-resume
13. 800×500 dark/reduced-motion keyboard 접근성과 narrow inspector overlay

UI E2E는 반드시 아래처럼 E2E bridge가 포함된 artifact를 만든 뒤 실행한다.

```bash
. ./bin/activate-hermit
cd desktop
pnpm build:e2e
pnpm exec playwright test tests/e2e/schoolx-code.spec.ts --project=smoke
```

`pnpm run build`는 사용하지 않는다. 로컬 Python preview의 cold ESM 병렬 요청 reset을
피하기 위해 spec은 localhost asset module fetch만 직렬화한다. 반복 관찰된 `EPIPE`에는
50/100ms의 한정된 재시도만 적용하고, 그 밖의 transport 오류와 product command/page
failure는 재시도로 감추지 않는다.

## 6. 검증 결과

모든 Git/검증 명령 전에 `. ./bin/activate-hermit`를 실행했다.

```text
[x] targeted Biome check
[x] pnpm typecheck
[x] Code frontend focused tests: 50 passed, 0 failed, 0 skipped
[x] pnpm check:px-text
[x] cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
[x] cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
[x] native approval tests: 8 passed
[x] native contract tests: 7 passed, 1 manual ignored
[x] CodeRuntime regressions: 9 passed, pinned app-server manual 1 ignored
[x] exact pinned Codex 0.145 session permission round-trip manual run: 1 passed
[x] Changes pure parser/contract regressions: 7 passed
[x] actual-repository Changes status/binary/limit/drift regressions: 9 passed
[x] project_git_exec regressions: 10 passed
[x] project_git_diff regressions: 1 passed
[x] pnpm build:e2e
[x] focused SchoolX Code E2E: 13 passed
[x] re-entry EPIPE harness regression repeat-each=20: 20 passed
[x] focused SchoolX Code E2E repeat-each=3: 39 passed
[x] production 경로에 새 unsafe/unwrap()/expect() 없음
[x] post-Changes-closure SchoolX code-workspace suite: 121 passed, 0 failed, 3 ignored
[x] git diff --check
```

Pinned actual-boundary 수동 감사 명령은 다음과 같다.

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  code_workspace::runtime::tests::pinned_codex_0_145_session_permission_round_trip_is_manual_only \
  -- --ignored --exact --nocapture
```

전체 `pnpm check`의 Phase 1 native file-size known failure는 Phase 1E 인계에 기록된 기존
상태이며, 이번 단계에서 무관한 대형 파일 분할이나 ratchet 변경으로 범위를 넓히지 않았다.

## 7. 계속 범위 밖인 항목

- terminal drawer와 PTY command
- worktree remove/cleanup
- SchoolX Talk/Nostr 공유
- stage/commit/push/PR Git handoff
- review/model API
- 새 Codex version 지원

## 8. 작업 트리 주의

SchoolX Code와 무관한 기존 사용자 변경은 그대로 보존했다. 특히 아래 파일/디렉터리를
reset, 재포맷, 스테이징하거나 SchoolX Code 변경에 섞지 않는다.

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

이 Phase에서는 commit, stage, push 또는 PR 생성을 하지 않았다.

`desktop/src/features/code/ui/CodeChangesPanel.tsx`와 이 인계 문서는 untracked일 수 있어
일반 `git diff`만으로는 보이지 않는다. 반드시 `git status --short`도 확인한다.

## 9. 다음 세션 시작점

기존에 확정했던 다섯 후속 작업, C–F 네 후보와 Changes completeness/status closure까지
모두 완료됐다.

1. Changes freshness event/generation direct call-count E2E
2. E2E bridge start/resume exact-binding fail-closed
3. pinned Codex 0.145 permission response actual app-server boundary
4. Git replacement-object immutable-base 차단
5. version/init/runtime diagnostic native egress redaction
6. C: cross-platform app-server descendant cleanup
7. D: permission display/authority 분리
8. E: authoritative runtime checkpoint와 listener-first recovery
9. F: generation Changes/prompt state reconciliation
10. Changes completeness/status, bounded manifest와 drift retry

따라서 다음 세션은 이 열 작업을 다시 구현하지 않는다. 사용자가 작업 순서에 따른 구현을
승인했으므로 다음 순서는 Phase 2의 첫 수직 슬라이스인 **exact bound-thread PTY terminal
drawer/lifecycle**다. 첫 구현 전 기존 `commands/project_terminal.rs`, 저장소의 PTY dependency,
Codex app-server command execution과 별도 OS shell PTY의 소유권 경계를 먼저 확인한다. 그 뒤
native session ownership → typed resize/stdin/terminate command → `⌘J` drawer 순서의 최소
수직 슬라이스로 진행한다.

worktree cleanup, thread 검색/이름 변경/archive/fork, model/reasoning selector는 같은 Phase 2라도
첫 PTY 슬라이스와 섞지 않는다. 위에 기록한 content-only race의 더 강한 fingerprint도 별도
hardening 후보이며, 현재 Changes 결과를 atomic snapshot이라고 표현하거나 unsafe한
`git hash-object --no-filters -- <path>`로 보완하지 않는다.

첫 명령은 반드시 다음과 같다.

```bash
. ./bin/activate-hermit && git status --short
```

모든 Git/검증 명령은 Hermit 활성화 뒤 실행한다. UI E2E가 필요한 변경이라면 반드시
`pnpm build:e2e`로 새 artifact를 만든 뒤 실행한다. stage, commit, reset 또는 기존 사용자
변경 재포맷은 하지 않는다.

Pinned actual-boundary 회귀는 의도적으로 positive `session + strictAutoReview=false` 한 건만
실제 app-server에서 검증한다. permission decline과 turn grant 의미론은 frozen fixture,
native/frontend contract와 E2E에 남아 있다. 이를 actual-boundary variant 확장의 자동 근거로
보지 않는다.
