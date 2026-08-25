# SchoolX Code Codex 0.149 계약·운영 증거와 다음 slice decision gate

기준일: **2026-08-25**

문서 상태: **현재 Codex/제품 handoff + Phase 3 후속 구현 승인 대기**. 이 세션은
제품 코드를 변경하지 않았다. Codex 0.149 계약, 제품 진입점, 수동 운영 결과,
release 잔여 gate를 정리하고 다음 독립 slice 하나만 추천한다. 추천 기능은
사용자 승인 전에는 구현하지 않는다.

현재 상태를 읽는 순서는 다음과 같다.

1. Codex와 다음 제품 slice: 이 문서
2. 전체 SchoolX Code 설계: [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
3. Phase 3 Git write public/security contract:
   [`SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md`](SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md)
4. 구현된 Git write/crash/CAS 상태:
   [`SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md`](SESSION_HANDOFF_20260821_CODE_PHASE3_GIT_WRITE_IMPLEMENTATION.md)
5. XPC launch authority와 의도적 residual:
   [`SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md`](SESSION_HANDOFF_20260821_CODE_HELPER_LAUNCH_AUTHORITY_DECISION.md)
6. Artifact별 최신 local/canonical release 판정:
   [`SESSION_HANDOFF_20260825_CODE_RELEASE_READINESS.md`](SESSION_HANDOFF_20260825_CODE_RELEASE_READINESS.md)

2026-08-14의 0.145 handoff들은 당시 계약을 보존하는 역사적 기록이다. 이 문서가
그 내용을 삭제하거나 당시 판정을 소급 변경하지 않는다. 다만 현재 지원과
검증 상태를 판단할 때는 이 문서와 checked-in 0.149 fixture/test가 우선한다.

## 1. 증거 등급과 사용 원칙

| 증거 | 이 문서에서 답하는 것 | 답하지 않는 것 |
|---|---|---|
| OpenAI 공식 app-server 문서 | `thread/read`의 비-resume summary read 의미, source kind의 공개 의미 | SchoolX가 고정한 0.149 exact bytes와 설치본의 버전별 차이 |
| Repository schema/wire fixture + native tests | exact 0.149 schema, SchoolX request/parser, recovery wire와 fail-closed 조건 | signed 배포 artifact와 실제 사용자의 end-to-end 결과 |
| Mock bridge Playwright | UI route, payload shape, 상태·focus·회귀 | 실제 Codex process, 실제 model 응답 또는 실제 filesystem mutation |
| 수동 설치 앱 관측 | 특정 설치 환경에서 task/prompt/file-change가 실제 완료됐는지 | 재현 가능한 CI, canonical artifact provenance 또는 일반 플랫폼 지원 |
| Local signed/notarized artifact 검증 | 검사한 exact local app/DMG의 architecture/signature/runtime/ticket | canonical release workflow가 같은 byte를 만들고 검증하는지 |
| Canonical CI/release | 게시 가능한 provenance와 반복 가능한 platform gate | 아직 실행하지 않은 lane의 성공 |

OpenAI 공식 문서는 `thread/read`가 저장된 thread를 resume/subscribe하지 않고 읽으며
`includeTurns:false` 또는 생략 시 summary만 반환한다고 설명한다. 이 의미 확인에만
[공식 Codex app-server 문서](https://learn.chatgpt.com/docs/app-server)를 사용했다.
버전별 실제 wire 판정은 아래 repository fixture와 tests를 우선한다.

## 2. 현재 Codex CLI 계약

### 2.1 지원 상태를 정확히 표현하는 방법

| 항목 | 현재 판정 |
|---|---|
| 최신 exact audited snapshot | **`codex-cli 0.149.0`** |
| Runtime admission | `0.145.<numeric patch>` 또는 `0.149.<numeric patch>`, prerelease/build suffix 없음 |
| `0.145.0` 의미 | 최초 exact fixture이자 역사적 compatibility baseline; runtime 호환 범위에는 계속 남음 |
| 모든 numeric patch의 schema 동일성 | **증명하지 않음**. Exact schema 증명은 0.145.0과 0.149.0 snapshot에 한정 |
| 0.146–0.148, 0.150+ | Startup 전에 unsupported로 거부 |

따라서 “현재 검증 버전은 0.149.0”은 맞지만 “0.145는 더 이상 지원하지 않는다”는
표현은 현재 source와 모순이다. 반대로 numeric patch family admission을 근거로 모든
0.149 patch의 schema가 검증됐다고 표현해서도 안 된다.

근거:

- exact generator/version, 291개 generated schema, 66개 selected schema와 hash:
  [`codex-0.149.0-schema-manifest.json`](../../desktop/src-tauri/src/code_workspace/fixtures/codex-0.149.0-schema-manifest.json)
- representative request/response/notification:
  [`codex-0.149.0-wire.json`](../../desktop/src-tauri/src/code_workspace/fixtures/codex-0.149.0-wire.json)
- 두 minor family만 admit하는 startup gate:
  [`discovery.rs`](../../desktop/src-tauri/src/code_workspace/discovery.rs)
- schema hash, 0.145→0.149 delta, native builder/parser와 notification/approval 대조:
  [`contract_tests.rs`](../../desktop/src-tauri/src/code_workspace/contract_tests.rs)

0.149 selected schema 66개 중 45개는 exact unchanged, 21개는 changed다. SchoolX가
사용하는 request property는 제거되지 않았고 required request field도 drift하지 않았다.
Response/parser 쪽에는 required Thread `projectId`, optional Model `modelSpecialty`와
`multiAgentVersion`, ModelUpgradeInfo `retirementAt`가 추가돼 strict parser에 반영됐다.

### 2.2 `vscode` / `appServer` source compatibility의 범위

Checked-in schema는 두 source를 모두 정의한다. 실제 0.149 recovery에서는 SchoolX가
app-server로 만든 thread가 `vscode`로 보고될 수 있고, 0.145/schema spelling은
`appServer`다. Native recovery filter와 list-absent thread parser는 두 값만 admit한다.
그러나 source 문자열 하나로 신뢰하지 않고 다음을 독립적으로 다시 증명한다.

- exact native SchoolX preparation marker
- exact thread/session ID 관계
- canonical execution root
- parent/fork ancestry
- non-ephemeral, quiescent status와 empty-turn 조건

`cli`, unknown/new source, marker 누락/오염, 다른 root, ambiguous ancestry는 fail closed한다.
이 compatibility는 recovery와 authoritative graph의 관측 차이를 흡수하는 좁은 계약이다.
모든 RPC flow에서 `vscode`와 `appServer`가 완전히 동등하다는 뜻은 아니다. 예를 들어
fork response validation에는 별도의 method-specific source 조건이 남아 있으므로 후속
버전 작업은 해당 flow의 exact wire test를 따로 추가해야 한다.

근거:

- recovery list/read builder: [`protocol.rs`](../../desktop/src-tauri/src/code_workspace/protocol.rs)
- strict source/marker parser: [`thread_lifecycle.rs`](../../desktop/src-tauri/src/code_workspace/thread_lifecycle.rs)
- real-0.149-shaped `vscode` positive와 marker/source negative tests:
  [`thread_lifecycle/tests.rs`](../../desktop/src-tauri/src/code_workspace/thread_lifecycle/tests.rs)
- method-specific fork validation:
  [`code_thread_fork.rs`](../../desktop/src-tauri/src/commands/code_thread_fork.rs)

### 2.3 loaded-only `thread/read(includeTurns:false)` 복구

Empty/new thread나 response-loss 경계에서는 thread가 memory에 loaded됐지만 persisted
`thread/list`에 아직 없을 수 있다. Recovery는 다음 순서로만 이를 보완한다.

```text
active + archived thread/list (bounded, all audited source kinds)
                       +
thread/loaded/list (bounded, duplicate/cursor rejection)
                       |
             ID already listed ──► keep listed membership
                       |
             list-absent loaded ID
                       |
 exact bound/deferred target or pending fork expectation?
             no ──► fail closed
             yes
              └──► thread/read({threadId, includeTurns:false})
                     └──► exact ID/marker/root/ancestry/source/quiescence
                           one match ──► bind/recover
                           zero or many ──► fail closed
```

`thread/read`는 thread를 resume하거나 subscription을 여는 mutation으로 사용하지 않는다.
Full turn transcript도 읽지 않는다. List-absent ID가 단지 loaded됐다는 이유만으로 새
authority가 되지 않으며, unrelated/unbound ID, duplicate, cursor cycle, non-empty deferred
root 또는 여러 pending journal에 맞는 candidate는 거부한다.

핵심 회귀는
[`runtime.rs`](../../desktop/src-tauri/src/code_workspace/runtime.rs)의
`authoritative_graph_paginates_both_memberships_and_loaded_deferred_threads`와
`authoritative_graph_admits_only_the_exact_list_absent_pending_fork`다. 첫 test는 source
`vscode`인 loaded-only ID를 exact `includeTurns:false`로 한 번 읽어 Active membership에
합치고, 둘째 test는 exact marker/root/ancestry만 허용한다.

## 3. 제품 진입점과 실제 작업 증거

### 3.1 프로젝트 카드/목록과 상세의 두 진입점

SchoolX Code는 프로젝트 상세에만 숨어 있지 않다.

- Grid card와 list row의 visible `SchoolX Code` action:
  [`ProjectCards.tsx`](../../desktop/src/features/projects/ui/ProjectCards.tsx)
- 프로젝트 상세 tab header의 visible action:
  [`ProjectWorkspaceTabs.tsx`](../../desktop/src/features/projects/ui/ProjectWorkspaceTabs.tsx)
- 상세에서 active branch/base를 결박해 Code route로 이동:
  [`ProjectDetailScreen.tsx`](../../desktop/src/features/projects/ui/ProjectDetailScreen.tsx)
- `/projects/$projectId/code` route:
  [`projects.$projectId.code.tsx`](../../desktop/src/app/routes/projects.$projectId.code.tsx)

Fresh-build mock E2E는 768px/900px project card와 detail 양쪽에서 action이 visible하고
키보드/클릭으로 Code route와 breadcrumb까지 도달함을 검증한다:
[`schoolx-code.spec.ts`](../../desktop/tests/e2e/schoolx-code.spec.ts).

### 3.2 task 생성·prompt·파일 변경

2026-08-25 수동 설치 앱 smoke에서 다음 사용자 여정이 실제로 성공한 것으로
운영 관측을 기록한다.

1. SchoolX Code task 생성
2. Bound thread에 prompt 제출
3. Agent turn 완료
4. 대상 파일 변경과 Changes 확인

이 결과는 **manual operator observation**이다. Repository에는 이 file-changing run의
credential-safe immutable transcript/artifact가 없으므로 mock E2E나 canonical release
증거로 승격하지 않는다. 반대로 mock E2E는 기존 exact Tauri payload, prompt 전송,
timeline과 Changes rendering을 회귀하지만 실제 Codex/model/filesystem 실행을 증명하지
않는다. Release-readiness handoff의 signed x86_64 Rosetta task는 실제 XPC task 증거지만
의도적으로 no-op prompt였으므로 file-change 증거로 재사용하지 않는다.

## 4. Signed arm64 XPC와 남은 release gate

### 4.1 Local signed arm64 결과

Corrected local arm64 release artifact는 다음 계약을 통과했다.

- app executable, embedded XPC와 bundled sidecar를 포함한 7개 Mach-O가 모두 thin `arm64`
- app ID `io.github.schoolx520.app`, XPC ID
  `io.github.schoolx520.app.schoolx-code-git`
- app/XPC/sidecar의 동일한 non-empty Team, nested strict signature
- committed app entitlement, hardened runtime와 secure timestamp
- app/XPC plist `10.15`, Mach-O platform `1`, minos `11.0`, system Swift dependency
- repository signature/entitlement/runtime verifier
- Apple app submission `Accepted`, ticket staple, `stapler validate`, Gatekeeper
- stapled app으로 재조립·서명한 final arm64 DMG의 `Accepted`, staple와 mounted-app 통합 gate

이는 검사한 local exact artifact의 positive evidence다. Canonical workflow provenance로
확대하지 않는다. Hash, local path와 detailed command는
[`SESSION_HANDOFF_20260825_CODE_RELEASE_READINESS.md`](SESSION_HANDOFF_20260825_CODE_RELEASE_READINESS.md)에
보존한다.

### 4.2 아직 닫히지 않은 gate

| Gate | Local evidence | 남은 canonical/CI 조건 |
|---|---|---|
| macOS x86_64 | signed thin x86_64 app/XPC와 Rosetta exact-XPC task는 통과 | 정상 `desktop-v*` lane의 immutable provenance와 updater-enabled artifact 재검증 |
| 공증 | local arm64/x86_64 app과 final DMG 네 제출은 모두 Accepted | pinned signing action이 final rebuilt DMG 자체를 sign/notarize/staple하도록 외부 계약 수정; updater archive에서 추출한 app 검증 |
| Linux | QEMU 진단의 과거 `.deb`, launcher 일부와 pinned helper integrity만 있음 | native X64 Ubuntu 24.04 release-profile trace, 새 `.deb`/AppImage build, desktop+5 sidecar clean runtime smoke |
| CI/release | repository-side fail-closed workflow contract는 있음 | upstream/canonical workflow 실행과 evidence 보존; updater secret이 필요한 lane 완료 |
| file-size | launcher 신규 파일은 gate를 새로 악화하지 않음 | 기존 Phase 0–3 누적 19개 desktop ratchet violation 분할; limit/allowlist 완화 금지 |

따라서 “x86_64와 공증이 전부 미검증”도, “local 성공으로 canonical release까지
완료”도 둘 다 틀리다. Local dual-architecture/notarization은 닫혔고 canonical
x86 provenance, final-DMG/updater notary path, native Linux, CI와 file-size gate가 남았다.

## 5. 의도적으로 남은 security residual

다음은 완료로 위장하지 않고 현재 risk/availability 계약으로 유지한다.

- App와 XPC helper가 동시에 비정상 종료하면 별도 OS guardian이 없어 살아 있는
  Git process group의 cleanup 주체가 사라질 수 있다.
- Descriptor-bound cwd를 사용해도 `GIT_WORK_TREE`, `GIT_DIR`, `GIT_COMMON_DIR`에는
  absolute repository pathname이 전달된다. Exact revalidation/CAS가 완화하지만
  repository namespace 전체가 atomic descriptor closure인 것은 아니다.
- Linux support는 Rust public API의 no-fork guarantee가 아니라 pinned Rust/glibc/
  Ubuntu tuple과 runtime probe에 결박된다. Tuple/probe drift는 mutation 전에 unsupported다.
- Helper/child 종료를 증명할 수 없는 경우 authority를 성공으로 풀지 않고 fail-closed로
  보유한다. 안전을 위한 의도적 availability residual이다.
- macOS root/OS update와 system Git/xcode-select resolution, Linux root/package-manager는
  명시적 TCB다.

이 residual을 해소한다는 이유로 generic executable/shell, public Git argv, caller path/ref/OID,
unsafe production shim 또는 자동 Talk egress를 열면 안 된다.

## 6. Phase 3 후속 후보 비교

현재 whole-file 단건 stage/unstage와 staged-only commit은 strict durable journal,
owned lock, index publish와 detached-HEAD CAS까지 완료됐다. 다음 비교는 그 public
authority를 그대로 보존하는 독립 slice 기준이다.

| 후보 | 사용자 가치 | public authority 확장 | crash/CAS 부담 | UI 범위 | 테스트 비용 | 판정 |
|---|---|---|---|---|---|---|
| hunk/stage-all | 높음 | stage-all은 새 batch intent, hunk는 opaque partial-index coordinate 필요 | 높음 | 중간 | 매우 높음 | 후속. 첫 publish가 snapshot을 소비하므로 frontend loop 금지 |
| branch/push/PR 연결 | 매우 높음 | ref/remote/network/credential authority 필요 | 매우 높음 | 큼 | 매우 높음 | 이번 제약과 충돌 |
| inline diff note → 다음 turn context | 높음 | **없음**. 기존 prompt authority만 사용 | **새 부담 없음** | 중간 | 중간 | **추천** |
| 선택적 SchoolX Talk 공유 | 중간~높음 | relay/channel/audience egress 필요 | response-loss publish 필요 | 중간 | 높음 | audience/approval seam 전에는 보류 |
| Local/Archived Git write | 중간 | attached branch/shared checkout 또는 archived lifecycle write | 매우 높음 | 중간 | 매우 높음 | 기존 contract가 의도적으로 금지 |
| 직접 편집기 평가 | 불확실 | 평가만 하면 없음, 실제 write는 새 file authority | 실제 도입 시 높음 | 매우 큼 | 높음 | Phase 4 research로 유지 |

`hunk/stage-all`은 작은 button 추가가 아니다. Stage-all 첫 mutation이 snapshot을
소비하고 receipt/ack가 다음 operation을 막으므로 여러 단건 command를 frontend에서
연속 호출할 수 없다. Hunk는 partial index, filter/EOL, patch drift와 opaque hunk
coordinate를 새 transaction에 결박해야 한다.

Talk 공유는 반드시 선택적이어야 하지만, 현재 보안 계약은 derived content audience
교집합과 게시 직전 membership 재검증을 실제 publish seam에 아직 연결하지 않았다.
따라서 자동 업로드는 물론 이 slice의 부수 기능으로도 넣지 않는다.

## 7. 추천 slice: inline diff note → 다음 idle turn context

### 7.1 좁은 범위

첫 slice는 다음 상태에만 열린다.

- stable `active` managed worktree binding
- idle thread
- `code_thread_git_status.state === "ready"`의 complete Task diff
- binary가 아니고 patch가 truncate되지 않은 실제 old/new text line

사용자는 한 줄에 review note를 작성하고 composer 위의 pending-note tray에서 전송될
exact path/side/line/body를 다시 본다. 명시적 **Send**는 먼저 기존 read-only
`code_thread_git_status({scope, threadId})`를 한 번 다시 읽는다. Runtime generation,
status revision, snapshot과 frontend line digest가 모두 같은 eligible projection일 때만
note들을 versioned, stable-order review-context block으로 직렬화해 기존 next
`turn/start`의 `prompt` 문자열에 포함한다. 첫 slice는 active turn의 `turn/steer`, hunk
mutation, editor write, PR/Talk publish를 포함하지 않는다.

Note 자체가 사용자 지시이므로 별도 free-form composer text가 없어도 explicit Send는
가능하게 하는 것을 추천한다. 숨은 context는 없고 최종 전송 preview가 UI에 보이는
내용과 byte-equivalent여야 한다.

### 7.2 Local-only draft와 bound

Draft는 React community/project remount 경계 안의 frontend 임시 상태다. Module-level
singleton이나 새 durable store를 만들지 않는다.

```text
scope key: communityId + projectDtag + repositoryIdentity + threadId
source: runtimeGeneration + statusRevision + snapshotId
anchor: exact returned display path + old|new + line + versioned frontend patch/line digest
body: user-authored note
```

- Inspector close/open과 같은 project/thread 내 panel remount에서는 보존한다.
- Thread key가 다르면 서로 보이거나 전송되지 않는다.
- Community/project remount와 app restart에서는 폐기한다.
- Task-only `fileId`는 동일 projection의 status read에서도 재발급될 수 있으므로 draft
  identity나 freshness proof로 사용하지 않는다. 같은 generation/revision/snapshot과
  exact path/patch/line digest면 draft를 유지한다.
- Runtime generation, source revision/snapshot 또는 exact path/patch/line digest가 바뀌면
  silent re-anchor하지 않고 stale로 표시해 Send를 막는다. 사용자가 현재 diff line에서
  다시 작성해야 한다.
- Path는 prompt 안의 quoted user-visible content일 뿐 native path authority가 아니다.
  Native/Tauri가 이를 parse해 filesystem/Git input으로 사용하면 안 된다.
- 최대 20 notes, body당 UTF-8 4 KiB, review-context block 전체 64 KiB로 제한한다.
  합성 prompt는 기존 native 1 MiB bound도 그대로 통과해야 한다.

## 8. 구현 전 decision gate

상태: **WAITING FOR USER APPROVAL**

승인 대상은 정확히 다음 한 slice다.

1. Active managed/idle/ready Task diff의 complete text line만 대상으로 한다.
2. Frontend local draft + existing read-only `code_thread_git_status` + existing
   `turn/start(prompt)`만 사용한다.
3. Note-only explicit Send를 허용한다.
4. 새 native/Tauri command, path/ref/OID/argv authority, Git mutation/journal/CAS,
   generic shell, `turn/steer`, Talk/PR/media/network upload는 0개다.
5. Send 직전 status를 한 번 다시 읽고 stale runtime/projection/anchor는 자동 retarget하지
   않은 채 turn start를 차단한다.
6. File-size guard를 완화하지 않고 이미 큰 shared diff/Code screen을 먼저 leaf로
   추출하거나 순증가를 피한다.

승인 전에는 아래 public contract나 제품 파일을 변경하지 않는다.

## 9. Public contract

새 실행/mutation command는 없다. Send sequence는 기존 두 shape만 그대로 사용한다.

```text
code_thread_git_status({
  input: { scope, threadId }
})

code_turn_start({
  input: {
    scope,
    threadId,
    prompt,
    model,
    effort
  }
})
```

다음은 모두 불변이다.

- `code_turn_start` top-level `{input}`와 다섯 input field
- `code_turn_steer`, `code_thread_changes`와 여섯 Git command의 shape/순서
- Codex 0.145/0.149 schema manifest/archive/wire fixture
- Git write generation/snapshot/file ID, journal/receipt/ack와 crash/CAS contract
- Binding v4, lifecycle, safe-remove input/receipt와 XPC/Linux launch authority

새 `context`, `path`, `line`, `ref`, `OID`, `argv`, `operationId`, Talk target 또는
generic payload field를 Tauri boundary에 추가하지 않는다. Review block은 기존 bounded
user prompt의 visible text다. Send는 status read를 정확히 한 번 수행한 뒤 proof가 같을
때만 `code_turn_start`를 정확히 한 번 수행하며 둘 다 자동 retry하지 않는다. Status read에서
돌아온 Task-only `fileId`는 비교하거나 다음 input으로 보내지 않는다.

## 10. Fault matrix

| 조건 | UI/상태 수렴 | 외부/native 호출 |
|---|---|---|
| Local/Archived, non-active, active turn, Git blocked/recovery/busy | Add/Send disabled + plain reason | 새 turn/Git/Talk 0 |
| Binary, hunk header, missing/truncated patch | Line action 없음 | 0 |
| Empty/invalid/control body, note/count/context byte cap 초과 | Inline error, draft 보존 | 0 |
| Scope/thread 전환 | Exact key 격리, 다른 task에 노출 없음 | 0 |
| 동일 projection refetch, Task-only `fileId`만 변경 | Draft와 digest anchor 유지 | `code_thread_git_status` read 1 |
| Runtime generation/revision/snapshot/digest 변경 또는 A→B→A | Stale alert, silent retarget·Send 금지 | status read 1, turn/mutation/Talk 0 |
| Inspector close/open | Same project/thread draft 유지 | 0 |
| Community/project remount 또는 app restart | 비권위 draft 폐기 | 0 |
| Double click/`⌘↵` | Freshness read부터 existing submitting fence | status read 1, `code_turn_start` 1 |
| Start validation/transport failure 또는 response unknown | Prompt+submitted-note IDs 보존, 자동 retry 없음 | status read 1, start 1, 추가 0 |
| Start success | Exact submitted note IDs만 clear; 전송 중 새 note 보존 | status read 1, `code_turn_start` 1 |
| 어떤 경로든 Talk/media/project Git mutation/stage/commit/push/PR | 호출하지 않음 | freshness status read 외 0 |

새 durable crash journal이나 CAS는 없다. Turn start response-loss는 기존 runtime/preparation
recovery 계약을 그대로 사용하고, frontend는 성공을 증명하기 전에 note를 지우거나 같은
payload를 자동 재전송하지 않는다.

## 11. UI·접근성·E2E 완료 기준

### 11.1 UI와 접근성

- Eligible line에 `Add review note for <full path>, <old|new> line N` accessible name을
  가진 keyboard-focusable sibling button을 둔다. Button nesting은 금지한다.
- Inline editor는 visible label, hint/error `aria-describedby`, Save/Cancel과 Esc focus
  restore를 가진다. Mention, media와 request-changes UI는 넣지 않는다.
- Composer 위 tray는 note count, full path/side/line, body preview, Remove와 Return to
  line을 제공한다.
- Stale note는 `role=alert`, send blocker는 인접 plain text, add/remove/save 완료는
  한 polite live region으로 알린다.
- Send 전 serialized review context 전체를 검사할 수 있고 hidden metadata를 추가하지 않는다.
- Success 후 exact submitted tray만 사라지고 failure/unknown에서는 그대로 남는다.

### 11.2 Unit/component

- Stable serializer order, escaping, LF normalization과 4/20/64 KiB cap
- Exact scope/thread isolation과 community/project remount disposal
- Same projection과 changing Task-only file ID는 keep, runtime/revision/snapshot/digest ABA는
  stale, no auto re-anchor
- Submit snapshot과 concurrent-added note의 clear-exact-IDs
- Note-only send, disabled reason, double-submit fence와 failure retention
- Diff line action의 keyboard/focus/accessible name, PR inline-comment regression
- Send 전 local validation negative는 native 0회, freshness-stale은 status read 1회와
  turn/Git mutation/Talk 0회

### 11.3 Fresh-build E2E

새 작은 spec으로 최소 두 scenario를 둔다.

1. Project card 또는 detail → Code → idle bound thread → keyboard로 line note 작성 →
   inspector close/open과 same-projection Task-only file ID 변경에도 tray 유지 → explicit
   Send. Exact existing status-read key가 먼저 한 번, `code_turn_start` key가 다음 한 번이고
   deterministic visible prompt이며 Talk/Git mutation/push/PR command는 0회다.
2. Note 작성 뒤 Send-time authoritative runtime/status revision/snapshot/digest 변경 →
   status read 한 번 뒤 stale alert와 turn start 차단 → 현재 line에서 재작성 → start response
   failure에서 draft 유지, automatic duplicate command 0회.

반드시 `pnpm --dir desktop build:e2e` 뒤 smoke project로 실행하고 screenshot을 추가하면
`waitForAnimations`와 distinct hash 규칙을 따른다. 완료 gate는 targeted unit/component/E2E,
frontend 전체 test, typecheck, px-text, Biome와 `git diff --check`다. File-size checker도
반드시 실행하되 현재 알려진 19개 violation의 exact baseline을 기록하고 결과가 그 subset이며
새/touched-file violation이 0개일 때 이 slice의 no-regression gate를 통과한다. Limit/allowlist를
완화하지 않으며 repository 전체 0건 closure는 별도 release blocker로 계속 남긴다.

## 12. 예상 수정 파일

승인 후 예상 범위이며 이 세션에서는 수정하지 않는다.

- 새 `desktop/src/features/code/lib/codeReviewContext.ts`
- 새 `desktop/src/features/code/lib/codeReviewContext.test.mjs`
- 새 `desktop/src/features/code/state/useCodeReviewContext.ts`
- 새 `desktop/src/features/code/state/useCodeReviewContext.test.mjs`
- 새 `desktop/src/features/code/ui/CodeInlineDiffReview.tsx`
- `desktop/src/features/code/ui/CodeChangesPanel.tsx`
- `desktop/src/features/code/ui/CodeComposer.tsx`와 focused component test
- `desktop/src/features/code/ui/CodeWorkspaceScreen.tsx`의 최소 wiring. 현재 1,000줄에
  가까우므로 필요하면 submit orchestration을 새 sibling hook/component로 먼저 이동해
  순 line count를 줄인다.
- `desktop/src/features/projects/ui/ProjectPullRequestFilesChangedPanel.tsx`에서 generic
  line-action seam을 새 `ProjectDiffPreview.tsx`로 추출. 현재 파일이 이미 1,000줄을
  넘으므로 새 로직을 직접 누적하지 않고 PR inline comment regression을 유지한다.
- 새 `desktop/tests/e2e/schoolx-code-review-context.spec.ts`와
  `desktop/playwright.config.ts` smoke registration. 기존 대형
  `schoolx-code.spec.ts`에는 scenario를 더 누적하지 않는다.
- 필요할 때만 mock helper를
  `desktop/tests/helpers/schoolxCodeFixtures.ts`에 추가한다.

**수정하지 않을 범위:** Rust/Tauri command, Codex schema/wire fixture, Git write engine,
XPC/Linux launcher, Talk/Nostr/media/PR API와 generic shell.

## 13. 승인 후 시작 조건

사용자가 “이 decision gate로 진행”이라고 명시적으로 승인한 뒤에만 위 한 slice를
구현한다. 승인 전에는 여러 후보를 묶거나 product code를 선행 변경하지 않는다.
