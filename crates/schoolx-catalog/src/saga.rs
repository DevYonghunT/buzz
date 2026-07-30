//! idempotent saga 실행기.
//!
//! 단계는 채널 생성 → 시작 캔버스 → owner 확인이다. 각 단계는 provenance를
//! 보고 완료면 건너뛰고, 실행하고, provenance를 갱신한다.
//!
//! **실패해도 되돌리지 않는다.** 채널을 만든 뒤 캔버스에서 실패하면 채널을
//! 지우지 않고 상태만 기록한다. 재시도가 캔버스부터 이어서 한다.
//!
//! **내용이 있는 캔버스는 덮어쓰지 않는다.** 캔버스 단계는 쓰기 전에 그 방의
//! 현재 캔버스를 읽고, 지켜야 할 내용이 있으면 쓰지 않고 `skipped`로 적는다.

use crate::catalog::Catalog;
use crate::channel_id::derive_channel_id;
use crate::effects::{CatalogEffects, ChannelSpec, CreateOutcome, EffectError};
use crate::ledger::{
    decision_label, Ledger, LedgerItem, Outcome, UserAction, ADOPTED_DECISION, DELETED_DECISION,
    NOT_OWNED_DECISION,
};
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

/// 단계 1이 끝난 뒤, 이 방이 어디서 왔는가.
///
/// 아래 owner 게이트가 이 값 하나로 갈린다. **이번 실행이 직접 만든 방만**
/// 게이트를 건너뛴다 — 만든 사람이 곧 owner이기 때문이다. 나머지는 전부
/// 이번 실행 이전부터 있던 방이고, 그 방이 적용자의 방이라는 근거가 없다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelOrigin {
    /// 이번 실행이 직접 만들었다.
    CreatedHere,
    /// provenance가 채널 단계를 완료로 적고 있어 생성을 건너뛰었다. 만든
    /// 것은 **이전 실행**이고, 그게 이번 적용자였다는 보장은 없다.
    CreateSkipped,
    /// 생성이 `duplicate`로 거부됐는데 그 방에 접근이 된다.
    CreateDuplicate,
}

async fn apply_item(
    catalog: &Catalog,
    effects: &dyn CatalogEffects,
    relay_scope: &str,
    now: &str,
    plan: PreflightItem,
) -> LedgerItem {
    // 채택 경로에서만 바뀐다 — 아래 `Duplicate` 분기 참고.
    let mut decision = decision_label(plan.decision).to_string();

    let blocked = |action: UserAction| LedgerItem {
        item_key: plan.item_key.clone(),
        name: plan.name.clone(),
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
                // `Retired`에서는 `None`이다 — preflight가 그렇게 싣는다.
                // 여기서 `item_key`로 메우면 UI가 "이름을 모른다"를 이름으로
                // 읽는다.
                name: plan.name.clone(),
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
    // 단계 3(owner 확인)이 `Ok(false)`일 때만 쓴다 — 아래 그 분기의 주석 참고.
    let mut user_action: Option<UserAction> = None;

    // 단계 1이 이 방을 어떻게 얻었는가 — 아래 owner 게이트가 이 값으로
    // 갈린다. 기본값이 `CreateSkipped`인 이유: provenance가 채널 단계를
    // 완료로 적고 있으면 아래 블록이 통째로 실행되지 않는다.
    let mut origin = ChannelOrigin::CreateSkipped;

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
            Ok(CreateOutcome::Created) => {
                origin = ChannelOrigin::CreatedHere;
                provenance.steps.channel = StepStatus::Done;
            }
            // §7의 `deleted` 조건은 두 절이다: `duplicate` **그리고** 접근
            // 불가. `duplicate` 하나만으로는 두 상태를 구분할 수 없다.
            Ok(CreateOutcome::Duplicate) if !plan.channel_present => {
                // ID가 이미 점유돼 있는데 접근 가능 목록에 없다 —
                // 예전에 만들었다가 삭제된 항목이다. 자동 재생성하지 않는다.
                return LedgerItem {
                    item_key: plan.item_key.clone(),
                    name: plan.name.clone(),
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
                // ID가 점유돼 있는데 접근도 된다. 결정론적 ID라 이건 우리
                // catalog가 만든 방이다 — relay는 생성을 커밋했는데
                // 클라이언트가 provenance를 쓰기 전에 죽은 경우다(§5가
                // 결정론적 ID를 두는 이유가 정확히 이것이다).
                //
                // 그렇다고 **쓸 권한**이 생기지는 않는다. 판단은 아래 owner
                // 게이트가 한다. 채널 단계를 여기서 완료로 적지 않는 이유도
                // 그것이다 — 게이트가 막으면 ledger가 하지도 않은 완료를
                // 보고하게 된다.
                origin = ChannelOrigin::CreateDuplicate;
            }
            Err(e) => {
                provenance.steps.channel = StepStatus::Failed;
                error = Some(e.0);
            }
        }
    }

    // 권한 게이트 — **이번 실행이 만들지 않은 방은 첫 쓰기 전에 owner를
    // 확인한다.**
    //
    // 규칙은 하나다. 만든 사람이 곧 owner이므로 `CreatedHere`만 건너뛰고,
    // 나머지 둘은 이번 실행 이전부터 있던 방이라 확인이 필요하다. 그 둘이
    // 각각 §7의 채택 경로와 재개 경로다.
    //
    // - 채택(`CreateDuplicate`): 증명서를 남기지 못한 것은 관리자 A인데,
    //   그 방의 멤버일 뿐인 관리자 B가 적용을 돌릴 수 있다. B에게는 방이
    //   보이고 증명서는 안 보이므로 판정이 여기까지 그대로 온다.
    // - 재개(`CreateSkipped`): provenance는 **채널 스코프**라 owner가 아니라
    //   **멤버**면 읽힌다. B가 미완료 증명서를 읽으면 preflight가 `Resume`을
    //   내고, 그러면 단계 1이 통째로 건너뛰어져 게이트 없이 캔버스로 내려간다.
    //
    // 재개 쪽이 오히려 더 흔하다. 채택은 증명서가 아예 없고 방 이름까지
    // 바뀌어야 도달하지만, 미완료 증명서는 부분 실패의 **정상적인 결과**다.
    //
    // 어느 쪽이든 캔버스를 먼저 쓰고 나중에 확인하면, 확인이 실패한 시점에는
    // 팀이 그 방에 써 둔 내용이 이미 사라진 뒤다 — 되돌릴 수 없다.
    if error.is_none() && origin != ChannelOrigin::CreatedHere {
        match effects.is_owner(channel_id).await {
            Ok(true) => {
                if origin == ChannelOrigin::CreateDuplicate {
                    // 이번 실행이 만든 방이 아니라 이미 있던 방을 이어받는
                    // 것이다. ledger가 그 차이를 말하게 한다. 재개는 이미
                    // 증명서가 있는 항목이라 판정이 `resume` 그대로다.
                    decision = ADOPTED_DECISION.to_string();
                    provenance.steps.channel = StepStatus::Done;
                }
            }
            Ok(false) => {
                // 남의 방이다. 아무것도 쓰지 않고 사용자에게 넘긴다 —
                // `Conflict`와 같은 모양이다. 단계 상태는 preflight가 읽어 온
                // 그대로 보고한다: 채택 경로에서는 전부 `Pending`이고, 재개
                // 경로에서는 이전 실행이 남긴 실제 진행이다. 여기서 지어내면
                // ledger가 이번 실행이 하지 않은 일을 보고한다.
                return LedgerItem {
                    item_key: plan.item_key.clone(),
                    name: plan.name.clone(),
                    decision: NOT_OWNED_DECISION.to_string(),
                    channel_id: Some(channel_id),
                    generation: plan.generation,
                    steps: provenance.steps,
                    outcome: Outcome::Blocked,
                    user_action: Some(UserAction::RequestOwnership),
                    error: None,
                };
            }
            Err(e) => {
                // owner인지 모르는 채로는 채택도 차단도 확정할 수 없다.
                // 쓰지 않는 쪽이 안전한 실패다 — `error`가 실리므로 캔버스
                // 단계로 내려가지 않고, 채널 단계가 `Done`이 아니라
                // provenance도 발행되지 않는다. 재개 경로에서는 이미 `Done`
                // 이던 값을 `Failed`로 덮어써서 그 발행을 막는다. relay에
                // 남아 있는 증명서는 건드리지 않았으므로 재시도가 다시 재개
                // 판정을 받고 여기서 다시 묻는다.
                provenance.steps.channel = StepStatus::Failed;
                error = Some(e.0);
            }
        }
    }

    // 단계 2 — 시작 캔버스. **내용이 있는 캔버스는 덮어쓰지 않는다.**
    //
    // 위 owner 게이트는 *누가* 써도 되는가만 가른다. 쓸 권한이 있어도 그
    // 방에 지켜야 할 내용이 있을 수 있다 — 캔버스 단계가 미완료로 남는 것은
    // 부분 실패의 정상적인 결과이고(§8 「실패해도 되돌리지 않는다」), 그 사이
    // 팀이 그 방을 쓰기 시작하는 것도 정상이다. 그 상태에서 관리자가 재시도를
    // 돌리면, 조건 없이 쓰는 saga는 팀이 써 둔 내용을 catalog 기본값으로
    // 지운다. 되돌릴 수 없다.
    //
    // 그래서 쓰기 전에 현재 캔버스를 읽는다. 읽기를 이번 실행이 만든 방까지
    // 포함해 **무조건** 하는 이유는 규칙을 하나로 두기 위해서다 — 방금 만든
    // 방은 어차피 비어 있는 것으로 읽힌다.
    if error.is_none() && !provenance.steps.canvas.is_settled() {
        match effects.read_canvas(channel_id).await {
            Ok(Some(existing)) if !existing.trim().is_empty() => {
                // 쓰지 않았다는 사실을 `Done`과 **구별해서** 남긴다. 조용히
                // 넘어가면 ledger는 catalog 캔버스를 넣은 것처럼 보이고,
                // 사용자는 자기 내용이 그대로 남았다는 사실도 이 항목이
                // 왜 catalog와 다른지도 읽을 방법이 없다.
                provenance.steps.canvas = StepStatus::Skipped;
            }
            // 공백만 있는 캔버스는 지켜야 할 내용이 아니다. 그걸 내용으로
            // 세면 사실상 비어 있는 방에 시작 캔버스가 영영 들어가지 않는다.
            Ok(_) => match effects.set_canvas(channel_id, &item.canvas).await {
                Ok(()) => provenance.steps.canvas = StepStatus::Done,
                Err(e) => {
                    provenance.steps.canvas = StepStatus::Failed;
                    error = Some(e.0);
                }
            },
            Err(e) => {
                // 지켜야 할 내용이 있는지 **모르는** 채로는 쓰지 않는다.
                // 위 owner 확인 실패와 같은 규칙이다: 잘못 쓰면 되돌릴 수
                // 없고, 쓰지 않으면 잃는 것은 이번 실행의 진행뿐이다.
                // `Failed`로 남으므로 다음 실행이 `resume`으로 들어와 이
                // 단계부터 이어서 하고 그때 다시 묻는다 — 재시도가 막히지
                // 않는다.
                provenance.steps.canvas = StepStatus::Failed;
                error = Some(e.0);
            }
        }
    }

    // 단계 3 — owner 확인.
    //
    // 위 게이트가 이미 물어본 경로(채택·재개)에서도 여기서 다시 묻는다.
    // 게이트는 "써도 되는가"를 가르는 것이고, 이 단계는 그 사실을 증명서에
    // 남기는 것이다. 게이트를 통과했다고 이 단계를 완료로 지어내면, 게이트를
    // 지나지 않는 생성 경로에서는 이 단계가 **유일한** owner 검증인데 그
    // 사실이 코드에서 보이지 않게 된다.
    if error.is_none() && provenance.steps.membership != StepStatus::Done {
        match effects.is_owner(channel_id).await {
            Ok(true) => provenance.steps.membership = StepStatus::Done,
            Ok(false) => {
                provenance.steps.membership = StepStatus::Failed;
                // 하드코딩한 문장 대신 이미 있는 어휘를 쓴다. 위 owner
                // 게이트가 `Ok(false)`일 때 내는 것과 같은 사실 — 적용자가
                // 이 채널의 owner가 아니다 — 이므로 같은 `UserAction`을
                // 싣는다. `not_owned`/`request_ownership`는 이미
                // `catalog.decision.*`·`catalog.userAction.*`로 번역돼
                // 있어(`desktop/src/shared/i18n/locales/{en,ko}.ts`) 설정
                // 카드가 그대로 현지화해 보여준다 — `error`에 넣는 문자열은
                // 그 카드가 번역 없이 그대로 렌더한다(`ledgerItem.error`,
                // `WorkspaceCatalogSettingsCard.tsx`). `decision`은 바꾸지
                // 않는다: 이 분기는 위 게이트와 달리 캔버스 단계를 이미
                // 지난 뒤라 provenance를 계속 발행해야 하므로(바로 아래),
                // `not_owned`가 §7에서 약속하는 "아무것도 쓰지 않는다"와
                // 어긋난다 — `outcome: Partial` + `user_action`만으로
                // 충분하다.
                user_action = Some(UserAction::RequestOwnership);
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
        name: plan.name,
        decision,
        channel_id: Some(channel_id),
        generation: plan.generation,
        steps: provenance.steps,
        outcome: if applied {
            Outcome::Applied
        } else {
            Outcome::Partial
        },
        user_action,
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

    /// 팀이 그 방에 직접 써 넣은 내용 — 시작 캔버스가 덮어쓰면 되돌릴 수 없는
    /// 바로 그 값이다.
    const TEAM_CANVAS: &str = "팀이 직접 정리한 회의 규칙";

    /// 팀이 이미 쓰고 있는 방으로 만든다.
    ///
    /// 캔버스가 비어 있는 상태를 `is_empty()`로 확인하는 채택 경로와 달리,
    /// 재개 경로는 그 방에 **이미 내용이 있는 것**이 전제다(부분 실패 뒤 팀이
    /// 방을 쓰기 시작했다). 값을 넣어 두어야 "덮어쓰지 않았다"를 관측할 수
    /// 있다.
    fn seed_team_canvas(fx: &FakeEffects, channel_id: Uuid) {
        // catalog 값과 같으면 덮어써도 assert가 통과한다.
        assert_ne!(TEAM_CANVAS, canvas_of("meeting"));
        fx.seed_canvas(channel_id, TEAM_CANVAS);
    }

    /// saga가 그 방에 캔버스를 **쓰지 않았다**는 것을 두 각도에서 확인한다.
    ///
    /// 저장된 값만 보면 saga가 같은 값을 다시 쓴 경우와 구별되지 않고,
    /// 호출 횟수만 보면 어떤 값이 남았는지 알 수 없다. 덮어쓰기 금지가 이
    /// 변경의 전부이므로 둘 다 본다.
    fn assert_canvas_untouched(fx: &FakeEffects, channel_id: Uuid, expected: &str) {
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&expected.to_string()),
            "이미 내용이 있는 캔버스를 덮어썼다"
        );
        assert_eq!(
            fx.call_count("set_canvas"),
            0,
            "내용이 있는 방에 set_canvas를 보냈다"
        );
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
            // ledger도 사람이 읽는 표면이다 (UI와 CLI가 같이 읽는다).
            // `ledger_serializes_for_ui_and_cli`의 golden은 손으로 만든
            // ledger라, 실제 saga가 이 값을 싣는지는 여기서만 관측된다.
            assert_eq!(
                entry.name.as_deref(),
                Some(
                    crate::builtin()
                        .item(&entry.item_key)
                        .expect("catalog item")
                        .name
                        .as_str()
                ),
                "{}의 catalog 표시 이름이 ledger에 실리지 않았다",
                entry.item_key
            );
            // 세 단계 전부가 desired state다. `Applied`만 보면 owner 확인을
            // 아예 하지 않는 saga도 통과한다 — `outcome`은 `is_complete()`와
            // `error`에서만 나오고, 단계를 건너뛰면 그 둘 다 조용히 통과한다.
            assert_eq!(entry.steps.channel, StepStatus::Done, "{}", entry.item_key);
            assert_eq!(entry.steps.canvas, StepStatus::Done, "{}", entry.item_key);
            assert_eq!(
                entry.steps.membership,
                StepStatus::Done,
                "{} owner 확인이 끝나지 않았다",
                entry.item_key
            );
        }
        assert_eq!(fx.channels.lock().expect("lock").len(), 2);
        // owner 확인을 항목마다 실제로 relay에 물었다.
        assert_eq!(fx.call_count("is_owner"), 2);
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

    /// 요청이 relay에 닿지도 못한 생성 실패.
    ///
    /// 이 경우 relay에는 아무 흔적도 없다 — ID도 타지 않았다. 그러므로
    /// "중복이 안 생겼다"는 여기서 검증할 수 있는 성질이 **아니다**(만들어진
    /// 첫 방이 애초에 없다). 이 테스트가 검증하는 것은 두 가지다: 실패가
    /// ledger에 실제로 실리는가(`Failed` + `error`), 그리고 재시도가 깨끗한
    /// 첫 시도로서 방을 만드는가. 커밋된 뒤의 실패는 아래
    /// `channel_create_that_commits_then_fails_is_adopted_on_retry`가 맡는다.
    #[tokio::test]
    async fn channel_failure_before_commit_is_reported_and_retry_creates_the_room() {
        let fx = FakeEffects::new();
        fx.fail_next("create_channel");

        let first = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");
        let entry = item(&first, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        // 실패를 실패로 적었다. `Partial`만 보면 단계 상태를 지어내는 saga도
        // 통과한다.
        assert_eq!(entry.steps.channel, StepStatus::Failed);
        assert!(entry.error.is_some(), "생성 실패 사유가 실려야 한다");
        assert_eq!(entry.steps.canvas, StepStatus::Pending);
        assert_eq!(entry.steps.membership, StepStatus::Pending);
        // 뒤 단계로 넘어가지 않았다.
        assert!(fx.canvases.lock().expect("lock").is_empty());
        // 채널 단계가 완료가 아니므로 provenance도 발행하지 않는다.
        assert!(fx.published.lock().expect("lock").is_empty());
        assert_eq!(fx.channels.lock().expect("lock").len(), 0);
        // relay에 닿지 못한 실패라 ID도 타지 않았다 — 재시도는 진짜 첫
        // 시도다. 이 사실이 위 doc comment가 말하는 전제다.
        assert!(fx.burned_ids.lock().expect("lock").is_empty());

        let second = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        let entry = item(&second, "meeting");
        assert_eq!(entry.decision, "create_or_recreate");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.steps.membership, StepStatus::Done);
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        assert_eq!(fx.created.lock().expect("lock").len(), 1);
    }

    /// relay가 생성을 커밋한 **뒤에** 실패한 경우 — 응답 유실이나 앱 종료다.
    ///
    /// 여기서만 "중복 없음"이 의미를 갖는다. 첫 시도가 방을 실제로 만들고 ID를
    /// 태웠으므로, 재시도가 `Duplicate`를 어떻게 처리하느냐에 따라 방이 두 개가
    /// 되거나 남의 방으로 오인될 수 있다. 손으로 시드하지 않고 실제 생성 경로로
    /// 그 상태를 만든다.
    #[tokio::test]
    async fn channel_create_that_commits_then_fails_is_adopted_on_retry() {
        let fx = FakeEffects::new();
        fx.fail_next_after_commit("create_channel");

        let first = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");
        let entry = item(&first, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        assert_eq!(entry.steps.channel, StepStatus::Failed);
        assert!(entry.error.is_some());
        // relay 쪽에는 방이 남았다 — 클라이언트만 실패로 봤다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        // 그런데 증명서는 없다. 다음 실행에는 "적용한 적 없음"으로 보인다.
        assert!(fx.published.lock().expect("lock").is_empty());

        // 팀이 방 이름을 바꿨다 (§7의 `renamed`는 판정이 아니라 플래그다).
        // 이 한 줄이 채택 분기의 도달 조건이다: 이름이 catalog 값 그대로면
        // preflight가 "증명서 없음 + 동명 채널 있음"을 보고 `conflict`로
        // 막아서 생성 시도 자체를 하지 않는다.
        fx.channels.lock().expect("lock")[0].name = "2026 전체회의".into();

        let second = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        let entry = item(&second, "meeting");
        assert_eq!(entry.decision, "adopted");
        assert_eq!(entry.outcome, Outcome::Applied);
        // 방이 두 개가 되지 않았다. 첫 시도가 진짜로 하나 만들었으므로 이
        // assert가 처음으로 의미를 갖는다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        assert_eq!(fx.created.lock().expect("lock").len(), 1);
        let channel_id = entry.channel_id.expect("channel id");
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&canvas_of("meeting").to_string())
        );
    }

    /// owner 확인이 relay 오류로 실패하면 `applied`가 아니다. 재시도가 그
    /// 단계만 이어서 하고, 그때 비로소 desired state에 닿는다.
    #[tokio::test]
    async fn membership_check_error_is_partial_and_retry_confirms_ownership() {
        let fx = FakeEffects::new();
        fx.fail_next("is_owner");

        let first = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");
        let entry = item(&first, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        assert_eq!(entry.steps.channel, StepStatus::Done);
        assert_eq!(entry.steps.canvas, StepStatus::Done);
        assert_eq!(entry.steps.membership, StepStatus::Failed);
        assert!(entry.error.is_some(), "owner 확인 실패 사유가 실려야 한다");
        // 보상하지 않는다 — 방도 캔버스도 그대로다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        {
            // 여기까지의 진행이 durable하게 남아야 재시도가 처음부터 하지 않는다.
            let stored = fx.provenance.lock().expect("lock");
            assert_eq!(stored.len(), 1);
            assert_eq!(stored[0].1.steps.membership, StepStatus::Failed);
        }

        let second = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        let entry = item(&second, "meeting");
        assert_eq!(entry.decision, "resume");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.steps.membership, StepStatus::Done);
        // 재시도가 중복을 만들지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        assert_eq!(fx.created.lock().expect("lock").len(), 1);
        let stored = fx.provenance.lock().expect("lock");
        assert_eq!(stored.len(), 1, "NIP-33 LWW — 항목당 정확히 하나다");
        assert!(
            stored[0].1.is_complete(),
            "증명서가 desired state로 갱신되지 않았다"
        );
    }

    /// 재개 경로도 **이번 실행이 만들지 않은 방**이므로 쓰기 전에 owner를
    /// 확인한다. 아니면 **아무것도 쓰지 않고** 막는다.
    ///
    /// 도달 경로 — 채택보다 오히려 흔하다. 관리자 A가 방을 만들었는데 캔버스
    /// 단계가 일시적으로 실패해 증명서가 `channel: done, canvas: failed`로
    /// 남았다(§8이 말하는 부분 실패의 정상적인 결과다). 팀이 그 방을 쓰기
    /// 시작해 자기 캔버스를 채워 넣는다. 그 방의 **멤버**일 뿐인 B가 적용을
    /// 돌린다 — provenance는 채널 스코프라 owner가 아니라 멤버면 읽히므로
    /// preflight가 `Resume`을 내고, 단계 1은 `channel == done`이라 통째로
    /// 건너뛴다.
    ///
    /// 캔버스를 먼저 쓰고 나중에 owner를 확인하는 saga는 이 시점에 팀이 써
    /// 둔 캔버스를 이미 지운 뒤다 — 되돌릴 수 없다. 채택 경로의
    /// `duplicate_channel_we_do_not_own_blocks_without_writing_anything`과
    /// 같은 규칙이고, 도달 조건만 다르다: 채택은 증명서가 아예 없고 방 이름
    /// 까지 바뀌어야 하지만 여기는 미완료 증명서 하나면 된다.
    #[tokio::test]
    async fn resume_by_non_owner_blocks_without_writing_anything() {
        let fx = FakeEffects::new();
        let steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        let channel_id = seed_applied(&fx, "meeting", "메인 회의방", steps);
        // 방은 있고 증명서도 읽히지만 적용자는 이 방의 owner가 아니다.
        fx.owned.lock().expect("lock").remove(&channel_id);
        // 팀이 그 사이 자기 내용을 써 넣었다. 이게 지켜져야 하는 값이다.
        seed_team_canvas(&fx, channel_id);

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        // 그 방에 아무것도 쓰지 않았다. 이게 이 테스트의 핵심이라 먼저
        // 확인한다 — `outcome`만 보면 캔버스를 쓴 **뒤에** 막힌 saga도
        // 통과하고, 그때 이미 팀이 써 둔 내용은 사라진 뒤다.
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&TEAM_CANVAS.to_string()),
            "owner가 아닌 방의 캔버스를 덮어썼다"
        );
        assert!(
            fx.published.lock().expect("lock").is_empty(),
            "owner가 아닌 방에 provenance를 발행했다"
        );
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Blocked);
        assert_eq!(entry.user_action, Some(UserAction::RequestOwnership));
        assert_eq!(entry.decision, "not_owned");
        assert_eq!(entry.channel_id, Some(channel_id));
        // 단계를 완료로 지어내지도, 이전 실행이 남긴 진행을 지우지도 않았다.
        assert_eq!(entry.steps, steps);

        // owner 권한을 받고 재시도하면 그때 비로소 이어서 끝난다.
        fx.owned.lock().expect("lock").insert(channel_id);
        let second = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        let entry = item(&second, "meeting");
        // 채택이 아니다 — 증명서가 이미 있는 항목이라 판정은 `resume` 그대로다.
        assert_eq!(entry.decision, "resume");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.steps.membership, StepStatus::Done);
        assert_eq!(entry.error, None);
        // 권한을 받았다고 팀이 써 둔 내용을 지우지는 않는다. owner 게이트는
        // *누가* 써도 되는가만 가르고, 그 다음 질문 — 지켜야 할 내용이 있는가
        // — 은 캔버스 단계가 한다. 이 재시도는 그 둘이 모두 걸리는 유일한
        // 지점이다: 게이트는 통과하고 캔버스는 건너뛴다.
        assert_canvas_untouched(&fx, channel_id, TEAM_CANVAS);
        assert_eq!(
            entry.steps.canvas,
            StepStatus::Skipped,
            "쓰지 않았는데 `done`으로 보고했다"
        );
        // 아무것도 새로 만들지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        assert!(
            fx.created.lock().expect("lock").is_empty(),
            "이미 있는 방에 생성 요청을 보냈다"
        );
        let stored = fx.provenance.lock().expect("lock");
        assert_eq!(stored.len(), 1, "NIP-33 LWW — 항목당 정확히 하나다");
        assert!(
            stored[0].1.is_complete(),
            "증명서가 desired state로 갱신되지 않았다"
        );
    }

    /// 재개 경로에서도 owner인지 **모르는** 채로는 쓰지 않는다.
    ///
    /// `Ok(false)`만 막고 `Err`를 흘려보내면, relay가 잠깐 느린 것만으로 팀의
    /// 캔버스가 사라진다. 채택 경로의
    /// `adoption_ownership_check_error_writes_nothing`과 같은 규칙이다.
    #[tokio::test]
    async fn resume_ownership_check_error_writes_nothing() {
        let fx = FakeEffects::new();
        let steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        // 적용자는 실제로 owner다(`seed_applied`가 그렇게 시드한다) — 막히는
        // 이유는 권한이 없어서가 아니라 **확인 자체가 실패**해서다.
        let channel_id = seed_applied(&fx, "meeting", "메인 회의방", steps);
        seed_team_canvas(&fx, channel_id);
        fx.fail_next("is_owner");

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&TEAM_CANVAS.to_string()),
            "owner 여부를 모르는 방의 캔버스를 덮어썼다"
        );
        assert!(
            fx.published.lock().expect("lock").is_empty(),
            "확인이 끝나지 않았는데 provenance를 발행했다"
        );

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        // 채택 경로와 같은 모양이다: 채널 단계를 `failed`로 적고 멈춘다.
        // 별도 판정을 만들지 않으므로 `decision`은 `resume` 그대로다.
        assert_eq!(entry.decision, "resume");
        assert_eq!(entry.steps.channel, StepStatus::Failed);
        assert!(entry.error.is_some(), "확인 실패 사유가 실려야 한다");
        // relay에 남아 있는 증명서는 건드리지 않았다 — 재시도가 다시 재개한다.
        {
            let stored = fx.provenance.lock().expect("lock");
            assert_eq!(stored.len(), 1);
            assert_eq!(stored[0].1.steps, steps);
        }

        // relay가 돌아오면 재시도가 이어서 끝난다 — 팀이 써 둔 캔버스는
        // 그대로 두고서다. owner 확인이 통과했다는 것과 그 방에 써도 된다는
        // 것은 다른 이야기다.
        let retry = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        let entry = item(&retry, "meeting");
        assert_eq!(entry.decision, "resume");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_canvas_untouched(&fx, channel_id, TEAM_CANVAS);
        assert_eq!(entry.steps.canvas, StepStatus::Skipped);
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
    }

    /// 재개가 **내용이 있는** 캔버스를 덮어쓰지 않는다.
    ///
    /// owner 게이트가 덮지 못하는 자리다. 게이트는 *누가* 써도 되는가만
    /// 가르는데, 여기서는 적용자가 진짜 owner다 — 그런데도 써서는 안 된다.
    ///
    /// 도달 경로는 부분 실패의 정상적인 결과다(§8 「실패해도 되돌리지
    /// 않는다」). 관리자가 적용을 돌렸는데 캔버스 단계가 일시적으로 실패해
    /// 증명서가 `channel: done, canvas: failed`로 남는다. 팀이 그 방을 쓰기
    /// 시작해 자기 내용을 채운다. 관리자가 재시도를 돌린다 — 조건 없이 쓰는
    /// saga는 이 지점에서 팀의 내용을 catalog 기본값으로 지운다. 되돌릴 수
    /// 없고, 이것이 Phase 3 수용 기준 「catalog upgrade가 사용자 수정 채널이나
    /// 사용자 template copy를 덮어쓰거나 삭제하지 않는다」가 말하는 바로 그
    /// 사고다.
    #[tokio::test]
    async fn resume_does_not_overwrite_a_canvas_that_has_content() {
        let fx = FakeEffects::new();
        let steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        // `seed_applied`는 적용자를 owner로 넣는다 — 권한 문제가 아니라는
        // 것이 이 테스트의 전제다.
        let channel_id = seed_applied(&fx, "meeting", "메인 회의방", steps);
        seed_team_canvas(&fx, channel_id);

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        assert_canvas_untouched(&fx, channel_id, TEAM_CANVAS);

        let entry = item(&ledger, "meeting");
        // 건너뛴 것을 `done`으로 적으면 ledger가 하지 않은 쓰기를 보고한다 —
        // 사용자는 자기 내용이 남았다는 사실도, 이 방이 catalog와 다른
        // 이유도 읽을 방법이 없다.
        assert_eq!(
            entry.steps.canvas,
            StepStatus::Skipped,
            "쓰지 않았는데 `done`으로 보고했다"
        );
        assert_eq!(entry.decision, "resume");
        assert_eq!(entry.error, None);
        // 조용히 아무것도 하지 않은 것이 아니다 — 남은 단계는 끝냈다.
        assert_eq!(entry.steps.membership, StepStatus::Done);
        assert_eq!(entry.outcome, Outcome::Applied);

        // 건너뛴 사실이 durable하게 남는다. 여기까지 확인해야 다음 실행이
        // 이 항목을 다시 미완료로 보지 않는다.
        {
            let stored = fx.provenance.lock().expect("lock");
            assert_eq!(stored.len(), 1, "NIP-33 LWW — 항목당 정확히 하나다");
            assert_eq!(stored[0].1.steps.canvas, StepStatus::Skipped);
        }

        // 두 번째 실행은 아무것도 하지 않는다. `skipped`를 미완료로 세면
        // 이 항목은 영원히 `resume`/`partial`로 보이고, 재시도가 도달할 수
        // 있는 결론은 "쓰지 않는다" 하나뿐인데도 사용자에게는 끝나지 않는
        // 실패로 보인다.
        let reads_before = fx.call_count("read_canvas");
        let second = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("second");
        let entry = item(&second, "meeting");
        assert_eq!(entry.decision, "no_change");
        assert_eq!(entry.outcome, Outcome::Unchanged);
        assert_canvas_untouched(&fx, channel_id, TEAM_CANVAS);
        assert_eq!(
            fx.call_count("read_canvas"),
            reads_before,
            "끝난 캔버스 단계를 다시 실행했다"
        );
    }

    /// 더 새 버전이 적어 둔 **모르는** 상태 값은 미완료가 아니다 — 그 단계를
    /// 다시 실행하지 않는다.
    ///
    /// `StepStatus::Unrecognized`를 settled로 센 결과가 여기서 관측된다.
    /// 미완료로 세면 preflight가 `resume`을 내고 saga가 캔버스 단계를 다시
    /// 실행한다 — 지금은 덮어쓰기 guard가 그 뒤를 막아 주지만, 새 버전이
    /// 내린 판단을 구버전이 뒤집는다는 사실 자체가 문제다. 관리자 둘이 서로
    /// 다른 앱 버전을 쓰는 것만으로 도달한다.
    #[tokio::test]
    async fn an_unrecognized_step_status_is_not_re_run() {
        let fx = FakeEffects::new();
        // 이 빌드가 모르는 값이 캔버스 단계에 적혀 있다 — 더 새 버전이 쓴
        // 증명서를 읽어 온 상태다.
        let channel_id = seed_applied(
            &fx,
            "meeting",
            "메인 회의방",
            StepStates {
                channel: StepStatus::Done,
                canvas: StepStatus::Unrecognized,
                membership: StepStatus::Done,
            },
        );
        seed_team_canvas(&fx, channel_id);

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.decision, "no_change");
        assert_eq!(entry.outcome, Outcome::Unchanged);
        assert_canvas_untouched(&fx, channel_id, TEAM_CANVAS);
        assert_eq!(
            fx.call_count("read_canvas"),
            0,
            "끝난 캔버스 단계를 다시 실행했다"
        );
    }

    /// 같은 재개인데 방이 비어 있으면 시작 캔버스를 **쓴다**.
    ///
    /// 위 테스트와 이 테스트의 차이는 캔버스에 내용이 있느냐 하나뿐이다.
    /// 이게 없으면 "캔버스를 아예 쓰지 않는" saga도 위 테스트를 통과한다 —
    /// 그러면 부분 실패한 항목이 영영 시작 캔버스를 받지 못한다.
    #[tokio::test]
    async fn resume_writes_the_starter_canvas_when_the_room_is_empty() {
        let fx = FakeEffects::new();
        let channel_id = seed_applied(
            &fx,
            "meeting",
            "메인 회의방",
            StepStates {
                channel: StepStatus::Done,
                canvas: StepStatus::Failed,
                membership: StepStatus::Pending,
            },
        );
        // 캔버스를 심지 않는다 — 방은 있는데 캔버스 이벤트가 없는 상태다.

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.steps.canvas, StepStatus::Done);
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&canvas_of("meeting").to_string()),
            "빈 방에 시작 캔버스를 쓰지 않았다"
        );
        assert_eq!(fx.call_count("set_canvas"), 1);
    }

    /// 공백만 있는 캔버스는 지켜야 할 내용이 아니다.
    ///
    /// 캔버스 이벤트는 있지만 본문이 공백뿐인 방 — 사람이 열어 보면 비어
    /// 있다. `is_empty()`로만 판정하면 이런 방은 영영 시작 캔버스를 받지
    /// 못하고, 사용자 눈에는 catalog 적용이 그 항목만 조용히 건너뛴 것으로
    /// 보인다.
    #[tokio::test]
    async fn a_blank_canvas_is_treated_as_empty_and_gets_the_starter_text() {
        let fx = FakeEffects::new();
        let channel_id = seed_applied(
            &fx,
            "meeting",
            "메인 회의방",
            StepStates {
                channel: StepStatus::Done,
                canvas: StepStatus::Failed,
                membership: StepStatus::Pending,
            },
        );
        fx.seed_canvas(channel_id, "   \n\t \n");

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.steps.canvas, StepStatus::Done);
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&canvas_of("meeting").to_string()),
            "공백뿐인 캔버스를 지켜야 할 내용으로 셌다"
        );
    }

    /// 내용이 있는지 **모르는** 채로는 쓰지 않는다.
    ///
    /// 읽기가 실패했을 때 그냥 쓰면, relay가 잠깐 느린 것만으로 팀의 캔버스가
    /// 사라진다 — 이 변경이 막으려는 사고 그 자체다. 반대로 쓰지 않으면 잃는
    /// 것은 이번 실행의 진행뿐이고, 그건 되돌릴 수 있다. owner 확인 실패를
    /// 다루는 규칙(`resume_ownership_check_error_writes_nothing`)과 같다.
    ///
    /// 대신 조용히 넘어가서도 안 된다: `failed` + 사유를 실어야 다음 실행이
    /// `resume`으로 들어와 이 단계부터 이어서 하고 그때 다시 묻는다.
    #[tokio::test]
    async fn canvas_read_failure_writes_nothing_and_retry_makes_progress() {
        let fx = FakeEffects::new();
        let steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        let channel_id = seed_applied(&fx, "meeting", "메인 회의방", steps);
        seed_team_canvas(&fx, channel_id);
        fx.fail_next("read_canvas");

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        assert_canvas_untouched(&fx, channel_id, TEAM_CANVAS);

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        assert_eq!(entry.steps.canvas, StepStatus::Failed);
        assert!(entry.error.is_some(), "읽기 실패 사유가 실려야 한다");
        // 뒤 단계로 넘어가지 않았다.
        assert_eq!(entry.steps.membership, StepStatus::Pending);
        // 이전 실행이 남긴 진행을 지우지 않았다 — 재시도가 여기서 이어서 한다.
        {
            let stored = fx.provenance.lock().expect("lock");
            assert_eq!(stored.len(), 1);
            assert_eq!(stored[0].1.steps, steps);
        }

        // relay가 돌아오면 재시도가 막히지 않고 끝난다 — 여전히 쓰지 않고서다.
        let retry = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        // 되돌릴 수 없는 쪽을 먼저 확인한다 — 단계 상태만 보면 팀의 내용을
        // 지운 **뒤에** 끝난 saga도 여기까지는 같은 값을 낸다.
        assert_canvas_untouched(&fx, channel_id, TEAM_CANVAS);
        let entry = item(&retry, "meeting");
        assert_eq!(entry.decision, "resume");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.steps.canvas, StepStatus::Skipped);
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

    /// 도출된 ID의 방이 이미 있고 접근도 되는 상태를 만든다. provenance는
    /// 없다 — relay가 생성을 커밋한 뒤 증명서를 쓰기 전에 클라이언트가
    /// 죽은 상태다.
    ///
    /// 이름은 catalog 값과 다르게 둔다 — 이름이 판정에 끼어 있으면 (§7이
    /// 금지한다) 이 케이스가 `Conflict`로 새어 나가 채택 분기에 아예 도달하지
    /// 못한다. `owned`는 호출자가 정한다: 채택은 owner일 때만 허용된다.
    fn seed_orphaned_channel(fx: &FakeEffects, item_key: &str, owned: bool) -> Uuid {
        let channel_id = derive_channel_id("wss://relay.test", "schoolx.default", item_key, 1);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: "2026 전체회의".into(),
        });
        fx.burned_ids.lock().expect("lock").insert(channel_id);
        if owned {
            fx.owned.lock().expect("lock").insert(channel_id);
        }
        channel_id
    }

    /// 생성이 `duplicate`인데 그 채널에 접근이 되고 적용자가 owner면
    /// "삭제됨"이 아니다 — 이미 있는 방을 이어받는다.
    ///
    /// relay가 생성을 커밋한 뒤 provenance를 쓰기 전에 클라이언트가 죽으면
    /// 이 상태가 된다 — §5가 결정론적 ID를 두는 이유가 정확히 이 경우를
    /// 흡수하기 위해서다. §7의 `deleted`는 `duplicate` **그리고** 접근
    /// 불가일 때만 성립한다.
    #[tokio::test]
    async fn duplicate_but_owned_channel_is_adopted_and_finished() {
        let fx = FakeEffects::new();
        let channel_id = seed_orphaned_channel(&fx, "meeting", true);

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");
        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.user_action, None);
        // 새로 만든 방과 구별된다. `create_or_recreate`로 보고하면 사용자는
        // 이미 있던 방을 넘겨받았다는 사실을 읽을 방법이 없다.
        assert_eq!(entry.decision, "adopted");
        assert_eq!(entry.steps.channel, StepStatus::Done);
        assert_eq!(entry.steps.membership, StepStatus::Done);
        // 방을 하나 더 만들지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        // 막히지 않고 캔버스 단계까지 이어서 끝냈다.
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&canvas_of("meeting").to_string())
        );
    }

    /// 채택 경로도 **내용이 있는** 캔버스를 덮어쓰지 않는다.
    ///
    /// 재개 경로의 `resume_does_not_overwrite_a_canvas_that_has_content`와 같은
    /// 사고인데, 그 방이 이미 쓰이고 있을 가능성은 이쪽이 더 높다: 증명서가
    /// 없다는 것은 이 방이 catalog 적용과 무관하게 존재해 온 기간이 있다는
    /// 뜻이고, 이름까지 바뀌어 있다면 팀이 실제로 쓰고 있다는 신호다.
    ///
    /// 이 테스트가 없으면 채택 경로에서만 읽기를 건너뛰는 구현 — "결정론적
    /// ID가 맞았으니 우리 catalog가 만든 방이고, 그러면 비어 있다" — 이
    /// 스위트 전체를 통과한다. 채택 경로의 캔버스 검증이 전부 빈 방
    /// (`seed_orphaned_channel`은 캔버스를 심지 않는다)을 대상으로 하기
    /// 때문이다. 그 추론은 틀렸다: 방이 우리 것이라는 사실과 그 방이 비어
    /// 있다는 사실은 별개이고, 둘 사이에는 팀이 그 방을 쓴 시간이 있다.
    #[tokio::test]
    async fn adoption_does_not_overwrite_a_canvas_that_has_content() {
        let fx = FakeEffects::new();
        // 증명서 없이 방만 있고, 적용자는 그 방의 owner다 — owner 게이트는
        // 통과한다. 막아야 하는 이유가 권한이 아니라는 것이 이 테스트의
        // 전제다.
        let channel_id = seed_orphaned_channel(&fx, "meeting", true);
        // 팀이 그 방을 쓰고 있다. 이게 지켜져야 하는 값이다.
        seed_team_canvas(&fx, channel_id);

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        // 되돌릴 수 없는 쪽을 먼저 확인한다 — 단계 상태만 보면 팀의 내용을
        // 지운 **뒤에** 끝난 saga도 같은 값을 낸다.
        assert_canvas_untouched(&fx, channel_id, TEAM_CANVAS);

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.decision, "adopted");
        assert_eq!(
            entry.steps.canvas,
            StepStatus::Skipped,
            "쓰지 않았는데 `done`으로 보고했다"
        );
        // 조용히 아무것도 하지 않은 것이 아니다 — 나머지 단계는 끝냈다.
        assert_eq!(entry.steps.channel, StepStatus::Done);
        assert_eq!(entry.steps.membership, StepStatus::Done);
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.error, None);
        // 방을 하나 더 만들지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);

        // 건너뛴 사실이 durable하게 남아야 다음 실행이 이 항목을 다시
        // 미완료로 보지 않는다.
        let stored = fx.provenance.lock().expect("lock");
        assert_eq!(stored.len(), 1, "NIP-33 LWW — 항목당 정확히 하나다");
        assert_eq!(stored[0].1.steps.canvas, StepStatus::Skipped);
    }

    /// 위 상태가 **실제 시퀀스로** 만들어진다 — 손으로 시드한 상태가 아니다.
    ///
    /// `provenance_publish_failure_is_partial_not_applied`가 만드는 상태 그대로
    /// 시작한다: 세 단계가 다 끝났는데 증명서 발행만 실패해 relay에는 방이
    /// 남고 증명서는 없다. 팀이 그 방 이름을 바꾸고(이름이 catalog 값 그대로면
    /// preflight가 `conflict`로 막아 채택 분기에 닿지 못한다) 자기 캔버스를
    /// 채운다. 같은 관리자가 재시도를 돌린다 — 자기가 만든 방이라 owner
    /// 게이트도 통과한다.
    ///
    /// `assert_canvas_untouched`를 쓰지 않는 이유는 하나뿐이다: 첫 실행이
    /// 정상적으로 `set_canvas`를 한 번 불렀으므로 누적 호출 수가 0이 아니다.
    /// 대신 재시도 **전후**의 호출 수를 비교해 같은 성질을 본다.
    #[tokio::test]
    async fn adoption_after_a_publish_failure_keeps_the_canvas_the_team_wrote() {
        let fx = FakeEffects::new();
        fx.fail_next("publish_provenance");

        let first = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("first");
        let entry = item(&first, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        let channel_id = entry.channel_id.expect("channel id");
        // 증명서가 남지 않아야 다음 실행이 채택 경로로 들어간다.
        assert!(
            fx.provenance.lock().expect("lock").is_empty(),
            "증명서가 남았다면 이 시퀀스는 채택이 아니라 재개다"
        );

        // 팀이 방을 자기 것으로 쓰기 시작한다.
        fx.channels.lock().expect("lock")[0].name = "2026 전체회의".into();
        seed_team_canvas(&fx, channel_id);
        let writes_before = fx.call_count("set_canvas");

        let retry = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");

        // 되돌릴 수 없는 쪽 먼저.
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&TEAM_CANVAS.to_string()),
            "채택이 팀의 캔버스를 덮어썼다"
        );
        assert_eq!(
            fx.call_count("set_canvas"),
            writes_before,
            "이미 내용이 있는 방에 set_canvas를 보냈다"
        );

        let entry = item(&retry, "meeting");
        assert_eq!(entry.decision, "adopted");
        assert_eq!(entry.steps.canvas, StepStatus::Skipped);
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(entry.error, None);
        // 방이 두 개가 되지 않았다.
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
        assert_eq!(fx.created.lock().expect("lock").len(), 1);
    }

    /// 채택은 owner일 때만 한다. 아니면 **아무것도 쓰지 않고** 막는다.
    ///
    /// 도달 경로: 관리자 A가 방을 만들었는데 증명서 발행이 실패했다
    /// (`Partial`). 그 방의 멤버지만 owner는 아닌 관리자 B가 적용을 돌린다.
    /// B에게는 방이 보이고 증명서는 안 보이므로 채택 분기까지 그대로 온다.
    /// 캔버스를 먼저 쓰고 나중에 owner를 확인하는 saga는 이 시점에 팀이
    /// 써 둔 캔버스를 이미 지운 뒤다 — 되돌릴 수 없다.
    #[tokio::test]
    async fn duplicate_channel_we_do_not_own_blocks_without_writing_anything() {
        let fx = FakeEffects::new();
        let channel_id = seed_orphaned_channel(&fx, "meeting", false);

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        // 그 방에 아무것도 쓰지 않았다. 이게 이 테스트의 핵심이라 먼저
        // 확인한다 — `outcome`만 보면 캔버스를 쓴 **뒤에** 막힌 saga도
        // 통과하고, 그때 이미 팀이 써 둔 내용은 사라진 뒤다.
        assert!(
            fx.canvases.lock().expect("lock").is_empty(),
            "owner가 아닌 방의 캔버스를 덮어썼다"
        );
        assert!(
            fx.published.lock().expect("lock").is_empty(),
            "owner가 아닌 방에 provenance를 발행했다"
        );
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);

        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Blocked);
        assert_eq!(entry.user_action, Some(UserAction::RequestOwnership));
        assert_eq!(entry.decision, "not_owned");
        assert_eq!(entry.channel_id, Some(channel_id));
        // 단계를 하나도 완료로 지어내지 않았다.
        assert_eq!(entry.steps, StepStates::default());
    }

    /// owner인지 **모르는** 채로는 채택하지도 막지도 않는다. 쓰지 않는 쪽이
    /// 안전한 실패다.
    ///
    /// `Ok(false)`만 막고 `Err`를 채택으로 흘려보내면, relay가 잠깐 느린
    /// 것만으로 남의 방 캔버스가 사라진다.
    #[tokio::test]
    async fn adoption_ownership_check_error_writes_nothing() {
        let fx = FakeEffects::new();
        seed_orphaned_channel(&fx, "meeting", true);
        fx.fail_next("is_owner");

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");
        let entry = item(&ledger, "meeting");
        assert_eq!(entry.outcome, Outcome::Partial);
        assert_eq!(entry.steps.channel, StepStatus::Failed);
        assert!(entry.error.is_some(), "확인 실패 사유가 실려야 한다");
        assert!(
            fx.canvases.lock().expect("lock").is_empty(),
            "owner 여부를 모르는 방의 캔버스를 덮어썼다"
        );
        assert!(
            fx.published.lock().expect("lock").is_empty(),
            "채널 단계가 완료가 아닌데 provenance를 발행했다"
        );

        // relay가 돌아오면 재시도가 채택으로 끝난다.
        let retry = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("retry");
        let entry = item(&retry, "meeting");
        assert_eq!(entry.decision, "adopted");
        assert_eq!(entry.outcome, Outcome::Applied);
        assert_eq!(fx.channels.lock().expect("lock").len(), 1);
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

    /// 남의 채널에 실린 증명서는 적용을 **막지 못한다.**
    ///
    /// preflight의 `provenance_published_in_another_channel_is_ignored`가 판정을
    /// 보고, 여기서는 그 판정의 결과 — 방이 실제로 만들어지는가 — 를 본다.
    /// 이 defect가 아프게 나타나는 자리가 정확히 여기다. 학생 하나가 자기
    /// open 채널에 `steps` 전부 완료인 kind 39500을 발행해 두면, 그 레코드를
    /// 권위로 읽는 saga는 `no_change`/`unchanged`를 보고하고 학교의 표준 방을
    /// **영원히** 만들지 않는다 — 관리자에게는 우회로가 없다. 그 이벤트는
    /// 공격자의 채널에 있어 지울 수 없고, NIP-33 LWW는 `(kind, pubkey, d)`별
    /// 이라 덮어쓸 수도 없다.
    #[tokio::test]
    async fn a_foreign_channel_provenance_does_not_block_the_apply() {
        let fx = FakeEffects::new();
        // 공격자가 자기 open 채널에 발행한 `meeting` 완료 증명서. relay의
        // 읽기 ACL은 open 채널을 통과시키므로 관리자에게 그대로 읽힌다.
        let attacker_channel =
            Uuid::parse_str("11111111-2222-4333-8444-555555555555").expect("고정 UUID");
        fx.seed_provenance_in_channel(
            attacker_channel,
            "학생회 잡담방",
            Provenance {
                catalog_id: "schoolx.default".into(),
                catalog_version: 1,
                item_key: "meeting".into(),
                generation: 1,
                steps: StepStates {
                    channel: StepStatus::Done,
                    canvas: StepStatus::Done,
                    membership: StepStatus::Done,
                },
                applied_at: "2026-07-28T09:00:00Z".into(),
            },
        );

        let ledger = apply(crate::builtin(), &fx, &["meeting".to_string()])
            .await
            .expect("apply");

        let entry = item(&ledger, "meeting");
        assert_eq!(
            entry.outcome,
            Outcome::Applied,
            "남의 채널에 실린 증명서가 적용을 막았다"
        );
        assert_eq!(entry.decision, "create_or_recreate");
        let channel_id = entry.channel_id.expect("channel id");
        assert_eq!(
            channel_id,
            derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1)
        );
        // 방이 진짜로 만들어졌다. `outcome`만 보면 아무것도 하지 않고 완료를
        // 보고하는 구현도 통과한다.
        assert_eq!(fx.created.lock().expect("lock").len(), 1);
        assert_eq!(
            fx.canvases.lock().expect("lock").get(&channel_id),
            Some(&canvas_of("meeting").to_string())
        );
        // 공격자 채널에는 아무것도 쓰지 않았다.
        assert!(
            !fx.canvases
                .lock()
                .expect("lock")
                .contains_key(&attacker_channel),
            "공격자 채널에 캔버스를 썼다"
        );
        assert!(
            fx.published
                .lock()
                .expect("lock")
                .iter()
                .all(|(id, _)| *id == channel_id),
            "공격자 채널에 provenance를 발행했다"
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
        // catalog에서 빠진 항목이라 이름을 가져올 곳이 없다. `item_key`를
        // 대신 실으면 UI가 `finance`를 방 이름으로 보여준다.
        assert_eq!(entry.name, None);
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

        // (2) 네 `outcome`과 세 `user_action`, 그리고 saga만 만들어 내는
        //     `decision` 값들을 바이트 단위로 고정한다.
        let ledger = Ledger {
            catalog_id: "schoolx.default".into(),
            catalog_version: 1,
            items: vec![
                LedgerItem {
                    item_key: "meeting".into(),
                    name: Some("메인 회의방".into()),
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
                    name: Some("기획".into()),
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
                    name: Some("재무".into()),
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
                    name: Some("인사".into()),
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
                    name: Some("영업".into()),
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
                LedgerItem {
                    item_key: "ops".into(),
                    name: Some("운영".into()),
                    decision: "adopted".into(),
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
                    item_key: "library".into(),
                    name: Some("도서관".into()),
                    decision: "not_owned".into(),
                    channel_id: Some(Uuid::nil()),
                    generation: 1,
                    steps: StepStates::default(),
                    outcome: Outcome::Blocked,
                    user_action: Some(UserAction::RequestOwnership),
                    error: None,
                },
                // 캔버스에 이미 내용이 있어 쓰지 않은 항목. `applied`인데
                // 캔버스 단계만 `skipped`인 이 조합이, 사용자가 "이 방은
                // 시작 캔버스를 받지 않았다"를 읽을 수 있는 유일한 자리다.
                LedgerItem {
                    item_key: "notices".into(),
                    name: Some("공지".into()),
                    decision: "resume".into(),
                    channel_id: Some(Uuid::nil()),
                    generation: 1,
                    steps: StepStates {
                        channel: StepStatus::Done,
                        canvas: StepStatus::Skipped,
                        membership: StepStatus::Done,
                    },
                    outcome: Outcome::Applied,
                    user_action: None,
                    error: None,
                },
                // catalog에서 빠진 항목. `name`이 `null`인 유일한 자리이고,
                // 그래서 여기서만 "이름을 모른다"의 wire 표현이 고정된다 —
                // `item_key`를 대신 실어 버리는 구현은 UI에서 `clubs`가
                // 방 이름으로 보이는데 이 golden만 통과한다. `retired`
                // 판정 자체도 여기서 처음으로 wire format에 고정된다.
                LedgerItem {
                    item_key: "clubs".into(),
                    name: None,
                    decision: "retired".into(),
                    channel_id: None,
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
                    "name": "메인 회의방",
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
                    "name": "기획",
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
                    "name": "재무",
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
                    "name": "인사",
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
                    "name": "영업",
                    "decision": "no_change",
                    "channel_id": "00000000-0000-0000-0000-000000000000",
                    "generation": 1,
                    "steps": { "channel": "done", "canvas": "done", "membership": "done" },
                    "outcome": "unchanged",
                    "user_action": null,
                    "error": null
                },
                {
                    "item_key": "ops",
                    "name": "운영",
                    "decision": "adopted",
                    "channel_id": "00000000-0000-0000-0000-000000000000",
                    "generation": 1,
                    "steps": { "channel": "done", "canvas": "done", "membership": "done" },
                    "outcome": "applied",
                    "user_action": null,
                    "error": null
                },
                {
                    "item_key": "library",
                    "name": "도서관",
                    "decision": "not_owned",
                    "channel_id": "00000000-0000-0000-0000-000000000000",
                    "generation": 1,
                    "steps": { "channel": "pending", "canvas": "pending", "membership": "pending" },
                    "outcome": "blocked",
                    "user_action": "request_ownership",
                    "error": null
                },
                {
                    "item_key": "notices",
                    "name": "공지",
                    "decision": "resume",
                    "channel_id": "00000000-0000-0000-0000-000000000000",
                    "generation": 1,
                    "steps": { "channel": "done", "canvas": "skipped", "membership": "done" },
                    "outcome": "applied",
                    "user_action": null,
                    "error": null
                },
                {
                    "item_key": "clubs",
                    "name": null,
                    "decision": "retired",
                    "channel_id": null,
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
