# SchoolX Code worktree removal decision gates

상태: **authority/proof+journal+physical removal implemented, public surface open**
계약 버전: **1**
기준일: **2026-08-19**

이 문서는 managed worktree의 physical removal과 현재 public surface가 함께 지켜야 하는 네
decision gate의 normative 계약이다. Machine-readable mirror는
`desktop/src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json`이다.

현재 production은 네 gate의 native 구현과 exact public safe-remove 수직 슬라이스를 함께 열었다.

- Binding store v4는 native-only `mergeTargets`와 strict `claimed | removing | removed` removal records를
  저장한다. Legacy V1/V2/V3는 removal authority를 추론하지 않고 empty v4 namespace로 migration한다.
- Pure store CAS, definitely-not-started cancellation, sticky `removing`, atomic tombstone swap과 identity
  non-reuse가 구현됐다.
- Linux/macOS private engine은 digest-addressed strict manifest sidecar, reciprocal Git-admin proof,
  sealed claim/verified-absence authority, proof ref, no-replace quarantine와 handle-relative no-follow
  deletion을 구현한다. Startup은 pending journal을 다른 reconciliation보다 먼저 recovery한다.
- `code_worktree_remove`는 exact top-level `{input}`만 받고 public input을 `{scope, threadId}`로 제한한다.
  Public merge-proof command는 없으며 path/ref/OID/proof/removal ID는 계속 native-only다.
- Inventory input은 exact `{scope}`이고 모든 row는 `preserved: true`다. Native positive merge proof까지
  통과해 blocker가 하나도 없는 stable Archived binding만 `canRemove: true`다.
- Frontend는 eligible row에만 explicit confirmation action을 렌더링한다. Receipt 전후 모두 optimistic
  row removal을 하지 않고 authoritative inventory+thread list가 target absence를 확인해야 cache를 교체한다.
- 이 문서는 inventory 결과를 deletion receipt로 승격하지 않는다.

동결 fixture는 호환성을 위해 version 1과 `futureSurface`/`futureReceipt` key 이름을 유지하지만, 현재
status는 exact `authorityProofJournalPhysicalRemovalImplementedPublicSurfaceOpen`이며 surface flags는
모두 true다.

## 1. 현재 public surface의 exact boundary

첫 safe-remove mutation의 input은 다음 exact shape만 허용한다.

```text
CodeWorktreeRemoveInput {
  scope: CodeThreadBindingScope
  threadId: string
}
```

Top-level argument는 exact `input` 하나다. Frontend는 path, descriptor, worktree ID, base/HEAD
commit, target ref/commit, lifecycle, blocker, `canRemove`, merge proof, force flag, request/removal ID를
보낼 수 없다. Native가 `(scope, threadId)`의 단 하나의 live removal record 또는 tombstone을 찾아
retry하고 canonical UUID `removalId`를 발급한다.

성공 receipt는 native-derived 다음 필드만 가진다.

```text
removalId
scope
threadId
worktreeId
headCommit
mergedIntoRef
mergedIntoCommit
transcriptDisposition: "preserved"
executionDisposition: "removed"
```

이 input/output DTO, Tauri command registration, strict frontend adapter, mock bridge와 confirmation UI가
같은 수직 슬라이스로 production code에 추가됐다. Input과 receipt는 unknown field를 거부한다. Receipt의
`removalId`는 canonical lowercase UUID v4이고 worktree ID, Git OID와 direct-local ref도 native contract와
같은 형식으로 frontend에서 다시 검증한다.

같은 `(scope, threadId)`의 in-flight/retry와 native commit 뒤 response loss는 저장된 journal/tombstone에
합류해 exact 같은 9-field receipt로 수렴한다. Caller는 recovery를 위해 새 operation ID나 proof를 만들지
않으며 같은 두 필드만 다시 보낸다.

### 1.1 Confirmation과 authoritative reconciliation

- UI는 `canRemove: true`인 stable Archived binding row에만 thread-specific remove action을 보인다.
- Confirmation dialog는 native-derived execution path를 정보로 보여 주되 command input에 전달하지 않는다.
  Cancel은 command를 호출하지 않고 원래 trigger로 focus를 돌린다.
- Confirm 직전에 exact scope cache에 사용자 확인을 받은 `threadId`와 native inventory row를 recovery
  coordinate로 보존한다. 이것은 deletion authority가 아니며 native command는 모든 authority를 다시 검증한다.
- Command가 반환되기 전과 receipt가 반환된 뒤 모두 inventory/thread row를 optimistic하게 삭제하지 않는다.
- Receipt 뒤 exact scope의 `code_worktrees_list`와 `code_threads_list`를 새로 읽는다. 두 authoritative 결과에서
  target thread가 모두 사라졌을 때만 두 cache를 함께 교체하고 완료를 announce한다.
- Native outcome이 receipt 없이 끝나면 sidebar unmount/remount와 target row refetch-absence 뒤에도 exact
  attempt coordinate와 outcome-unknown 안내를 보존한다. 같은 input의 destructive retry를 다시 확인받아야
  하며, retry는 removal을 완료하거나 existing tombstone receipt를 회수할 수 있다.
- Receipt를 회수한 뒤 list refresh가 실패하면 receipt를 exact-scope cache에 보존하고 다른 remove를 막은 채
  list read만 retry한다. Sidebar unmount/remount도 이 committed state를 잃지 않는다.
- 성공 receipt와 UI는 `transcriptDisposition: "preserved"`를 명시한다. Worktree removal은 Codex transcript
  삭제나 thread archive/delete가 아니다.

## 2. Gate `mergedAuthority`

### 2.1 Authority capture

`baseRef`의 immutable OID만으로 merged target을 추론하지 않는다. Binding store v4는
preparation 시 선택한 base가 같은 Git common-dir의 **direct local branch**로 유일하게 확정되는
경우에만 exact `refs/heads/<name>`을 optional merge-target authority로 저장한다.

Public `CodeThreadBinding`은 기존 exact 8 fields를 유지한다. Native-only sibling record의 persisted
shape는 exact
`{communityId, projectDtag, repositoryIdentity, codexThreadId, worktreeId, targetRef}`이며 binding과
0:1로 join한다. V4 top level은 `mergeTargets`와 `removals`를 required collection으로 저장하고,
preparation 안의 optional `mergeTargetRef`는 public preparation/inventory projection 전에 scrub한다.

- `HEAD`는 source checkout의 HEAD가 attached이고 그 symbolic target이 direct local branch일 때만
  authority가 된다.
- Short branch name도 native가 정확히 `refs/heads/<name>`으로 resolve하고 그 commit이 persisted
  `baseRef`와 같을 때만 저장한다.
- Tag, raw OID, `refs/remotes/*`, `origin/HEAD`, reflog/range/revision expression, arbitrary ref는
  authority가 아니다.
- Fork는 source의 persisted authority만 atomic copy한다. Destination HEAD나 다른 ref에서 새 target을
  추론하지 않는다.
- V1/V2/V3 binding은 in-memory migration에서도 authority absent다. `baseRef`나 현재 branch 이름으로
  보정하지 않는다.

따라서 legacy/non-local-base task는 첫 safe-remove slice에서도 fail closed한다. 별도의 사용자 target
선택이 필요해지면 독립된 authority-setting 설계로 다루며, remove caller가 ref/OID/proof를 직접
제출하게 만들지 않는다.

### 2.2 Exact graph proof

Private removal admission은 stable Archived managed binding, exact nest/repository/worktree identity와
persisted authorized ref를 같은 store snapshot에서 얻어야 한다. 현재 internal proof helper는 exact
binding/authority snapshot을 읽고 lifecycle eligibility를 만들지 않은 채 hardened, bounded Git read로
현재 detached HEAD `H`와 authorized ref의 commit `T`를 읽어 다음만 merged proof로 인정한다.

```text
git merge-base --is-ancestor H T  # exit 0 only
```

Proof receipt는 exact
`{repositoryIdentity, worktreeId, headCommit, targetRef, targetCommit}`에 결박한다. Proof 전후에
HEAD, target ref commit, common-dir와 managed-root identity를 다시 읽어 모두 같아야 한다. Exit 1은
not merged다. 다른 exit/signal, timeout, missing/non-commit object, malformed graph, pre/post drift는
proof unavailable이다.

이 capture와 graph proof는 native-only production code로 구현되어 inventory eligibility와 public remove
admission 양쪽에 연결됐다. Public proof command나 caller-supplied proof는 없다. Inventory는 store의 exact
binding/authority snapshot과 list의 공유 deadline으로 proof를 수행하며, remove command는 mutation 직전
native authority와 physical identity를 다시 증명한다.

Git read는 기존 hardened environment와 같은 차단을 유지하고 lazy object fetch/network도 금지한다.
`GIT_NO_REPLACE_OBJECTS=1`을 사용하며, 별도 ancestry input인 non-empty
`$GIT_COMMON_DIR/info/grafts`도 거부한다. Shallow/promisor repository가 로컬 object만으로 exit 0을
증명하지 못하면 false negative를 허용하고 제거를 거부한다.

결과 해석은 다음과 같다.

- `H == T` 또는 `H`가 merge commit을 통해 `T`의 ancestor이면 proven이다.
- `H == baseRef`만으로는 proven이 아니다.
- Authorized ref가 아닌 다른 branch가 `H`를 포함해도 proven이 아니다.
- Squash/cherry-pick/patch-equivalent commit은 ancestry가 아니므로 proven이 아니다.
- Positive proof는 dirty/attached/lifecycle/journal/deletion-boundary gate를 대신하지 않는다.

Inventory는 archived row의 authorized ancestry proof가 positive이면 committed work의 제거 관점에서
`headDrift`를 해소하고 `mergeProofUnavailable`를 추가하지 않는다. Proof가 not-merged/unavailable이거나
binding/lifecycle/authority/removal join이 proof 뒤 바뀌면 `mergeProofUnavailable`로 닫는다. 다른 blocker까지
모두 비어 있을 때만 `canRemove: true`다. 이 projection은 action eligibility일 뿐 deletion authority나
receipt가 아니며 public command는 quiescence, proof와 pinned physical boundary를 다시 검증한다.

## 3. Gate `durableRemovalJournal`

Binding index v4에는 archive lifecycle과 start/fork preparation 어느 쪽도 재사용하지 않는 별도 required
`removals` namespace가 있다. Strict tagged records와 다음 exact state machine을 pure store로 구현했다.

```text
claimed -> removing -> removed
```

| State | Durable meaning | 허용되는 다음 동작 |
|---|---|---|
| `claimed` | Exact Archived binding, merge proof와 deletion coordinates가 sync됐고 deletion mutation은 0회다. | Proof를 재검증해 `removing`으로 전환하거나, definitely-not-started가 증명될 때만 exact cancellation |
| `removing` | 최초 Git/filesystem mutation 전에 sync됐다. Mutation이 일부 또는 전부 수행됐을 수 있다. | Same journal만 recovery; rollback/cancel/new target 선택 금지 |
| `removed` | Original/quarantine root와 exact Git-admin absence를 검증하고 live binding을 tombstone으로 atomic retire했다. | 같은 receipt 반환; execution 복구 금지 |

Removal record는 최소 다음 immutable authority를 가진다.

- Native-issued `removalId`
- 원래 8-field `CodeThreadBinding` 전체와 literal `threadLifecycleAtClaim: archived`
- Exact merge proof receipt
- Physical manifest digest와 original/quarantine/admin coordinates
- Literal `transcriptDisposition: preserved`, `executionDisposition: removed`

현재 v4 wire record는 `state`와 위 immutable authority가 같은 strict object에 놓인다. Authority의 exact
top-level fields는 `removalId`, `binding`, `threadLifecycleAtClaim`, `mergeProof`,
`physicalManifestDigest`, `physical`, `transcriptDisposition`, `executionDisposition`이다. `mergeProof`는
exact `{repositoryIdentity, worktreeId, headCommit, targetRef, targetCommit}`, `physical`은 exact
`{managedRootParent, managedRoot, quarantineName, gitAdminParent, gitAdminEntry}`다. Unknown/missing field,
non-canonical UUID, invalid digest/ref/OID/path, wrong literal과 join drift는 load 전에 bytes를 보존한 채
fail closed한다. `quarantineName`은 caller가 정하지 않고 native-issued `removalId`에서
`.schoolx-removing-<removalId>`로 파생한다.

V4 removal namespace는 raw bytes를 실제 tagged record type으로 한 번 더 decode해
`serde_json::Value` 변환이 숨길 수 있는 duplicate `removals`, `state`, authority/binding/proof/physical
member도 거부한다. 이 probe는 v4 removal에만 적용하므로 v1/v2/v3 migration의 byte/mtime-preserving
read 동작은 바꾸지 않는다. Proof의 `headCommit`과 `targetCommit`은 original binding `baseRef`와 같은
Git object-id 길이여야 하므로 한 repository authority 안에서 SHA-1/SHA-256 형식을 섞을 수 없다.

`claimed | removing`은 exact live managed binding, stable Archived lifecycle과 merge-target sibling에 1:1
join한다. `removed`는 live binding/lifecycle/merge-target과 공존하지 않는다. Final store swap은 이 세
live records를 한 atomic write에서 tombstone으로 retire한다. 모든 상태의 record가 thread ID, worktree ID,
execution root를 예약하므로 binding/preparation/recovery가 이를 재사용하면 index load/admission을
fail closed한다. Recovery candidate filtering도 live binding과 tombstone reservation을 함께 사용한다.
Journal은 4,096 records와 기존 4 MiB store cap 안에서만 claim하며 capacity가 없으면 mutation은 0회다.

`(scope, threadId)` retry는 이미 저장된 record/tombstone을 새 proof/target으로 바꾸지 않고 같은
`removalId`로 반환한다. Exact `claimed` cancellation은 definitely-not-started일 때만 가능하고 cancel
response loss 뒤의 absence도 idempotent하다. 새 record가 같은 key를 차지한 ABA 상황은 stale CAS로
거부한다. `removing`은 response loss나 crash 뒤 sticky하다. Physical deletion이 끝났지만 final index
write가 실패해도 rollback하지 않고 absence를 다시 검증한 뒤 final atomic swap만 재시도한다.

Private physical engine과 startup pending-removal recovery dispatcher가 구현됐다. Runtime startup은
binding-store mutex 아래 recovery를 먼저 끝낸 뒤 emitter/runtime start와 lifecycle/start/fork
reconciliation을 진행한다. Store-level lifecycle 전이는 pending removal의 stable Archived join을 깨뜨리려
하면 save 전에 fail closed한다. Archived rename과 raw binding lookup을 쓰는 turn interrupt도 같은 mutex
아래 exact `claimed/removing` ownership을 확인해 RPC 전에 거부한다.

Public command가 새 claim을 요청할 수 있지만 caller authority는 exact `(scope, threadId)`에서 끝난다.
Native는 lifecycle authority가 ready이고 shutdown 전인 상태에서 binding mutex를 획득한 뒤 runtime idle,
exact PTY-owner absence와 native lookup을 증명한 sealed activity-clearance로만 private engine에 들어간다.
Manifest-derived journal claim input은 removal module 밖에서 구성할 수 없고, finalization은 pinned inspector만
만드는 opaque single-use verified-absence capability를 소비한다.

Merge target object graph는 journal `claimed` sync와 `removing` 전환 뒤 reserved
`refs/schoolx/removal-claims/<removalId>`가 exact `targetCommit`을 가리키도록 pin한다. 이 internal ref
write가 첫 Git mutation이며 journal보다 앞설 수 없다. Tombstone finalization 후 exact expected OID를
사용한 compare-and-delete로 정리한다. Cleanup이 중단되면 removed tombstone의 coordinate만 사용해
startup/retry에서 재시도하며 broad ref scan/prune을 하지 않는다. 다른 OID가 같은 ref 이름을 차지하면
replacement를 보존하고 fail closed한다.

Ref backend는 Git `files` format만 허용한다. Private proof ref는 symbolic/ambiguous raw ref를 거부하고
`--no-deref` compare-create/delete를 사용하며, exact loose regular file을 no-follow로 확인한다. Mutating
`update-ref`는 reference fsync를 강제하고 ref file과 parent ref/common directories를 fsync한다.

## 4. Gate `bindingTranscriptSemantics`

Pure store의 tombstone semantics가 구현됐지만 removal은 thread archive/delete가 아니다.

- Root와 exact Git-admin entry의 absence가 검증될 때까지 live binding과 lifecycle을 삭제하지 않는다.
- Finalization은 한 atomic index write에서 live binding+lifecycle을 제거하고 원래 binding 전체를
  permanent `removed` tombstone으로 옮기며 joined merge-target sibling도 함께 retire한다. Production
  finalization은 pinned inspector만 만들 수 있는 opaque single-use verified-absence capability를 소비한다.
  Pure-store fault-injection용 private test seam은 atomic store swap simulation일 뿐 production absence
  evidence가 아니다.
- Codex `$CODEX_HOME` transcript는 삭제, 이동, 복제, relay/Nostr 게시하지 않는다.
- Tombstone은 scope, Codex thread ID와 원래 descriptor/proof receipt를 보존해 SchoolX recovery
  coordinate와 idempotent response를 유지한다.
- Tombstone은 executable binding이 아니다. Resume, turn, PTY, Changes, rename, unarchive, fork의
  admission source가 될 수 없다.
- 향후 transcript 표시는 tombstone-aware `thread/read(includeTurns:true)` read-only 경로로만 한다.
- Codex transcript가 외부에서 사라져도 tombstone을 자동 삭제하거나 worktree를 재생성하지 않는다.
- "Restore removed task"는 새 preparation/root를 만드는 별도 future operation이다.

Archive lifecycle에 `removing/removed`를 추가하지 않고, start/fork preparation을 cleanup journal로
해석하지 않는다. Remove recovery가 `thread/archive`, `thread/unarchive`, `thread/start`, `thread/fork`
또는 thread close/delete RPC를 호출하지 않는다.

Live managed-worktree inventory는 `removed` tombstone을 missing/unavailable root로 다시 투영하지
않는다. Pending `claimed/removing` ownership은 positive eligibility로 승격되지 않으며 preserved blocker
상태로 닫힌다. Inventory row는 mutation receipt가 아니며 optimistic row removal도 금지한다.

## 5. Gate `pinnedDeletionBoundary`

Linux/macOS용 private deletion engine을 구현했다. Journal이 저장하는 Git-admin coordinate는 claim 전에
pinned common-dir/worktree admin의 reciprocal identity와 strict physical manifest에서 파생한다. 임의의
crate caller는 manifest-derived claim authority나 verified-absence token을 만들 수 없다. Public remove
command는 exact coordinate와 app-owned admission context만 native entrypoint에 전달하며, startup sticky
recovery와 동일한 sealed physical engine을 우회할 수 없다.

### 5.1 Inventory와 현재 helper를 재사용하지 않는 이유

Inventory의 `dirty:false`는 tracked/untracked status일 뿐 ignored file, empty directory, nested special
entry를 증명하지 않는다. Inventory 뒤 multi-process pathname/Git state도 바뀔 수 있다. 따라서 list의
descriptor, status, device/inode와 blocker 결과는 deletion authority가 아니다.

착수 당시 pinned Git helper도 target handle 안에서 Git을 실행한 뒤 같은 pathname chain이 남아 있음을
post-check한다. 성공 시 target name 자체가 사라져야 하는 deletion에는 맞지 않고 parent handle도
전달하지 않으므로 재사용하지 않았다. 이후 production helper launch 자체는
[`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md)의
descriptor-bound macOS XPC/Linux authority로 교체됐지만, inventory가 deletion authority가 아니라는 이 절의
결론은 그대로다.

### 5.2 Physical manifest

Claim 단계에서 exact root를 다시 열고 no-follow physical manifest를 만든다. 현재 engine이 삭제할 수 있는
entry는 linked-worktree `.git` file, Git index의 tracked file/symlink와 그 ancestor directory뿐이다.

- Ignored/untracked file, untracked empty directory, socket/FIFO/device, nested mount/cross-device entry,
  submodule/nested repository, unknown file type가 하나라도 있으면 claim을 거부한다.
- Git-admin top-level `locked` marker와 어느 depth의 `*.lock`도 locked/concurrent mutation evidence로
  해석해 claim 전에 거부한다.
- Stage-0 index와 proven `HEAD` tree의 path/mode/object ID가 exact 일치하고, 실제 regular-file/symlink
  bytes의 Git blob ID도 그 object ID와 같아야 한다. `assume-unchanged` 같은 status 은폐는 권한이 아니다.
- 모든 HEAD blob은 network/lazy fetch 없이 local primary object database에서 exact `blob`이어야 한다.
  Alternate/shared object storage, symlink/special/nested-mount object storage와 non-files ref backend는
  zero-mutation으로 거부한다.
- Tracked symlink는 link 자체만 manifest에 포함하며 target을 따라가지 않는다.
- Manifest는 canonical JSON bytes의 SHA-256으로 주소 지정한 strict v1 sidecar에 atomic write+sync하며,
  journal의 `physicalManifestDigest`와 exact 일치해야 recovery할 수 있다.
- Sidecar directory/file은 code data directory에서 same-mount handle-relative로만 접근하고, persist/load/remove와
  이미 absent인 cleanup 경로도 named directory/file identity를 재검증한다. Name replacement나 identity
  drift는 cleanup success로 해석하지 않는다.
- Manifest identity는 device/inode만이 아니라 durable birth-time과 지원되는 generation을 포함한다.
  Manifest 이후 entry type/identity/content authority가 drift하면 `removing` recovery-required로 남고
  새 entry나 replacement를 삭제하지 않는다.

이는 보수적으로 ignored build output도 보존한다. SchoolX가 대신 `git clean/reset`하거나 이를
regenerable data로 추정하지 않는다.

### 5.3 Parent-handle claim과 quarantine

Linux/macOS implementation은 active nest부터 `WORKTREES`, repository bucket, exact worktree root 및 Git
common-dir의 `worktrees/<admin-id>` parent/entry까지 `O_NOFOLLOW` handle로 pin한다. 최초 mutation 전에
`removing`을 sync한 뒤 parent-relative atomic no-replace rename으로 UUID root를 deterministic
quarantine name으로 옮긴다.

```text
<worktree-id> -> .schoolx-removing-<removal-id>
```

Moved object가 claim에서 pin한 exact object와 다르면 recursive deletion 전에 멈춘다. 이후 manifest의
entry만 `openat/fstatat/unlinkat` 계열 no-follow, handle-relative operation으로 삭제한다. Root가 사라진
뒤 reciprocal metadata를 검증한 exact Git-admin entry만 같은 방식으로 제거한다. Broad
`remove_dir_all`이나 pathname-recursive fallback은 없다.

Original/quarantine/admin coordinate 각각은 `absent | expected | replacement`로 관측한다. Recovery는
journal의 deterministic coordinate와 reciprocal `.git`/admin proof를 다시 pin해 known prefix만
진행한다. Process crash 뒤의 raw fd나 device/inode만 authority로 쓰지 않는다. Replacement가 original
name에 생겨도 삭제하지 않으며 success로 finalize하지 않는다. Quarantine/admin replacement, impossible
state 또는 identity ambiguity도 sticky `removing`으로 닫는다.

그 밖의 platform은 동일한 no-follow, parent-handle, atomic no-replace claim과 identity-stable deletion을 증명한
platform implementation/test가 없으면 `unsupported`로 zero-mutation fail closed한다.

같은 UID의 악의적인 process가 claimed subtree inode를 rename하거나 새 entry를 주입하는 것을 portable
POSIX API만으로 완전히 막지는 못한다. 계약이 보장하는 것은 외부 replacement/symlink/sibling tree로
recursive deletion을 redirect하지 않는 것과 ambiguity를 success로 보고하지 않는 것이다.

## 6. Frozen mutation ordering

Private implementation은 다음 순서를 바꾸지 않는다.

1. Exact scope/thread의 stable Archived managed binding과 native merge authority를 load한다.
2. Idle/no-PTY/no-approval/preparation conflict, exact root와 physical manifest를 다시 증명한다.
3. Digest-addressed strict manifest sidecar를 atomic write+sync한다.
4. `claimed` journal을 atomic write+sync한다.
5. Proof/identity를 다시 확인하고 `removing`을 atomic write+sync한다.
6. Exact private proof ref가 `targetCommit`을 가리키도록 만들고 다시 검증한다.
7. Parent-relative no-replace rename으로 root를 quarantine한다.
8. Quarantine에서 frozen manifest만 no-follow 삭제한다.
9. Exact Git-admin entry를 제거한다.
10. Original/quarantine/admin absence와 sibling non-mutation을 검증한다.
11. 한 atomic index write로 live binding+lifecycle을 permanent `removed` tombstone으로 옮긴다.
12. Exact loose proof ref를 expected `targetCommit` OID로 compare-and-delete하고 ref storage를 sync한다.
13. Digest-bound manifest sidecar를 unlink하고 directory absence를 sync한 뒤 같은 receipt를 반환한다.

Sidecar는 journal보다 먼저 durable해야 `claimed` crash recovery가 manifest 원문을 항상 찾을 수 있다. Claim
store commit 전에 실패해 남는 content-addressed orphan은 executable authority가 아니며 broad GC 대상으로
채택하거나 삭제하지 않는다.

Removed cleanup은 proof-ref absence가 durable해진 뒤 sidecar absence를 완료 marker로 사용한다. Original
common-dir가 offline/이동 상태면 startup은 계속하되 sidecar를 보존하고 exact coordinate cleanup을 defer한다.

각 durable/mutation 경계의 crash와 response loss를 fault test로 고정한다. Known state가 아니면 새 operation이나
새 target으로 넘어가지 않는다.

## 7. 명시적으로 금지되는 우회

- `--force`, `git clean`, `git reset`
- `git worktree remove`, `git worktree prune`
- Broad/pathname-based `remove_dir_all`
- Archive/fork/start recovery의 implicit cleanup
- Directory scan orphan의 implicit adoption/removal
- Inventory result나 frontend path/ref/OID/proof를 deletion receipt로 사용
- Merge proof를 위한 fetch/network/credential helper/PR API
- Codex transcript/thread delete

## 8. Acceptance test matrix

Native mutation과 public surface의 contract/fault/UI tests는 다음을 고정한다.

- Direct local authorized ref, merge commit ancestry, `H == T`는 proof success다.
- Unmerged, other-ref-only, squash/cherry-pick-only, legacy authority absent, ref/HEAD drift,
  replace/graft-only ancestry, timeout/missing object는 fail closed다.
- Merge proof는 binding index, refs, Git admin, worktree에 zero mutation이다.
- Claim admission failure는 zero mutation이고 `removing`은 절대 rollback하지 않는다.
- 모든 mutation boundary의 crash/retry와 response loss가 같은 `removalId`/receipt로 수렴한다.
- Final store failure는 absence 재검증 뒤 tombstone finalization만 재시도한다.
- Ignored/untracked/empty/special/cross-device entry와 Git-admin lock evidence는 삭제 전에 거부한다.
- Symlink target, same-name replacement와 sibling worktree는 byte-for-byte 보존한다.
- Manifest sidecar directory/file replacement는 fail closed하고, offline/moved common-dir cleanup은 sidecar marker를
  보존한 채 exact coordinate가 돌아올 때까지 defer한다.
- Removed identity는 재사용되지 않고 transcript coordinate는 보존되며 실행 gate는 모두 닫힌다.
- Unsupported platform은 zero mutation이다.
- Tauri command는 `code_worktree_remove`와 top-level `{input}`만 노출하고 input은 exact
  `{scope, threadId}`, output은 native-derived exact 9-field receipt다.
- Inventory는 native positive proof와 모든 blocker 부재를 만족한 stable Archived row만
  `canRemove: true`로 투영하며, 다른 row와 sibling inspection failure를 보존한다.
- Confirmation cancel은 zero command이고, confirm 뒤 native response가 오기 전 row는 그대로 남는다.
- Receipt 없는 outcome-unknown attempt는 sidebar unmount/remount와 target row absence 뒤에도 exact
  coordinate를 보존한다. Destructive retry를 다시 확인받고 같은 input만 사용해 removal을 완료하거나 동일
  receipt를 회수하며, 새 ID나 caller proof로 재구성하지 않는다.
- Receipt 뒤 inventory와 thread list 두 authoritative read가 target absence를 함께 확인해야 UI cache에서
  제거한다. Reconciliation failure/unmount 뒤에도 receipt를 보존하고 list read만 retry하며 transcript
  preserved 상태를 계속 표시한다.

Regression sentinel은 별도 frozen fixture, native/frontend contract tests, current pinned-helper의 removal
operation decode 거부, private removal helper 분리, startup recovery ordering, rename/interrupt pending gate,
strict public DTO/receipt, proof-based inventory eligibility, populated/empty/error inventory UI의 exact action 수,
explicit confirmation, no-optimistic-row와 response-loss/authoritative-reconciliation E2E를 함께 검사한다.
