# SchoolX Code Phase 1D 인계

작성일: 2026-08-14
상태: Phase 1D frontend adapter/state 계층 구현 완료, UI 시작 전 세션 경계

## 구현 결과

Phase 1C에서 고정한 16개 Tauri command와
`schoolx-code-workspace-event`를 React/TypeScript가 안전하게 소비하는 경계를 추가했다.

- `api/types.ts`, `api/schemas.ts`: camelCase DTO와 strict Zod decoder
- `api/codeWorkspace.ts`: 16개 typed invoke wrapper, typed listen/unlisten, replay와 live
  event의 race-free merge
- `state/codeSessionReducer.ts`: scope-owned pure reducer와 selector
- `state/codeSessionQueries.ts`: runtime, preparation, binding/thread, worktree status의
  scalar React Query key/options
- native `tauri-contract-v1.json`과 `codex-0.145.0-wire.json`을 직접 읽는 frontend
  compatibility test

module-level community cache는 추가하지 않았다. 상태는 scope마다
`createCodeSessionState()`로 생성하므로 `resetCommunityState()` 변경도 없다.

## 유지해야 할 상태 계약

- listener를 먼저 등록하고 live event를 buffer한 뒤 replay를 적용한다.
- subscription epoch가 다른 live/replay action은 거부한다.
- native sequence는 desktop-wide global sequence다. 숫자 점프만으로 gap을 만들지 않는다.
- generation 교체가 replay 중 관측되면 새 generation의 sequence 0부터 다시 replay한다.
- full replay는 event-derived state를 항상 다시 만든다.
- 실제 truncation이나 frontend buffer overflow는 같은 generation에서 sticky incomplete다.
  검증된 full replay 또는 generation 교체 전에는 일반 incremental replay가 완전 상태로
  바꾸지 않는다.
- malformed, pre-cursor, mixed-generation replay batch는 fail closed한다.
- pending approval은 generation, request ID의 JSON type, exact scope, thread, turn을
  identity로 사용하며 item/sequence도 stale UI action을 막는 데 사용한다.
- numeric request ID와 generation/sequence는 JavaScript safe integer 범위 밖이면 decoder가
  거부한다.
- non-ready runtime은 transient active turn/approval을 허가하지 않는다. 같은 generation의
  늦은 status 응답은 caller가 부여한 monotonic revision으로 거부한다.
- AbortSignal, bounded buffer, once-only cleanup, fatal decode error propagation을 유지한다.

## 주요 파일

```text
desktop/src/features/code/
├── api/
│   ├── codeWorkspace.ts
│   ├── codeWorkspace.contract.test.mjs
│   ├── schemas.ts
│   └── types.ts
└── state/
    ├── codeSessionQueries.ts
    ├── codeSessionQueries.test.mjs
    ├── codeSessionReducer.ts
    └── codeSessionReducer.test.mjs
```

## 검증 결과

Hermit 환경에서 다음을 확인했다.

```text
[x] pnpm exec biome check src/features/code
[x] pnpm typecheck
[x] Phase 1D focused tests: 32 passed
[x] pnpm test: 3,967 passed
[x] pnpm build
[x] git diff --check
```

`pnpm check` 전체는 Phase 1D 밖의 기존 미포맷 native JSON fixture 3개 때문에 Biome
단계에서 실패한다. `personaCatalogRelay.test.mjs`의 기존 template-literal 제안 2개도
출력되지만 info다. Phase 1D의 8개 파일은 Biome을 통과하며 모두 1000줄 이하다.
전체 file-size gate를 별도로 실행하면 Phase 1 native 대형 파일과 기존 ratchet 증가가
남아 있다. 이번 단계에서 그 파일을 재포맷하거나 분할하지 않았다.

## 다음 범위

다음 단계는 이 계층 위에 SchoolX Code UI shell을 연결하는 것이다. 프로젝트 문맥의
route/진입점, runtime 상태, scoped preparation/thread 목록, 선택한 thread timeline의
최소 수직 슬라이스부터 시작한다. approval card는 reducer의 exact pending identity와
generation/sequence/item guard를 그대로 사용해야 한다.

아직 범위 밖인 항목은 terminal drawer, worktree remove/cleanup, Talk/Nostr 공유, Git
handoff, 새 Codex version 지원이다. 기존 native Phase 1B/1C 안전 계약과 현재 사용자
변경은 보존한다.
