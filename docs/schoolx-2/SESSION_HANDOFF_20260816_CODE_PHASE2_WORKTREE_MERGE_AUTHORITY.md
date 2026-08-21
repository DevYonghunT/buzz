# SchoolX Code Phase 2 worktree merge authority/proof 세션 인계

작성일: 2026-08-16

## 1. 현재 결론

Managed-worktree safe remove의 첫 구현 slice인 **non-destructive v4 merge authority/proof**를
완료했다. 실제 remove mutation은 추가하지 않았다.

- Binding store current schema는 v4다.
- Public `CodeThreadBinding`은 기존 exact 8 fields를 유지한다.
- Direct-local merge target은 native-only sibling `mergeTargets`로 저장한다.
- Preparation의 private `mergeTargetRef`는 root preparation에서만 capture하고 fork는 source authority만
  copy한다.
- V1/V2/V3는 target을 추론하지 않고 authority absent로 in-memory migration한다.
- Required `removals`는 이번 build에서 exact `[]`만 decode한다.
- Hardened native ancestry proof는 구현했지만 public command, inventory eligibility와 UI에 연결하지
  않았다.
- `code_worktree_remove`, merge-proof Tauri command, frontend remove adapter/button, Git/filesystem
  deletion은 여전히 없다.
- Inventory는 계속 모든 row가 `preserved: true`, `canRemove: false`이고 Archived row는
  `mergeProofUnavailable`를 가진다.

Normative 계약은
[`SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`](SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md), machine mirror는
`desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`이다.

## 2. 작업 트리 보존 규칙

이 checkout은 이전 Phase 1/2와 사용자 변경이 함께 dirty/untracked다. 기존 파일을
reset/checkout/delete하지 않았고 stage/commit도 하지 않았다.

다음 세션의 첫 명령도 다음과 같이 실행한다.

```bash
. ./bin/activate-hermit && git status --short
```

Hermit 활성화 없이 Git/Rust/Node command나 hook을 실행하지 않는다. `.dockerignore`, `.gitignore`,
`crates/`, `deploy/`, `brand/`, `supabase/`, 기존 SchoolX Code 변경과 이전 handoff 문서를 임의로
정리하지 않는다.

## 3. Binding store v4

V4 top-level persisted shape는 다음 collection을 포함한다.

```text
version: 4
bindings: [exact existing 8-field public binding]
lifecycles: [...]
preparations: [... optional private mergeTargetRef ...]
mergeTargets: [...]
removals: []
```

`mergeTargets` record는 exact 다음 6 fields다.

```text
communityId
projectDtag
repositoryIdentity
codexThreadId
worktreeId
targetRef
```

한 committed managed binding과 0:1 join한다. Duplicate/orphan/wrong-worktree record, non-local ref,
invalid scope는 index 전체를 fail closed한다. Public binding에는 9번째 field를 추가하지 않았다.

Preparation의 optional `mergeTargetRef`는 managed start에만 허용한다. Public preparation list와
inventory authority snapshot은 이 field 및 recovery baseline을 scrub한다. Preparation commit은 같은
atomic index write에서 private authority를 sibling record로 옮긴다.

V1/V2/V3용 strict wire structs를 각각 decode한다. Legacy files는 load만으로 write/mtime을 바꾸지
않고 v4 in-memory index로 변환되며 `mergeTargets=[]`, `removals=[]`, preparation authority `None`이다.
다음 mutation이 v4를 atomic persist한다. V3에 forged `mergeTargetRef`, v4 missing required collection,
unknown/future fields와 non-empty `removals`는 bytes를 보존한 채 거부한다.

## 4. Direct-local authority capture/copy

Root managed-worktree preparation은 source repository와 base commit을 resolve한 뒤 첫 worktree Git/FS
mutation 전에 optional authority를 capture한다.

- Attached `HEAD`, short local branch와 exact `refs/heads/<name>`만 후보가 된다.
- Exact local ref commit이 immutable resolved `baseRef`와 같을 때만 저장한다.
- Detached `HEAD`, tag, raw 40/64-hex OID, remote-tracking ref, `origin/HEAD`, arbitrary ref와 revision
  expression은 authority absent다.
- Local execution mode는 authority를 저장하지 않는다.
- Fork preparation은 destination HEAD/OID/ref에서 recapture하지 않는다. Exact source binding sibling의
  `Some(ref)` 또는 `None`을 같은 store write에서 그대로 copy한다.

Caller/webview는 `mergeTargetRef`를 보내거나 받지 않는다.

## 5. Hardened ancestry proof

Private proof input은 exact store binding/authority snapshot, managed nest와 caller-owned deadline이다.
Authority absent는 closed `None`이고 `baseRef`나 inventory row에서 추론하지 않는다.

Proof는 current managed-worktree HEAD `H`와 persisted authorized ref의 current commit `T`를 pre/post
snapshot하고 다음 command의 exit 0만 positive로 인정한다.

```text
git merge-base --is-ancestor --end-of-options H T
```

Positive receipt는 exact 다음 fields에 결박한다.

```text
repositoryIdentity
worktreeId
headCommit
targetRef
targetCommit
```

HEAD/ref/root/common-dir identity가 pre/post 동일해야 한다. Exit 1은 `NotMerged`; 다른 exit/signal,
timeout/budget exhaustion, missing ref/object, malformed output, HEAD/ref/root/common-dir drift는 error로
fail closed한다. Proof 자체는 lifecycle eligibility나 `canRemove`를 만들지 않는다. Future remove
admission이 별도로 stable Archived lifecycle, idle gate, journal과 physical boundary를 결합해야 한다.

Git subprocess는 typed argv, pinned target directory, bounded output/time과 다음 environment를 쓴다.

- `GIT_NO_REPLACE_OBJECTS=1`
- `GIT_NO_LAZY_FETCH=1`
- Unix `GIT_GRAFT_FILE=/dev/null`
- `GIT_OPTIONAL_LOCKS=0`
- system/global config, credentials, hooks, fsmonitor와 protocol 차단

Non-empty common-dir `info/grafts`는 command 전후 거부한다. Squash/cherry-pick equivalence, 다른 ref의
reachability와 replacement/graft-only ancestry는 proof가 아니다.

## 6. 실제 Git acceptance matrix

Actual Git tests가 다음을 고정한다.

- `H == T` positive
- `--no-ff` merge commit을 통한 ancestry positive
- unmerged task, authorized-vs-other-ref distinction negative
- squash-only, cherry-pick-only negative
- replacement-only ancestry negative
- non-empty graft unavailable
- missing target와 missing target object unavailable
- expired deadline unavailable
- proof 중 target ref 또는 HEAD drift unavailable
- proof 전후 binding index, refs, managed HEAD/status와 `.git` coordinate 불변
- literal typed argv와 no-replace/no-lazy-fetch/no-graft environment

## 7. 계속 부재하는 surface

- `code_worktree_remove`
- Public merge-proof command/DTO
- Frontend path/ref/OID/proof claim
- Remove adapter, mutation hook, confirmation/button와 optimistic row removal
- `canRemove: true`
- `--force`, `git clean/reset`, `git worktree remove/prune`, `remove_dir_all`
- Fetch/network/credential/PR merge claim
- Removal journal record와 tombstone execution
- Git-admin/root deletion 또는 Codex transcript/thread mutation

Frozen Tauri command list는 기존 27개이고 pinned helper는 remove/delete/clean/prune 계열 operation을
strict decode 단계에서 계속 거부한다.

## 8. 이번 slice의 주요 변경 파일

- `desktop/src-tauri/src/code_workspace/bindings.rs`
- `desktop/src-tauri/src/code_workspace/bindings/lifecycle.rs`
- `desktop/src-tauri/src/code_workspace/bindings/lifecycle/tests.rs`
- `desktop/src-tauri/src/code_workspace/worktrees.rs`
- `desktop/src-tauri/src/code_workspace/mod.rs`
- `desktop/src-tauri/src/commands/code_workspace.rs`
- `desktop/src-tauri/src/commands/code_thread_fork/tests.rs`
- `desktop/src-tauri/src/code_workspace/worktree_inventory/tests.rs`
- `desktop/src-tauri/src/code_workspace/contract_tests.rs`
- `desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`
- `desktop/src/features/code/api/codeWorkspace.contract.test.mjs`
- `docs/schoolx-2/SCHOOLX_CODE_DESIGN.md`
- `docs/schoolx-2/SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`
- 이 handoff 문서

## 9. 실행한 검증

모든 command는 Hermit 활성화 뒤 실행했다.

```bash
cargo check --manifest-path desktop/src-tauri/Cargo.toml
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib -- -D warnings

cargo test --manifest-path desktop/src-tauri/Cargo.toml code_workspace::bindings::
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::worktrees::tests::merge_proof_
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::worktrees::tests::captures_only_same_commit_direct_local_branch_authority -- --exact
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::worktrees::tests::pinned_git_helper_envelope_is_bounded_and_strict -- --exact
cargo test --manifest-path desktop/src-tauri/Cargo.toml code_workspace::worktree_inventory::tests::
cargo test --manifest-path desktop/src-tauri/Cargo.toml commands::code_workspace::tests::
cargo test --manifest-path desktop/src-tauri/Cargo.toml commands::code_thread_fork::tests::
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests::worktree_removal_decision_gates_are_frozen_while_the_surface_is_absent \
  -- --exact
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests::tauri_command_input_enum_and_event_contract_is_exact -- --exact

cd desktop
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/code/api/codeWorkspace.contract.test.mjs \
  src/features/code/ui/CodeWorktreeInventorySection.test.mjs
pnpm exec biome check src/features/code/api/codeWorkspace.contract.test.mjs
```

최종 결과:

- Binding/lifecycle: 47 passed
- Merge proof: 5 passed
- Capture/pinned envelope: 2 passed
- Inventory: 10 passed
- Workspace command: 22 passed
- Fork command: 5 passed
- Frozen Rust contracts: 2 passed
- Frontend contract/component: 29 passed
- Cargo check/fmt, production lib clippy와 targeted Biome: passed

`cargo clippy --lib --tests -- -D warnings`는 이번 slice가 아닌 기존
`commands/code_terminal.rs::open_terminal_for_test`의 8-argument `clippy::too_many_arguments` 한 건에서
멈춘다. 이번 authority/proof code의 clippy 지적은 모두 해소했고, 해당 기존 test helper를 이 작업에
섞어 수정하거나 allow하지 않았다.

## 10. 다음 권장 slice

다음 작업은 **pure removal journal/tombstone**으로 제한한다. 아직 Git/filesystem deletion과 public
command/UI를 추가하지 않는다.

1. V4의 empty-only `removals` placeholder를 strict tagged record types로 교체한다.
2. Exact `(scope, threadId)`에 하나뿐인 native-issued `removalId`와
   `claimed -> removing -> removed` CAS state machine을 구현한다.
3. `claimed/removing`은 exact stable Archived live binding/lifecycle과 1:1 join하고 `removed`는 원래
   8-field binding, proof/physical coordinates와 literal transcript/execution disposition을 가진 permanent
   tombstone으로 만든다.
4. Thread/worktree/root identity 재사용을 binding/preparation/fork/recovery admission에서 막는다.
5. Save failure, crash/response loss, retry, definitely-not-started cancellation, sticky removing과 final
   tombstone swap을 pure-store fault injection으로 고정한다.
6. Inventory projection, Tauri command set, frontend와 filesystem/Git state는 계속 바꾸지 않는다.

Pinned quarantine/deletion engine은 그 다음 별도 slice다.

## 11. 다음 세션 복사용 시작 요청

```text
SCHOOLX_CODE_DESIGN.md와 최신
SESSION_HANDOFF_20260816_CODE_PHASE2_WORKTREE_MERGE_AUTHORITY.md, 현재 작업 트리를 먼저 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Phase 1/2 terminal/search/rename/archive/fork/inventory와 removal decision contract, non-destructive v4
direct-local merge authority와 hardened ancestry proof는 완료됐으므로 다시 구현하지 마. 실제 remove,
public proof/remove command, frontend adapter/button와 Git/filesystem deletion도 추가하지 마.

다음 독립 slice는 pure v4 removal journal/tombstone으로 제한해줘. Empty-only removals placeholder를
strict claimed/removing/removed records로 교체하고 exact binding/lifecycle join, native canonical UUID,
retry/CAS, identity non-reuse, crash/response-loss/final-save fault injection을 구현해줘. Codex transcript는
preserved이고 removed tombstone은 executable authority가 아니어야 해. Inventory와 public Tauri/frontend
surface는 그대로 유지해줘.

Model/reasoning selector, Git write handoff, dirty patch fork, automatic orphan cleanup, command/exec,
pinned physical deletion engine과 기존 quality-gate 부채를 섞지 마. 기존 사용자 변경과 untracked
파일을 보존하고 stage나 commit은 하지 마.
```
