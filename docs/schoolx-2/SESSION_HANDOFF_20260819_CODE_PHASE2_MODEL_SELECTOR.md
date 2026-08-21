# SchoolX Code Phase 2 model/reasoning selector 세션 인계

기준일: **2026-08-19**

## 1. 이번 slice 결과

Phase 2의 마지막 독립 제품 slice였던 Codex model/reasoning selector를 닫았다. Pinned
Codex 0.145 `model/list` 계약, native runtime authority, installation-global 최근 선택,
strict Tauri/TypeScript adapter와 keyboard-accessible header UI를 한 수직 경계로 연결했다.

- Runtime generation에 결박된 visible model catalog
- Catalog 전체 페이지를 읽은 뒤 한 번에 publish하는 bounded pagination
- 선택한 model이 광고한 open-string reasoning effort만 허용하는 native 검증
- 같은 runtime lock 안에서 mutation 검증부터 JSONL write까지 이어지는 TOCTOU 경계
- `thread/start|resume|fork` 응답의 authoritative model/nullable reasoning effort 복구
- 별도 `code/model-selection.json`에 저장하는 최근 UX 선택
- Model과 reasoning 두 Radix radio menu, keyboard focus 복귀와 busy-state `aria-disabled`
- Catalog 실패 시 composer를 막지 않는 Codex-default `null` override fallback
- Exact fixture, contract, unit/component와 fresh-build Playwright 회귀

이 slice는 binding index v4, fork의 frozen wire, safe-remove 입력/receipt/journal과 Git authority를
변경하지 않았다. Phase 2의 제품 slice는 여기까지 완료됐고 stage/unstage/commit/push/PR은 계속
Phase 3 범위다.

## 2. Pinned Codex 0.145 계약

새로 선택한 leaf schema는 다음 두 개다.

| Schema | Canonical SHA-256 |
|---|---|
| `v2/ModelListParams.json` | `35720f7fea38aedaa83f2d0ec4dd2dfd64385f1a30204927712a1a187471a2b3` |
| `v2/ModelListResponse.json` | `7d82fdd93beae12e546213628244a0b7123f94dc391b241e1bc790fdf400849d` |

선택 schema는 66개이며 aggregate는
`1ce5af96175ce83bb1d91db7939e8dcc243984255cf44777f19e58e0afe6549a`, leaf aggregate는
`b8d695b56e3ea5255857e2eb2dc9685d5ad65b735f276a5c743363d792677c73`다.

Native가 보내는 첫 요청은 정확히 다음 정책을 사용한다.

```text
model/list({ includeHidden: false, limit: 100 })
```

다음 page에는 opaque `cursor`만 추가한다. Caller/frontend는 cursor, limit, hidden policy를 받거나
결정하지 않는다. 응답 row의 핵심 pinned shape는 다음과 같다.

```text
{
  id,
  model,
  displayName,
  description,
  hidden,
  supportedReasoningEfforts: [{ reasoningEffort, description }],
  defaultReasoningEffort,
  isDefault
}
```

`id`와 실제 RPC에 쓰는 `model`은 같다고 가정하지 않는다. Reasoning effort는 enum이 아니라
trimmed non-empty open string이고 선택한 model의 현재 광고 목록으로만 검증한다.

`thread/start`와 `thread/resume`는 top-level `model` override를 받지만 reasoning effort 필드는 없다.
`turn/start`만 `model`과 `effort`를 함께 받으며, `turn/steer`는 둘 다 받지 않는다.
`thread/start|resume|fork` 성공 응답은 top-level `model`을 요구하고 `reasoningEffort`는
missing/null/non-null 모두 허용한다.

## 3. Public Tauri 계약

Catalog command는 argument가 없다.

```text
code_models_list() -> {
  runtimeGeneration,
  models: [{
    id,
    model,
    displayName,
    description,
    isDefault,
    defaultReasoningEffort,
    supportedReasoningEfforts: [{ reasoningEffort, description }]
  }],
  recentSelection: { model, reasoningEffort } | null
}
```

최근 선택 command는 정확히 한 input만 받는다.

```text
code_model_selection_set({
  input: { model, reasoningEffort }
}) -> { model, reasoningEffort }
```

Frontend adapter는 반환 선택이 요청 pair와 정확히 같은지도 다시 확인한다. Public
`CodeBoundThreadOpenResult`는 기존 thread payload에 `model`과 nullable `reasoningEffort`를 추가하며,
Codex thread-open 응답이 현재 thread의 effective selection authority다.

## 4. Native catalog와 mutation authority

Catalog는 최대 64 page, 전체 4,096 model, page당 100 model과 별도 string/effort/cursor cap을 둔다.
모든 page가 끝나기 전에는 결과를 publish하지 않으며 다음을 거부한다.

- Empty catalog, hidden row와 unknown wire field
- Duplicate `id`, duplicate `model`, model 안의 duplicate effort
- 지원 목록에 없는 default effort와 둘 이상의 default model
- Empty/control/oversize token, NUL/oversize description
- Repeated cursor, page/model/effort/cursor cap 초과

`CodeRuntime::model_catalog`는 runtime의 process mutex를 잡은 상태에서 모든 `model/list` request를
수행한다. Non-null `thread/start|resume|recover` model과 non-null `turn/start` pair도 같은 guard 안에서
fresh catalog를 읽고 `begin_request_with_delivery`가 JSONL을 쓸 때까지 lock을 유지한다. 따라서
frontend에서 미리 본 catalog는 UX 데이터일 뿐 mutation authority가 아니다.

`turn/start`는 `(model, effort)`가 둘 다 null이거나 둘 다 non-null이어야 한다. Non-null pair는 현재
선택 model이 광고한 exact effort여야 하고, 실패하면 turn request byte를 쓰지 않는다. Mutation timeout이나
response loss 뒤 `turn/start`를 model 선택 때문에 자동 retry하지 않으며 기존 uncertain-turn gate를 따른다.

Ordinary resume와 response-loss recovery는 `model:null`을 유지한다. Fork public input과 Codex wire에도
model을 추가하지 않는다. 대신 start/resume/fork 성공 응답의 `model`과 nullable `reasoningEffort`를
authoritative current-thread 값으로 사용한다. 이미 성공한 open 응답의 model이 새 catalog에서 hidden/retired된
경우 성공을 input failure로 재분류하지 않고 unavailable current value로 노출한다.

## 5. 최근 선택 persistence

최근 선택은 SchoolX app data의 versioned `code/model-selection.json`에 installation-global로 저장한다.
이는 새 작업의 UX default일 뿐 Codex thread authority가 아니다.

- Binding v4, lifecycle/removal journal 또는 Codex `config.toml`에 넣지 않는다.
- Strict `{version: 1, selection: {model, reasoningEffort}}`만 읽고 쓴다.
- 파일 0600, directory 0700, owner/symlink/regular-file/size 검사를 거친다.
- Temp file과 atomic replace로 저장한다.
- Catalog read 때 visible exact pair와 다시 대조하고 stale/unsupported 값은 `null`로 정규화한다.
- Picker 선택은 native catalog로 다시 검증한 뒤 저장한다. 저장 실패 시 UI는 이전 선택으로 rollback하고
  명시적 Retry를 제공한다.

## 6. Frontend 선택 상태와 UX

Catalog query key는 runtime-global
`["schoolx-code", "runtime", "models", runtimeGeneration]`이다. Runtime generation이 바뀌면 이전
catalog와 per-thread draft를 폐기하며, 응답 generation이 요청과 다르면 adapter/query가 거부한다.
Stale catalog refresh 중이나 error 상태에서는 cached row를 mutation에 사용하지 않고 fail closed한다.

Header의 `Model: …`과 `Reasoning effort: …` trigger는 Radix radio menu를 사용한다. Model menu는 display
name, plain-text description과 Default 표지를 보여주고 effort menu는 선택 model이 광고한 값만 보여준다.
알려진 effort는 사람이 읽기 좋은 label을 쓰되 unknown string도 그대로 보존한다. Model을 바꾸면 현재
effort가 새 model에서도 지원될 때 유지하고, 아니면 advertised default를 사용한다. Catalog validator가
default를 지원 목록 안으로 강제하므로 별도 hard-coded fallback은 없다.

선택 menu는 클릭만으로 app-server mutation을 보내지 않는다. 다음 idle `turn/start`에 exact
`catalog.model`과 effort를 함께 싣는다. Turn pending/active 동안 trigger는 mounted/focusable 상태를 유지하되
`aria-disabled`이고 다음 turn부터 바꿀 수 있다는 안내를 제공한다. Menu selection/Escape는 정확한 trigger로
focus를 복원하고 async catalog load/thread switch는 focus를 빼앗지 않는다.

Thread 흐름은 다음과 같다.

1. 새 작업은 검증된 최근/default model을 `thread/start`에 보낼 수 있고 effort는 첫 `turn/start`까지 pending이다.
2. Existing resume와 response-loss recovery는 `model:null`로 열고 응답 effective pair로 selector를 seed한다.
3. Fork는 frozen five-key Codex params를 유지하고 응답의 inherited pair로 seed한다.
4. Open 값이 unavailable/effort-unknown이면 그대로 표시하지만 untouched 상태에서는 `turn/start` override를
   `null/null`로 보낸다. 사용자가 visible pair를 명시적으로 선택하면 그때부터 exact pair를 보낸다.
5. Catalog가 load 실패/empty/malformed이면 inline alert와 Retry를 보여주되 composer와 새 작업은 계속
   사용할 수 있고 `thread/start` model 및 `turn/start` pair는 null fallback이다.

## 7. 보존한 frozen 경계

- Public fork input은 계속 `{input:{scope,threadId}}`다.
- Pinned `thread/fork` params는 기존 다섯 key이고 model/effort를 추가하지 않았다.
- Ordinary resume/recovery는 model override를 null로 유지한다.
- Binding index v4와 public binding 8-field shape는 그대로다.
- `code_worktree_remove` input은 exact `{input:{scope,threadId}}`다.
- Safe-remove 성공은 계속 native-derived 9-field receipt다.
- Removal eligibility, journal, proof ref, quarantine, transcript `preserved`, authoritative reconciliation을
  변경하지 않았다.
- Git path/ref/OID/proof/removal ID와 model catalog cursor는 caller authority가 아니다.

## 8. 주요 변경 파일

Native/runtime:

- `desktop/src-tauri/src/code_workspace/model_catalog.rs`
- `desktop/src-tauri/src/code_workspace/model_catalog/tests.rs`
- `desktop/src-tauri/src/code_workspace/runtime.rs`
- `desktop/src-tauri/src/code_workspace/protocol.rs`
- `desktop/src-tauri/src/commands/code_workspace.rs`
- `desktop/src-tauri/src/app_state.rs`
- `desktop/src-tauri/src/lib.rs`

Contract/fixtures:

- `desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-schema-manifest.json`
- `desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-selected-schemas.tar.gz.base64`
- `desktop/src-tauri/src/code_workspace/fixtures/codex-0.145.0-wire.json`
- `desktop/src-tauri/src/code_workspace/fixtures/tauri-contract-v1.json`
- `desktop/src-tauri/src/code_workspace/contract_tests.rs`

Frontend/product:

- `desktop/src/features/code/api/codeModelSchemas.ts`
- `desktop/src/features/code/api/codeWorkspace.ts`
- `desktop/src/features/code/api/schemas.ts`
- `desktop/src/features/code/api/types.ts`
- `desktop/src/features/code/lib/codeModelSelection.ts`
- `desktop/src/features/code/state/codeSessionQueries.ts`
- `desktop/src/features/code/state/useCodeModelSelection.ts`
- `desktop/src/features/code/state/useCodeChangesInvalidation.ts`
- `desktop/src/features/code/ui/CodeModelSelector.tsx`
- `desktop/src/features/code/ui/CodeWorkspaceHeader.tsx`
- `desktop/src/features/code/ui/CodeWorkspaceScreen.tsx`
- `desktop/src/testing/e2eBridge.ts`
- `desktop/tests/e2e/schoolx-code.spec.ts`
- 관련 contract/helper/query/component tests

Normative docs:

- `docs/schoolx-2/SCHOOLX_CODE_DESIGN.md`
- 이 handoff

## 9. 검증

Hermit 활성화 뒤 다음 native gate가 통과했다.

```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
cargo check --manifest-path desktop/src-tauri/Cargo.toml --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --lib -- -D warnings

cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::model_catalog::tests --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  turn_selection_lists_before_write_and_rejects_unadvertised_pair_without_turn_bytes \
  --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  thread_open_preserves_nullable_reasoning_effort --lib -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  code_workspace::contract_tests --lib -- --nocapture
```

Model catalog/store 11개, runtime selection wire 1개, open-response nullable effort 1개가 통과했다.
Contract suite는 8개 통과, pinned CLI가 필요한 manual audit 1개 ignored다. 별도 focused runtime
reconnect/recovery와 fork 테스트도 통과했다.

Frontend는 다음을 검증했다.

```bash
pnpm --dir desktop typecheck
pnpm --dir desktop check:px-text
pnpm --dir desktop test
pnpm --dir desktop exec biome check <이번 selector 관련 파일>
pnpm --dir desktop exec node --import ./test-loader.mjs \
  --experimental-strip-types --test \
  src/features/code/api/codeWorkspace.contract.test.mjs \
  src/features/code/lib/codeModelSelection.test.mjs \
  src/features/code/state/codeSessionQueries.test.mjs \
  src/features/code/ui/CodeModelSelector.test.mjs
pnpm --dir desktop build:e2e
pnpm --dir desktop exec playwright test tests/e2e/schoolx-code.spec.ts \
  --project=smoke --grep 'persists model and reasoning|falls back to null model'
```

Targeted frontend suite 42/42, 전체 desktop unit suite 4,021/4,021와 fresh `build:e2e` 기반
focused Playwright 2/2가 통과했다. E2E는
`id != model`, keyboard focus 복귀, 저장 실패 rollback/Retry, active-turn lock, exact model/effort payload,
catalog Retry 재실패 뒤 null fallback을 포함한다. Screenshot은 생성하지 않았다.

`check:file-sizes`는 이 dirty Phase 2 작업 트리에 이미 누적된 여러 1,000줄 초과 파일 때문에 전체로는
실패한다. 이번 slice에서 새로 추가한 model catalog와 model schema는 하위 모듈로 분리했고 selector
screen/component/helper는 각각 1,000줄 이하를 유지했다. 전체 `just ci`와 infrastructure integration
suite는 이 targeted handoff 범위에서 실행하지 않았다. `git diff --check`와 fixture JSON/gzip 검증은
통과했다.

## 10. 작업 트리 주의

Repository는 시작부터 다수의 tracked/untracked 사용자 변경을 포함한 dirty worktree였다. 이 slice는
관련 파일만 수정했고 stage/commit/reset/clean하지 않았다. 다음 세션도 첫 명령을
`. ./bin/activate-hermit && git status --short`로 실행하고 기존 변경을 보존해야 한다.

## 11. 다음 독립 slice

설계 문서에 남은 다음 제품 단계는 **Phase 3 Git handoff와 Talk 공유**다. 첫 독립 slice는 exact-bound
Git stage/unstage/commit 경계를 먼저 설계하는 편이 안전하다. Branch/push/PR과 Talk transcript 공유를
한꺼번에 열지 않는다.

구체적인 다음 task 계약과 완료 조건은
[`SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md`](SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md)에
이어 적었다. 다음 구현 세션은 이 새 문서를 시작점으로 사용한다.

Phase 3에서도 safe-remove proof/receipt를 Git write authority로 재사용하지 않는다. Caller-supplied cwd,
path, ref, OID, merge proof와 shell argv를 새 mutation authority로 받지 않으며, exact persisted binding과
native repository revalidation에서 새로 권한을 도출해야 한다. Model selector와 Git mutation도 결합하지 않는다.

## 12. 다음 세션 복사용 시작 요청

```text
SESSION_HANDOFF_20260820_CODE_PHASE3_GIT_WRITE.md를 먼저 읽고,
SCHOOLX_CODE_DESIGN.md, SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md와
SESSION_HANDOFF_20260819_CODE_PHASE2_MODEL_SELECTOR.md를 대조한 뒤 현재 작업 트리를 확인해줘.
첫 명령은 `. ./bin/activate-hermit && git status --short`로 실행해줘.

Phase 2 model/reasoning selector와 public safe-remove는 완료 상태로 보존해줘. Runtime-generation catalog,
native same-lock pair validation, thread-open authority, null fallback, installation-global recent preference,
fork five-key wire, exact safe-remove input과 9-field receipt를 바꾸지 마.

다음 독립 Phase 3 slice로 exact-bound Git stage/unstage/commit authority와 UX를 먼저 설계·구현해줘.
Branch/push/PR과 Talk 공유는 별도 slice로 남기고, caller path/ref/OID/shell authority를 열지 마.
기존 사용자 변경/untracked 파일을 보존하고 stage/commit/reset/clean하지 마.
```
