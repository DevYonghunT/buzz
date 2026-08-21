# SchoolX Code Phase 2 worktree removal gates 세션 인계

작성일: 2026-08-16

## 1. 현재 결론

Actual managed-worktree remove 전에 필요했던 네 decision gate를 문서, machine-readable contract와
native/frontend/E2E regression sentinel로 고정했다. 실제 remove implementation은 추가하지 않았다.

- Normative design은
  [`SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`](SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md)다.
- Frozen mirror는
  `desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`이다.
- Binding store는 계속 v3이며 v4 removal record/DTO를 production에서 decode하지 않는다.
- `code_worktree_remove`, merge-proof command, frontend adapter/mutation/button은 없다.
- Inventory는 exact `{scope}`, 모든 row의 `preserved: true`, `canRemove: false`, Archived의
  `mergeProofUnavailable`를 유지한다.

## 2. 작업 트리 보존 규칙

이 checkout은 이전 Phase 1/2/inventory 구현과 사용자 변경이 함께 dirty/untracked다. 기존 파일을
reset/checkout/delete하지 않았고 stage/commit도 하지 않았다.

다음 세션의 첫 명령도 반드시 다음과 같이 실행한다.

```bash
. ./bin/activate-hermit && git status --short
```

`.dockerignore`, `.gitignore`, `crates/buzz-core/src/relay.rs`, `deploy/`, `brand/`, `supabase/`, 기존
SchoolX Code native/frontend/test와 이전 `SESSION_HANDOFF_*.md`는 기존 작업이므로 임의로 정리하지
않는다. Hermit 활성화 없이 Git/Rust/Node command나 hook을 실행하지 않는다.

## 3. 고정한 future public contract

첫 safe-remove mutation의 future input은 exact 다음 두 필드뿐이다.

```text
CodeWorktreeRemoveInput {
  scope: CodeThreadBindingScope
  threadId: string
}
```

Top-level argument는 `input` 하나다. Caller는 path, descriptor, worktree ID, base/HEAD/target OID,
target ref, lifecycle, blocker, `canRemove`, force/request/removal ID나 proof claim을 보낼 수 없다.
`(scope, threadId)`가 retry key이며 native가 canonical UUID `removalId`를 발급한다.

Future success receipt의 frozen fields는 다음이다.

```text
removalId
scope
threadId
worktreeId
headCommit
mergedIntoRef
mergedIntoCommit
transcriptDisposition: preserved
executionDisposition: removed
```

이 DTO는 아직 Rust/TypeScript production type으로 추가하지 않았다. Gate fixture만 future shape를
고정한다.

## 4. Gate 1 — merged authority

Native가 preparation의 selected base를 같은 Git common-dir의 direct local
`refs/heads/<name>`으로 유일하게 확정하고 그 commit이 persisted immutable base와 같을 때만 future
v4 authority로 저장한다.

- `HEAD`는 source checkout이 attached local branch일 때만 확정한다.
- Tag/raw OID/remote-tracking/arbitrary ref는 authority가 아니다.
- Fork는 source authority만 atomic copy한다.
- Existing V1/V2/V3 binding은 authority absent이며 `baseRef`/현재 branch에서 추론하지 않는다.
- Remove caller가 ref/OID를 선택하거나 제출하지 않는다.

Proof는 exact detached HEAD `H`와 authorized ref commit `T`의
`merge-base --is-ancestor H T` exit 0만 인정한다. Proof 전후 HEAD/ref/common-dir/root identity가 같아야
한다. Exit 1은 unmerged, 나머지 error/timeout/drift/missing object는 unavailable이다. Replacement object,
non-empty `info/grafts`, lazy fetch/network는 금지한다. Squash/cherry-pick/다른 containing ref도 proof가
아니다.

Positive proof는 future removal에서 committed `H != baseRef`의 `headDrift`를 해소할 수 있지만,
현재 inventory blocker/projection은 바꾸지 않았다.

## 5. Gate 2 — durable removal journal

Future binding index v4는 lifecycle/preparation과 별도인 `removals` namespace를 사용한다.

```text
claimed -> removing -> removed
```

- `claimed`: exact Archived binding, merge/physical proof가 durable sync됐고 deletion mutation은 0회다.
  Definitely-not-started일 때만 exact cancellation 가능하다.
- `removing`: 최초 Git/filesystem mutation 전에 sync되는 sticky state다. Crash/response loss 뒤
  rollback, cancel, 새 target 선택을 하지 않는다.
- `removed`: exact original/quarantine root와 Git-admin absence 뒤에만 진입한다. 같은 atomic index
  write에서 live binding+lifecycle을 permanent tombstone으로 retire한다.

`claimed/removing`은 live stable Archived managed binding과 1:1 join하고 `removed`는 live binding과
공존하지 않는다. 모든 removal record는 thread/worktree/root identity를 예약한다. Startup recovery는
lifecycle reconciliation과 start/fork recovery보다 먼저 pending removal을 처리한다.

`refs/schoolx/removal-claims/<removalId>` proof ref는 `removing` sync 뒤 만드는 첫 Git mutation이며,
journal보다 앞설 수 없다.

## 6. Gate 3 — binding/transcript semantics

- Verified physical absence 전에는 live binding/lifecycle을 삭제하지 않는다.
- Finalization은 original binding 전체를 permanent removal tombstone으로 옮긴다.
- Codex `$CODEX_HOME` transcript/thread는 삭제·이동·복제·게시하지 않는다.
- Tombstone은 recovery coordinate와 idempotent receipt를 보존하지만 executable binding이 아니다.
- Resume/turn/PTY/Changes/rename/unarchive/fork admission에서 tombstone을 제외한다.
- Removed transcript 표시는 future tombstone-aware read-only `thread/read` 경로다.
- Restore는 새 preparation/worktree를 만드는 별도 future operation이다.

Archive lifecycle에 removing/removed를 섞거나 start/fork preparation을 cleanup journal로 해석하지
않는다.

## 7. Gate 4 — pinned deletion boundary

Inventory의 `dirty:false`는 ignored file과 empty directory를 보지 않으므로 deletion proof가 아니다.
현재 pinned Git helper도 parent handle이 없고 success 후 target name이 사라지는 postcondition을
표현하지 못하므로 재사용하지 않는다.

Future claim은 no-follow physical manifest를 새로 만들고 linked-worktree `.git`, tracked file/symlink와
ancestor directory만 허용한다. Ignored/untracked/empty/special/cross-device/submodule/nested repository
entry가 있으면 mutation 전에 거부한다.

Journal `removing` sync 뒤 parent-handle의 UUID root를 deterministic
`.schoolx-removing-<removalId>`로 atomic no-replace rename한다. Frozen manifest만 handle-relative,
no-follow deletion하고 root absence 뒤 exact reciprocal Git-admin entry를 제거한다. Original,
quarantine 또는 admin replacement는 삭제하지 않고 sticky recovery로 닫는다. 동등한 non-Unix
boundary가 없으면 unsupported/zero-mutation이다.

금지된 우회:

- `--force`, `git clean/reset`
- `git worktree remove/prune`
- broad/pathname `remove_dir_all`
- inventory receipt reuse와 frontend path/ref/OID/proof claim
- archive/fork/start/orphan implicit cleanup
- fetch/network/credential/PR merge proof
- Codex transcript/thread delete

## 8. 이번 slice의 변경 파일

새 파일:

- `docs/schoolx-2/SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`
- `desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`
- 이 handoff 문서

업데이트:

- `docs/schoolx-2/SCHOOLX_CODE_DESIGN.md`
- `desktop/src-tauri/src/code_workspace/contract_tests.rs`
- `desktop/src-tauri/src/code_workspace/worktrees.rs` — test-only helper operation rejection
- `desktop/src/features/code/api/codeWorkspace.contract.test.mjs`
- `desktop/src/features/code/ui/CodeWorktreeInventorySection.test.mjs`
- `desktop/tests/e2e/schoolx-code.spec.ts`

Production store/schema/command/adapter/component implementation은 추가하지 않았다.

## 9. 고정한 regression sentinel

Native contract test는 다음을 exact 검증한다.

- Gate order/state, future input/receipt와 v3→future v4 경계
- Direct local branch authority, proof snapshot/rejected evidence
- `claimed/removing/removed`, physical mutation order와 acceptance matrix
- Transcript tombstone, physical manifest/quarantine policy와 forbidden operations
- Frozen Tauri command/strict input/output에 remove surface가 없음
- 실제 `generate_handler!`에도 destructive worktree command가 없음

Current pinned helper test는 `remove`, `worktreeRemove`, `delete`, `destroy`, `cleanup`, `clean`, `prune`,
`purge`, `discard` operation JSON을 모두 strict decode 단계에서 거부한다.

Frontend contract test는 command/adapter absence, gate fixture, inventory input의 thread/path/worktree/
descriptor/lifecycle/removal claim 거부를 고정한다. Inventory component test는 populated/empty view의
button 1개(refresh), error view의 button 2개(refresh+retry)를 exact count해 이름 없는 remove action도
막는다. E2E는 destructive button과 command invocation이 0개임을 검증한다.

## 10. 실행해 통과한 검증

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests::worktree_removal_decision_gates_are_frozen_while_the_surface_is_absent \
  -- --exact

cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::worktrees::tests::pinned_git_helper_envelope_is_bounded_and_strict \
  -- --exact

cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests::tauri_command_input_enum_and_event_contract_is_exact \
  -- --exact

cd desktop
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/code/api/codeWorkspace.contract.test.mjs \
  src/features/code/ui/CodeWorktreeInventorySection.test.mjs

pnpm build:e2e
pnpm exec playwright test --project=smoke \
  tests/e2e/schoolx-code.spec.ts \
  --grep "lists only preserved managed roots"
```

결과:

- Rust targeted tests: 3 passed
- Node contract/component tests: 29 passed
- E2E targeted inventory test: 1 passed
- E2E-mode TypeScript/Vite build: passed

## 11. 남은 작업과 권장 순서

Actual remove command/button을 바로 추가하지 않는다. 구현은 다음 독립 slices로 나누는 편이 안전하다.

1. **Non-destructive v4 authority/proof slice**
   - Optional direct-local merge-target capture/migration-absent policy
   - Typed hardened ancestry proof와 actual-Git zero-mutation matrix
   - Store v4 strict empty-removal migration만 열고 non-empty record는 아직 fail closed
2. **Pure journal/tombstone slice**
   - Removal record strict types/join/CAS/state machine
   - Crash/retry/final-save fault injection과 identity non-reuse
   - 여전히 Git/filesystem deletion과 public command 없음
3. **Pinned quarantine/deletion engine slice**
   - Physical manifest와 handle-relative no-follow deletion
   - Original/quarantine/admin full observation matrix와 platform-specific tests
   - Public Tauri/frontend surface 없음
4. **Explicit safe-remove mutation/UI slice**
   - 앞 세 slice의 authority와 engine만 조합
   - Exact command/receipt, confirmation UI, mock/E2E
   - 모든 crash boundary와 response-loss test 통과 후에만 등록

Model/reasoning selector, Git write handoff, dirty patch fork, automatic orphan cleanup,
`command/exec`, 기존 quality-gate 부채는 이 흐름에 섞지 않는다.

## 12. 다음 세션 복사용 시작 요청

```text
SCHOOLX_CODE_DESIGN.md와 최신
SESSION_HANDOFF_20260816_CODE_PHASE2_WORKTREE_REMOVAL_GATES.md, 현재 작업 트리를 먼저 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Phase 1의 열 개 closure와 Phase 2 terminal/search/rename/archive/fork/inventory는 완료됐고,
managed-worktree removal의 네 decision gate도 문서와 frozen contract/sentinel로 고정됐으므로
다시 설계하거나 실제 remove command/button을 추가하지 마.

다음 독립 slice는 non-destructive v4 merged-target authority/proof로 제한해줘. Preparation에서
same-common-dir direct local refs/heads authority만 optional capture하고, fork는 source authority만
복사하며 V1/V2/V3는 authority absent로 migrate해줘. Hardened merge-base ancestry proof의
merge/unmerged/squash/other-ref/drift/replacement/graft/timeout/zero-mutation actual-Git tests를 고정해줘.
Store v4를 열더라도 non-empty removal journal은 아직 fail closed하고 public merge-proof/remove
command, frontend adapter/button, Git/filesystem deletion은 추가하지 마.

Model/reasoning selector, Git write handoff, dirty patch fork 복사, automatic orphan cleanup,
command/exec, 기존 quality-gate 부채 정리를 섞지 마. 기존 사용자 변경과 untracked 파일을 보존하고
stage나 commit은 하지 마.
```
