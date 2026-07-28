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
use crate::ledger::{decision_label, Ledger, LedgerItem, Outcome, UserAction};
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
                steps: StepStates {
                    channel: StepStatus::Done,
                    canvas: StepStatus::Done,
                    membership: StepStatus::Done,
                },
                outcome: Outcome::Unchanged,
                user_action: None,
                error: None,
            }
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
        steps: StepStates::default(),
        applied_at: now.to_string(),
    };

    if plan.decision == Decision::Resume {
        if let Ok(existing) = effects.fetch_provenance(&catalog.catalog_id).await {
            if let Some(p) = existing.iter().find(|p| p.item_key == plan.item_key) {
                provenance.steps = p.steps;
            }
        }
    }

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
            Ok(CreateOutcome::Duplicate) => {
                // ID가 이미 점유돼 있는데 접근 가능 목록에 없다 —
                // 예전에 만들었다가 삭제된 항목이다. 자동 재생성하지 않는다.
                return LedgerItem {
                    item_key: plan.item_key.clone(),
                    decision: "deleted".to_string(),
                    channel_id: Some(channel_id),
                    generation: plan.generation,
                    steps: provenance.steps,
                    outcome: Outcome::Blocked,
                    user_action: Some(UserAction::ConfirmRecreate),
                    error: None,
                };
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

    let complete = provenance.is_complete();
    LedgerItem {
        item_key: plan.item_key,
        decision,
        channel_id: Some(channel_id),
        generation: plan.generation,
        steps: provenance.steps,
        outcome: if complete {
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
    use crate::effects::fake::FakeEffects;
    use crate::effects::ChannelRef;
    use crate::ledger::Outcome;

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
    }

    #[tokio::test]
    async fn one_item_failing_does_not_block_the_other() {
        let fx = FakeEffects::new();
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: uuid::Uuid::new_v4(),
            name: "기획".into(),
        });

        let ledger = apply(crate::builtin(), &fx, &both()).await.expect("apply");
        assert_eq!(item(&ledger, "planning").outcome, Outcome::Blocked);
        assert_eq!(item(&ledger, "meeting").outcome, Outcome::Applied);
    }

    #[tokio::test]
    async fn ledger_serializes_for_ui_and_cli() {
        let fx = FakeEffects::new();
        let ledger = apply(crate::builtin(), &fx, &both()).await.expect("apply");
        let json = serde_json::to_string(&ledger).expect("serialize");
        assert!(json.contains("\"outcome\":\"applied\""));
    }
}
