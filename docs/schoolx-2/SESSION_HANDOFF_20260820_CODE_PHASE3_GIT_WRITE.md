# SchoolX Code Phase 3 exact-bound Git write 착수 인계

기준일: **2026-08-20**

문서 상태: **역사적 착수 task brief (superseded)**. 이 문서를 작성한 세션에서는 Phase 3 제품 코드를
구현하지 않았다. 현재 구현/검증 상태에는 이 문서의 작업 요청이나 17절 시작 프롬프트를 사용하지 말고
[`SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md`](SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md)의
0절을 우선한다.

## 1. 다음 세션의 목표

Phase 2는 model/reasoning selector까지 완료됐다. 다음 독립 수직 slice는 SchoolX Code의
exact bound-thread Changes inspector에 **whole-file stage/unstage와 staged-only commit**을 추가하는
작업이다.

첫 slice는 다음 범위로 고정한다.

- `active` lifecycle의 managed worktree binding만 허용한다.
- Managed worktree의 detached `HEAD`만 허용한다.
- 한 번에 한 파일 전체를 stage 또는 unstage한다.
- Commit은 현재 staged tree만 포함하고 unstaged working bytes는 보존한다.
- Caller는 scope/thread와 native가 발급한 opaque snapshot/file coordinate 및 commit message만 보낸다.
- Git mutation은 native journal, candidate index publish와 detached `HEAD` CAS로 exact-once에 수렴한다.
- 결과를 optimistic하게 표시하지 않고 native receipt 뒤 authoritative snapshot으로만 UI를 바꾼다.

다음 항목은 별도 slice로 남긴다.

- Local-checkout write와 Archived thread write
- Stage all, multi-select batch, partial hunk, patch apply와 interactive staging
- Rename/copy 전용 의미, symlink, submodule/gitlink와 sparse-index 지원
- Amend, allow-empty, merge, rebase, cherry-pick와 conflict resolution
- Branch 생성, push, PR, review/start, inline diff comment와 Talk 공유
- Repository hook, commit signing과 author/identity 편집 UI

기존 project-level branch/push/PR command는 이 slice의 Git write authority가 아니다. 연결은 stage/commit
경계가 닫힌 뒤 별도 작업으로 진행한다.

## 2. 반드시 먼저 읽을 문서

다음 순서로 읽는다.

1. 이 문서
2. [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
3. [`SESSION_HANDOFF_20260819_CODE_PHASE2_MODEL_SELECTOR.md`](SESSION_HANDOFF_20260819_CODE_PHASE2_MODEL_SELECTOR.md)
4. [`SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md`](SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md)
5. [`SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md`](SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md)
6. [`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md)
7. Repository root의 [`AGENTS.md`](../../AGENTS.md)

첫 명령은 반드시 다음과 같이 실행한다.

```bash
. ./bin/activate-hermit && git status --short
```

현재 repository는 Phase 0~2 구현과 사용자 변경이 함께 있는 큰 dirty worktree다. 기존 tracked/untracked
파일을 보존하고 이 shared checkout 자체에는 `git add`, `git commit`, reset, checkout, clean을 실행하지
않는다. 제품 테스트가 만든 격리된 임시 Git repository 안의 mutation만 예외다.

## 3. 현재 출발점

완료된 기반은 다음과 같다.

- Persisted exact scope와 binding index v4
- Thread별 managed worktree와 immutable base commit
- Runtime generation, active/uncertain turn와 approval admission
- Exact bound-thread PTY ownership과 archive/fork/remove lifecycle gate
- Persisted-base 대비 현재 task 전체를 읽는 `code_thread_changes`
- Pinned directory/common-dir identity와 hardened read-only Git helper
- Exact-scope inventory, fork, archive/unarchive와 public safe removal
- Pinned Codex 0.145 model catalog, native pair validation과 selector UX

현재 `code_thread_changes({input:{scope,threadId}})`는 persisted `baseRef`에서 working tree까지의 task diff를
한 벌로 합쳐 반환한다. Commit 뒤에도 전체 task diff를 계속 검토할 수 있다는 장점이 있지만
`HEAD -> index`와 `index -> worktree`를 구분하지 않으므로 stage/commit authority로 사용할 수 없다.

기존 public input과 6-field output은 compatibility sentinel로 그대로 둔다. 새 mutation용 control plane은
별도 `code_thread_git_status`에서 한 번의 stable native snapshot으로 다음 세 축을 함께 읽는다.

- `task`: persisted base -> working tree. 기존 Changes의 의미다.
- `staged`: current `HEAD` -> index. 다음 commit에 들어갈 내용이다.
- `unstaged`: index -> working tree + untracked. 아직 commit에 들어가지 않을 내용이다.

부분 stage된 같은 path는 `staged`와 `unstaged` 양쪽에 동시에 나타날 수 있다. `baseRef` diff에서 staged
상태를 추론하거나 서로 다른 시점의 두 read를 frontend에서 합치지 않는다.

## 4. 첫 slice의 authority 원칙

Public webview intent에서 실제 Git write까지의 경계는 다음 순서를 따른다.

```text
scope + thread + opaque snapshot/file intent
  -> persisted active managed binding
  -> authoritative idle + no approval + no PTY admission
  -> pinned root/admin/common-dir + detached HEAD 재검증
  -> fresh HEAD/index/worktree preimage 일치
  -> durable native Git-operation claim
  -> typed candidate index 또는 commit object
  -> index publish 또는 detached HEAD compare-and-swap
  -> durable native-derived receipt
  -> authoritative status read
  -> UI cache 교체
```

다음을 caller authority로 받지 않는다.

- cwd, repository path, execution root, `.git` path와 worktree ID
- 파일 path/pathspec, ref, branch, HEAD/OID/tree/blob와 base commit
- Shell argv, Git subcommand, environment, config, hook와 filter
- Author/committer identity, timestamp, signing key와 trailers
- Force, reset, clean, amend, allow-empty, merge proof와 removal receipt
- Git mutation input의 native operation ID, journal state와 recovery proof. 아래 acknowledgement의
  completed-receipt correlation만 예외다.

Public output에 native-derived path나 OID가 표시되는 것은 가능하지만 다음 mutation input으로 되돌아와
권한이 되면 안 된다. `snapshotId`, `fileId`, `writeGeneration`도 단독 authority가 아니라 exact binding과
live repository evidence를 다시 확인하기 위한 짧은 수명의 concurrency coordinate다.

## 5. 권장 public Tauri 계약

아래 shape를 먼저 Rust fixture와 TypeScript strict schema에 고정한 뒤 Git write를 구현한다. 이름이나 필드가
달라져야 한다면 mutation code보다 contract/위협 모델을 먼저 함께 갱신한다.

### 5.1 Status

```text
code_thread_git_status({
  input: { scope, threadId }
}) ->
  {
    state: "ready",
    runtimeGeneration,
    statusRevision,
    writeGeneration,
    snapshotSequence,
    scope,
    threadId,
    snapshotId,
    headCommit,
    task: CodeGitChangeSet,
    staged: CodeGitChangeSet,
    unstaged: CodeGitChangeSet,
    hasConflicts,
    commitIdentity: { name, email } | null,
    capabilities: {
      stage: { enabled, reason: string | null },
      unstage: { enabled, reason: string | null },
      commit: { enabled, reason: string | null }
    },
    blockingReceipt: CodeGitMutationReceipt | null
  }
  | {
      state: "blocked",
      runtimeGeneration,
      statusRevision,
      writeGeneration,
      scope,
      threadId,
      reason,
      remediation
    }
  | {
      state: "recoveryRequired",
      runtimeGeneration,
      statusRevision,
      writeGeneration,
      scope,
      threadId,
      operation: {
        operationId,
        operation: "stage" | "unstage" | "commit",
        journalState: "pending" | "recovering" | "uncertain"
      }
    }
```

`CodeGitChangeSet`은 다음 strict shape다.

```text
{
  files: [{
    fileId,
    path,
    status,
    binary,
    additions,
    deletions,
    patch,
    truncated
  }],
  totalFiles,
  filesTruncated,
  additions,
  deletions
}
```

Status enum은 기존 `added | modified | deleted | typeChanged | unmerged | untracked`를 유지한다. Path는
display-only이고 mutation input에 다시 보내지 않는다. Ready write snapshot의 staged/unstaged manifest는
완전해야 하며 파일 cap을 넘거나 non-UTF-8/ambiguous path 또는 미모델링 index state가 있으면 file ID를
발급하지 않고 `blocked`를 반환한다. 따라서 ready staged/unstaged set의 `filesTruncated`는 항상 false다.
`task` patch와 file list는 기존 review cap을 유지할 수 있다. `enabled == true`이면 reason은 null이고 false면
plain-text reason이 non-empty여야 한다. Missing identity는 Commit만 막을 수 있지만 conflict, active turn,
approval, PTY 또는 completed-awaiting-ack는 세 action을 모두 막는다.

`blockingReceipt`는 Git mutation은 완료됐지만 frontend가 fresh ready snapshot을 확인하고 acknowledge하지 않은
경우에만 non-null이다. 이 상태에서도 snapshot을 반환해 UI가 결과를 검토할 수 있지만 모든 write capability는
false이고 다음 operation은 admission되지 않는다. Pending/recovering/uncertain journal은 새 file ID를 발급하지
않고 `recoveryRequired`를 반환한다. 완전한 write model 자체를 만들 수 없는 `blocked` 상태에서는 frontend가
기존 `code_thread_changes`를 한 번 읽어 read-only Task diff만 표시한다.

각 set 안의 path와 file ID는 unique하고 stable path order여야 한다. 부분 stage처럼 같은 logical path가
여러 set에 있으면 같은 snapshot 안에서 동일 file ID를 사용하고 서로 다른 path는 ID를 공유하지 않는다.
Set별 totals는 반환 files와 정확히 일치해야 한다.

`snapshotId`와 `fileId`는 path나 content에서 decode할 수 없는 native-issued random 256-bit lowercase hex로
권장한다. Native bounded/TTL cache는 exact `(scope, thread, runtime/write generation, binding/root/admin
identity, HEAD, index, worktree preimage, canonical commit-identity/config digest)`를 보존한다.
`writeGeneration`은 read epoch가 아니라 성공한
index/HEAD publish 또는 그 recovery가 완료될 때만 전진하는 mutation epoch다. 직전과 동일한
authoritative projection의 status refetch는 generation을 바꾸거나 기존 snapshot을 소모하지 않는다.

직전 authoritative projection과 같은 preimage의 status refetch는 기존 unexpired snapshot/ID를
재사용한다. 다른 preimage/capability/journal projection을 발견하면 같은 write generation 안에서도
`statusRevision`을 올리고, ready면 더 큰 `snapshotSequence`와 새 random snapshot/file ID를 발급하며
기존 snapshot을 mutation admission에서 소모한다. A -> B -> A로 돌아와도 과거 A의 revision/ID를
재사용하지 않는다. Consumed/history snapshot은 bounded TTL 동안 journal/exact-retry 판정용으로만
유지할 수 있다. Mutation 성공도 해당 binding의 이전 snapshot을 모두 소모한다.

`statusRevision`은 runtime generation 안의 binding-scoped monotonic total-order token이다. 단순 read로는
증가하지 않고 Git preimage 발견, journal pending/recovering/published/completed/ack/uncertain,
lifecycle/turn/approval/PTY 및 capability projection이 바뀌 때 증가한다. `runtimeGeneration`이 restart
epoch를 구분한다. Frontend는 exact scope/thread/runtime generation을 먼저 확인하고
`statusRevision`이 낮은 late response를 폐기한다. Ready 응답이 같은 revision이면
`(writeGeneration, snapshotSequence)`를 lexicographic으로 비교한다. 같은 preimage/projection은
직전 authoritative observation 이후 아무 상태 전환이 없을 때만 같은 revision, sequence,
snapshot/file ID를 재사용한다.
Process restart로 cache가 사라지면 Refresh를 요구한다.

### 5.2 Whole-file stage/unstage

```text
code_thread_git_stage({
  input: { scope, threadId, writeGeneration, snapshotId, fileId }
}) -> CodeGitIndexMutationReceipt

code_thread_git_unstage({
  input: { scope, threadId, writeGeneration, snapshotId, fileId }
}) -> CodeGitIndexMutationReceipt
```

- Stage는 exact snapshot의 `unstaged` row file ID만 받는다.
- Unstage는 exact snapshot의 `staged` row file ID만 받는다.
- 부분 stage된 파일을 Stage하면 검토한 current working version 전체가 index entry가 된다.
- Unstage는 persisted base가 아니라 current `HEAD` entry로 index만 되돌리고 working bytes를 보존한다.
- 한 command는 한 file ID만 처리한다. Batch와 Stage all은 후속 slice다.
- Empty, expired, consumed, wrong-lane, wrong-scope 또는 unknown ID는 zero mutation이다.

권장 receipt는 native-derived exact shape다.

```text
{
  operationId,
  operation: "stage" | "unstage",
  scope,
  threadId,
  requestGeneration,
  beforeSnapshotId,
  fileId,
  disposition: "staged" | "unstaged"
}
```

`operationId`는 output-only다. 동일 public input의 response-loss retry는 새 mutation이 아니라 journal의
같은 receipt를 회수한다.

### 5.3 Commit

```text
code_thread_git_commit({
  input: { scope, threadId, writeGeneration, snapshotId, message }
}) -> {
  operationId,
  operation: "commit",
  scope,
  threadId,
  requestGeneration,
  beforeSnapshotId,
  previousHead,
  commit,
  tree,
  disposition: "committed"
}
```

Commit message는 `message == message.trim()`인 non-empty UTF-8, NUL/CR 및 tab/LF 이외 control character
없음으로 검증한다. Canonical identity trailer를 붙인 최종 LF-normalized UTF-8 commit message 전체가 최대
64 KiB여야 한다. Frontend와 native가 같은 canonical LF text를 사용하며 native가 조용히 다른
subject/body로 바꾸지 않는다.

Commit identity는 caller가 보내지 않는다. Pinned repository의 local/worktree `user.name`과 `user.email`을
includes/global fallback 없이 각각 정확히 한 effective 값으로 읽는다. Name/email 모두 trim 전후가 같고
bounded non-empty/control-free여야 하며 `<`, `>`, CR/LF와 whitespace-containing email을 거부한다. Email은
정확히 하나의 `@`가 처음/끝이 아닌 위치에 있어야 한다. 없거나 duplicate/malformed면 commit을 막고 repository에
identity를 설정하라고 안내한다. Snapshot은 사용자가 본 identity digest를 결박하고 commit claim 직전에 exact
config 값이 같은지 다시 확인한다. 첫 slice는 author와 committer를 이 exact identity로 고정한다.

Repository의 managed-agent 규칙에 맞춰 native가 동일 identity의 canonical trailer를 다음 순서로 한 번만
붙인다.

```text
Co-authored-by: Human Name <human@email>
Signed-off-by: Human Name <human@email>
```

Caller message의 trailing trailer block을 case-insensitive로 parse해 `Co-authored-by` 또는 `Signed-off-by`가
이미 있으면 중복/identity spoofing을 피하기 위해 첫 slice에서는 거부한다. 다중 co-author 편집은 별도 기능이다.

### 5.4 Response-loss reconciliation

```text
code_thread_git_reconcile({
  input: { scope, threadId }
}) ->
  { state: "none", scope, threadId }
  | {
      state: "pending",
      scope,
      threadId,
      operationId,
      operation: "stage" | "unstage" | "commit"
    }
  | {
      state: "recovering",
      scope,
      threadId,
      operationId,
      operation: "stage" | "unstage" | "commit"
    }
  | { state: "completed", receipt: CodeGitMutationReceipt }
  | {
      state: "uncertain",
      scope,
      threadId,
      operationId,
      operation: "stage" | "unstage" | "commit",
      message
    }
```

```text
code_thread_git_acknowledge({
  input: {
    scope,
    threadId,
    operationId,
    writeGeneration,
    snapshotId
  }
}) -> {
  scope,
  threadId,
  operationId,
  disposition: "acknowledged"
}
```

Binding당 pending/recovering/completed-awaiting-ack/uncertain operation을 합쳐 정확히 하나만 blocking할 수 있으므로
`reconcile({scope,threadId})`가 모호하지 않다. `pending`은 원 invoke가 아직 실행 중이므로 기다린 뒤 다시
확인하며 새 mutation을 보내지 않는다. `recovering`은 startup/crash recovery가 native에서 증거를
판정 중이므로 같은 status/reconcile read만 bounded polling한다. `none`은 durable claim이 없어 mutation을
증명할 수 없다는 뜻이므로
fresh status 뒤 사용자가 명시적으로 다시 시도할 수 있지만 자동 retry하지 않는다. `completed` 뒤에는 ready
status를 새로 읽고 receipt와 일치하는 post-state를 확인한다.

Completed record는 acknowledge 전까지 다음 operation을 막는다. Acknowledge는 exact completed operation ID와
그 뒤에 읽은 ready snapshot generation/ID를 검증한 뒤 canonical original mutation input digest,
receipt와 ack coordinate를 담은 `acknowledged` tombstone을 durable sync한다. 그 뒤에만 journal blocker를
해제하며 Git byte를 쓰지 않는다. Tombstone은 ack response loss의 exact ack retry와 늦게 도착한
same-input mutation retry 모두를 같은 receipt로 수렴시킨다. 이 한 command에서만
caller가 native operation ID를 correlation으로 되돌려 보내며, Git write authority로 사용하지 않는다.
UI에서 응답을 못 받은 `outcomeUnknown`과 native가 before/expected-after 어느 쪽도 증명하지 못한 durable
`uncertain`을 구분한다.

새 여섯 command는 기존 public command 순서를 바꾸지 않고 contract에 append한다. 현재 Codex schema archive와
wire fixture에는 Git method를 추가하지 않는다. 이 기능은 app-server RPC가 아니라 native-only Git surface다.

## 6. Native admission과 lock 순서

새 `GitWriteActivityClearance`를 별도로 만든다. Safe-remove의
`RemovalActivityClearance` 구현 패턴은 참고할 수 있지만 capability, proof, receipt와 journal type은 재사용하지
않는다.

Mutation admission은 다음을 모두 증명한다.

1. App-wide binding mutex를 잡고 exact scope/thread의 persisted binding을 다시 읽는다.
2. Lifecycle가 stable `active`이고 preparation/removal/fork/archive transition이 없음을 증명한다.
3. `executionMode == worktree`, worktree ID 존재와 detached `HEAD`를 증명한다.
4. `runtime.ensure_thread_idle`로 authoritative quiescent 상태를 읽는다.
5. Active/starting/uncertain turn과 pending/reserved approval이 없음을 확인한다.
6. Exact scope/thread의 PTY owner가 없음을 확인한다.
7. Runtime idle admission guard를 잡고 PTY absence를 다시 확인한다.
8. Repository-identity Git write mutex를 잡고 root/admin/common-dir/index/HEAD를 pin/revalidate한다.
9. Snapshot generation과 exact selected file preimage를 재검증한다.
10. Journal claim부터 Git publish와 receipt sync까지 guard를 유지한다.

기존 PTY는 묵시적으로 닫지 않는다. 사용자 shell 작업을 보호하기 위해 명확한 오류와 Close terminal 후 Retry를
제공한다. Git write pending/recovering/completed-awaiting-ack/uncertain journal은 turn start/steer, PTY open,
archive/fork/remove와 다른 Git write를
반대로 막아 frozen evidence가 변하지 않게 한다.

Lock matrix는 `binding -> ensure_thread_idle RPC -> 첫 PTY absence proof -> retained
runtime/events/approvals admission -> 두 번째 PTY absence proof -> repository Git-write -> journal -> index ->
detached HEAD` 순서로 고정한다. 반대편 turn/start, PTY open, fork/archive/remove도 binding/Git-journal gate를
같은 순서로 확인한다. Long-running Git process와 dispatcher/event lock의 deadlock을 별도 fault/concurrency
test로 검증한다.

## 7. Pinned Git write engine

현재 `CodePinnedReadCommand`에 arbitrary mutation args를 추가하지 않는다. 별도 closed typed helper와 sealed
request enum을 만들고 caller-controlled `GIT_INDEX_FILE`, path, argv 또는 config가 도달하지 않게 한다.

### 7.1 공통 환경

- Pinned Git executable과 pinned worktree/admin/common-dir handles를 유지한다.
- `env_clear` 뒤 필요한 OS locale/temp/path만 전달하고 auth/Nostr secret은 넣지 않는다.
- `GIT_NO_REPLACE_OBJECTS=1`, no graft, no lazy fetch와 no network를 유지한다.
- Global/system config, credential helper, fsmonitor, pager, editor와 prompt를 끈다.
- Repository hook은 절대 실행하지 않고 signing도 끈다.
- External diff/textconv와 local/worktree clean/smudge/process filter를 전부 발견해 실행을 막는다. Selected
  path에 filter/text/eol/ident/working-tree-encoding 변환이 있거나 local `core.autocrlf`가 raw-byte staging과
  다르면 첫 slice에서는 stage를 거부한다.
- Loose-files ref backend와 primary local object database만 허용하고 reftable, alternates/shared object
  storage를 거부한다.
- 지원 Git에서 `core.fsync=added,reference`, `core.fsyncMethod=fsync`를 고정하고 실제 durability를 별도
  fsync 검증으로 확인한다.
- Literal native-resolved path만 쓰며 option/pathspec injection을 허용하지 않는다.
- Timeout, process group cleanup과 bounded stdout/stderr를 적용한다.
- 기존 `index.lock`은 busy로 반환하고 삭제하거나 steal하지 않는다.

Generic `project_git_exec::run_git`의 process/timeout 아이디어는 재사용할 수 있지만 public/generic argv runner를
Code write authority로 사용하지 않는다. Remote credential 경로도 호출하지 않는다.

### 7.2 Stage/unstage transaction

Porcelain `git add`/`git restore`를 live index에 바로 실행하지 않는다.

1. Exact linked-worktree HEAD, index digest/entries와 selected path의 mode/type/content bytes를 pinned handle에서
   한 번 읽어 digest와 함께 frozen preimage로 보존한다.
2. Git object database나 live admin directory를 쓰지 않는 app-private candidate index를 만든다.
3. Stage는 frozen exact regular/deletion/binary/empty/executable bytes의 blob OID를 filter 없이 계산하고 typed
   `update-index --info-only --index-info -z` 의미로 candidate에 반영한다. Unstage는 current `HEAD`의 exact
   entry를 candidate에 반영하며 worktree는 쓰지 않는다.
4. 선택하지 않은 stage-0 entry의 path/mode/OID가 semantic하게 동일하고 candidate digest가 예상과 같은지
   검증한다. Optional cache extension의 byte 보존은 약속하지 않으며 split-index/sharedindex, sparse index와
   지원하지 않는 required extension은 candidate 생성 전에 거부한다.
5. Before/expected-after digest, candidate digest와 exact admin directory 안에 만들 random index/HEAD guard
   artifact name을 `prepared` journal에 sync한다. 여기까지 live Git object/index/ref mutation은 0회다.
6. Prepared claim 뒤에만 frozen bytes를 stdin으로 넘겨 필요한 blob을
   `hash-object -w --stdin --no-filters`로 object database에 넣고 반환 OID가 candidate OID와 같은지 확인한다.
   Source path를 다시 읽지 않는다.
7. Exact admin directory에 unique index artifact와 HEAD guard artifact를 no-follow/O_EXCL로 만들고 fsync한 뒤
   admin directory도 fsync한다. Device/inode/owner/mode/current link-count 1/content digest와 pinned-parent
   identity를 journal에 추가 sync한다.
8. 아직 `index.lock`이 없을 때만 index artifact를 no-replace hard link로 `index.lock`에 만들고 admin
   directory를 fsync한다. 다음으로 HEAD guard를 같은 방식으로 `HEAD.lock`에 만들고 다시 directory를
   fsync한다. 두 artifact/standard-lock 쌍을 다시 stat해 exact same inode와 link-count 2를 증명한 뒤
   journal을 `lockArtifactsReady`로 sync한다. 둘 중 foreign lock이 있으면 known-owned link만 exact
   unlink/fsync하고 live index/HEAD는 쓰지 않는다.
9. 두 lock 아래 live index가 before digest, detached HEAD가 frozen OID와 같음을 다시 확인한다.
10. Known-owned `index.lock`을 index로 atomic rename하고 admin directory를 fsync한 뒤 journal을
    `indexPublished`로 sync한다. HEAD는 쓰지 않고 guard만 유지한다.
11. Durable `indexPublished`를 publish 성공의 linearization proof로 삼아 receipt를 journal의
    `completedAwaitingAck`와 함께 durable sync한 뒤에만 반환한다. Fresh index/HEAD drift는
    성공한 publish를 다시 실패로 분류하지 않고 다음 authoritative status에 반영한다. Known-owned
    `HEAD.lock`과 남은
    artifact를 exact unlink하고 매 directory mutation을 fsync한다. 새 status coordinate는 별도 authoritative
    status read가 발급한다.

표준 `index.lock`/`HEAD.lock` 이름은 exact artifact identity가 journal에 durable해진 뒤에만 만든다. 따라서
crash 뒤 blocking lock을 지우는 경우에도 journal의 pinned parent/device/inode/owner/mode/link-count/digest와
hard-link identity가 모두 맞아야 한다.
Foreign/stale lock은 절대 삭제하지 않는다. Prepared claim 뒤 object database에 남은 unreachable blob은 crash
시 허용되는 side effect지만 index/ref나 working tree가 부분적으로 바뀌면 안 된다.

Stage/unstage recovery는 journal state와 exact live evidence로만 닫는다.

- Journal의 durable `indexPublished`는 native publish가 이미 성공했다는 증거이므로 같은 receipt를
  `completedAwaitingAck`로 finalize한다. 이후 external drift는 새 status에서 별도로 보여준다.
- `indexPublished` sync 전이라도 live index가 expected-after digest이고 HEAD/root가 claim과 같으면
  atomic publish를 증명해 같은 receipt를 finalize한다.
- Live index가 before digest, HEAD/root가 claim과 같고 exact known-owned lock/artifact evidence가 남아 있으면
  같은 candidate publish를 한 번만 재개한다.
- `indexPublished` sync 전 다른 index/HEAD/root evidence는 sticky `uncertain`이며 추가
  index/object mutation은 0회다.

`index.lock -> index` rename은 standard index lock 이름을 소비하는 linearization point다. 그 뒤는
외부 Git writer를 계속 lock했다고 가정하지 않고 journal을 즉시 sync한다. Publish와 journal sync
사이 crash/drift는 위 before/expected-after/third-state evidence로만 판정한다.

### 7.3 Commit transaction

첫 slice는 porcelain `git commit`을 재실행하지 않는다.

1. Stable index가 non-empty staged delta를 갖고 conflict/operation marker가 없는지 확인한다.
2. Native identity, canonical message/trailers, author/committer timestamp, previous HEAD와 index digest를
   `prepared` claim에 고정하고 fsync한다. 이 전에는 tree/commit/ref object를 쓰지 않는다.
3. Claim에 결박된 app-private frozen index copy를 native-only `GIT_INDEX_FILE`로 사용해 `write-tree`를
   실행하고 exact tree OID를 journal에 sync한다. Live index에는 이 command를 실행하지 않는다.
4. `commit-tree <tree> -p <previousHead>`로 exact single-parent, frozen identity/timestamp/message의
   hook/signing/editor 없는 commit object를 만들고 expected commit OID를 journal에 fsync한다.
5. Stage transaction과 같은 proven-owned artifact 방식으로 standard `index.lock`, 그 다음 expected commit
   OID와 newline을 담은 `HEAD.lock`을 획득하고 각각 admin directory를 fsync한다. `write-tree`는 standard
   index lock을 스스로 만들 수 있으므로 반드시 그 전에 private candidate에서 끝내야 한다. 두
   artifact/standard-lock 쌍의 exact inode/link-count 2를 재검증하고 journal을 `locksReady`로 sync한다.
6. 두 lock 아래 live index digest와 detached loose-file HEAD가 아직 claim과 같은지 다시 확인한다.
7. Known-owned `HEAD.lock`을 HEAD로 atomic rename해 `previousHead -> expectedCommit`을 compare-and-swap하고
   admin directory를 fsync한다. 첫 slice는 `update-ref`를 호출하거나 `logs/HEAD` reflog를 만들/수정하지
   않으므로 reference-transaction hook과 reflog partial-write 경계를 열지 않는다.
8. HEAD durability를 확인하고 journal을 `headPublished`로 sync한다. Receipt를
   `completedAwaitingAck`와 함께 durable sync한 뒤에만 반환하고 exact owned index lock/artifact를
   해제한 후 directory를 fsync한다.

Recovery는 다음 규칙만 사용한다.

- Journal의 durable `headPublished`는 native HEAD CAS가 이미 성공했다는 증거이므로 같은 receipt를
  `completedAwaitingAck`로 finalize한다. 이후 external HEAD drift는 새 status에서 별도로 보여준다.
- `headPublished` sync 전 `HEAD == expectedCommit`이고 tree/index/root evidence가 claim과 같다:
  atomic CAS를 증명해 같은 receipt로 finalize한다.
- `HEAD == previousHead`이고 tree/index/root evidence가 claim과 같다: 같은 expected commit CAS를 한 번 재개한다.
- Publish sync 전 다른 HEAD/root/index evidence: sticky `uncertain`, 추가 ref/object mutation 0회다.

Index/HEAD guard는 final live evidence 재검증부터 각자의 atomic publish까지만 직렬화를
증명한다. `HEAD.lock -> HEAD` rename은 HEAD lock 이름을 소비하는 linearization point이며, 이후
journal을 즉시 sync한다. Publish부터 receipt sync 사이 crash는 위 recovery tri-state로 닫고
외부 writer의 후속 drift를 계속 lock한다고 가정하지 않는다. Object 생성 중 drift가 생기면
ref/index를 publish하지 않고 orphan object만 남긴 채 stale/uncertain 규칙으로 닫는다. Commit 뒤 index는
새 HEAD tree와 같으므로 staged set은 비고, unstaged working bytes는 그대로 남아야 한다. 응답 유실 뒤
`git commit`을 다시 실행해 두 번째 commit을 만들지 않는다. Prepared claim 뒤 crash로 생긴
unreachable blob/tree/commit object는 visible index/HEAD publish 전 허용되는 유일한 Git side effect다.

## 8. 첫 slice에서 거부할 Git 상태

다음 상태는 read-only Changes는 계속 보여줄 수 있지만 Git write file ID를 발급하지 않거나 mutation을
zero-mutation 거부한다.

- Unmerged/non-stage-0 index entry와 conflict
- Merge/rebase/cherry-pick/revert/bisect/sequencer 진행 marker
- Attached branch, unborn HEAD와 local checkout
- Submodule/gitlink 변경과 nested repository
- Split/shared/sparse index, unsupported required index extension, skip-worktree, assume-unchanged와
  intent-to-add/zero OID
- Symlink와 아직 정확히 모델링하지 않은 type change/special file
- Selected path의 filter/text/eol/ident/working-tree-encoding 변환과 incompatible local `core.autocrlf`
- Reftable/unknown ref backend와 alternate/shared object database
- Ignored file, non-UTF-8/control/oversize path와 case/normalization collision
- Incomplete/overflow manifest 또는 unsafe patch inventory drift
- Root, `.git`, common-dir, admin-dir, index, HEAD 또는 Git executable replacement
- Existing/foreign index 또는 HEAD lock
- Missing/invalid repository-local commit identity는 commit만 차단
- Any pending/recovering/completed-awaiting-ack/uncertain Git-operation 또는 safe-remove journal

Rename/copy detection은 끄고 delete + add로 표시한다. Untracked directory 자체는 mutation unit이 아니며 bounded
complete inventory에서 개별 regular file로 펼친다. `-f`, reset, clean과 worktree checkout은 사용하지 않는다.

현재 exact fd-pinning boundary는 Unix 전용이다. 동등한 pinned index/ref CAS가 생기기 전까지 macOS/Linux만
capability-enabled이고 Windows는 pathname fallback 없이 deterministic unsupported + zero mutation으로 둔다.

## 9. Git-operation journal

Binding v4, model preference와 safe-remove journal/receipt를 재사용하지 않는다. 별도 versioned private store,
예를 들어 `appData/code/git-operations.json`, 그리고 필요할 때만 private candidate-index sidecar를 사용한다.

- Directory 0700, file 0600과 owner/regular-file/symlink/size/count 검증
- Temp file, file fsync, atomic replace와 parent-directory fsync
- Scope/thread/operation kind와 canonical full-input digest 결박
- Binding/root/admin identity, before/expected-after index 또는 HEAD evidence 저장
- Bounded message/identity/timestamp와 native operation ID 저장
- Path, source content, removal proof/receipt와 model selection은 public receipt에 넣지 않음
- Binding당 pending/recovering/completed-awaiting-ack/uncertain operation을 합쳐 최대 1개
- Acknowledged operation tombstone은 original mutation input digest+receipt와 ack coordinate를 보존하는
  exact mutation/ack retry용 bounded history

Index journal의 최소 상태는
`prepared -> lockArtifactsReady -> indexPublished -> completedAwaitingAck -> acknowledged | uncertain`, commit은
`prepared -> objectWritten -> locksReady -> headPublished -> completedAwaitingAck -> acknowledged | uncertain`이다.
같은 exact input retry는 journal을
snapshot expiry/consumption 검사보다 journal에서 먼저 조회할 뿐 새 mutation을 시작하지 않는다. Payload가
다른 coordinate 재사용은 conflict다.

Startup은 어떤 filesystem/Git recovery보다 먼저 safe-remove와 Git-operation journal을 모두 strict load하고
exact binding으로 cross-join한다. 같은 binding에 둘 다 있으면 어느 recovery도 mutation하지 않고 fail
closed한다. 이 공동 preflight를 통과한 서로 다른 binding에 대해서만 기존 safe-remove recovery를 먼저,
Git-operation recovery를 그 다음 수행한 뒤 runtime/lifecycle reconciliation을 시작한다.

## 10. Frontend UX와 상태

Changes inspector는 다음 두 view를 제공한다.

- `Working changes`: `Staged changes`와 `Unstaged changes` section, whole-file actions와 commit trigger
- `Task diff`: 기존 persisted-base 대비 전체 작업 diff. Commit 뒤에도 남는다.

첫 slice에는 row별 `Stage <full path>` / `Unstage <full path>`만 제공하고 Stage all은 넣지 않는다. Commit은
status가 `state:"ready"`이고 `capabilities.commit.enabled == true`일 때만 `Commit staged changes` dialog를 연다.
자동 stage는 하지 않는다. Dialog에는 native가 읽은 author identity와 commit에 canonical
`Co-authored-by`/`Signed-off-by`가 추가된다는 사실, staged/binary/patch-truncated file 수와
detached local target을 submit 전에 명시한다. 사용자의 Commit action이 이 sign-off와 whole-file 범위에
대한 명시적 확인이어야 한다.

상태 규칙은 다음과 같다.

- Stage/Unstage는 사용자가 본 `writeGeneration + snapshotId + fileId`, Commit은 같은 generation/snapshot과
  message만 보낸다.
- Pending 동안 같은 binding의 Git controls와 composer start/steer를 잠근다.
- Cache의 row를 optimistic하게 lane 사이로 옮기지 않는다.
- Active managed ready 상태에서는 `status.task`를 Task diff에 사용하고 동시에 legacy
  `code_thread_changes`를 읽어 merge하지 않는다. Archived/local 또는 status `blocked/recoveryRequired` fallback에서만
  legacy command를 한 번 읽는다. Shared renderer adapter는 file ID를 제거하고 `{...task,
  commitBody:null}`을 보충한다.
- Initial status transport failure나 strict schema decode error는 tagged `blocked`로 위조하지 않는다. Inline
  `role=alert` + Retry를 보여주고 write를 fail closed한 채 legacy `code_thread_changes`를 read-only로
  정확히 한 번 fallback한다. Legacy read도 실패하면 두 오류를 구분해 보여준다.
- Success receipt 뒤 QueryClient dedupe 밖에서 exact status를 직접 읽는다. Exact scope/thread/runtime,
  pre-mutation보다 큰 `statusRevision`, receipt의 request generation보다 전진한 write generation과
  현재 cache보다 낮지 않은 snapshot sequence를 모두 strict 검증한 뒤에만 cache를 교체한다.
- Receipt 뒤 status read 실패는 mutation 실패가 아니라 `completed-awaiting-refresh`로 표시한다. 이전 rows를
  보여주더라도 Git controls와 composer/start/steer는 계속 fail closed하고 mutation을 재호출하지 않는다.
- Ready post-status가 `blockingReceipt`로 같은 receipt를 확인한 뒤 acknowledgement를 보내며, ack 완료 전에는
  native와 UI 모두 다음 operation을 허용하지 않는다. Ack 성공 뒤에도 direct status를 한 번 더
  읽어 더 큰 `statusRevision`과 `blockingReceipt:null`을 확인한 뒤에만 attempt를 clear한다.
  각 action은 이 새 status의 capability를 그대로 따라 enable/disable하며, capability가 하나도
  활성화되지 않아도 ack된 attempt를 남기지 않는다. Ack response loss나 post-ack status 실패는
  attempt/blocker를 보존하고 mutation을 재호출하지 않는다.
- Timeout/response loss는 attempt를 scope+thread 기준 `gcTime: Infinity`로 보존하고 자동 mutation retry하지 않는다.
- Runtime generation change로 unknown attempt를 지우지 않는다. Native `reconcile`의 pending/recovering은
  기다리고, none은 “적용을 확인하지 못함”으로 안내한 뒤 fresh status만 읽는다.
  None attempt는 fresh status 성공 뒤에만 clear하고 status 실패에서는 blocker를 유지한다. Completed는
  status+ack+status, uncertain은 sticky blocker로 수렴한다.
- Native `uncertain`은 sticky inline blocker로 표시하고 새 turn/PTY/lifecycle/Git write를 모두 막는다.
- Archived/local/unsupported platform은 기존 Task diff를 read-only로 유지한다.

Commit dialog는 실패나 outcome-unknown 때 message를 보존한다. Commit success 뒤 staged rows가 사라져도
unstaged rows와 Task diff가 보존되는지 확인한다. 최대 250-file complete manifest는 action lane에서
기존 `shared/ui/VirtualizedList.tsx`를 우선 재사용·확장해 windowing하고 patch body는 expand할 때만
mount한다. Focused pending row는 range/overscan에 pin해 scroll로 unmount되지 않게 한다.
Native cap을 넘으면
`blocked` + legacy Task fallback이며 일부만 action 가능하게 만들지 않는다. Branch/push/PR CTA는 이 slice에서
만들지 않는다.

## 11. 접근성·focus 계약

- Staged/Unstaged는 각각 `section aria-labelledby`와 이름이 있는 list로 구성한다.
- Diff row의 open/select button과 Stage/Unstage button은 sibling이다. Button 안에 button을 중첩하지 않는다.
- 화면 path가 truncate돼도 full path를 accessible name과 title에 제공한다.
- Disabled row는 title-only가 아니라 인접 plain-text reason을 제공한다.
- 부분 stage된 같은 path에는 두 lane 모두 `Partially staged`를 표시한다.
- Mutation region은 `aria-busy`, 진행/완료는 하나의 polite live region, failure/unknown은 inline `role=alert`다.
- Commit은 Radix `Dialog`와 semantic form을 사용한다. Textarea에는 visible label, `id`,
  `name="commit-message"`, `autoComplete="off"`, hint/error `aria-describedby`를 연결한다.
- Submit 전 Invalid/Cancel/Escape는 textarea 또는 connected trigger로 정상 focus를 복원한다. Mutation pending
  동안은 dialog Escape, outside interaction, 기본 close X와 Cancel을 막고 dialog를 유지한다.
- Outcome-unknown으로 전환되면 dialog를 닫을 수 있지만 message/attempt와 inspector의 persistent
  `Check operation status` button은 보존한다. Dialog portal event가 bubble해도 inspector의 Escape handler가
  함께 panel을 닫지 않도록 dialog-open/defaultPrevented guard와 stopPropagation을 둔다.
- 원래 Stage trigger가 계속 focus였으면 authoritative refresh 뒤 같은 path의 Unstage trigger로 focus를 옮긴다.
  Virtualized opposite-lane row가 offscreen이면 `scrollToIndex -> mount -> focus`를 수행하고, 대상을
  증명하지 못하면 Changes heading으로 fallback한다. 사용자가 다른 곳으로 이동했다면 focus를
  빼앗지 않는다.
- Commit 뒤 대상 row가 없어지면 Commit trigger, Changes heading, Refresh 순서로 fallback한다. Programmatic
  focus를 받는 Changes/blocker heading은 `tabIndex={-1}`을 가진다.
- Pending 중 focused control을 unmount하지 않고 disabled/busy 상태를 명확히 알린다.
- Reconcile button은 pending/recovering이면 focus를 유지한다. `none`이면 Refresh/heading,
  `completed`이면 status(blocking)+ack+status(ack-cleared) 뒤 새 capability가 허용하는
  opposite-lane action 또는 Commit/heading fallback, `uncertain`이면 persistent blocker heading으로 이동한다.

## 12. 구현 파일 전략

기존 대형 파일을 더 키우지 않는다.

Native 권장 구조:

- 새 `desktop/src-tauri/src/code_workspace/git_write/` 아래 protocol, snapshot, candidate-index, journal,
  pinned mutation, recovery와 tests 분리
- 새 `desktop/src-tauri/src/commands/code_git_handoff.rs`
- `code_workspace/mod.rs`, `commands/mod.rs`, `app_state.rs`, `lib.rs`에는 최소 registration/state wiring만 추가
- `project_git_diff.rs`에서는 필요하면 pure parser/collector leaf만 추출하고 기존 Changes 계약은 바꾸지 않음
- 새 `code_workspace/fixtures/git-write-gates-v1.json`
- `tauri-contract-v1.json`과 `contract_tests.rs`에 strict command/receipt sentinel 추가

`protocol.rs`, `commands/code_workspace.rs`, `worktrees.rs`는 이미 1,000줄을 크게 넘는다. 새 Git write engine을
그 파일들에 직접 누적하지 않는다.

Frontend 권장 구조:

- 새 `desktop/src/features/code/api/codeGitTypes.ts`
- 새 `desktop/src/features/code/api/codeGitSchemas.ts`
- 새 `desktop/src/features/code/api/codeGitWorkspace.ts`
- `codeWorkspace.ts`의 frozen command 배열에는 새 command만 append하고 adapter는 compose
- `codeSessionQueries.ts`에는 generation-bound status query와 scope/thread-bound attempt state 추가
- 새 `state/useCodeGitHandoff.ts`
- 새 `ui/CodeGitChangesActions.tsx`, `ui/CodeCommitDialog.tsx`
- `CodeChangesPanel.tsx`는 query/header shell과 두 view 조립만 담당
- `codeWorkspaceView.ts`에 `canReadChanges`와 별도인 `canWriteChanges` 추가
- `CodeComposer.tsx`의 local submit-pending을 상위 Git-operation gate와 연결
- `useCodeChangesInvalidation.ts`에서 Codex file events가 새 status도 invalidate하도록 확장

`schemas.ts`는 977줄, `CodeWorkspaceScreen.tsx`는 994줄, `codeWorkspace.ts`는 약 900줄이고 shared
`ProjectPullRequestFilesChangedPanel.tsx`도 1,000줄을 넘는다. 이 파일들에 큰 DTO/controller/UI를 직접 추가하지
않는다. 특히 Screen은 6줄 여유뿐이므로 composer/Git gate wiring 전에 orchestration을 별도 hook/component로
추출해 실제 line count를 줄이는 것을 선행한다. Shared diff renderer에는 nested action button을 넣지 말고 leaf를
추출하거나 Code 전용 sibling action wrapper를 사용한다.

## 13. 테스트 완료 조건

### 13.1 Contract/fixture

- 기존 command 순서/shape 불변 + 새 여섯 command exact `{input}` 등록
- Rust `deny_unknown_fields`, strict Zod와 exact key-count 교차검증
- Git mutation input의 path/cwd/ref/OID/operationId/argv/identity/timestamp/force unknown field 거부와
  acknowledgement에서만 exact operation ID 허용
- Status set별 totals/unique/sort와 partial-stage cross-set duplicate 규칙
- Random opaque ID가 path/model/OID와 구별되고 wrong scope/lane/generation에서 무효
- Existing `code_thread_changes`, fork/model/remove fixture no-change sentinel

### 13.2 Native real-Git/fault tests

- Unstaged, staged, both/partial, untracked, deleted, binary, empty와 executable 분류
- Stage한 한 파일만 index에 들어가고 선택하지 않은 stage-0 entry의 path/mode/OID는 semantic하게 동일하며
  worktree/HEAD/sibling repo bytes는 보존
- Unstage한 한 파일만 current HEAD로 돌아가고 working bytes와 immutable base는 보존
- Commit이 staged tree만 정확히 한 번 commit하고 unstaged edit를 보존
- Hook/filter/editor/signing/network helper가 한 번도 실행되지 않음
- Missing identity, conflict, operation marker, local/archive/attached HEAD와 unsupported type zero mutation
- Active/starting/uncertain turn, approval, PTY, removal/fork/archive race zero mutation
- Root/`.git`/common-dir/admin/index/HEAD/Git executable replacement와 foreign `index.lock`/`HEAD.lock` 거부
- Owned index/HEAD lock artifact의 file/admin-directory fsync, nlink 1 -> 2 state sync, exact-inode recovery와
  foreign lock 미삭제
- Loose detached `HEAD` CAS가 reflog/reference-transaction hook를 쓰지 않고 reftable/alternates를 거부
- Stale/expired/consumed/cross-scope file ID와 generation ABA zero mutation
- Claim 전, claim sync 뒤, artifact/standard-lock link sync 전/뒤, index publish 뒤, object write 뒤,
  HEAD CAS 전/뒤, `headPublished`/receipt sync 전/뒤 fault injection
- Durable `indexPublished`/`headPublished` 뒤 external index/HEAD drift에서 same receipt + fresh status 수렴
- Recovery 때 before/expected-after exact 수렴과 third-state sticky uncertain/no second mutation
- Unsupported platform pathname fallback 0회

### 13.3 Frontend/unit/component

- Generation/statusRevision별 status cache 격리와 late ready/blocked/recovery response 폐기
- 직전과 동일한 authoritative projection의 status refetch는 `writeGeneration`을 전진시키거나
  기존 unexpired snapshot/file ID를 소모하지 않음
- `blocked`/`recoveryRequired`는 file ID를 발급하지 않고 ready capability reason은 strict shape로 검증
- Attempt state는 runtime generation 변경과 panel/sidebar remount 뒤에도 보존
- No optimistic lane/cache update와 success receipt 뒤 direct authoritative read
- `completed + refresh failure`와 mutation failure 문구 구분
- Completed receipt의 status 확인 + acknowledgement 전 다음 mutation 차단, ack response-loss idempotency와
  ack 후 direct status 확인 전 attempt fail-closed; ack-cleared 후는 새 capability를 그대로 적용
- Ready active에서 legacy `code_thread_changes` 0회, blocked/recovery/local fallback에서 exact 1회 +
  write action 0회
- Status transport/schema error의 inline alert+Retry, legacy read-only fallback exact 1회와 write action 0회
- Reconcile `pending/recovering`은 mutation/ack 0회, `none`은 fresh status 성공 뒤에만 attempt clear,
  fresh status 실패는 blocker 유지, `uncertain`은 sticky blocker
- Partial staged path가 양 lane에 보이고 Task diff가 commit 뒤 유지
- Semantic section/list, exact accessible action name와 inline disabled reason
- Commit label/message validation, Cancel command 0회와 failure/unknown message 보존
- Stage -> Unstage, dialog/commit/reconcile focus restoration과 dialog Escape의 inspector propagation 차단
- Virtual scroll이 focused pending row를 unmount하지 않고 offscreen opposite-lane focus는
  scroll-to-mount 또는 heading fallback

### 13.4 Desktop E2E

Fresh `pnpm build:e2e` 뒤 focused scenario 두 개를 최소로 둔다.

1. Keyboard로 opaque ID Stage -> Status(blocking) -> Ack -> Status(ack-cleared) -> Unstage -> Status(blocking) ->
   Ack -> Status(ack-cleared) -> Stage -> Status(blocking) -> Ack -> Status(ack-cleared) -> Commit ->
   Status(blocking) -> Ack -> Status(ack-cleared). Mutation
   payload에 path/ref/OID/operationId가 없고 unstaged 파일은 보존되며 detached HEAD는 한 번만 전진하고 push/PR
   command는 0회다.
2. Commit HEAD CAS 뒤 response loss -> outcome unknown -> inspector/sidebar remount -> explicit Reconcile -> 같은
   receipt -> Status(blocking) -> Ack -> Status(ack-cleared). Commit mutation과 HEAD 전진은 정확히 한 번이다.

기존 E2E의 “stage/commit/push/PR 모두 없음” sentinel은 stage/commit 부분만 교체하고 branch/push/PR 부재는
계속 검증한다. Screenshot을 추가한다면 repository 규칙대로 animation 완료와 distinct hash를 확인한다.

## 14. 보존할 frozen 경계

- `code_thread_changes` exact `{input:{scope,threadId}}`와 기존 combined 6-field output
- Public binding exact 8-field shape와 binding store v4
- Fork public `{input:{scope,threadId}}`와 pinned Codex five-key wire, model/effort 없음
- Ordinary resume/recovery의 `model:null`
- `code_models_list()`와 `code_model_selection_set({input})`의 catalog/selection shape
- Bound thread-open의 authoritative model + nullable reasoning effort
- `code_worktree_remove` exact `{input:{scope,threadId}}`, native-derived 9-field receipt
- Safe-remove `claimed -> removing -> removed`, merge proof, proof ref, quarantine와 transcript `preserved`
- `worktree-removal-gates-v1.json` byte/shape와 removal journal namespace
- Codex 0.145 schema manifest/archive/wire와 app-server method set

Git journal은 safe-remove와 상호 배타적으로 gate하지만 authority, receipt, operation ID와 storage를 공유하지
않는다. Model selector와 Git mutation도 결합하지 않는다.

## 15. 권장 검증 명령

작업 중 관련 범위부터 실행하고 마지막에 가능한 전체 gate를 확인한다.

```bash
. ./bin/activate-hermit

cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo check --manifest-path desktop/src-tauri/Cargo.toml --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib -- -D warnings
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::git_write --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests --lib -- --nocapture

pnpm --dir desktop typecheck
pnpm --dir desktop check:px-text
pnpm --dir desktop check:file-sizes
pnpm --dir desktop test
pnpm --dir desktop exec biome check \
  src/features/code src/testing/e2eBridge.ts tests/e2e/schoolx-code.spec.ts

git diff --check
```

4173 listener가 stale Playwright preview임을 첫 `lsof` 출력으로 확인한 뒤에만 다음 종료 루프를
실행한다. 확인 전에 listener를 자동 종료하지 않는다.

```bash
lsof -nP -iTCP:4173 -sTCP:LISTEN
```

출력의 listener가 stale Playwright preview임을 확인했다면 그때만 다음을 실행한다.

```bash
lsof -tiTCP:4173 -sTCP:LISTEN | while IFS= read -r phase3_preview_pid; do
  kill "$phase3_preview_pid"
done
pnpm --dir desktop build:e2e
pnpm --dir desktop exec playwright test tests/e2e/schoolx-code.spec.ts \
  --project=smoke --grep 'Git stage|Git commit response loss'
```

Repository 지침대로 preview를 종료하고 반드시 `build:e2e`를 새로 실행한다. 전체
`check:file-sizes`는 현재 dirty Phase 2 tree의 inherited oversized files 때문에 baseline 실패할 수 있다.
새 Phase 3 파일은 1,000줄 이하로 분리하고 기존 실패 목록을 늘리지 않는다.

## 16. 완료 정의

다음 조건이 모두 만족돼야 이 slice를 완료로 본다.

- Active managed detached binding에서 whole-file stage/unstage와 staged-only commit이 실제 Git으로 동작한다.
- Webview input에는 path/ref/OID/argv/identity가 없고 native exact evidence가 모든 권한을 도출한다.
- Runtime/approval/PTY/lifecycle/removal/Git write가 같은 binding에서 동시에 admission되지 않는다.
- Index publish와 commit HEAD CAS는 crash/response loss 뒤 same receipt 또는 sticky uncertain로 수렴한다.
- Duplicate commit과 partial live-index publish가 fault tests에서 발생하지 않는다.
- UI는 optimistic update 없이 pending/completed/outcomeUnknown/native uncertain을 구분한다.
- Commit 후 unstaged bytes와 persisted-base Task diff가 보존된다.
- Keyboard/focus/live-region 계약과 fresh-build E2E 두 개가 통과한다.
- Phase 2 model/fork/remove/binding/Codex fixture sentinel이 모두 통과한다.
- Existing dirty worktree의 unrelated 변경을 stage/commit/reset/clean하지 않는다.

## 17. 다음 세션 복사용 시작 요청

```text
SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md를 먼저 읽고,
SCHOOLX_CODE_DESIGN.md, SESSION_HANDOFF_20260819_CODE_PHASE2_MODEL_SELECTOR.md,
SESSION_HANDOFF_20260819_CODE_PHASE2_PUBLIC_REMOVAL.md와
SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md를 대조해줘.

첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘. 현재 shared dirty worktree의
tracked/untracked 사용자 변경을 보존하고 이 checkout 자체를 stage/commit/reset/clean하지 마.

Phase 3 첫 slice로 active managed detached bound thread의 whole-file stage/unstage와 staged-only commit을
구현해줘. 기존 code_thread_changes 계약은 그대로 두고 별도 code_thread_git_status에서 task/staged/unstaged
원자 snapshot과 monotonic statusRevision, opaque writeGeneration/snapshotId/fileId를 발급해줘.
Caller path/ref/OID/argv/identity를
열지 말고, native activity clearance, candidate-index atomic publish, 별도 Git-operation journal,
commit-tree + detached HEAD CAS와 explicit reconcile로 response loss를 exact-once에 수렴시켜줘.
Completed receipt는 fresh status로 확인한 뒤 explicit acknowledge하기 전까지 다음 mutation을 막고,
ack response loss도 같은 receipt로 idempotent하게 수렴시켜줘. Ack 뒤에도 fresh status로 blocker
해제를 확인한 뒤에만 다음 action을 열어줘.

Local/archive write, stage-all/hunk, branch/push/PR, Talk 공유와 hook/signing은 별도 slice로 남겨줘.
Phase 2 model selector, fork five-key wire, binding v4와 public safe-remove input/9-field receipt/journal을
변경하지 마. Contract/fault/unit/component와 fresh build Playwright를 통과할 때까지 이어서 진행해줘.
```

## 18. 구현 후속 인계

2026-08-21에 public contract와 Native/Frontend vertical slice에 이어 strict crash-boundary journal evidence,
owned-lock proof, startup safe-remove cross-preflight, fault/response-loss matrix와 frontend recovery closure까지
구현하고 전체 회귀를 통과했다. 최종 완료 상태와 미해결 app helper process-launch authority 신뢰 경계는
[`SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md`](SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md)에서
확인한다.
