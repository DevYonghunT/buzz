# SchoolX Code Phase 3 Git write 구현 완료 인계

기준일: **2026-08-21**

문서 상태: **0.4절의 app-process launch 신뢰 경계를 전제로 한 Phase 3 P0 구현 완료 handoff**. 원본 착수
문서가 요구한 strict durable transaction, crash-boundary exact-once recovery, owned-lock/startup 보안 경계와
frontend recovery UX까지 구현하고 전체 회귀를 통과했다. 아래 1절, 3~5절과 7절은 구현 착수 당시의
요구와 gap을 추적하기 위해 보존한 스냅샷이다. 2절의 shared-worktree 지침, 6절의 public 계약, 8절의
재검증 명령과 9절의 다음 세션 prompt는 현재도 유효하며, 상태 판정은 이 0절이 우선한다.

## 0. 2026-08-21 후속 구현 결과

### 0.1 완료한 Native 경계

- Git journal은 `Prepared -> [ObjectWritten] -> ArtifactsReady -> LocksReady ->
  IndexPublished | HeadPublished -> CompletedAwaitingAck -> Acknowledged`의 strict durable phase와
  `Uncertain` fail-closed 상태를 기록한다. `ObjectWritten`은 object를 만드는 stage/commit에만 존재하며
  unstage와 deletion stage는 건너뛴다.
- Journal evidence는 exact root/admin/common-dir/object database, Git executable, index/HEAD preimage와
  expected result, frozen candidate artifact digest와 blob/tree/commit OID, identity/timestamp/canonical message 및
  owned artifact의 path/device/inode/owner/mode/link-count/digest를 결박한다.
- Stage/unstage와 commit은 app-private artifact를 먼저 durable하게 완성하고 proven-owned hard link로만
  `index.lock`/`HEAD.lock`을 획득한다. Publish 직전 projection/CAS/lock identity를 다시 검증하며 foreign 또는
  교체된 lock/artifact는 삭제하지 않는다.
- Publish 뒤 receipt를 먼저 durable하게 저장하고 restartable exact cleanup을 완료한 뒤 응답한다. Mutation
  crash/응답 손실은 live Git+journal evidence로 동일 receipt에 수렴하고, acknowledge 응답 손실은 durable
  `Acknowledged` tombstone으로 수렴한다. Duplicate commit이나 partial live-index publish를 만들지 않는다.
- Startup은 safe-remove journal과 Git journal을 모두 strict load/cross-preflight한 뒤 same-binding 충돌을
  zero-mutation으로 막고, disjoint binding에 대해 safe-remove recovery, Git recovery, runtime/lifecycle
  reconciliation 순서를 지킨다.
- Linked-worktree authority는 root/admin/common-dir의 exact reciprocal backlink와 filesystem identity를 검증한다.
  외부 fake admin redirect와 journal parent 이탈은 mutation 전에 거부한다.
- 모든 Git pathspec은 literal로 고정했다. `:(glob)*` 같은 파일명도 단일 선택 파일로 처리되어 sibling을
  집계하지 않는다.
- Typed Git launcher는 macOS에서 system candidate `/usr/bin/git`만, Linux에서 `/usr/bin/git` 우선 후 동일
  검증을 통과한 PATH fallback만 허용한다. Canonical executable과 모든 ancestor가 uid 0,
  group/other non-writable이고 Desktop effective user에게 `W_OK`가 없으며 executable에 set-id가 없음을
  확인한다. Helper의 pathname exec 직전에 pinned device/inode/owner/mode/link-count/size/digest와 trust chain을
  다시 검증하며 root로 실행한 Desktop의 Git write는 지원하지 않는다. macOS의 `/usr/bin/git` shim 이후
  root-controlled xcode-select/OS tool resolution은 명시적 TCB이고 cleared environment가 `DEVELOPER_DIR`
  override를 제거한다.
- 실제 native command boundary에서 runtime turn/steer, approval, PTY, fork/archive/remove와 Git write가
  상호 배타적임을 검증했고, 거부 경로는 journal/Git zero mutation을 보장한다.

여섯 public Tauri command 이름, top-level `{input}` envelope와 caller authority surface는 변경하지 않았다.
Local/archive write, stage-all/hunk, branch/push/PR/Talk 공유와 hook/signing도 열지 않았다.

### 0.2 완료한 Frontend 경계

- Git attempt 상태를 scope+thread QueryClient cache로 옮겨 inspector remount와 runtime generation 변경 뒤에도
  유지한다. Community remount에서는 기존 community-keyed QueryClient 경계가 상태를 폐기하므로 별도
  module-level singleton/reset은 추가하지 않았다.
- Mutation은 자동 재시도하지 않는다. 응답 손실은 outcome-unknown 상태인 `unknown`으로 전환하고 exact
  reconcile을 수행한다.
- Receipt, acknowledge, cleared-blocker 각 응답은 exact revision/generation/receipt를 확인한 fresh authoritative
  status만 수용한다. Pending/recovering은 bounded 4회 poll 뒤 명시적 Retry를 제공한다.
- Git blocker는 Changes inspector를 닫아도 workspace controller에 남아 composer start/steer를 비활성화하고
  이유를 표시한다. Commit outcome이 불명확할 때 dialog는 닫거나 메시지를 편집할 수 있지만 자동 resubmit하지
  않는다.
- 기존 UI component와 stock rem text token을 사용하고 accessible inline status/Retry를 유지했다.
- Fresh-build Playwright가 commit response-loss, reconcile, acknowledge와 blocker clear까지 검증한다.

### 0.3 최종 검증

- `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check`
- `cargo check --manifest-path desktop/src-tauri/Cargo.toml --lib`
- `cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib --tests -- -D warnings`
- Git write targeted tests: **66 passed, 2 ignored, 0 failed**
- Native admission/gate tests: **7 passed, 0 failed**
- Native contract tests: **8 passed, 1 ignored, 0 failed**
- Tauri 전체 lib tests: **2429 passed, 21 ignored, 0 failed**
- Frontend 전체 tests: **4037 passed, 0 failed**
- `pnpm --dir desktop typecheck`
- `pnpm --dir desktop check:px-text`
- Targeted Biome check
- Fresh `pnpm --dir desktop build:e2e`
- `schoolx-code.spec.ts --project=smoke`: **26 passed, 0 failed**
- `git diff --check`

Ignored Tauri tests는 기존 환경/수동/feature-flag sentinel과 test harness가 subprocess에서 직접 실행하는
private Git/safe-remove/worktree/Codex helper entrypoint다. 이 작업에서는 shared checkout에 `git add`,
`git commit`, `git reset`, `git checkout` 또는 `git clean`을 실행하지 않았다.

### 0.4 명시적 신뢰 경계와 후속 hardening

> 이 절의 미해결 self-reexec 상태는 이후
> [`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md)의
> 선택 B 구현으로 superseded됐다. 아래 내용은 당시 launch 경계를 보존한 역사적 기록이다.

Git launcher namespace/bytes를 바꿀 수 있는 root, root 권한의 OS/package update와 macOS xcode-select tool
resolution은 TCB다. User-owned package manager/custom Git은 허용하지 않는다. 별도로 parent는
`Command::new(current_exe())` pathname으로 helper를 재실행하며 app executable/ancestor를 pin하거나
trust-verify하지 않는다. 따라서 현재 Git transaction 보장은 앱 설치·실행 경로를 교체할 권한이 없는 actor를
전제로 한다. 그 경로를 제어하는 same-UID actor는 typed Git 검증 전에 helper를 redirect할 수 있으므로 이는
미해결 process-launch authority 경계이며, crash/CAS 완료와 별도로 hardening해야 한다. 이를 강화할 때도 public
Git argv authority를 열거나 no-`unsafe` 계약을 완화하면 안 된다.

이 경계를 구현하는 다음 세션의 범위, platform decision gate와 회귀 명령은
[`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY.md)에
고정했다.

## 1. 착수 당시 목표 (완료)

착수 세션은 새 UI 기능을 추가하지 않고 당시 구현을
[`SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md`](SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md)의
완료 정의에 맞게 강화한다.

착수 당시 P0 목표는 다음과 같았으며 모두 0절에서 완료됐다.

1. Git-operation journal을 prepared/published/completed evidence를 가진 strict durable transaction으로 확장한다.
2. Stage/unstage index publish와 commit detached-HEAD CAS를 crash 뒤 live evidence만으로
   same receipt, safe resume 또는 sticky uncertain 중 하나로 수렴시킨다.
3. Standard `index.lock`/`HEAD.lock`을 proven-owned artifact와 inode/link-count 증명 없이 삭제하지 않게 한다.
4. Safe-remove journal과 Git-operation journal을 runtime startup 전에 cross-preflight하고 recovery 순서를 고정한다.
5. Fault/crash/response-loss matrix와 commit E2E를 추가한다.
6. Native recovery가 닫힌 뒤 frontend remount persistence와 composer start/steer disabled UX를 마무리한다.

아래 절의 미완료 표현은 작업 전 gap 기록이며 현재 상태 판정에는 0절을 사용한다.

## 2. 반드시 먼저 읽을 문서

다음 순서로 읽는다.

1. 이 문서
2. [`SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md`](SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md)
3. [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
4. [`SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`](SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md)
5. [`SESSION_HANDOFF_20260819_CODE_PHASE2_MODEL_SELECTOR.md`](SESSION_HANDOFF_20260819_CODE_PHASE2_MODEL_SELECTOR.md)
6. [`SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md`](SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md)
7. [`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md)
8. Repository root의 [`AGENTS.md`](../../AGENTS.md)

첫 명령은 반드시 다음과 같이 실행한다.

```bash
. ./bin/activate-hermit && git status --short
```

현재 checkout은 Phase 0~3 작업과 사용자 변경이 함께 있는 큰 dirty worktree다. 기존 tracked/untracked
파일을 보존한다. 이 checkout 자체에 `git add`, `git commit`, reset, checkout 또는 clean을 실행하지 않는다.
제품 테스트가 만든 격리된 임시 Git repository 안의 mutation만 예외다.

## 3. 최초 vertical slice에서 구현한 범위 (역사적 기록)

### 3.1 Native protocol과 명령

새 모듈은 `desktop/src-tauri/src/code_workspace/git_write/`에 있다.

- `protocol.rs`: strict public input/output DTO
- `engine.rs`: snapshot memory, journal, mutation/reconcile/ack orchestration
- `repository.rs`: repository projection, candidate index, commit object와 publish helper
- `tests.rs`: 실제 임시 Git repository happy-path 회귀
- `mod.rs`: internal/public re-export

추가한 public commands는 다음 여섯 개다.

- `code_thread_git_status`
- `code_thread_git_stage`
- `code_thread_git_unstage`
- `code_thread_git_commit`
- `code_thread_git_reconcile`
- `code_thread_git_acknowledge`

Command facade는 `desktop/src-tauri/src/commands/code_git_handoff.rs`에 있다. Caller input은 exact
`scope + threadId`와 native-issued `writeGeneration + snapshotId + fileId`, 또는 commit message/ack
coordinate만 받는다. Path, cwd, ref, OID, argv와 identity는 받지 않는다.

`AppState`에 `CodeGitWriteState`를 추가했고 `lib.rs` invoke handler와
`fixtures/tauri-contract-v1.json`, Rust/TypeScript contract sentinel에 여섯 명령을 기존 명령 뒤에 append했다.

### 3.2 최초 Native 동작

- Active managed-worktree binding과 detached `HEAD`만 write-ready가 된다.
- Status는 한 projection에서 `task`, `staged`, `unstaged`, capability와 opaque random 256-bit
  snapshot/file ID를 반환한다.
- Same path가 staged/unstaged 양쪽에 있으면 같은 snapshot 안에서 같은 file ID를 쓴다.
- Worktree changed bytes/mode와 index/HEAD를 snapshot preimage에 결박한다.
- Stage/unstage는 app-private candidate index에서 한 file 전체를 갱신한 뒤 `index.lock -> index`로 publish한다.
- Commit은 frozen candidate index의 `write-tree`, `commit-tree` 결과를 detached `HEAD.lock -> HEAD`로 publish한다.
- Commit 대상은 staged tree뿐이며 unstaged working bytes를 보존한다.
- Completed receipt를 fresh authoritative status로 확인하고 acknowledge하기 전까지 다음 mutation을 막는다.
- Completed exact-input response retry는 journal receipt를 재사용한다.
- Git journal blocker가 있으면 같은 binding의 resume/start/steer, PTY open과 archive를 Native에서 거부한다.
- Mutation clearance는 binding lock, active binding, root revalidation, authoritative idle/no approval와 no PTY를
  확인하고 runtime idle admission guard를 mutation 동안 유지한다.

신규 Rust 파일은 모두 1,000줄 이하로 분리됐다. 최종 상태에서 큰 파일은 다음과 같다.

```text
journal.rs                 989
owned_lock.rs              981
git_command.rs             953
transaction/recovery.rs    914
repository.rs              891
engine.rs                  856
commands/.../gate_tests.rs 702
transaction.rs             680
crash_tests/fixture.rs     566
```

### 3.3 최초 Frontend 동작

추가/변경된 중심 파일은 다음과 같다.

- `desktop/src/features/code/api/codeGitTypes.ts`
- `desktop/src/features/code/api/codeGitSchemas.ts`
- `desktop/src/features/code/api/codeWorkspace.ts`
- `desktop/src/features/code/state/codeSessionQueries.ts`
- `desktop/src/features/code/state/useCodeGitHandoff.ts`
- `desktop/src/features/code/ui/CodeChangesPanel.tsx`
- `desktop/src/features/code/ui/CodeGitChangesActions.tsx`
- `desktop/src/features/code/ui/CodeCommitDialog.tsx`

Frontend는 strict Zod decode를 사용한다. Caller-supplied Git authority field를 거부하고 ready action lane의
완전성, totals, stable sort/uniqueness, capability reason과 partial-stage shared ID를 검증한다.

Managed ready 상태에서는 `status.task`만 Task diff로 사용한다. Local/blocked/recovery/transport failure만
기존 `code_thread_changes`를 read-only fallback으로 읽는다. Stage/unstage/commit 뒤 row를 optimistic하게
옮기지 않고 다음 순서를 요구한다.

```text
mutation receipt
  -> fresh status with higher revision/generation and matching blockingReceipt
  -> acknowledge
  -> fresh status with higher revision and cleared blocker
  -> controls reopen
```

Working changes는 semantic Staged/Unstaged section과 full-path accessible action name을 사용한다. Commit dialog는
visible label, staged/binary/truncated counts, detached staged-only 설명과 native identity/trailer 설명을 제공한다.
UI는 baseline-ui 지침을 적용했으며 px/arbitrary text size를 추가하지 않았다.

### 3.4 최초 Mock/E2E

`desktop/src/testing/e2eBridge.ts`에 optional `schoolxCodeGitStatuses` sequence와 여섯 Git command mock을
추가했다. Config가 없으면 기존 local/read-only E2E를 보존하도록 blocked status를 반환한다.

`desktop/tests/e2e/schoolx-code.spec.ts`의 새 시나리오는 다음을 검증한다.

```text
stage click
  -> exact stage payload
  -> completed blocking status
  -> exact acknowledge payload
  -> acknowledged status
  -> authoritative Unstage row 표시
```

## 4. 착수 당시 잔여 P0 (모두 완료)

이 절은 0절 구현 전의 gap 분석을 그대로 보존한다. 아래의 “현재”, “아직”, “필요” 표현은 모두 착수
시점을 가리키며, 완료 근거와 최종 검증은 0절에 있다.

### 4.1 Journal/recovery가 원본 계약보다 얕다

현재 durable record는 사실상 다음 정보만 가진다.

```text
key, operationId, operation,
Pending | CompletedAwaitingAck | Uncertain,
inputDigest, optional receipt/message, acknowledged
```

다음 evidence가 없다.

- pinned root/admin/common-dir/Git executable identity
- before/expected-after index digest 또는 previous/expected HEAD
- candidate/artifact path와 inode/device/owner/mode/link-count/digest
- prepared/objectWritten/locksReady/indexPublished/headPublished phase
- frozen blob/tree/commit OID, identity, timestamp와 canonical message digest

따라서 process crash 뒤 `Pending`은 현재 `reconcile`에서 계속 `pending`만 반환하며 자동 수렴하지 않는다.
Publish와 `CompletedAwaitingAck` journal sync 사이 crash를 same receipt로 증명할 수 없다. 이 상태를 유지한 채
exact-once 완료라고 주장하지 않는다.

### 4.2 Publish/lock proof가 아직 충분하지 않다

- Candidate index를 표준 lock에 `create_new` + write한 뒤 rename하지만, random owned artifact와 hard-link
  identity/link-count proof를 사용하지 않는다.
- Commit은 HEAD가 preimage와 같은지 lock 생성 전에 확인한다. 두 standard lock 획득 뒤 publish 직전
  index/HEAD/root를 다시 증명하지 않는다.
- Cleanup은 locally constructed lock path를 best-effort unlink하지만 inode provenance journal이 없다.
- Crash 뒤 foreign lock과 owned lock을 구분해 안전하게 정리할 수 없다.
- Stage source는 snapshot status 재검사 뒤 path로 다시 읽는다. Open handle에 frozen bytes/mode를 결박하고
  candidate/blob OID를 검증하는 경계로 바꿔야 한다.

원본 문서 7.2/7.3의 prepared artifact, hard-link acquisition, re-stat, publish phase sync와 tri-state recovery를
그대로 구현한다.

### 4.3 Startup cross-preflight가 없다

현재 app startup은 Git journal을 safe-remove journal과 먼저 strict cross-join하지 않는다. 다음 순서를 추가한다.

```text
strict safe-remove journal load
  -> strict Git-operation journal load
  -> same-binding conflict fail closed
  -> safe-remove recovery for disjoint bindings
  -> Git-operation recovery
  -> runtime/lifecycle reconciliation
```

같은 binding에 두 journal blocker가 있으면 어느 쪽도 filesystem/Git mutation을 하면 안 된다.

### 4.4 Journal/filesystem hardening이 부족하다

다음이 필요하다.

- directory 0700, file 0600
- regular-file, owner, symlink, size와 record-count 검증
- bounded acknowledged tombstone history
- temp file identity와 exact parent sync 검증
- startup malformed/oversized journal byte 보존 + zero mutation tests
- candidate directory/file permission과 symlink replacement tests

### 4.5 Typed Git helper closure가 부족하다

현재 helper는 typed fixed call-site만 사용하고 environment를 clear하지만 다음이 없다.

- timeout과 process-group descendant cleanup
- bounded stdout/stderr streaming capture
- fsync capability/config enforcement
- split/shared/sparse index와 unsupported extension detection
- skip-worktree, assume-unchanged, intent-to-add/zero OID detection
- filters, text/eol/ident/working-tree-encoding와 incompatible `core.autocrlf` refusal
- alternates/shared object store, reftable/unknown ref backend refusal
- nested repository/gitlink/case-normalization collision closure

기존 pinned read helper의 timeout/process cleanup 패턴을 재사용하되 generic public argv authority는 열지 않는다.

### 4.6 Commit determinism이 부족하다

Author/committer timestamp가 prepared claim에 고정되지 않았다. Crash retry가 같은 commit OID를 재생성하도록
identity, timestamp, canonical message, parent와 tree를 journal evidence에 결박해야 한다. Repository-local
effective config/identity digest도 snapshot과 claim에서 동일하게 재검증한다.

### 4.7 Frontend recovery UX가 아직 완결되지 않았다

- Attempt state는 component-local이라 inspector remount 뒤 유지되지 않는다.
- `pending/recovering` reconcile 결과에 bounded polling/backoff가 없다.
- Git busy state가 composer start/steer controls에 시각적으로 공유되지 않는다. Native admission은 막지만
  frontend도 이유를 포함해 disabled 상태를 보여야 한다.
- Transport/decode failure alert에 명시적 Retry action이 없다. Header refresh만 존재한다.
- Commit response-loss/reconcile E2E와 keyboard/focus recovery E2E가 없다.

Native recovery 경계를 먼저 닫고 query/external store 기반 attempt persistence를 추가한다. Module-level store를
추가하면 community switching reset을 `resetCommunityState()`에 반드시 연결한다.

## 5. 착수 당시 권장 구현 순서 (완료)

1. 현재 public Tauri/TypeScript contract shape를 변경하지 말고 journal 내부 schema v1을 재설계한다.
2. `repository.rs`를 prepare/publish/recover 단계로 나눈다.
3. Stage candidate를 live object/index write 전에 완성하고 before/after semantic digest를 계산한다.
4. Commit identity/timestamp/message/index를 freeze하고 object/ref evidence phase를 journal에 sync한다.
5. Proven-owned artifact와 standard lock acquisition/revalidation/publish helper를 공통화한다.
6. Startup safe-remove/Git journal preflight와 recovery를 runtime start 앞에 연결한다.
7. 각 durable boundary fault injection + subprocess crash matrix를 만든다.
8. Cross-gate concurrency tests를 추가한다.
9. Frontend remount/reconcile/composer UX와 commit response-loss E2E를 추가한다.
10. Targeted tests 후 전체 frontend/Tauri 회귀를 실행한다.

`engine.rs`나 `repository.rs`를 다시 1,000줄 이상으로 키우지 않는다. Journal, transaction evidence,
owned-lock/recovery와 fault tests를 별도 module로 분리한다.

## 6. 보존해야 할 public 계약

- 기존 `code_thread_changes` input/output을 변경하지 않는다.
- 여섯 Git command 이름과 top-level `{input}` envelope 순서를 유지한다.
- Mutation input에 path/ref/OID/argv/identity/operationId를 추가하지 않는다. Ack correlation만 operationId를 받는다.
- Ready staged/unstaged action set은 complete/non-truncated여야 한다.
- `statusRevision` late response rejection과 `(writeGeneration, snapshotSequence)` 비교를 유지한다.
- Mutation success 후 fresh blocking status와 explicit ack 없이는 다음 action을 열지 않는다.
- Local checkout과 archived lifecycle write를 열지 않는다.
- Phase 2 binding v4, model selector, fork wire와 safe-remove public receipt를 변경하지 않는다.
- 기존 invoke command order에는 새 명령을 중간 삽입하지 않는다.

## 7. 최초 vertical slice 검증 결과 (역사적 기록)

다음이 통과했다.

- `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check`
- `cargo check --manifest-path desktop/src-tauri/Cargo.toml --lib`
- `cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib -- -D warnings`
- Tauri 전체 lib tests: **2357 passed, 19 ignored, 0 failed**
- 실제 Git stage/ack/staged-only commit/unstaged-byte preservation test
- Native contract tests: **8 passed, 1 ignored**
- `pnpm --dir desktop typecheck`
- `pnpm --dir desktop check:px-text`
- Frontend 전체 tests: **4024 passed, 0 failed**
- Targeted Biome check
- Fresh `pnpm --dir desktop build:e2e`
- 새 Git stage/receipt/ack Playwright: **1 passed**
- `git diff --check`

전체 Tauri suite는 기존 production-cost와 subprocess crash tests 때문에 약 515초 걸렸다. Frontend 전체 suite는
약 74초 걸렸다. Test output의 quota/network/corrupt-audio diagnostic은 의도된 negative-path fixture였고 실패가
아니다.

이번 세션에서는 shared checkout에 stage/commit/reset/checkout/clean을 실행하지 않았다.

## 8. 재검증 명령

```bash
. ./bin/activate-hermit && git status --short

cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo check --manifest-path desktop/src-tauri/Cargo.toml --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib --tests -- -D warnings
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::git_write --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  commands::code_git_handoff::gate_tests --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib

pnpm --dir desktop typecheck
pnpm --dir desktop check:px-text
pnpm --dir desktop test
pnpm --dir desktop exec biome check \
  src/features/code src/testing/e2eBridge.ts tests/e2e/schoolx-code.spec.ts

git diff --check
```

Fresh E2E 전에는 `AGENTS.md`의 stale preview 지침대로 port 4173 listener를 읽기 전용으로 확인하고,
기존 listener가 stale Playwright preview임이 확인된 경우에만 종료한다. 반드시 `build:e2e`를 사용한다.

```bash
lsof -nP -iTCP:4173 -sTCP:LISTEN
pnpm --dir desktop build:e2e
pnpm --dir desktop exec playwright test tests/e2e/schoolx-code.spec.ts \
  --project=smoke
```

## 9. 다음 세션 복사용 시작 요청

```text
SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY.md를 먼저 읽고,
SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md와
SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md의 7~12절 및 완료 정의와 대조해줘.
SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md와 SECURITY_CONTRACT.md도 함께 확인해줘.

첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘. 현재 shared dirty worktree의
tracked/untracked 사용자 변경을 보존하고 이 checkout 자체를 stage/commit/reset/checkout/clean하지 마.

0절의 Phase 3 P0 완료 상태와 최종 검증을 먼저 확인하고, public contract와 crash/CAS/owned-lock/startup
recovery 계약을 회귀시키지 마. 다음 제품 slice가 별도로 정해지지 않았다면 기능 범위를 임의로 넓히지 말고
현재 dirty worktree와 검증 상태만 점검해줘.

신규 handoff의 decision gate와 세 production Code Git helper 범위를 지켜 app `current_exe()` pathname helper
launch authority를 다뤄줘. Root-trusted Git executable 정책, typed fixed command surface, no-`unsafe`, 신규
Phase 3 파일 1,000줄 제한을 유지하고, local/archive write, stage-all/hunk, branch/push/PR/Talk 공유 또는
hook/signing을 그 작업에 섞지 마.
```
