# SchoolX Code Phase 1C 인계

작성일: 2026-08-14
상태: Phase 1C native contract/fixture freeze 구현 완료, Phase 1D 시작 전 세션 경계

## 1. 먼저 읽을 문서

새 세션은 아래 순서로 문맥을 복구한다.

1. [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
2. 이 문서
3. native binding과 recovery/Git 안전 경계의 상세 근거가 필요할 때만
   [`SESSION_HANDOFF_20260814_CODE_PHASE1B.md`](SESSION_HANDOFF_20260814_CODE_PHASE1B.md)

Phase 1B 인계의 “checked-in Codex schema/fixture가 아직 없다”와 “Phase 1C가 다음
범위”라는 설명은 당시의 역사적 상태다. 현재는 아래 fixture와 compatibility test가
native/public wire 계약의 변경 감지 기준이다.

## 2. 이번 단계에서 고정한 public Tauri 계약

`tauri-contract-v1.json`이 현재 frontend 경계의 16개 command 이름과 top-level
argument 이름을 고정한다.

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

- Rust DTO의 frontend 표현은 camelCase다.
- execution/preparation/runtime/approval enum 값과 tagged approval response를 fixture로
  고정했다.
- normalized event 이름은 `schoolx-code-workspace-event`이며 scope, runtime generation,
  sequence, optional thread/turn/item ID, kind, redacted payload 모양을 고정했다.
- optional ID와 nullable output은 JSON에서 의도한 `null` 표현을 유지한다.
- strict input fixture는 알려지지 않은 필드를 허용하지 않는 현재 Serde 경계를 확인한다.
- public preparation output에는 durable recovery를 위한 private
  `recoveryThreadBaseline`이 노출되지 않는다.

`thread-bindings-v1.json`은 schema version 1 store의 binding 8개 필드와 `Starting`
preparation의 private recovery baseline을 함께 고정한다. 같은 저장물을 public DTO로
내보낼 때 baseline이 scrub되는 계약도 compatibility test 대상이다.

## 3. Codex 0.145.0 schema와 wire 기준

로컬 `codex-cli 0.145.0`의 non-experimental
`codex app-server generate-json-schema` 결과를 canonicalize한 뒤 SchoolX Code가 실제로
사용하는 54개 schema를 manifest에 기록했다.

- source version: `codex-cli 0.145.0`
- experimental schema: `false`
- selected canonical aggregate SHA-256:
  `df00b0eff4563354d1a6ab799f1d1f446dcf439745bfb1e04742ae566ffedcd5`
- `codex-0.145.0-selected-schemas.tar.gz.base64`는 선택한 54개 canonical schema
  원문을 보관한다. 일반 CI test가 archive를 해제해 파일 집합, 개별 hash, aggregate
  hash를 다시 계산하고 wire fixture의 nested payload를 원문 schema로 검증한다.
- manifest는 source/generator, canonicalization 규칙, 선택 파일별 hash와 구조적 사실을
  함께 기록한다. 수동 regeneration audit는 설치된 exact 0.145.0 CLI에서 전체 273개와
  선택 54개의 aggregate를 다시 계산한다.

현재 fixture가 증명하는 upstream 버전은 **0.145.0 하나**다. Runtime의 엄격한
`codex-cli 0.145.<numeric patch>` gate는 prerelease/build suffix를 허용하지 않는 실행
허용 범위이지 모든 0.145 patch가 같은 wire schema라는 증명이 아니다. 새 버전을
지원할 때는 생성물을 다시 canonicalize하고 manifest/hash, wire fixture, compatibility
test를 함께 갱신해야 한다.

`codex-0.145.0-wire.json`은 다음 좁은 native 사용 범위를 고정한다.

- `initialize` request/response와 `initialized` notification
- `thread/start`, `thread/list`, `thread/loaded/list`, `thread/read`, `thread/resume`
- `turn/start`, `turn/steer`, `turn/interrupt`
- command execution, file change, permission approval request/response
- 현재 native normalizer가 지원하는 thread/turn/item/delta/error/warning notification

Compatibility test는 fixture의 method/field/enum 모양과 manifest의 selected schema
facts가 현재 native DTO 및 protocol mapper와 어긋나면 실패하도록 구성했다. 실험 API,
review/model/PTY 설계 후보는 이번 freeze에 포함하지 않았다.

## 4. 재시작과 recovery 시나리오

Fake app-server, 임시 app data, 실제 임시 Git repository를 사용해 UI 없이 두 개의
cross-layer 흐름을 고정했다.

### 정상 start/bind 뒤 재시작

```text
prepare
  → thread/start
  → durable binding commit
  → binding store reload
  → app-server stop/start(runtime generation 교체)
  → scoped list
  → resume
  → turn/start
```

- 재시작 뒤 preparation은 남지 않고 exact scope의 binding 하나가 유지된다.
- list/resume/turn은 caller path가 아니라 저장된 canonical execution root를 사용한다.
- turn sandbox writable root도 저장된 root 하나로 제한한다.
- thread를 다시 생성하지 않고 기존 ID/source/root를 이어 간다.

### uncertain start 뒤 exact recovery

```text
prepare
  → pre-start exact-root baseline 수집
  → Starting durable claim
  → thread/start가 server error로 끝나지만 thread는 생성됨
  → process/store reconstruction
  → persisted + loaded thread bounded discovery
  → baseline/bound thread 제외
  → exact threadSource 후보 read/resume 검증
  → durable binding commit
```

- 불확실한 `thread/start`는 자동 재전송하지 않는다.
- store reload 뒤에도 `Starting`과 recovery baseline이 유지된다.
- exact root와 `schoolx-code/<preparation-uuid>` source를 가진 유일 후보만 복구한다.
- read와 resume에서 ID/source/root를 다시 검증한 뒤에만 binding으로 교체한다.

테스트를 위해 Tauri wrapper 내부의 native 동작을 private synchronous core로 분리했지만
새 public command나 read/write capability는 추가하지 않았다. Test-only runtime executable
injection도 production API가 아니다.

## 5. 주요 파일

```text
desktop/src-tauri/src/code_workspace/
├── contract_tests.rs
├── fixtures/
│   ├── codex-0.145.0-schema-manifest.json
│   ├── codex-0.145.0-selected-schemas.tar.gz.base64
│   ├── codex-0.145.0-wire.json
│   ├── tauri-contract-v1.json
│   └── thread-bindings-v1.json
├── mod.rs
└── runtime.rs

desktop/src-tauri/src/commands/code_workspace.rs
```

Phase 1B의 binding/worktree/protocol 구현과 기존 Tauri registration은 그대로 권위 있는
production 경로다. Fixture는 그 경계를 복제하는 새 API가 아니라 변경 감지 기준이다.

## 6. 유지해야 할 불변식

- binding schema v1의 8개 필드와 public preparation scrub 경계를 임의로 확장하지 않는다.
- command 이름, camelCase DTO, event envelope, Codex method를 바꾸면 fixture와
  compatibility test도 같은 변경에서 갱신한다.
- frontend가 raw app-server JSON-RPC, child stdin, 임의 execution root, Git argv를 직접
  소유하지 않는다.
- caller path로 persisted binding root를 덮어쓰지 않는다.
- uncertain start를 자동 재시도하거나 `Starting`을 추측으로 해제하지 않는다.
- scope 없는 thread event를 frontend로 내보내지 않는다.
- worktree/thread를 자동 remove/reset/clean하지 않는다.
- Code transcript를 사용자 action 없이 relay/Nostr에 게시하지 않는다.
- production path에 새 `unsafe`, `unwrap()`, `expect()`를 추가하지 않는다.
- Codex 0.145.0 schema hash는 upstream compatibility 증거다. Runtime의 patch gate와
  혼동하지 않는다.

## 7. 검증 결과

Hermit 환경을 활성화하고 다음 native 검증을 완료했다.

```text
[x] cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
      code_workspace::contract_tests::
[x] cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
      code_workspace::contract_tests::refresh_schema_snapshot_is_manual_only \
      -- --ignored --exact
[x] cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
      commands::code_workspace::tests::native_binding_survives_process_restart_list_resume_and_turn
[x] cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
      commands::code_workspace::tests::uncertain_start_survives_store_reload_and_exact_recovery
[x] cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib
[x] cargo clippy --manifest-path desktop/src-tauri/Cargo.toml \
      --all-targets -- -D warnings
[x] cargo build --manifest-path desktop/src-tauri/Cargo.toml --bin buzz-desktop
[x] cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
[x] git diff --check
```

- contract fixture: 7 passed, 1 manual regeneration audit ignored
- exact 0.145.0 manual regeneration audit: 1 passed
- `code_workspace` 전체: 95 passed, 2 ignored
- desktop native library 전체: 2,175 passed, 16 ignored
- clippy/build/fmt/diff: passed

Git이나 hook을 실행하기 전에 `. ./bin/activate-hermit`를 사용한다. Desktop Tauri는
root workspace에서 제외되므로 native command에는 항상
`--manifest-path desktop/src-tauri/Cargo.toml`을 명시한다.

## 8. 아직 구현하지 않은 것

- React/TypeScript typed invoke/listen adapter와 pure reducer/query state
- SchoolX Code route와 화면/styling
- 작업 사이드바, timeline/composer, approval card, Changes/Files inspector
- PTY terminal drawer
- worktree rename/archive/remove와 safe cleanup UI
- branch/stage/commit/push/PR handoff UI
- Talk/Nostr 공유
- 새 Codex version 지원과 설치/업데이트 UX

기존 frontend 코드는 이번 단계에서 수정하지 않았다.

## 9. 권장 다음 범위: Phase 1D frontend state 계층

다음 세션은 UI를 그리기 전에 frozen native contract 위에 얇은 TypeScript 상태 경계를
구현한다.

1. 16개 Tauri command의 typed invoke adapter를 만든다.
2. `schoolx-code-workspace-event`의 typed listen/unlisten adapter를 만든다.
3. runtime generation + sequence를 기준으로 stale event를 거부하고 dedup하는 pure
   reducer를 만든다.
4. scoped binding/preparation list, runtime state, replay gap/truncation, pending approval을
   query/reducer state로 표현한다.
5. camelCase fixture를 TypeScript 쪽 compatibility test에서도 소비해 native drift를
   잡는다.
6. community remount 경계에 module-level cache를 추가한다면 명시적 reset을
   `resetCommunityState()`에 연결한다.

이번 Phase 1D에서도 화면, route, styling, terminal, worktree remove, Talk/Nostr 공유,
Git handoff, 새 Codex version 지원은 제외한다. 먼저 adapter와 pure state transition을
단위 테스트로 고정한 뒤 다음 UI 단계로 넘어간다.

## 10. 작업 트리 주의

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

기존 사용자 변경과 frontend 코드를 보존한다. Phase 1D도 현재 native 파일을 일괄
재작성하거나 Phase 1B/1C 안전 계약을 약화하지 않는다.

## 11. 새 세션 시작 문구

```text
docs/schoolx-2/SCHOOLX_CODE_DESIGN.md와
docs/schoolx-2/SESSION_HANDOFF_20260814_CODE_PHASE1C.md, 현재 작업 트리를 확인하고
SchoolX Code Phase 1D frontend state 계층을 구현해줘.

Phase 1B native binding/recovery/Git 안전 계약과 Phase 1C의 16개 Tauri camelCase DTO,
event envelope, store v1 public scrub, Codex 0.145.0 schema/wire fixture를 유지하고 기존
사용자 변경을 보존해줘. typed invoke/listen adapter와 runtime generation + sequence 기반
pure reducer/query state를 단위 테스트로 먼저 고정해줘. 이번 단계에서는 화면/route/
styling, terminal, worktree remove, Talk/Nostr, Git handoff, 새 Codex version 지원까지
넓히지 말아줘.
```
