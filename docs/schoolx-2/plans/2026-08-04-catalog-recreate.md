# catalog 재생성 구현 계획 (세션 D3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `deleted`·`not_owned`로 막힌 catalog 항목을 사용자가 한 번의 확인으로 다음 세대에 만들 수 있게 한다.

**Architecture:** 새 판정도 새 단계도 만들지 않는다. `apply`의 인자를 `Vec<String>`에서 `Vec<Selection>`으로 넓혀 `recreate_from: Option<u32>`를 받고, 그 값이 preflight가 보고한 세대와 **일치할 때만** 그 항목의 계획을 한 세대 올린다. `apply_item`이 세대에서 채널 ID를 직접 도출하므로 나머지(도출·provenance·ledger)는 따라온다.

**Tech Stack:** Rust 1.95 / `schoolx-catalog` 크레이트 / Tauri 2 / React 19 / Playwright

## Global Constraints

- 작업 위치는 **메인 체크아웃** `/Users/kim-yonghun/Development/schoolX_v2.0`, 브랜치 `codex/schoolx-2-foundation`. 워크트리에서는 `just desktop-tauri-fmt`가 실패해 pre-commit이 막힌다.
- 시작 전 `. ./bin/activate-hermit`.
- `unsafe` 금지. 프로덕션 경로에 새 `unwrap()`/`expect()` 금지.
- 새 public API에는 doc comment를 단다.
- 데스크톱 텍스트 크기는 rem 토큰만 (`text-base`, `text-sm`, `text-xs`, `text-2xs`, `text-3xs`).
- i18n 키를 더할 때는 `en`, `ko`를 **한 번에** 바꾼다.
- **`Provenance`와 `Ledger`의 와이어 포맷을 바꾸지 않는다.** `generation`은 이미 둘 다에 있다. `steps`도 건드리지 않으므로 §4의 리더-우선 순서는 이 변경과 무관하다.
- Playwright 스펙은 `pnpm test:e2e:smoke`로 돈다 (`pnpm run build`는 mock bridge를 뗀다). 스크린샷 전에는 `waitForAnimations(page)`.
- 스펙: [`CATALOG_RECREATE.md`](../CATALOG_RECREATE.md) (커밋 `70bdc977`).

## 시작 상태 (2026-08-04, 커밋 `70bdc977`)

- `apply(catalog, effects, selected: &[String])`. 항목 필터는 `selected.contains(&step.item_key)` 한 줄이다.
- `apply_item`이 `derive_channel_id(relay_scope, catalog_id, item_key, plan.generation)`으로 채널 ID를 **직접 도출한다**. `plan.channel_id`는 `Conflict`·`Retired`·`NoChange` 조기 반환에서만 쓰인다.
- 신규 항목의 세대는 `preflight.rs`의 `None` 분기에서 항상 리터럴 `1`이다.
- `deleted`는 `Duplicate` + `!plan.channel_present`, `not_owned`는 `Duplicate` + 접근 가능 + `is_owner == false`에서 나온다. **둘 다 preflight 판정은 `CreateOrRecreate`다** — 증명서가 읽히지 않아서다.
- 같은 이름으로 선점하면 `conflict`로 먼저 걸린다. 그래서 선점의 약한 형태는 **다른 이름**을 쓰고 `not_owned`로 떨어진다.

---

## File Structure

| 파일 | 책임 |
|---|---|
| `crates/schoolx-catalog/src/saga.rs` | `Selection` 타입, `apply`의 세대 상승 |
| `desktop/src-tauri/src/commands/workspace_catalog.rs` | `Selection`을 받아 넘긴다 |
| `desktop/src/shared/api/tauriWorkspaceCatalog.ts` | `CatalogSelection` 타입과 인자 |
| `desktop/src/features/workspace-catalog/hooks.ts` | mutation 인자 |
| `desktop/src/features/settings/ui/WorkspaceCatalogSettingsCard.tsx` | 재생성 컨트롤 |
| `desktop/src/shared/i18n/locales/{en,ko}.ts` | 재생성 문구 |
| `desktop/tests/e2e/workspace-catalog.spec.ts` | 재생성 스펙 |
| `docs/schoolx-2/{IMPLEMENTATION_HANDOFF,WORKSPACE_CATALOG,BASELINE}.md` | 결과 기록 |

---

## Task 1: 크레이트가 세대를 올린다

**Files:**
- Modify: `crates/schoolx-catalog/src/saga.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Selection {
      pub item_key: String,
      pub recreate_from: Option<u32>,
  }
  ```
  그리고 `pub async fn apply(catalog: &Catalog, effects: &dyn CatalogEffects, selected: &[Selection]) -> Result<Ledger, EffectError>`

- [ ] **Step 1: 실패하는 테스트를 쓴다**

`saga.rs` 테스트 모듈에 넷을 더한다. 기존 테스트가 쓰는 `both()`와 문자열 벡터는 Step 4에서 함께 고친다.

```rust
    /// 평소 적용. 기존 테스트가 쓰던 `vec!["meeting".into()]`을 대신한다.
    fn pick(item_key: &str) -> Vec<Selection> {
        vec![Selection {
            item_key: item_key.to_string(),
            recreate_from: None,
        }]
    }

    /// 세대 `from`이 막힌 것을 보고 누른 재생성.
    fn recreate(item_key: &str, from: u32) -> Vec<Selection> {
        vec![Selection {
            item_key: item_key.to_string(),
            recreate_from: Some(from),
        }]
    }

    /// 방을 만들고, relay에서 지워 `deleted`가 나오는 상태를 만든다.
    ///
    /// fake의 `channels`에서 빼면 접근 불가가 되고, `burned_ids`는 남아
    /// `create_channel`이 `Duplicate`를 돌려준다 — relay의 soft delete가
    /// ID를 계속 점유하는 것과 같은 모양이다.
    async fn make_then_delete(fx: &FakeEffects) -> Uuid {
        apply(crate::builtin(), fx, &pick("meeting"))
            .await
            .expect("first apply");
        let id = fx.channels.lock().expect("lock")[0].id;
        fx.channels.lock().expect("lock").clear();
        fx.provenance.lock().expect("lock").clear();
        id
    }

    #[tokio::test]
    async fn recreate_moves_the_item_to_the_next_generation() {
        let fx = FakeEffects::new();
        let gone = make_then_delete(&fx).await;

        // 확인: 재생성 없이는 막힌다.
        let blocked = apply(crate::builtin(), &fx, &pick("meeting"))
            .await
            .expect("blocked apply");
        assert_eq!(item(&blocked, "meeting").decision, "deleted");
        assert_eq!(item(&blocked, "meeting").generation, 1);

        let ledger = apply(crate::builtin(), &fx, &recreate("meeting", 1))
            .await
            .expect("recreate");

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.generation, 2);
        let fresh = entry.channel_id.expect("channel id");
        assert_ne!(fresh, gone, "세대를 올렸는데 같은 방을 다시 썼다");
        assert_eq!(
            fresh,
            derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 2)
        );
    }

    #[tokio::test]
    async fn recreating_twice_only_moves_one_generation() {
        let fx = FakeEffects::new();
        make_then_delete(&fx).await;
        apply(crate::builtin(), &fx, &recreate("meeting", 1))
            .await
            .expect("first recreate");

        // 같은 요청을 다시 낸다. 세대 1은 이제 관심 밖이고 세대 2가 살아 있다.
        let ledger = apply(crate::builtin(), &fx, &recreate("meeting", 1))
            .await
            .expect("second recreate");

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Unchanged);
        assert_eq!(entry.generation, 2, "재생성 요청이 세대를 또 올렸다");
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn recreate_from_a_stale_generation_is_ignored() {
        let fx = FakeEffects::new();
        apply(crate::builtin(), &fx, &pick("meeting"))
            .await
            .expect("apply");

        // 화면이 낡았다 — 사용자가 본 세대는 0이고 지금은 1이다.
        let ledger = apply(crate::builtin(), &fx, &recreate("meeting", 0))
            .await
            .expect("stale recreate");

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Unchanged);
        assert_eq!(entry.generation, 1);
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn a_recreated_generation_that_is_also_burned_reports_the_new_one() {
        let fx = FakeEffects::new();
        make_then_delete(&fx).await;
        // 세대 2도 이미 타 있다: 그 ID로 만든 뒤 지운다.
        let gen2 = derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 2);
        fx.burn_channel_id(gen2);

        let ledger = apply(crate::builtin(), &fx, &recreate("meeting", 1))
            .await
            .expect("recreate");

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.decision, "deleted");
        assert_eq!(
            entry.generation, 2,
            "다음 확인이 가리켜야 할 세대는 방금 시도한 것이다"
        );
        assert_eq!(entry.user_action, Some(UserAction::ConfirmRecreate));
    }
```

- [ ] **Step 2: 테스트가 실패하는지 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog recreate`
Expected: FAIL — `Selection`도 `burn_channel_id`도 없어 컴파일 에러

- [ ] **Step 3: fake에 「탄 ID」 시더를 더한다**

`effects.rs`의 `mod fake`에 추가한다. `created` 로그에만 넣고 `channels`에는 넣지 않는 것이 곧 "점유됐지만 접근 불가"다.

fake는 이미 `burned_ids: Mutex<HashSet<Uuid>>`로 이것을 모델링한다 —
`create_channel`이 `insert`의 반환이 `false`면(이미 있으면) `Duplicate`를
돌려준다. 그래서 시더는 그 집합에 넣기만 한다. 새 저장소를 만들지 않는다.

```rust
        /// 이 ID를 이미 쓴 것으로 만든다 — `create_channel`이 `Duplicate`를
        /// 돌려주고 `channels`에는 없으므로 접근 불가다. relay의 soft delete가
        /// 남긴 상태와 같은 모양이다(`WORKSPACE_CATALOG.md` §6).
        pub(crate) fn burn_channel_id(&self, channel_id: Uuid) {
            self.burned_ids.lock().expect("lock").insert(channel_id);
        }
```

같은 이유로 Step 1의 `make_then_delete`가 `channels`만 비우면 된다 —
`burned_ids`는 그대로 남아 다음 생성이 `Duplicate`가 된다.

- [ ] **Step 4: `Selection`을 만들고 `apply`를 바꾼다**

`saga.rs`의 `apply` 위에 추가한다.

```rust
/// 적용할 항목 하나와, 그 항목에 대한 사용자의 재생성 결정.
///
/// `item_key`만 있던 이전 인자를 넓힌 것이다. 재생성이 별도 command가 아니라
/// 같은 적용의 한 필드인 이유: 재생성은 새 동작이 아니라 **어느 세대에**
/// 평소 적용을 거는가의 문제이고, 두 경로로 나누면 멱등성 규칙도 둘이 된다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// catalog 항목 키.
    pub item_key: String,
    /// 「세대 `g`가 막힌 것을 보았다. 다음 것을 만들어라」.
    ///
    /// `None`이면 평소 적용이다. `Some(g)`는 **preflight가 지금 보고하는
    /// 세대가 `g`일 때만** 효력이 있다 — 다르면 사용자가 본 화면이 낡았거나
    /// 다른 관리자가 이미 처리한 것이므로 무시하고 평소 적용을 한다. 그
    /// 규칙이 같은 요청을 두 번 내도 세대가 한 번만 오르게 한다.
    ///
    /// 한 번에 한 칸만 올린다. `g + 1`도 막혀 있으면 그 실행이 다시
    /// `g + 1`을 보고하고 사용자가 다시 누른다 — 화면에 보이는 것과 일어나는
    /// 일을 같게 두기 위해서다. 설계 근거: `docs/schoolx-2/CATALOG_RECREATE.md` §3.
    pub recreate_from: Option<u32>,
}
```

`apply`의 시그니처와 루프를 바꾼다.

```rust
pub async fn apply(
    catalog: &Catalog,
    effects: &dyn CatalogEffects,
    selected: &[Selection],
) -> Result<Ledger, EffectError> {
    let plan = preflight(catalog, effects).await?;
    let relay_scope = effects.relay_scope().await;
    let now = effects.now_rfc3339().await;

    let mut items = Vec::new();

    for mut step in plan {
        let Some(choice) = selected.iter().find(|s| s.item_key == step.item_key) else {
            continue;
        };
        // 사용자가 본 세대가 지금 세대와 같을 때만 한 칸 올린다. 낡은 화면의
        // 요청은 무시하고 평소 적용을 한다 — 그래야 같은 요청을 두 번 내도
        // 세대가 한 번만 오른다.
        if choice.recreate_from == Some(step.generation) {
            step.generation += 1;
            // 새 세대는 새 방이다. 이전 세대에서 읽어 온 상태를 그대로 두면
            // saga가 채널 단계를 건너뛰고 있지도 않은 방에 캔버스를 쓴다.
            step.steps = StepStates::default();
            step.channel_present = false;
            step.channel_id = Some(derive_channel_id(
                &relay_scope,
                &catalog.catalog_id,
                &step.item_key,
                step.generation,
            ));
        }
        items.push(apply_item(catalog, effects, &relay_scope, &now, step).await);
    }

    Ok(Ledger {
        catalog_id: catalog.catalog_id.clone(),
        catalog_version: catalog.catalog_version,
        items,
    })
}
```

doc comment에 한 문단 더한다.

```rust
/// 선택한 항목을 적용한다.
///
/// `selected`에 없는 항목은 건드리지 않는다. 한 항목의 실패가 다른 항목을
/// 막지 않는다.
///
/// [`Selection::recreate_from`]이 지금 세대와 일치하는 항목은 한 세대 위에서
/// 적용한다. 이전 세대의 방·캔버스·증명서는 건드리지 않는다 — 새 세대는 새
/// 채널 ID이고, 이 함수는 그 ID에만 쓴다.
```

- [ ] **Step 5: 기존 테스트의 인자를 고친다**

`both()`와 `&["meeting".to_string()]` 형태를 전부 `Selection`으로 바꾼다.
`both()`는 이렇게 둔다.

```rust
    fn both() -> Vec<Selection> {
        vec![
            Selection { item_key: "meeting".into(), recreate_from: None },
            Selection { item_key: "planning".into(), recreate_from: None },
        ]
    }
```

**의미를 바꾸지 않는다** — 지금까지의 모든 호출은 평소 적용이었다.
컴파일러가 남은 자리를 전부 알려준다.

- [ ] **Step 6: 테스트가 통과하는지 확인한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test -p schoolx-catalog`
Expected: PASS — 기존 80개 + 새 4개

- [ ] **Step 7: 판별력을 실증한다**

두 번 재주입하고 각각 되돌린다.

1. `if choice.recreate_from == Some(step.generation)`을 `if choice.recreate_from.is_some()`으로 바꾼다 → `recreating_twice_only_moves_one_generation`과 `recreate_from_a_stale_generation_is_ignored`가 실패해야 한다.
2. `step.steps = StepStates::default();`를 지운다 → 재생성이 채널 단계를 건너뛰므로 `recreate_moves_the_item_to_the_next_generation`이 실패해야 한다.

각 결과를 보고서에 적는다. 두 번째가 실패하지 않으면 그 줄이 필요 없다는 뜻이므로 지우고 왜인지 적는다.

- [ ] **Step 8: 커밋한다**

```bash
git add crates/schoolx-catalog
git commit -s -m "feat(schoolx-2): 세션 D3 — 재생성이 항목을 다음 세대로 옮긴다"
```

---

## Task 2: 어댑터와 화면

**Files:**
- Modify: `desktop/src-tauri/src/commands/workspace_catalog.rs`
- Modify: `desktop/src/shared/api/tauriWorkspaceCatalog.ts`
- Modify: `desktop/src/features/workspace-catalog/hooks.ts`
- Modify: `desktop/src/features/settings/ui/WorkspaceCatalogSettingsCard.tsx`
- Modify: `desktop/src/shared/i18n/locales/{en,ko}.ts`

- [ ] **Step 1: Tauri command 인자를 넓힌다**

`apply_workspace_catalog`의 인자를 바꾼다. `Selection`은 크레이트 타입이므로
serde 유도가 필요하다 — Task 1의 `Selection`에 `#[derive(Deserialize)]`를
더한다(같은 파일, `Serialize`는 필요 없다: 나가는 값이 아니다).

```rust
pub async fn apply_workspace_catalog(
    selected: Vec<schoolx_catalog_pkg::saga::Selection>,
    state: State<'_, AppState>,
) -> Result<Ledger, String> {
    require_community_admin(&state).await?;
    let effects = RelayEffects { state };
    schoolx_catalog_pkg::saga::apply(schoolx_catalog_pkg::builtin(), &effects, &selected)
        .await
        .map_err(|e| e.0)
}
```

- [ ] **Step 2: TS 타입과 호출을 맞춘다**

`tauriWorkspaceCatalog.ts`:

```ts
/**
 * 적용할 항목 하나. `recreate_from`은 「이 세대가 막힌 것을 보았다」는 뜻이고,
 * 백엔드는 preflight가 보고하는 세대가 그 값과 같을 때만 한 칸 올린다
 * (`crates/schoolx-catalog/src/saga.rs`의 `Selection`).
 */
export type CatalogSelection = {
  item_key: string;
  recreate_from: number | null;
};

export async function applyWorkspaceCatalog(
  selected: CatalogSelection[],
): Promise<CatalogLedger> {
  return invokeTauri<CatalogLedger>("apply_workspace_catalog", { selected });
}
```

`hooks.ts`의 `mutationFn` 인자 타입을 `CatalogSelection[]`으로 바꾼다.

`Option<u32>`는 serde에서 `null`을 받으므로 TS의 `number | null`과 맞는다.

- [ ] **Step 3: i18n 키를 양쪽에 더한다**

`en.ts`의 `catalog` 블록:

```ts
    recreate: {
      action: "Create it again",
      ownedByOther:
        "This room was created by someone else. Creating it again makes a new, separate room — the existing one is left alone.",
      pending: "Creating…",
    },
```

`ko.ts`의 같은 자리:

```ts
    recreate: {
      action: "다시 만들기",
      ownedByOther:
        "이 방은 다른 사람이 만들었습니다. 다시 만들면 별개의 방이 새로 생기고, 기존 방은 그대로 남습니다.",
      pending: "만드는 중…",
    },
```

- [ ] **Step 4: 카드에 컨트롤을 단다**

`WorkspaceCatalogSettingsCard.tsx`. 지금 `handleApply`가 `[...selected]`를
넘기는 자리를 `CatalogSelection[]`으로 바꾸고, 재생성 제출을 더한다.

```tsx
  function handleApply() {
    apply.mutate(
      [...selected].map((item_key) => ({ item_key, recreate_from: null })),
      {
        onSuccess: (result) => {
          setLedger(result);
          setSelected(new Set());
        },
      },
    );
  }

  /**
   * 「다시 만들기」. 사용자가 **본** 세대를 그대로 되돌려 보낸다 — 백엔드는
   * 그 값이 지금 세대와 같을 때만 한 칸 올린다. 화면이 낡았으면 아무 일도
   * 일어나지 않고 평소 적용이 된다.
   */
  function handleRecreate(entry: CatalogLedgerItem) {
    apply.mutate(
      [{ item_key: entry.item_key, recreate_from: entry.generation }],
      { onSuccess: (result) => setLedger(result) },
    );
  }
```

`CatalogItemRow`에 `onRecreate` prop을 넘기고, `user_action` alert 안에
버튼을 그린다.

```tsx
          {ledgerItem.user_action ? (
            <Alert
              className="border-amber-500/30 bg-amber-500/10"
              data-testid={`catalog-user-action-${item.item_key}`}
            >
              <AlertDescription className="space-y-2 text-amber-800 dark:text-amber-300">
                <p>{t(`catalog.userAction.${ledgerItem.user_action}`)}</p>
                {/*
                  `confirm_recreate`는 이 버튼이 유일한 답이다. `request_ownership`
                  에서는 부차 동작이다 — 그 판정은 선점과 정상적인 공동 관리를
                  구별하지 못하고, 후자에서 세대를 올리면 표준 업무방이 둘이
                  된다. 그래서 결과를 먼저 문장으로 말하고 버튼은 그 아래
                  ghost로 둔다. 설계 근거: docs/schoolx-2/CATALOG_RECREATE.md §4.
                */}
                {ledgerItem.user_action === "request_ownership" ? (
                  <p className="text-xs">{t("catalog.recreate.ownedByOther")}</p>
                ) : null}
                <Button
                  data-testid={`catalog-recreate-${item.item_key}`}
                  disabled={busy}
                  onClick={() => onRecreate(ledgerItem)}
                  size="sm"
                  type="button"
                  variant={
                    ledgerItem.user_action === "request_ownership"
                      ? "ghost"
                      : "default"
                  }
                >
                  {busy
                    ? t("catalog.recreate.pending")
                    : t("catalog.recreate.action")}
                </Button>
              </AlertDescription>
            </Alert>
          ) : null}
```

`busy`는 `apply.isPending`을 행까지 내려 준 값이다. `resolve_conflict`에는
버튼을 달지 않는다 — 그 판정은 세대를 올려서 풀리는 상태가 아니다. 위
`user_action === "request_ownership"` 분기와 대칭으로, 세 값 중 두 값에만
버튼이 뜨도록 조건을 명시한다.

```tsx
const RECREATABLE_ACTIONS = new Set<CatalogUserAction>([
  "confirm_recreate",
  "request_ownership",
]);
```

버튼 전체를 `RECREATABLE_ACTIONS.has(ledgerItem.user_action)`로 감싼다.

- [ ] **Step 5: 검증한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && cargo test --manifest-path desktop/src-tauri/Cargo.toml workspace_catalog && pnpm --dir desktop typecheck && pnpm --dir desktop check && pnpm --dir desktop test`
Expected: 전부 PASS. i18n parity 테스트가 `en`/`ko` 키 구조 일치를 확인한다.

- [ ] **Step 6: 커밋한다**

```bash
git add desktop/src desktop/src-tauri
git commit -s -m "feat(schoolx-2): 세션 D3 — 막힌 항목을 다시 만들 수 있다"
```

---

## Task 3: 스펙, 문서, 게이트

**Files:**
- Modify: `desktop/tests/e2e/workspace-catalog.spec.ts`
- Modify: `docs/schoolx-2/{IMPLEMENTATION_HANDOFF,WORKSPACE_CATALOG,BASELINE}.md`

- [ ] **Step 1: 스펙 둘을 더한다**

기존 파일에 더한다. mock bridge의 `workspaceCatalogLedger`로 `deleted`와
`not_owned` 상태를 세운다 — 두 상태가 다르게 그려지는 것이 §4의 핵심이다.

```ts
test("a deleted item offers the recreate control as its answer", async ({
  page,
}) => {
  await installMockBridge(page, {
    workspaceCatalogPreflight: PREFLIGHT_ITEMS,
    workspaceCatalogLedger: {
      catalog_id: "schoolx.default",
      catalog_version: 1,
      items: [
        {
          item_key: "meeting",
          name: "메인 회의방",
          decision: "deleted",
          channel_id: "33333333-4444-4555-8666-777777777777",
          generation: 1,
          steps: { channel: "pending", canvas: "pending", membership: "pending" },
          outcome: "blocked",
          user_action: "confirm_recreate",
          renamed: false,
          error: null,
        },
      ],
    },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  await page.locator("#workspace-catalog-item-meeting").click();
  await page.getByTestId("catalog-apply").click();

  await expect(page.getByTestId("catalog-user-action-meeting")).toBeVisible();
  await expect(page.getByTestId("catalog-recreate-meeting")).toBeVisible();
  // `deleted`에는 공동 관리 경고가 붙지 않는다 — 그 판정에는 그 위험이 없다.
  await expect(
    page.getByTestId("catalog-user-action-meeting"),
  ).not.toContainText("별개의 방");

  await waitForAnimations(page);
});

test("not_owned warns that recreating makes a second room", async ({
  page,
}) => {
  await installMockBridge(page, {
    workspaceCatalogPreflight: PREFLIGHT_ITEMS,
    workspaceCatalogLedger: {
      catalog_id: "schoolx.default",
      catalog_version: 1,
      items: [
        {
          item_key: "meeting",
          name: "메인 회의방",
          decision: "not_owned",
          channel_id: "33333333-4444-4555-8666-777777777777",
          generation: 1,
          steps: { channel: "done", canvas: "pending", membership: "pending" },
          outcome: "blocked",
          user_action: "request_ownership",
          renamed: false,
          error: null,
        },
      ],
    },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  await page.locator("#workspace-catalog-item-meeting").click();
  await page.getByTestId("catalog-apply").click();

  const alert = page.getByTestId("catalog-user-action-meeting");
  await expect(alert).toBeVisible();
  // 1차 안내는 여전히 "저 사람에게 부탁하라"이고, 재생성은 그 아래 부차
  // 동작이다. 결과를 먼저 말한다.
  await expect(alert).toContainText("별개의 방");
  await expect(page.getByTestId("catalog-recreate-meeting")).toBeVisible();

  await waitForAnimations(page);
});
```

**한국어 문구로 단언하는 것이 여기서는 맞다.** 두 상태의 차이가 곧 문구의
유무이고, testid로는 그 차이를 표현할 자리가 없다. 로케일이 영어로 바뀌면 이
두 줄을 영어 문구로 바꾼다 — 그때 이 테스트가 실패하는 것이 옳다.

- [ ] **Step 2: 스펙을 돌린다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0/desktop && . ../bin/activate-hermit && pnpm test:e2e:smoke workspace-catalog`
Expected: 5 passed

포트 4173에 이전 빌드의 서버가 살아 있으면 죽인 뒤 다시 돌린다.

- [ ] **Step 3: 판별력을 실증한다**

`RECREATABLE_ACTIONS`에서 `request_ownership`을 빼고 두 번째 스펙이 실패하는지
확인한 뒤 되돌린다. 보고서에 적는다.

- [ ] **Step 4: 문서를 갱신한다**

- `WORKSPACE_CATALOG.md` §6의 「구현 상태 — 프롬프트는 뜨지만 답할 방법이 없다」
  문단을 닫힌 것으로 바꾼다. `generation`을 올리는 경로가 무엇이고 어디까지만
  하는지 적는다.
- `IMPLEMENTATION_HANDOFF.md`: 세션 D 「넘긴 것」 7번과 세션 E1 「넘긴 것」
  1번에 닫힘 문단을 더한다. **E1 1번은 완전히 닫히지 않는다** — 영구 차단이
  유한한 경합이 된 것이고, 그렇게 적는다. 「구현되어 있는 것」과 「아직
  구현 또는 검증되지 않은 것」도 맞춘다. 세션 D3 절을 A·B·D·D2·E1 형식으로
  더한다.
- `SECURITY_CONTRACT.md` §5의 「남는 조건」 2번(선점의 약한 형태)을 갱신한다 —
  복구 경로가 생겼으나 선점 자체는 여전히 막지 못한다.

- [ ] **Step 5: 전체 게이트를 돌린다**

구성 레시피 14개를 하나씩 포그라운드로 돌리고, 이어서:

```bash
just test-e2e e2e_workspace_catalog     # 5/5
just test-e2e e2e_access_matrix         # 17/17
just schoolx-upstream-check             # 3/3
pnpm --dir desktop test:e2e:smoke workspace-catalog   # 5/5
```

각각의 시작 시각·exit·소요를 적는다.

- [ ] **Step 6: BASELINE에 기록한다**

`### 세션 D3 (2026-08-04, catalog 재생성)` 절을 세션 D2와 같은 표 형식으로
더한다. 재주입 셋의 결과도 함께 적는다.

- [ ] **Step 7: 커밋한다**

```bash
git add desktop/tests docs/schoolx-2
git commit -s -m "docs(schoolx-2): 세션 D3 — 재생성 구현 결과 기록"
```
