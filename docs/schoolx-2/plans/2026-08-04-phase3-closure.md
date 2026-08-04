# Phase 3 닫기 구현 계획 (세션 D2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 세션 D가 넘긴 1–3을 닫아 [`DEVELOPMENT_PLAN.md`](../DEVELOPMENT_PLAN.md) Phase 3의 완료 기준 7개 중 부분 충족인 셋(#3 `renamed`, #6 upgrade 경로, #7 UI 실행 증거)을 충족으로 올린다.

**Architecture:** 새 동작을 만들지 않는다. 셋 다 **이미 옳게 도는 것을 관측 가능하게** 만드는 작업이다 — `renamed`는 preflight에만 있던 값을 ledger 와이어 포맷에 싣고, upgrade 경로는 "v2를 v1 위에 돌려도 캔버스를 다시 쓰지 않는다"를 fixture로 고정하고, 카드는 mock bridge에 두 command를 더해 Playwright가 상태를 렌더할 수 있게 한다.

**Tech Stack:** Rust 1.95 / `schoolx-catalog` 크레이트 / Tauri 2 / React 19 / Playwright

> **실행 완료 (2026-08-04).** Task 1–4 전부 실행했다. 게이트는
> [`BASELINE.md`](../BASELINE.md) 세션 D2 절, 결과 서술은
> [`IMPLEMENTATION_HANDOFF.md`](../IMPLEMENTATION_HANDOFF.md) 세션 D2 절.
> 커밋 `ad9b9b2c`·`7933d0c5`·`f2008b4d`.
>
> **계획과 다르게 한 것 셋.**
>
> 1. **Task 2의 재주입 하나가 계획대로 되지 않았다.** 「캔버스 가드를 제거하면
>    이 테스트가 실패한다」를 예상했으나 실패하지 않았다 — 판정이 `no_change`라
>    saga가 캔버스 단계 앞에서 반환하기 때문이다. 대신 도출식 주입을 버전이
>    다를 때만 물게 좁혀 단독 방어선임을 보였다. 테스트 주석에 그 사실을 적었다.
> 2. **Task 3의 `installMockBridge` 호출 모양이 계획과 다르다.** 계획은
>    `{ mock: {...} }`로 감쌌으나 실제 2번째 인자가 곧 `MockBridgeOptions`라
>    평평하게 넘긴다.
> 3. **Task 3에 카드 소스 변경이 하나 늘었다.** 이름 변경 배지에 `data-testid`를
>    붙였다 — 문구로 단언하면 로케일과 번역 수정에 묶이는데, 지키려는 것은
>    문구가 아니라 표시가 나온다는 사실이다.

## Global Constraints

- 작업 위치는 **메인 체크아웃** `/Users/kim-yonghun/Development/schoolX_v2.0`, 브랜치 `codex/schoolx-2-foundation`. 워크트리에서는 `just desktop-tauri-fmt`가 실패해 pre-commit이 막힌다.
- 시작 전 `. ./bin/activate-hermit`.
- `unsafe` 금지. 프로덕션 경로에 새 `unwrap()`/`expect()` 금지 — `?`와 에러 타입을 쓴다.
- 새 public API에는 doc comment를 단다.
- 데스크톱 텍스트 크기는 rem 토큰만 (`text-base`, `text-sm`, `text-xs`, `text-2xs`, `text-3xs`). 임의 리터럴은 `pnpm check:px-text`가 막는다.
- i18n 키를 더할 때는 `en`, `ko`를 **한 번에** 바꾼다. 한쪽만 바꾸면 `fallbackLng`가 구제하지 못하고 한국어에 원시 키가 노출된다.
- **`steps`에 값을 더하지 않는다.** 이 계획은 `StepStatus` 어휘를 바꾸지 않는다. `LedgerItem`에 필드를 하나 더하지만 그건 `steps` 안이 아니다 — 아래 Task 1의 「왜 안전한가」를 읽는다.
- 파일 1000줄 상한. 걸리면 한계를 올리지 말고 줄인다.
- Playwright 스펙은 **반드시** `pnpm build:e2e`로 빌드된 번들에서 돈다. `pnpm run build`는 mock bridge를 떼어내므로 모든 mock 스펙이 `Cannot read properties of undefined (reading 'invoke')`로 죽는다. `pnpm test:e2e:smoke`를 쓰면 올바른 빌드를 대신 해 준다.
- 스펙에서 `page.screenshot()`이나 `locator.screenshot()` 앞에는 `waitForAnimations(page)`를 부른다.
- 근거 문서: [`WORKSPACE_CATALOG.md`](../WORKSPACE_CATALOG.md) §6·§7·§10, [`IMPLEMENTATION_HANDOFF.md`](../IMPLEMENTATION_HANDOFF.md) 세션 D 「넘긴 것」 1–3.

## 시작 상태 (2026-08-04, 커밋 `9ed8b3b4`)

- `PreflightItem::renamed`는 있고(`preflight.rs:73`) `LedgerItem`에는 없다.
- `catalog_version`은 `Provenance`와 `Ledger`에 **기록만** 되고 판정에서 읽히지 않는다. `derive_channel_id`가 버전을 입력에서 제외하므로(`channel_id.rs:20`) v2도 같은 채널 ID를 낸다 — 그 동작이 옳다는 판단은 서 있으나 **고정하는 테스트가 없다**.
- 카드(`WorkspaceCatalogSettingsCard.tsx`, 395줄)에는 `data-testid` 10개가 이미 붙어 있으나 이를 렌더하는 테스트가 데스크톱 단위 3,929개에도 Playwright 스펙에도 **하나도 없다**.
- **mock bridge에 `preflight_workspace_catalog`·`apply_workspace_catalog` 핸들러가 없다.** 핸드오프가 「스펙 하나로 닫힌다」고 본 것보다 비용이 크고, Task 3이 이 차이를 흡수한다. (`apply_workspace`는 커뮤니티 전환용 별개 command다 — 혼동하지 않는다.)
- 섹션은 `featureGate`가 아니라 **역할 검사**로 가려진다. `SettingsView.tsx`의 `visibleSections`가 `canManageCommunityMembers(...) || membershipRequired === false`를 본다. E1 계획서의 "섹션에 featureGate를 건다"는 구현 중에 뒤집혔고 이유가 `SettingsPanels.tsx:205` 주석에 있다 — 매니페스트에 없는 ID는 fail-open이라 게이트가 무음 no-op이 된다.

---

## File Structure

| 파일 | 책임 |
|---|---|
| `crates/schoolx-catalog/src/ledger.rs` | `LedgerItem`에 `renamed` 필드 |
| `crates/schoolx-catalog/src/saga.rs` | 생성 지점 5곳에 `renamed` 채우기, golden 갱신, v2 upgrade 테스트 |
| `desktop/src/shared/api/tauriWorkspaceCatalog.ts` | `CatalogLedgerItem`에 `renamed` |
| `desktop/src/features/settings/ui/WorkspaceCatalogSettingsCard.tsx` | 이름 변경 배지의 `data-testid` |
| `desktop/src/testing/e2eBridge.ts` | 두 catalog command의 mock 핸들러와 설정 노브 |
| `desktop/tests/helpers/bridge.ts` | 노브의 헬퍼 측 타입 |
| `desktop/tests/e2e/workspace-catalog.spec.ts` | **신규** — 카드 상태 렌더 스펙 |
| `desktop/playwright.config.ts` | `smoke` 프로젝트에 스펙 등록 |
| `docs/schoolx-2/{BASELINE,IMPLEMENTATION_HANDOFF,WORKSPACE_CATALOG}.md` | 결과 기록과 Phase 3 판정 갱신 |

---

## Task 1: `renamed`를 ledger 와이어 포맷에 싣는다

Phase 3 완료 기준 #3의 부족분이다. 추적 자체는 이미 증명돼 있고(`rename_is_a_flag_not_a_decision`, `renamed_complete_item_is_no_change`), §7이 요구한 "ledger에도 표시"만 없다.

**Files:**
- Modify: `crates/schoolx-catalog/src/ledger.rs:41-68`
- Modify: `crates/schoolx-catalog/src/saga.rs` (생성 지점 5곳 + golden)
- Modify: `desktop/src/shared/api/tauriWorkspaceCatalog.ts:84-95`

**Interfaces:**
- Consumes: `PreflightItem::renamed` (`preflight.rs:73`)
- Produces: `LedgerItem::renamed: bool`, TS `CatalogLedgerItem.renamed: boolean`

**왜 안전한가.** 세션 D 사실 6번의 "읽기 쪽 breaking change"는 **`steps`의 값**(`StepStatus` 어휘)에 대한 것이다. 그건 relay에 저장된 provenance를 **구버전이 읽다가** 파싱에 실패해 "적용한 적 없음"으로 오해하는 경로였다. `Ledger`는 다르다 — relay에 저장되지 않고 `apply_workspace_catalog`의 반환값으로만 산다. 생산자와 소비자가 같은 빌드 안에 있으므로 버전 skew가 없다. 그래서 필드 추가가 조용한 실패를 만들지 않는다.

**카드는 바꾸지 않는다.** 카드의 각 행은 항상 preflight 항목을 손에 쥐고 있고 이미 `item.renamed`로 배지를 그린다(`WorkspaceCatalogSettingsCard.tsx:329`). ledger 쪽 값을 UI에 또 그리면 같은 사실이 두 번 나온다. 이 필드가 필요한 소비자는 **ledger만 읽는 쪽**이다 — 아직 없는 CLI 적용 경로가 그것이고, `ledger_serializes_for_ui_and_cli`가 고정하는 와이어 포맷이 그 경로를 위한 것이다.

- [x] **Step 1: 실패하는 테스트를 쓴다**

`saga.rs`의 테스트 모듈에 추가한다. `rename_is_a_flag_not_a_decision`(`preflight.rs`)이 preflight 쪽을 이미 덮으므로, 여기서는 **그 값이 ledger까지 살아 오는지**만 본다.

```rust
    #[tokio::test]
    async fn renamed_survives_into_the_ledger() {
        let fx = FakeEffects::new();
        // 1차 적용으로 방을 만든다.
        apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first apply");

        // 팀이 방 이름을 바꿨다. catalog 표시 이름과 달라진다.
        let channel_id =
            derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1);
        for channel in fx.channels.lock().expect("lock").iter_mut() {
            if channel.id == channel_id {
                channel.name = "월요 정례".into();
            }
        }

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("second apply");

        let entry = item(&ledger, "meeting");
        assert!(
            entry.renamed,
            "이름이 바뀐 방인데 ledger가 그 사실을 싣지 않았다"
        );
        // `name`은 여전히 catalog 표시 이름이다 (§10). 그래서 `renamed`가
        // 없으면 ledger만 읽는 소비자는 이 방의 현재 이름을 알 방법이 없다.
        assert_eq!(entry.name.as_deref(), Some("메인 회의방"));
    }
```

- [x] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog renamed_survives_into_the_ledger`
Expected: FAIL — `LedgerItem`에 `renamed`가 없어 컴파일 에러

- [x] **Step 3: 필드를 더한다**

`ledger.rs`의 `LedgerItem`에서 `error` 필드 **앞**에 넣는다. 순서는 golden JSON의 키 순서와 함께 봐야 하므로 아래 Step 5와 맞춘다.

```rust
    /// 이 방의 현재 이름이 catalog 표시 이름과 다르다.
    ///
    /// `name`은 언제나 catalog가 정한 표시 이름이다(§10) — 그 방의 실제
    /// 이름이 아니다. 그래서 ledger만 읽는 소비자는 이 플래그 없이는
    /// `name`이 현재 이름인지 알 방법이 없다. 이름이 바뀐 방은 `adopted`로
    /// 끝나므로 실제로 도달하는 상태다.
    ///
    /// 판정이 아니라 표시용 플래그다 — 이름이 바뀌었다는 사실만으로
    /// `decision`이 달라지지는 않는다
    /// (`preflight::rename_is_a_flag_not_a_decision`).
    pub renamed: bool,
```

- [x] **Step 4: 생성 지점 다섯 곳을 채운다**

`saga.rs`의 `apply_item`(`plan: PreflightItem`이 스코프에 있다) 안 다섯 곳 전부에 `renamed: plan.renamed,`를 더한다. 줄번호는 편집하면서 밀리므로 **컴파일러가 남은 자리를 알려준다** — 필드가 빠진 구조체 리터럴은 빌드가 실패한다.

대상: `blocked` 클로저, `Retired | NoChange` 분기, `Deleted` 분기, `NotOwned` 분기, 그리고 함수 말미의 성공 경로.

지어내지 않는다 — 다섯 곳 모두 `plan.renamed`를 그대로 쓴다. `preflight`가 이미 판정한 값이고, saga가 그것을 다시 계산할 근거를 갖고 있지 않다.

- [x] **Step 5: golden을 갱신한다**

`ledger_serializes_for_ui_and_cli`의 손으로 만든 `LedgerItem` 9개와 그에 대응하는 `expected` JSON 9개 모두에 값을 더한다.

- `ops`(`decision: "adopted"`) 항목만 `renamed: true` / `"renamed": true`.
- 나머지 여덟(`meeting`, `planning`, `finance`, `hr`, `sales`, `library`, `notices`, `clubs`)은 `renamed: false` / `"renamed": false`.

`adopted`를 참으로 두는 이유는 그것이 이 플래그가 **실제로 도달하는** 상태이기 때문이다. 전부 `false`인 golden은 필드가 직렬화된다는 것만 보이고 `true`가 어떻게 나가는지는 고정하지 못한다.

JSON에서 키 위치는 구조체 필드 순서를 따른다 — `user_action` 다음, `error` 앞이다. Step 3에서 정한 위치와 어긋나면 `serde_json::to_value` 비교는 통과하지만(객체 비교라 순서 무관) 사람이 읽을 때 헷갈리므로 맞춰 둔다.

- [x] **Step 6: TS 타입을 맞춘다**

`tauriWorkspaceCatalog.ts`의 `CatalogLedgerItem`에서 `error` 앞에 더한다.

```ts
  /**
   * 이 방의 현재 이름이 catalog 표시 이름과 다르다. `name`은 언제나 catalog
   * 표시 이름이므로(§10) ledger만 읽는 소비자에게는 이 값이 유일한 단서다.
   *
   * 카드는 이 값을 쓰지 않는다 — 카드의 각 행은 preflight 항목을 쥐고 있고
   * 거기의 `renamed`로 이미 배지를 그린다. 같은 사실을 두 번 그리지 않는다.
   */
  renamed: boolean;
```

- [x] **Step 7: 테스트가 통과하는지 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog`
Expected: PASS — 기존 78개 + 새 1개

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && pnpm --dir desktop typecheck`
Expected: exit 0

- [x] **Step 8: 판별력을 실증한다**

Step 4의 다섯 곳 중 성공 경로 하나만 `renamed: false`로 임시 고정하고 `cargo test -p schoolx-catalog renamed_survives_into_the_ledger`가 실패하는지 확인한 뒤 정확히 되돌린다. 보고서에 적는다.

- [x] **Step 9: 커밋한다**

```bash
git add crates/schoolx-catalog desktop/src/shared/api/tauriWorkspaceCatalog.ts
git commit -s -m "feat(schoolx-2): 세션 D2 — ledger가 이름 변경 사실을 싣는다"
```

---

## Task 2: `catalog_version` upgrade 경로를 고정한다

Phase 3 완료 기준 #6의 부족분이다. 되돌릴 수 없는 사고(팀이 쓴 캔버스 덮어쓰기)는 재개·채택 양쪽에서 이미 막히고 테스트도 있다. 없는 것은 **실제 `catalog_version` upgrade를 태워 보는 테스트**다.

**Files:**
- Modify: `crates/schoolx-catalog/src/saga.rs` (테스트 모듈)

**Interfaces:**
- Consumes: `Catalog`, `CatalogItem`, `Visibility` (`catalog.rs`), `FakeEffects::seed_canvas`, `FakeEffects::call_count` (`effects.rs`)
- Produces: 없음 — 테스트만 더한다

**무엇을 고정하는가.** `derive_channel_id`는 입력에서 `catalog_version`을 **제외한다**(`channel_id.rs:20`) — catalog 버전이 올라가도 `meeting`은 같은 방이다. 그리고 preflight는 `item_key` 존재와 단계 완료도로만 판정하므로, v2를 v1 위에 돌리는 것은 v1을 다시 돌리는 것과 동작이 같다. **그 동작이 옳다**는 것이 세션 D의 판단이었고, 이 테스트가 그 판단을 실행 가능한 사실로 바꾼다. 특히 v2가 **다른 캔버스 본문을 들고 와도** 팀이 쓴 내용을 덮지 않는다는 것이 핵심이다.

- [x] **Step 1: 실패하는 테스트를 쓴다**

`saga.rs`의 테스트 모듈에 추가한다.

```rust
    /// v1과 같은 `catalog_id`·`item_key`를 쓰되 버전과 캔버스 본문이 다른
    /// catalog. upgrade가 실제로 무엇을 하는지 보려면 v2가 **다른 내용**을
    /// 들고 와야 한다 — 같은 내용이면 덮어썼는지 아닌지 구별되지 않는다.
    fn catalog_v2() -> Catalog {
        Catalog {
            catalog_id: "schoolx.default".into(),
            catalog_version: 2,
            items: vec![CatalogItem {
                item_key: "meeting".into(),
                name: "메인 회의방".into(),
                description: "v2 설명".into(),
                channel_type: "stream".into(),
                visibility: Visibility::Private,
                canvas: "v2 시작 캔버스".into(),
            }],
        }
    }

    #[tokio::test]
    async fn catalog_v2_over_applied_v1_does_not_touch_the_canvas() {
        let fx = FakeEffects::new();
        apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("v1 apply");

        let channel_id =
            derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1);

        // 팀이 시작 캔버스를 자기 내용으로 바꿨다. `seed_canvas`를 쓴다 —
        // `set_canvas`를 부르면 호출 횟수가 올라가 아래 단언이 의미를 잃는다.
        fx.seed_canvas(channel_id, "팀이 직접 쓴 회의록");
        let writes_before = fx.call_count("set_canvas");

        let ledger = apply(&catalog_v2(), &fx, &["meeting".to_string()])
            .await
            .expect("v2 apply");

        // ledger는 v2를 돌렸다고 적는다.
        assert_eq!(ledger.catalog_version, 2);
        // 그런데 방은 같은 방이다 — 도출식이 버전을 입력에서 뺐기 때문이다.
        assert_eq!(item(&ledger, "meeting").channel_id, Some(channel_id));
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        // 그리고 아무것도 다시 쓰지 않았다.
        assert_eq!(item(&ledger, "meeting").outcome, Outcome::Unchanged);
        assert_eq!(
            fx.call_count("set_canvas"),
            writes_before,
            "v2 upgrade가 캔버스를 다시 썼다"
        );
        assert_eq!(
            fx.canvases
                .lock()
                .expect("lock")
                .get(&channel_id)
                .map(String::as_str),
            Some("팀이 직접 쓴 회의록"),
            "팀이 쓴 내용이 v2 캔버스로 덮였다"
        );
    }
```

- [x] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog catalog_v2_over_applied_v1`
Expected: FAIL — 테스트 모듈에 `Catalog`·`CatalogItem`·`Visibility`가 import되지 않아 컴파일 에러

기대와 다르게 **통과하면 멈춘다.** 이 테스트는 새 동작을 요구하지 않으므로 import만 채우면 초록이 되는 것이 정상이다. 그 경우 Step 4의 재주입이 유일한 판별력 근거이므로 건너뛰지 않는다.

- [x] **Step 3: import를 채운다**

`saga.rs` 테스트 모듈의 `use` 목록에 부족한 것을 더한다. 이미 있는 것을 중복해 넣지 않는다 — `cargo test`가 `unused import` 경고를 내고 `just clippy`가 `-D warnings`로 그것을 실패로 바꾼다.

```rust
    use crate::catalog::{Catalog, CatalogItem, Visibility};
```

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog catalog_v2_over_applied_v1`
Expected: PASS

- [x] **Step 4: 판별력을 실증한다**

두 번 재주입한다. 각각 되돌린 뒤 다음으로 넘어간다.

1. `saga.rs`에서 캔버스 단계의 "이미 내용이 있으면 건드리지 않는다" 가드를 임시로 없애고(`read_canvas` 결과를 무시하고 항상 `set_canvas`를 부르게) 이 테스트가 실패하는지 확인한다. 실패해야 한다 — 캔버스가 `"v2 시작 캔버스"`로 덮인다.
2. `channel_id.rs`의 도출식에 `catalog_version`을 입력으로 **더하고** 이 테스트가 실패하는지 확인한다. 실패해야 한다 — v2가 다른 채널을 만들어 `channels.len()`이 2가 된다.

두 번째가 이 테스트의 진짜 값이다. 그 한 줄이 upgrade를 "같은 방을 이어 쓴다"에서 "버전마다 새 방을 만든다"로 조용히 바꾸는데, 그전까지는 그것을 잡는 테스트가 없었다.

보고서에 두 결과를 모두 적는다.

- [x] **Step 5: 커밋한다**

```bash
git add crates/schoolx-catalog
git commit -s -m "test(schoolx-2): 세션 D2 — v2를 v1 위에 돌려도 방과 캔버스가 그대로다"
```

---

## Task 3: 설정 카드의 실행 증거

Phase 3 완료 기준 #7의 부족분이다. machine-readable 절은 `ledger_serializes_for_ui_and_cli`가 이미 충족하고, UI 절만 비어 있다.

**Files:**
- Modify: `desktop/src/testing/e2eBridge.ts`
- Modify: `desktop/tests/helpers/bridge.ts`
- Modify: `desktop/tests/helpers/settings.ts`
- Create: `desktop/tests/e2e/workspace-catalog.spec.ts`
- Modify: `desktop/playwright.config.ts`

**Interfaces:**
- Consumes: `CatalogPreflightItem`, `CatalogLedger` (`shared/api/tauriWorkspaceCatalog.ts`), `installMockBridge` (`tests/helpers/bridge.ts`), `openSettings` (`tests/helpers/settings.ts`), `waitForAnimations` (`tests/helpers/animations.ts`)
- Produces: mock 노브 `workspaceCatalogPreflight`, `workspaceCatalogLedger`, `workspaceCatalogPreflightError`

**설정 화면 진입.** 라우트는 `/settings` 하나이고 섹션은 쿼리(`?section=…`)로 정해진다(`AppShell.tsx:155-162`). 스펙은 URL을 직접 치지 않고 기존 헬퍼 `openSettings(page, "workspace-catalog")`를 쓴다 — 프로필 메뉴 → 설정 → `settings-nav-workspace-catalog` 클릭까지 UI를 실제로 밟으므로, **역할 게이트가 nav 항목을 감추면 그 자리에서 실패한다.** URL로 바로 들어가면 그 신호를 잃는다.

**섹션이 보이는 조건.** `SettingsView`의 `visibleSections`가 `canManageCommunityMembers(myMembershipQuery.data) || myMembershipQuery.data?.membershipRequired === false`를 본다. mock 하네스의 relay는 NIP-43을 광고하지 않으므로 `membershipRequired === false`로 떨어지고 섹션이 **보인다**. 그래서 이 스펙은 역할을 따로 모킹하지 않아도 카드에 닿는다. 이건 우연이 아니라 의도된 escape hatch다 — 그 이유가 `SettingsView.tsx`의 `workspace-catalog` 분기 주석에 있다.

- [x] **Step 1: mock 노브를 선언한다**

`e2eBridge.ts`의 mock 설정 타입에 세 개를 더한다. 기존 노브들(`canvasReadError`, `applyCommunityDelayMs` 등)과 같은 블록이다.

```ts
    /** `preflight_workspace_catalog`이 돌려줄 항목들. 없으면 빈 배열. */
    workspaceCatalogPreflight?: CatalogPreflightItem[];
    /** `apply_workspace_catalog`이 돌려줄 ledger. */
    workspaceCatalogLedger?: CatalogLedger;
    /**
     * `preflight_workspace_catalog`을 이 문자열로 거부한다.
     *
     * 게이트 거부 두 종(`catalog-admin-required`,
     * `catalog-membership-unavailable`)을 그리려면 이것이 필요하다 —
     * 카드는 에러 **문자열**로 두 상태를 구별한다
     * (`features/workspace-catalog/catalogError.ts`).
     */
    workspaceCatalogPreflightError?: string;
```

같은 파일 상단의 import에 타입을 더한다.

```ts
import type {
  CatalogLedger,
  CatalogPreflightItem,
} from "@/shared/api/tauriWorkspaceCatalog";
```

- [x] **Step 2: 두 command를 처리한다**

`e2eBridge.ts`의 invoke 스위치에서 `case "apply_workspace"` 근처에 더한다.

```ts
      case "preflight_workspace_catalog": {
        const refusal = activeConfig?.mock?.workspaceCatalogPreflightError;
        // `invokeTauri`는 command의 `Err(String)`을 그대로 reject한다. 카드가
        // 읽는 것이 그 문자열이므로 여기서도 문자열로 거부한다 — Error로
        // 감싸면 실제 IPC 경로와 달라진다.
        if (refusal) return Promise.reject(refusal);
        return activeConfig?.mock?.workspaceCatalogPreflight ?? [];
      }
      case "apply_workspace_catalog":
        return (
          activeConfig?.mock?.workspaceCatalogLedger ?? {
            catalog_id: "schoolx.default",
            catalog_version: 1,
            items: [],
          }
        );
```

- [x] **Step 3: 헬퍼 측 타입을 맞춘다**

`desktop/tests/helpers/bridge.ts`의 mock 설정 타입에 같은 세 필드를 더한다. 기존 필드들과 같은 주석 밀도를 따른다.

```ts
  /** Items `preflight_workspace_catalog` returns; see e2eBridge mock config. */
  workspaceCatalogPreflight?: unknown[];
  /** Ledger `apply_workspace_catalog` returns; see e2eBridge mock config. */
  workspaceCatalogLedger?: unknown;
  /** Reject `preflight_workspace_catalog` with this string. */
  workspaceCatalogPreflightError?: string;
```

- [x] **Step 4: 헬퍼의 섹션 목록을 넓힌다**

`desktop/tests/helpers/settings.ts`의 로컬 `SettingsSection` 유니온에는 `workspace-catalog`가 없다. 더한다.

```ts
  | "community-members"
  | "workspace-catalog"
```

이 유니온은 `SettingsPanels.tsx`의 것과 별개로 손으로 유지되는 사본이고 이미 `experimental`·`moderation` 등이 빠져 있다. 이번에 필요한 하나만 더하고 전체 동기화는 하지 않는다 — 이 계획의 범위가 아니다.

- [x] **Step 5: 이름 변경 배지에 testid를 붙인다**

`WorkspaceCatalogSettingsCard.tsx`의 `item.renamed` 배지(현재 `catalog-item-key-*` 바로 아래, 329행 근처)에 형제들과 같은 규칙의 testid를 준다.

```tsx
              data-testid={`catalog-renamed-${item.item_key}`}
```

문구로 단언하지 않기 위해서다. 이 배지의 문구는 `t("catalog.renamed")`라 로케일에 따라 "멤버가 이름을 변경함"일 수도 영어일 수도 있고, e2e 하네스의 로케일은 스펙이 정하는 값이 아니다. 문구를 단언하면 번역을 고칠 때마다 이 테스트가 깨지는데, 그건 이 테스트가 지키려는 것이 아니다 — 지키려는 것은 **이름이 바뀐 항목에 그 표시가 나온다**는 사실이다.

카드의 다른 testid 9개가 이미 `catalog-<무엇>-<item_key>` 꼴이므로 새 규칙을 만들지 않는다.

- [x] **Step 6: 스펙을 쓴다**

`desktop/tests/e2e/workspace-catalog.spec.ts`를 만든다. 세 테스트가 완료 기준 #7의 "상태가 UI에 표시"를 나눠 덮는다.

```ts
import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

/**
 * 워크스페이스 catalog 설정 카드의 실행 증거.
 *
 * 카드는 `outcome` 4종, `user_action` 3종, `decision` 8종, 캔버스
 * `skipped`/`unrecognized`를 모두 구별해 그린다. 세션 D는 그 구별을 만들었으나
 * 이를 렌더하는 테스트를 남기지 않았고, 그래서 Phase 3 완료 기준 #7의 UI 절이
 * 부분 충족으로 남아 있었다 (`docs/schoolx-2/IMPLEMENTATION_HANDOFF.md` 세션 D
 * 「넘긴 것」 1번).
 *
 * 여기서 고르는 세 상태는 서로 다른 렌더 분기다 — 항목 목록, 적용 결과, 게이트
 * 거부. 하나가 깨져도 나머지가 초록으로 남는다.
 */

const PREFLIGHT_ITEMS = [
  {
    item_key: "meeting",
    name: "메인 회의방",
    decision: "create_or_recreate",
    channel_id: null,
    channel_present: false,
    generation: 1,
    steps: { channel: "pending", canvas: "pending", membership: "pending" },
    renamed: false,
  },
  {
    item_key: "planning",
    name: "기획",
    decision: "adopted",
    channel_id: "11111111-2222-4333-8444-555555555555",
    channel_present: true,
    generation: 1,
    steps: { channel: "done", canvas: "done", membership: "done" },
    renamed: true,
  },
];

test("renders one row per catalog item, with the rename badge", async ({
  page,
}) => {
  await installMockBridge(page, {
    mock: { workspaceCatalogPreflight: PREFLIGHT_ITEMS },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  const card = page.getByTestId("settings-workspace-catalog");
  await expect(card).toBeVisible();

  await expect(page.getByTestId("catalog-item-meeting")).toBeVisible();
  const planning = page.getByTestId("catalog-item-planning");
  await expect(planning).toBeVisible();
  // `name`은 catalog 표시 이름이지 그 방의 현재 이름이 아니다(§10) — 이름이
  // 바뀐 방은 이름 자리가 아니라 배지로 그 사실을 알린다.
  await expect(planning).toContainText("기획");
  await expect(page.getByTestId("catalog-renamed-planning")).toBeVisible();
  // 이름이 그대로인 항목에는 배지가 없다. 이것이 없으면 배지를 항상 그리는
  // 카드도 위 단언을 통과한다.
  await expect(page.getByTestId("catalog-renamed-meeting")).toHaveCount(0);

  await waitForAnimations(page);
});

test("apply paints outcome, user action, and canvas notes from the ledger", async ({
  page,
}) => {
  await installMockBridge(page, {
    mock: {
      workspaceCatalogPreflight: PREFLIGHT_ITEMS,
      workspaceCatalogLedger: {
        catalog_id: "schoolx.default",
        catalog_version: 1,
        items: [
          {
            item_key: "meeting",
            name: "메인 회의방",
            decision: "create_or_recreate",
            channel_id: "22222222-3333-4444-8555-666666666666",
            generation: 1,
            steps: { channel: "done", canvas: "skipped", membership: "done" },
            outcome: "applied",
            user_action: null,
            renamed: false,
            error: null,
          },
          {
            item_key: "planning",
            name: "기획",
            decision: "not_owned",
            channel_id: "11111111-2222-4333-8444-555555555555",
            generation: 1,
            steps: { channel: "done", canvas: "pending", membership: "pending" },
            outcome: "blocked",
            user_action: "request_ownership",
            renamed: true,
            error: "이 방은 다른 사람이 만들었습니다",
          },
        ],
      },
    },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  await page.getByTestId("catalog-item-meeting").getByRole("checkbox").check();
  await page.getByTestId("catalog-apply").click();

  // 캔버스를 덮지 않고 건너뛴 것은 성공과 구별해서 말해야 한다.
  await expect(page.getByTestId("catalog-canvas-note-meeting")).toBeVisible();
  // 막힌 항목은 사용자가 할 일을 말한다.
  await expect(
    page.getByTestId("catalog-user-action-planning"),
  ).toBeVisible();
  await expect(page.getByTestId("catalog-error-planning")).toContainText(
    "이 방은 다른 사람이 만들었습니다",
  );

  await waitForAnimations(page);
});

test("a gate refusal explains itself and hides the apply button", async ({
  page,
}) => {
  await installMockBridge(page, {
    mock: { workspaceCatalogPreflightError: "catalog-admin-required" },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  await expect(page.getByTestId("catalog-admin-required")).toBeVisible();
  // 비활성 버튼은 "아직 아니다"로 읽힌다. 이 화면에서 사용자가 무엇을 해도
  // 켜지지 않으므로 아예 감춘다.
  await expect(page.getByTestId("catalog-apply")).toHaveCount(0);

  await waitForAnimations(page);
});
```

**체크박스 셀렉터 하나가 실행 중에 확정된다.** 행의 체크박스는 Radix `Checkbox`이고 `id`가 `workspace-catalog-item-<key>`다. Radix는 실제 `<input>` 대신 `role="checkbox"`인 버튼을 그리므로 `getByRole("checkbox")`가 맞을 것으로 보지만, 잡지 못하면 `page.locator("#workspace-catalog-item-meeting")`으로 바꾼다. 둘 중 어느 쪽이든 단언의 의미는 같다.

- [x] **Step 7: 스펙을 등록한다**

`desktop/playwright.config.ts`의 `smoke` 프로젝트 `testMatch` 배열에 더한다. `voice-settings.spec.ts` 근처가 자연스럽다.

```ts
        "**/workspace-catalog.spec.ts",
```

- [x] **Step 8: 스펙을 돌린다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0/desktop && . ../bin/activate-hermit && pnpm test:e2e:smoke --grep "workspace catalog|catalog"`
Expected: 3 passed

`pnpm run build`로 직접 빌드하지 않는다 — mock bridge가 빠져 세 테스트 모두 `Cannot read properties of undefined (reading 'invoke')`로 죽고, 그건 제품 버그처럼 보인다. 포트 4173에 이전 빌드의 서버가 살아 있으면 그것도 죽인 뒤 다시 돌린다(`reuseExistingServer: true`).

- [x] **Step 9: 판별력을 실증한다**

`WorkspaceCatalogSettingsCard.tsx`에서 `catalog-user-action-*` 블록을 임시로 주석 처리하고 두 번째 테스트가 실패하는지 확인한 뒤 되돌린다. 보고서에 적는다.

- [x] **Step 10: 커밋한다**

```bash
git add desktop/src desktop/tests desktop/playwright.config.ts
git commit -s -m "test(schoolx-2): 세션 D2 — catalog 설정 카드의 렌더 증거"
```

---

## Task 4: Phase 3 판정 갱신과 전체 게이트

**Files:**
- Modify: `docs/schoolx-2/IMPLEMENTATION_HANDOFF.md`
- Modify: `docs/schoolx-2/WORKSPACE_CATALOG.md`
- Modify: `docs/schoolx-2/BASELINE.md`
- Modify: `docs/schoolx-2/plans/2026-08-01-catalog-security.md`

- [x] **Step 1: Phase 3 판정표를 고친다**

`IMPLEMENTATION_HANDOFF.md` 세션 D의 7행 표에서 #3·#6·#7을 **부분 → 충족**으로 바꾸고, 각 행의 「증거」 칸에 이번에 더한 테스트 이름을 적는다.

- #3 → `renamed_survives_into_the_ledger`
- #6 → `catalog_v2_over_applied_v1_does_not_touch_the_canvas`
- #7 → `desktop/tests/e2e/workspace-catalog.spec.ts` 3개

그리고 snapshot 절의 Phase 상태를 **Phase 3 완료**로 바꾼다. 「넘긴 것」 1–3에는 각각 닫혔다는 문단을 더한다 — 세션 E1이 8·9에 한 것과 같은 형식이다.

**7개가 전부 충족인지 다시 확인하고 나서 바꾼다.** 표를 먼저 고치고 근거를 나중에 맞추지 않는다.

- [x] **Step 2: 아직 열린 것을 옮겨 적는다**

Phase 3을 닫아도 catalog 표면에 남는 것이 있다. 「아직 구현 또는 검증되지 않은 것」에 남기고, Phase 3 완료와 혼동되지 않게 적는다.

- `generation` 증가 경로 부재 → `deleted` 항목의 재생성 프롬프트가 답할 수 없다 (세션 D 「넘긴 것」 7번)
- 선점의 약한 형태와 위임 실행 요청 흐름 (세션 E1 「넘긴 것」 1·2번)
- catalog 적용의 CLI 경로 (세션 D 「넘긴 것」 6번)
- 나머지 8개 업무방 콘텐츠 (세션 D 「넘긴 것」 4번)

- [x] **Step 3: WORKSPACE_CATALOG.md를 맞춘다**

§7의 「구현 상태」 문단 중 `renamed`가 ledger에 없다고 적은 부분과, §6의 upgrade 경로가 검증되지 않았다고 적은 부분을 고친다. 문서가 코드보다 오래된 채로 남으면 다음 세션이 없는 gap을 다시 연다.

- [x] **Step 4: E1 계획서의 누락을 보탠다**

`plans/2026-08-01-catalog-security.md`의 실행 완료 헤더는 계획과 다르게 한 것을 둘만 적었다. 세 번째가 있다: **Task 5 Step 2의 「섹션에 featureGate를 건다」는 구현 중에 뒤집혔다.** 매니페스트에 없는 ID는 fail-open이라 게이트가 무음 no-op이 되고, 매니페스트에 넣으면 관리자에게도 preview opt-in을 요구하게 된다. 실제 게이트는 `SettingsView`의 `visibleSections` 역할 검사다. 근거는 `SettingsPanels.tsx:205`의 주석.

- [x] **Step 5: 전체 게이트를 돌린다**

`just ci`는 하네스 10분 한도에 걸리므로 구성 레시피를 하나씩 포그라운드로 돌린다: `fmt-check`, `clippy`, `desktop-check`, `desktop-tauri-fmt-check`, `desktop-tauri-clippy`, `web-check`, `mobile-check`, `test-unit`, `desktop-test`, `desktop-build`, `desktop-tauri-check`, `desktop-tauri-test`, `web-build`, `mobile-test`.

이어서 회귀:

```bash
just test-e2e e2e_workspace_catalog     # 5/5
just test-e2e e2e_access_matrix         # 17/17
just schoolx-upstream-check             # 3/3
```

각각의 시작 시각·exit·소요를 적는다. 실패하면 원인이 이번 변경인지 기존 조건인지 판별해 밝힌다.

- [x] **Step 6: BASELINE에 기록한다**

`BASELINE.md`의 「실행 증거」에 `### 세션 D2 (2026-08-04, Phase 3 닫기)` 절을 세션 E1 절과 같은 표 형식으로 더한다. 세 Task의 재주입 결과도 함께 적는다 — 어떤 주입이 어떤 테스트를 실패시켰는지.

- [x] **Step 7: 커밋한다**

```bash
git add docs/schoolx-2
git commit -s -m "docs(schoolx-2): 세션 D2 — Phase 3 완료 판정과 실행 기록"
```
