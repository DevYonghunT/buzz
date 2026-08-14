# SchoolX Code Phase 1E 인계

작성일: 2026-08-14
상태: Phase 1E UI shell과 최소 thread 수직 슬라이스 구현 완료, Phase 1F 시작 전 세션 경계

## 1. 새 세션에서 먼저 읽을 문서

아래 순서로 문맥을 복구한다.

1. [`SCHOOLX_CODE_DESIGN.md`](SCHOOLX_CODE_DESIGN.md)
2. 이 문서
3. frontend state 계약의 상세 근거가 필요할 때만
   [`SESSION_HANDOFF_20260814_CODE_PHASE1D.md`](SESSION_HANDOFF_20260814_CODE_PHASE1D.md)
4. native binding, recovery, Git 안전 경계가 필요할 때만
   [`SESSION_HANDOFF_20260814_CODE_PHASE1C.md`](SESSION_HANDOFF_20260814_CODE_PHASE1C.md)와
   [`SESSION_HANDOFF_20260814_CODE_PHASE1B.md`](SESSION_HANDOFF_20260814_CODE_PHASE1B.md)

Phase 1D 인계의 “16개 Tauri command”와 “UI 시작 전”은 당시의 역사적 상태다.
현재는 exact repository scope를 UI에서 만들기 위한 읽기 전용
`code_repository_inspect`가 추가되어 public command가 17개이며, 아래 UI가 그 typed
adapter/state 위에 연결되어 있다.

## 2. 이번 단계에서 완료한 것

- 프로젝트 상세에 `Code` 진입점을 추가했다.
- `/projects/$projectId/code` route를 추가하고 `baseRef`, `threadId`를 typed search
  parameter로 유지한다.
- native가 canonical Git common-dir에서 만든 SHA-256 repository identity를 반환하는
  읽기 전용 `code_repository_inspect` command를 추가했다. UI는 caller path나 자체
  hashing으로 scope identity를 추측하지 않는다.
- community ID + project dtag + native repository identity로 exact scope를 만들고,
  scope key가 바뀌면 Code subtree를 remount한다.
- Code 화면 최초 진입 시 runtime을 lazy start하고 status, retry, full replay action을
  제공한다.
- exact scope의 preparation과 metadata-only thread 목록을 표시한다.
- prepared/recovery 항목과 새 managed-worktree task 생성을 연결했다.
- route에서 선택한 thread를 먼저 exact resume한 뒤에만 historical/live event를
  timeline으로 투영한다.
- user message, public reasoning summary, plan, command output, file-change summary,
  warning/error, turn status를 deterministic semantic timeline으로 표시한다.
- prompt start, active turn steer, interrupt와 inline approval response를 연결했다.
- raw JSON, full patch, private reasoning은 기본 timeline에 노출하지 않는다.

주요 frontend 파일은 다음과 같다.

```text
desktop/src/features/code/
├── api/
├── state/
│   ├── codeSessionReducer.ts
│   ├── codeSessionQueries.ts
│   └── codeSessionStore.ts
├── lib/
│   ├── codeTimeline.ts
│   └── codeWorkspaceView.ts
└── ui/
    ├── ProjectCodeScreen.tsx
    ├── CodeWorkspaceScreen.tsx
    ├── CodeRuntimeStatus.tsx
    ├── CodeThreadSidebar.tsx
    ├── CodeTimeline.tsx
    ├── CodeComposer.tsx
    └── CodeApprovalCard.tsx
```

## 3. 유지해야 할 Phase 1D 계약과 UI race 방어

- listener 등록, live buffer, replay 적용 순서를 바꾸지 않는다.
- subscription epoch가 다른 live/replay action은 거부한다.
- native sequence는 desktop-wide global sequence이므로 숫자 점프만으로 gap을 만들지
  않는다.
- generation이 바뀌면 새 generation의 sequence 0부터 full replay하고, 이전 generation의
  resume/turn 결과와 UI state write를 버린다.
- 실제 truncation 또는 frontend buffer overflow는 검증된 full replay 전까지 sticky
  incomplete다.
- malformed, pre-cursor, mixed-generation replay는 fail closed한다.
- subscription decode/error 상태에서는 composer와 approval action을 허용하지 않는다.
- pending approval identity는 generation, JSON type을 보존한 request ID, exact scope,
  thread, turn, item, sequence를 모두 사용한다.
- 빠른 thread 전환은 resume을 직렬화하며, 먼저 선택된 thread의 늦은 응답이 현재 route를
  덮지 못한다.
- `turn/start` 응답 전에는 provisional pending turn을 사용한다. 이 사이의 두 번째
  prompt는 새 turn이 아니라 응답으로 받은 exact turn ID에 steer된다.
- turn event가 API 응답보다 먼저 도착하거나 이미 완료된 경우 늦은 응답으로 stale
  pending UI를 되살리지 않는다.
- module-level community cache를 추가하지 않았다. scope-owned store를 유지한다.

## 4. E2E 계약

mock bridge의 `schoolxCodeWorkspace` option은 다음을 제공한다.

- stopped → ready lazy runtime transition
- empty replay와 exact repository/scope 응답
- prepared row와 metadata-only thread list
- explicit thread resume 뒤 historical timeline hydration
- turn start와 event ACK 전 exact steer

`desktop/tests/e2e/schoolx-code.spec.ts`는 프로젝트 Code 진입, route/search 유지,
runtime start/replay, exact scope payload, preparation 표시, thread resume/timeline,
turn start/steer를 검증한다.

UI E2E는 반드시 `pnpm build:e2e`로 빌드한다. `pnpm run build`를 쓰지 않는다.

## 5. 검증 결과

Hermit 환경에서 다음을 확인했다.

```text
[x] Phase 1E 대상 Biome check
[x] pnpm typecheck
[x] Code frontend focused tests: 41 passed
[x] pnpm check:px-text
[x] cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
[x] cargo test --manifest-path desktop/src-tauri/Cargo.toml \
      code_workspace::contract_tests
    → 7 passed, 1 manual test ignored
[x] cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib -- -D warnings
[x] pnpm build:e2e + focused schoolx-code smoke spec
[x] 같은 E2E artifact로 3회 반복: 3 passed
[x] git diff --check
```

구현 검증 당시 전체 `pnpm check`는 Phase 1C에서 추가된 아래 native JSON fixture
3개의 기존 Biome format 문제 때문에 실패했다.

```text
src-tauri/src/code_workspace/fixtures/codex-0.145.0-schema-manifest.json
src-tauri/src/code_workspace/fixtures/codex-0.145.0-wire.json
src-tauri/src/code_workspace/fixtures/tauri-contract-v1.json
```

커밋 pre-hook의 `desktop-fix`가 위 3개 fixture를 자동 포맷했고, 이후 frontend contract
41개와 native contract 7개가 다시 통과했다. 따라서 다음 checkout에서는 이 fixture
format 문제가 더 이상 blocker가 아니다. `personaCatalogRelay.test.mjs`의 기존
`useTemplate` 제안 2개는 info다.

전체 `pnpm check`는 이제 file-size 단계에서 Phase 1 native 대형 파일과 기존 ratchet
증가 때문에 실패한다. Phase 1E UI는 새 file-size 위반을 추가하지 않았다. 이 known
failure를 Phase 1E UI 회귀로 오인하거나 무관한 대형 파일을 다음 단계에서 재포맷/분할하지
않는다.

Git이나 검증 명령을 실행하기 전에 항상 다음을 실행한다.

```bash
. ./bin/activate-hermit
```

## 6. 다음 범위: Phase 1F

Phase 1F는 Phase 1 완료 조건 중 아직 얇은 부분만 보강한다.

1. normalized mock event로 plan/command/file change/approval/turn-complete timeline을
   E2E에서 검증한다.
2. exact approval의 accepted/declined/stale-disabled 흐름과 runtime crash/recovery 또는
   수동 full replay 흐름을 E2E로 고정한다.
3. 기존 project Git read API를 재사용해 선택 thread의 execution root에 대한 읽기 전용
   `Changes` inspector를 최소 범위로 연결한다. stage/commit/push/PR action은 넣지 않는다.
4. 800×500 narrow layout, keyboard-only, light/dark, reduced-motion 가운데 실제 회귀 위험이
   큰 상태를 focused E2E로 추가한다.

Phase 1F에서도 typed Tauri adapter, exact scope, subscription epoch/replay, pure reducer,
pending approval identity를 우회하지 않는다. raw app-server payload를 UI가 직접 읽거나
별도 thread/approval state를 복제하지 않는다.

계속 범위 밖인 항목은 다음과 같다.

- terminal drawer와 PTY command
- worktree rename/archive/remove/cleanup
- Talk/Nostr 공유
- stage/commit/push/PR Git handoff
- review/start, model/list와 새 Codex version 지원

## 7. 작업 트리 주의

SchoolX Code와 무관한 기존 사용자 변경이 함께 있다. 특히 `.dockerignore`,
`.gitignore`, `crates/buzz-core/src/relay.rs`, `deploy/`, `brand/`, `supabase/`,
`managed_agents/restore.rs`, `managed_agents/runtime.rs`,
`managed_agents/runtime/tests.rs`, `managed_agents/runtime_commands.rs`는 별도 작업으로
간주한다. 다음 세션도 이를 reset, 재포맷, 스테이징하거나 SchoolX Code 변경과 섞지
않는다.
