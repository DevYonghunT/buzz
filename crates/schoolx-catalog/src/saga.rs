//! idempotent saga 실행기.
//!
//! 단계는 채널 생성 → 시작 캔버스 → owner 확인이다. 각 단계는 provenance를
//! 보고 완료면 건너뛰고, 실행하고, provenance를 갱신한다.
//!
//! **실패해도 되돌리지 않는다.** 채널을 만든 뒤 캔버스에서 실패하면 채널을
//! 지우지 않고 상태만 기록한다. 재시도가 캔버스부터 이어서 한다.

use crate::catalog::Catalog;
use crate::channel_id::derive_channel_id;
use crate::effects::{CatalogEffects, ChannelSpec, CreateOutcome, EffectError};
use crate::ledger::{decision_label, Ledger, LedgerItem, Outcome, UserAction, DELETED_DECISION};
use crate::preflight::{preflight, Decision, PreflightItem};
use crate::provenance::{Provenance, StepStates, StepStatus};

/// 선택한 항목을 적용한다.
///
/// `selected`에 없는 항목은 건드리지 않는다. 한 항목의 실패가 다른 항목을
/// 막지 않는다.
pub async fn apply(
    catalog: &Catalog,
    effects: &dyn CatalogEffects,
    selected: &[String],
) -> Result<Ledger, EffectError> {
    let plan = preflight(catalog, effects).await?;
    let relay_scope = effects.relay_scope().await;
    let now = effects.now_rfc3339().await;

    let mut items = Vec::new();

    for step in plan {
        if !selected.contains(&step.item_key) {
            continue;
        }
        items.push(apply_item(catalog, effects, &relay_scope, &now, step).await);
    }

    Ok(Ledger {
        catalog_id: catalog.catalog_id.clone(),
        catalog_version: catalog.catalog_version,
        items,
    })
}

async fn apply_item(
    catalog: &Catalog,
    effects: &dyn CatalogEffects,
    relay_scope: &str,
    now: &str,
    plan: PreflightItem,
) -> LedgerItem {
    let decision = decision_label(plan.decision).to_string();

    let blocked = |action: UserAction| LedgerItem {
        item_key: plan.item_key.clone(),
        decision: decision.clone(),
        channel_id: plan.channel_id,
        generation: plan.generation,
        steps: StepStates::default(),
        outcome: Outcome::Blocked,
        user_action: Some(action),
        error: None,
    };

    match plan.decision {
        Decision::Conflict => return blocked(UserAction::ResolveConflict),
        Decision::Retired | Decision::NoChange => {
            return LedgerItem {
                item_key: plan.item_key.clone(),
                decision,
                channel_id: plan.channel_id,
                generation: plan.generation,
                // provenance에 적힌 실제 상태다. `NoChange`는 정의상 전부
                // `Done`이지만 `Retired`는 아니다 — 미완료인 채로 catalog에서
                // 빠진 항목이 있다. 여기서 완료를 지어내면 ledger가 없었던
                // 성공을 보고한다.
                steps: plan.steps,
                outcome: Outcome::Unchanged,
                user_action: None,
                error: None,
            };
        }
        Decision::CreateOrRecreate | Decision::Resume => {}
    }

    let Some(item) = catalog.item(&plan.item_key) else {
        return blocked(UserAction::ResolveConflict);
    };

    let channel_id = derive_channel_id(
        relay_scope,
        &catalog.catalog_id,
        &plan.item_key,
        plan.generation,
    );

    let mut provenance = Provenance {
        catalog_id: catalog.catalog_id.clone(),
        catalog_version: catalog.catalog_version,
        item_key: plan.item_key.clone(),
        generation: plan.generation,
        // preflight가 이미 읽어 온 상태를 그대로 쓴다. 여기서 provenance를
        // 다시 읽지 않는다 — 두 번째 읽기가 실패하면 단계 상태가 전부
        // `Pending`으로 되돌아가고, 그러면 살아 있는 채널에 `create_channel`을
        // 걸어 `Duplicate`를 받은 뒤 "사용자가 지웠다"로 오판해 방을 하나 더
        // 만들게 된다. `CreateOrRecreate`에서는 이 값이 전부 `Pending`이다.
        steps: plan.steps,
        applied_at: now.to_string(),
    };

    let mut error: Option<String> = None;

    // 단계 1 — 채널 생성.
    if provenance.steps.channel != StepStatus::Done {
        match effects
            .create_channel(ChannelSpec {
                id: channel_id,
                name: item.name.clone(),
                description: item.description.clone(),
                channel_type: item.channel_type.clone(),
                visibility: item.visibility,
            })
            .await
        {
            Ok(CreateOutcome::Created) => provenance.steps.channel = StepStatus::Done,
            // §7의 `deleted` 조건은 두 절이다: `duplicate` **그리고** 접근
            // 불가. `duplicate` 하나만으로는 두 상태를 구분할 수 없다.
            Ok(CreateOutcome::Duplicate) if !plan.channel_present => {
                // ID가 이미 점유돼 있는데 접근 가능 목록에 없다 —
                // 예전에 만들었다가 삭제된 항목이다. 자동 재생성하지 않는다.
                return LedgerItem {
                    item_key: plan.item_key.clone(),
                    decision: DELETED_DECISION.to_string(),
                    channel_id: Some(channel_id),
                    generation: plan.generation,
                    steps: provenance.steps,
                    outcome: Outcome::Blocked,
                    user_action: Some(UserAction::ConfirmRecreate),
                    error: None,
                };
            }
            Ok(CreateOutcome::Duplicate) => {
                // ID가 점유돼 있는데 접근도 된다. 결정론적 ID라 이건 우리가
                // 만든 방이다 — relay는 생성을 커밋했는데 클라이언트가
                // provenance를 쓰기 전에 죽은 경우다(§5가 결정론적 ID를 두는
                // 이유가 정확히 이것이다). 생성은 이미 끝났으므로 다음
                // 단계로 넘어간다.
                provenance.steps.channel = StepStatus::Done;
            }
            Err(e) => {
                provenance.steps.channel = StepStatus::Failed;
                error = Some(e.0);
            }
        }
    }

    // 단계 2 — 시작 캔버스.
    if error.is_none() && provenance.steps.canvas != StepStatus::Done {
        match effects.set_canvas(channel_id, &item.canvas).await {
            Ok(()) => provenance.steps.canvas = StepStatus::Done,
            Err(e) => {
                provenance.steps.canvas = StepStatus::Failed;
                error = Some(e.0);
            }
        }
    }

    // 단계 3 — owner 확인.
    if error.is_none() && provenance.steps.membership != StepStatus::Done {
        match effects.is_owner(channel_id).await {
            Ok(true) => provenance.steps.membership = StepStatus::Done,
            Ok(false) => {
                provenance.steps.membership = StepStatus::Failed;
                error = Some("적용자가 채널 owner가 아닙니다".to_string());
            }
            Err(e) => {
                provenance.steps.membership = StepStatus::Failed;
                error = Some(e.0);
            }
        }
    }

    // 어디까지 됐든 provenance를 남긴다. 이게 없으면 재시도가 처음부터 한다.
    if provenance.steps.channel == StepStatus::Done {
        if let Err(e) = effects.publish_provenance(channel_id, &provenance).await {
            error.get_or_insert(e.0);
        }
    }

    // `Applied`는 세 단계가 끝난 것만으로는 부족하다 — provenance 발행이
    // 실패해도 단계는 전부 `Done`이다. 그 경우 durable하게 남은 것이
    // 없으므로 다음 실행은 provenance를 찾지 못하고 이 항목을 "적용한 적
    // 없음"으로 본다. `error`가 있는데 `applied`를 보고하면 ledger가 자기
    // 자신과 모순되고, `outcome`만 보는 소비자는 성공으로 읽는다.
    let applied = provenance.is_complete() && error.is_none();
    LedgerItem {
        item_key: plan.item_key,
        decision,
        channel_id: Some(channel_id),
        generation: plan.generation,
        steps: provenance.steps,
        outcome: if applied {
            Outcome::Applied
        } else {
            Outcome::Partial
        },
        user_action: None,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Visibility;
    use crate::effects::fake::FakeEffects;
    use crate::effects::ChannelRef;
    use crate::ledger::Outcome;
    use uuid::Uuid;

    fn both() -> Vec<String> {
        vec!["meeting".to_string(), "planning".to_string()]
    }

    fn item<'a>(ledger: &'a Ledger, key: &str) -> &'a LedgerItem {
        ledger
            .items
            .iter()
            .find(|i| i.item_key == key)
            .expect("item present")
    }

    fn canvas_of(item_key: &str) -> &'static str {
        &crate::builtin()
            .item(item_key)
            .expect("catalog item")
            .canvas
    }

    /// 이미 적용됐거나 적용 중이던 항목을 세대 1로 시드한다.
    ///
    /// provenance는 채널 스코프라 그 채널이 `channels`에 있어야
    /// `fetch_provenance`에 보인다. `burned_ids`도 같이 채운다 — relay는 한
    /// 번 쓴 ID를 영구히 점유하므로 "채널은 있는데 ID는 비어 있다"는 상태는
    /// 실제로 만들어질 수 없고, 그런 상태로 시드하면 재생성 경로가 테스트에서만
    /// 성공한다. `owned`는 적용자가 만든 방이라는 뜻이다.
    fn seed_applied(fx: &FakeEffects, item_key: &str, name: &str, steps: StepStates) -> Uuid {
        let channel_id = derive_channel_id("wss://relay.test", "schoolx.default", item_key, 1);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: name.into(),
        });
        fx.burned_ids.lock().expect("lock").insert(channel_id);
        fx.owned.lock().expect("lock").insert(channel_id);
        fx.provenance.lock().expect("lock").push((
            channel_id,
            Provenance {
                catalog_id: "schoolx.default".into(),
                catalog_version: 1,
                item_key: item_key.into(),
                generation: 1,
                steps,
                applied_at: "2026-07-28T09:00:00Z".into(),
            },
        ));
        channel_id
    }

    #[tokio::test]
    async fn first_apply_creates_both_rooms() {
        let fx = FakeEffects::new();
        let ledger = apply(crate::builtin(), &fx, &both()).await.expect("apply");

        assert_eq!(ledger.items.len(), 2);
        for entry in &ledger.items {
            assert_eq!(entry.outcome, Outcome::Applied, "{}", entry.item_key);
        }
        assert_eq!(fx.channels.lock().expect("lock").len(), 2);
    }

    #[tokio::test]
    async fn second_apply_changes_nothing() {
        let fx = FakeEffects::new();
        apply(crate::builtin(), &fx, &both()).await.expect("first");
        let before = fx.channels.lock().expect("lock").len();
        let published_before = fx.published.lock().expect("lock").len();

        let ledger = apply(crate::builtin(), &fx, &both()).await.expect("second");

        for entry in &ledger.items {
            assert_eq!(entry.outcome, Outcome::Unchanged, "{}", entry.item_key);
        }
        assert_eq!(fx.channels.lock().expect("lock").len(), before);
        // 변경 없음이면 provenance를 다시 발행하지도 않는다. `published`는
        // 필터 없는 append-only 로그라 발행 횟수를 그대로 센다.
        assert_eq!(fx.published.lock().expect("lock").len(), published_before);
    }

    #[tokio::test]
    async fn only_selected_items_are_applied() {
        let fx = FakeEffects::new();
        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        assert_eq!(ledger.items.len(), 1);
        assert_eq!(ledger.items[0].item_key, "meeting");
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn canvas_failure_leaves_the_channel_and_retry_finishes_it() {
        let fx = FakeEffects::new();
        fx.fail_next("set_canvas");

        let first = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");
        let entry = item(&first, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        assert_eq!(entry.steps.channel, StepStatus::Done);
        assert_eq!(entry.steps.canvas, StepStatus::Failed);
        // 보상하지 않는다 — 채널은 그대로 있다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);

        let second = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        let entry = item(&second, "meeting");
        assert_eq!(entry.outcome, Outcome::Applied);
        // 채널이 중복 생성되지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        // 재시도가 캔버스를 **실제로** 썼다. `Applied`만 확인하면 실패한
        // 단계를 그냥 완료로 표시하고 넘어가는 saga도 통과한다.
        let channel_id = entry.channel_id.expect("channel id");
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&canvas_of("meeting").to_string()),
            "재시도가 시작 캔버스를 쓰지 않았다"
        );
    }

    #[tokio::test]
    async fn channel_failure_retries_from_the_start_without_duplicates() {
        let fx = FakeEffects::new();
        fx.fail_next("create_channel");

        let first = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");
        assert_eq!(item(&first, "meeting").outcome, Outcome::Partial);
        assert_eq!(fx.channels.lock().expect("lock").len(), 0);

        let second = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        assert_eq!(item(&second, "meeting").outcome, Outcome::Applied);
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn deleted_channel_is_not_recreated() {
        let fx = FakeEffects::new();
        apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");

        // 사용자가 방을 지운다: 접근 가능 목록에서 사라지지만 ID는 계속 탄 채다.
        // provenance는 손대지 않는다 — 채널 스코프라 채널이 사라지는 것만으로
        // 읽을 수 없게 되고, fake도 그렇게 동작한다. 두 저장소를 같이 비우면
        // relay가 만들 수 없는 상태를 테스트가 대신 만들어 주는 셈이 된다.
        fx.channels.lock().expect("lock").clear();

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("after delete");
        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Blocked);
        assert_eq!(entry.user_action, Some(UserAction::ConfirmRecreate));
        assert_eq!(entry.decision, "deleted");
        // 자동으로 다시 만들지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 0);
    }

    #[tokio::test]
    async fn name_conflict_blocks_without_touching_anything() {
        let fx = FakeEffects::new();
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: uuid::Uuid::new_v4(),
            name: "기획".into(),
        });

        let ledger = apply(crate::builtin(), &fx, &["planning".to_string()])
            .await
            .expect("apply");
        let entry = item(&ledger, "planning");
        assert_eq!(entry.outcome, Outcome::Blocked);
        assert_eq!(entry.user_action, Some(UserAction::ResolveConflict));
        // 사용자 채널을 채택하지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        // 채널 **수**만 보면, 사용자가 만들어 둔 그 채널에 우리 캔버스와
        // provenance를 써 넣은 saga도 통과한다. 채널을 새로 만들지 않는 것과
        // 아무것도 건드리지 않는 것은 다르다.
        assert!(
            fx.created.lock().expect("lock").is_empty(),
            "생성 요청을 보내지 않았어야 한다"
        );
        assert!(
            fx.canvases.lock().expect("lock").is_empty(),
            "사용자 채널에 캔버스를 쓰지 않았어야 한다"
        );
        assert!(
            fx.published.lock().expect("lock").is_empty(),
            "사용자 채널에 provenance를 발행하지 않았어야 한다"
        );
    }

    #[tokio::test]
    async fn one_item_failing_does_not_block_the_other() {
        let fx = FakeEffects::new();
        // 막히는 항목은 catalog의 **첫 번째** 항목(`meeting`)이다. 마지막
        // 항목에 두면 첫 실패에서 루프를 중단해 버리는 saga도 통과한다.
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: Uuid::new_v4(),
            name: "메인 회의방".into(),
        });

        let ledger = apply(crate::builtin(), &fx, &both()).await.expect("apply");
        assert_eq!(ledger.items.len(), 2);
        assert_eq!(item(&ledger, "meeting").outcome, Outcome::Blocked);
        assert_eq!(item(&ledger, "planning").outcome, Outcome::Applied);
    }

    /// preflight가 이미 읽어 온 단계 상태를 saga가 그대로 쓴다 — provenance를
    /// 다시 읽지 않는다.
    ///
    /// 두 번째 읽기는 네트워크 오류 하나로 실패할 수 있고, 그 실패를 삼키면
    /// 단계가 전부 `Pending`으로 되돌아간다. 그러면 정의상 아직 살아 있는
    /// 채널에 `create_channel`을 걸어 `Duplicate`를 받고, 이미 끝난 단계를
    /// 처음부터 다시 하거나 "사용자가 지웠다"로 오판해 방을 하나 더 만든다.
    #[tokio::test]
    async fn resume_does_not_refetch_provenance() {
        let fx = FakeEffects::new();
        seed_applied(
            &fx,
            "meeting",
            "메인 회의방",
            StepStates {
                channel: StepStatus::Done,
                canvas: StepStatus::Done,
                membership: StepStatus::Pending,
            },
        );
        // preflight의 읽기가 1번째다. saga가 또 읽으면 그 2번째가 실패한다.
        fx.fail_nth("fetch_provenance", 2);

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");
        let entry = item(&ledger, "meeting");
        assert_eq!(entry.decision, "resume");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.error, None);
        assert_eq!(
            fx.call_count("fetch_provenance"),
            1,
            "provenance를 두 번 읽었다 — preflight가 이미 읽은 값을 써야 한다"
        );
        // 이미 `Done`인 단계를 다시 하지 않았다. 캔버스가 그 관측 지점이다.
        assert!(
            fx.canvases.lock().expect("lock").is_empty(),
            "완료된 캔버스 단계를 다시 실행했다"
        );
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    /// 생성이 `duplicate`인데 그 채널에 접근이 되면 "삭제됨"이 아니다.
    ///
    /// relay가 생성을 커밋한 뒤 provenance를 쓰기 전에 클라이언트가 죽으면
    /// 이 상태가 된다 — §5가 결정론적 ID를 두는 이유가 정확히 이 경우를
    /// 흡수하기 위해서다. §7의 `deleted`는 `duplicate` **그리고** 접근
    /// 불가일 때만 성립한다.
    #[tokio::test]
    async fn duplicate_but_accessible_channel_continues_instead_of_blocking() {
        let fx = FakeEffects::new();
        let channel_id = derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1);
        // 채널은 만들어졌고 접근도 된다. provenance만 없다.
        // 이름은 catalog 값과 다르게 둔다 — 이름이 판정에 끼어 있으면
        // (§7이 금지한다) 이 케이스가 `Conflict`로 새어 나가 이 테스트가
        // 검증하려는 경로에 아예 도달하지 못한다.
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: "2026 전체회의".into(),
        });
        fx.burned_ids.lock().expect("lock").insert(channel_id);
        fx.owned.lock().expect("lock").insert(channel_id);

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");
        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.user_action, None);
        assert_eq!(entry.decision, "create_or_recreate");
        assert_eq!(entry.steps.channel, StepStatus::Done);
        // 방을 하나 더 만들지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        // 막히지 않고 캔버스 단계까지 이어서 끝냈다.
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&canvas_of("meeting").to_string())
        );
    }

    /// provenance 발행이 실패하면 `applied`가 아니다.
    ///
    /// 세 단계가 전부 `Done`이어도 발행이 실패하면 durable하게 남은 것이
    /// 없다 — 다음 실행은 provenance를 찾지 못해 이 항목을 "적용한 적 없음"
    /// 으로 본다. `error`를 실은 채 `applied`를 보고하면 ledger가 자기
    /// 모순이고, `outcome`으로만 분기하는 소비자는 성공으로 읽는다.
    #[tokio::test]
    async fn provenance_publish_failure_is_partial_not_applied() {
        let fx = FakeEffects::new();
        fx.fail_next("publish_provenance");

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");
        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        assert!(entry.error.is_some(), "발행 실패 사유가 실려야 한다");
        // 세 단계는 실제로 끝났다 — 문제는 그 사실이 어디에도 남지 않은 것이다.
        assert_eq!(entry.steps.channel, StepStatus::Done);
        assert_eq!(entry.steps.canvas, StepStatus::Done);
        assert_eq!(entry.steps.membership, StepStatus::Done);
        assert!(
            fx.provenance.lock().expect("lock").is_empty(),
            "발행이 실패했으므로 저장된 provenance가 없어야 한다"
        );
    }

    /// catalog에서 빠진 항목은 provenance에 적힌 실제 단계 상태를 보고한다.
    ///
    /// `Retired`가 완료를 뜻하지는 않는다 — 적용이 끝나기 전에 catalog에서
    /// 빠질 수 있다(`preflight`의 `incomplete_item_dropped_from_catalog_is_retired`
    /// 가 그 경우다). 세 단계를 `done`으로 지어내면 실패한 단계가 조용히
    /// 사라지고 ledger가 없었던 성공을 보고한다.
    #[tokio::test]
    async fn retired_item_reports_its_real_steps() {
        let fx = FakeEffects::new();
        let steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        seed_applied(&fx, "finance", "재무", steps);

        let ledger = apply(crate::builtin(), &fx, &["finance".to_string()])
            .await
            .expect("apply");
        let entry = item(&ledger, "finance");
        assert_eq!(entry.decision, "retired");
        assert_eq!(entry.outcome, Outcome::Unchanged);
        assert_eq!(entry.steps, steps);
        // 기존 채널은 유지하고 아무것도 건드리지 않는다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        assert!(fx.canvases.lock().expect("lock").is_empty());
        assert!(fx.published.lock().expect("lock").is_empty());
    }

    /// 내장 방 둘 다 `private`으로 만들어진다.
    ///
    /// saga가 `ChannelSpec`에 실어 보내는 값을 검증한다 — `channels`는
    /// `ChannelRef`(id + 이름)만 남기므로 `visibility`·`description`·
    /// `channel_type`은 `created` 로그로만 관측된다.
    #[tokio::test]
    async fn builtin_rooms_are_created_private() {
        let fx = FakeEffects::new();
        apply(crate::builtin(), &fx, &both()).await.expect("apply");

        let created = fx.created.lock().expect("lock");
        assert_eq!(created.len(), 2);
        for catalog_item in &crate::builtin().items {
            let spec = created
                .iter()
                .find(|s| s.name == catalog_item.name)
                .unwrap_or_else(|| panic!("{} 생성 요청이 없다", catalog_item.item_key));
            assert_eq!(
                spec.visibility,
                Visibility::Private,
                "{}은 private으로 만들어져야 한다",
                catalog_item.item_key
            );
            assert_eq!(spec.description, catalog_item.description);
            assert_eq!(spec.channel_type, catalog_item.channel_type);
        }
    }

    /// Golden-value regression test for the result ledger wire format.
    ///
    /// 이 JSON은 데스크톱 앱과 CLI가 **함께** 읽는 wire format이다 — 두
    /// 소비자 모두 `outcome`과 `user_action` 문자열로 분기한다. 필드 이름이나
    /// enum 철자를 바꾸는 것은 breaking change이고, 한 소비자만 따라 고치면
    /// 다른 쪽이 조용히 깨진다.
    ///
    /// `"applied"` 하나만 확인하던 예전 형태로는 `blocked`/`partial`이나
    /// `confirm_recreate`/`resolve_conflict` 철자가 바뀌어도 통과했다. 실패를
    /// 없애려고 아래 리터럴을 고치지 말 것 — 회귀로 보고 코드를 고칠 것.
    #[tokio::test]
    async fn ledger_serializes_for_ui_and_cli() {
        // (1) saga가 실제로 내는 ledger가 이 형식으로 직렬화된다.
        let fx = FakeEffects::new();
        let produced = apply(crate::builtin(), &fx, &both()).await.expect("apply");
        let json = serde_json::to_string(&produced).expect("serialize");
        assert!(json.contains("\"outcome\":\"applied\""));

        // (2) 네 `outcome`과 두 `user_action`을 바이트 단위로 고정한다.
        let ledger = Ledger {
            catalog_id: "schoolx.default".into(),
            catalog_version: 1,
            items: vec![
                LedgerItem {
                    item_key: "meeting".into(),
                    decision: "create_or_recreate".into(),
                    channel_id: Some(Uuid::nil()),
                    generation: 1,
                    steps: StepStates {
                        channel: StepStatus::Done,
                        canvas: StepStatus::Done,
                        membership: StepStatus::Done,
                    },
                    outcome: Outcome::Applied,
                    user_action: None,
                    error: None,
                },
                LedgerItem {
                    item_key: "planning".into(),
                    decision: "resume".into(),
                    channel_id: Some(Uuid::nil()),
                    generation: 2,
                    steps: StepStates {
                        channel: StepStatus::Done,
                        canvas: StepStatus::Failed,
                        membership: StepStatus::Pending,
                    },
                    outcome: Outcome::Partial,
                    user_action: None,
                    error: Some("캔버스 적용 실패".into()),
                },
                LedgerItem {
                    item_key: "finance".into(),
                    decision: "deleted".into(),
                    channel_id: Some(Uuid::nil()),
                    generation: 1,
                    steps: StepStates::default(),
                    outcome: Outcome::Blocked,
                    user_action: Some(UserAction::ConfirmRecreate),
                    error: None,
                },
                LedgerItem {
                    item_key: "hr".into(),
                    decision: "conflict".into(),
                    channel_id: None,
                    generation: 1,
                    steps: StepStates::default(),
                    outcome: Outcome::Blocked,
                    user_action: Some(UserAction::ResolveConflict),
                    error: None,
                },
                LedgerItem {
                    item_key: "sales".into(),
                    decision: "no_change".into(),
                    channel_id: Some(Uuid::nil()),
                    generation: 1,
                    steps: StepStates {
                        channel: StepStatus::Done,
                        canvas: StepStatus::Done,
                        membership: StepStatus::Done,
                    },
                    outcome: Outcome::Unchanged,
                    user_action: None,
                    error: None,
                },
            ],
        };

        let actual = serde_json::to_value(&ledger).expect("serialize");
        let expected = serde_json::json!({
            "catalog_id": "schoolx.default",
            "catalog_version": 1,
            "items": [
                {
                    "item_key": "meeting",
                    "decision": "create_or_recreate",
                    "channel_id": "00000000-0000-0000-0000-000000000000",
                    "generation": 1,
                    "steps": { "channel": "done", "canvas": "done", "membership": "done" },
                    "outcome": "applied",
                    "user_action": null,
                    "error": null
                },
                {
                    "item_key": "planning",
                    "decision": "resume",
                    "channel_id": "00000000-0000-0000-0000-000000000000",
                    "generation": 2,
                    "steps": { "channel": "done", "canvas": "failed", "membership": "pending" },
                    "outcome": "partial",
                    "user_action": null,
                    "error": "캔버스 적용 실패"
                },
                {
                    "item_key": "finance",
                    "decision": "deleted",
                    "channel_id": "00000000-0000-0000-0000-000000000000",
                    "generation": 1,
                    "steps": { "channel": "pending", "canvas": "pending", "membership": "pending" },
                    "outcome": "blocked",
                    "user_action": "confirm_recreate",
                    "error": null
                },
                {
                    "item_key": "hr",
                    "decision": "conflict",
                    "channel_id": null,
                    "generation": 1,
                    "steps": { "channel": "pending", "canvas": "pending", "membership": "pending" },
                    "outcome": "blocked",
                    "user_action": "resolve_conflict",
                    "error": null
                },
                {
                    "item_key": "sales",
                    "decision": "no_change",
                    "channel_id": "00000000-0000-0000-0000-000000000000",
                    "generation": 1,
                    "steps": { "channel": "done", "canvas": "done", "membership": "done" },
                    "outcome": "unchanged",
                    "user_action": null,
                    "error": null
                }
            ]
        });

        assert_eq!(
            actual, expected,
            "result ledger wire format changed (field names, outcome/user_action/step spellings) — \
             데스크톱 앱과 CLI가 함께 읽는 형식이다, 위 doc comment 참고"
        );
    }
}
