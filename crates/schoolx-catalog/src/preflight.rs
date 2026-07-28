//! 적용 전 항목별 판정.
//!
//! 판정 근거는 이름이 아니라 provenance다. 이름은 `Conflict` 감지에만 쓴다.

use crate::catalog::Catalog;
use crate::channel_id::derive_channel_id;
use crate::effects::{CatalogEffects, EffectError};
use crate::provenance::Provenance;
use serde::Serialize;
use uuid::Uuid;

/// 항목 하나에 대한 판정.
///
/// `Serialize`는 Tauri command 반환 타입이라 필요하다 (Task 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// provenance가 없고 동명 채널도 없다. 생성을 시도한다. 거부되면
    /// 예전에 만들었다가 삭제된 것이다.
    CreateOrRecreate,
    /// provenance가 있고 일부 단계가 미완료다. 미완료 단계만 실행한다.
    Resume,
    /// provenance가 있고 전 단계가 완료다. 아무것도 하지 않는다.
    NoChange,
    /// provenance가 없는데 동명 채널이 있다. 자동 채택하지 않는다.
    Conflict,
    /// provenance는 있는데 catalog에서 항목이 빠졌다. 채널은 유지한다.
    Retired,
}

/// 항목 하나의 preflight 결과.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreflightItem {
    /// catalog 항목 키. `Retired`면 catalog에 더는 없는 키다.
    pub item_key: String,
    /// 판정.
    pub decision: Decision,
    /// 알려진 채널 ID. `CreateOrRecreate`면 앞으로 쓸 ID다.
    pub channel_id: Option<Uuid>,
    /// 채널 ID 도출에 쓸 세대.
    pub generation: u32,
    /// 사용자가 이름을 바꿨는가. 판정과 무관한 표시용 플래그다.
    pub renamed: bool,
}

/// catalog 전체를 판정한다.
pub async fn preflight(
    catalog: &Catalog,
    effects: &dyn CatalogEffects,
) -> Result<Vec<PreflightItem>, EffectError> {
    let relay_scope = effects.relay_scope().await;
    let channels = effects.list_channels().await?;
    let provenance = effects.fetch_provenance(&catalog.catalog_id).await?;

    let mut out = Vec::with_capacity(catalog.items.len());

    for item in &catalog.items {
        let known: Option<&Provenance> = provenance.iter().find(|p| p.item_key == item.item_key);

        match known {
            Some(p) => {
                let channel_id = derive_channel_id(
                    &relay_scope,
                    &catalog.catalog_id,
                    &item.item_key,
                    p.generation,
                );
                let live = channels.iter().find(|c| c.id == channel_id);
                out.push(PreflightItem {
                    item_key: item.item_key.clone(),
                    decision: if p.is_complete() {
                        Decision::NoChange
                    } else {
                        Decision::Resume
                    },
                    channel_id: Some(channel_id),
                    generation: p.generation,
                    renamed: live.is_some_and(|c| c.name != item.name),
                });
            }
            None => {
                let channel_id =
                    derive_channel_id(&relay_scope, &catalog.catalog_id, &item.item_key, 1);
                let name_taken = channels.iter().any(|c| c.name == item.name);
                out.push(PreflightItem {
                    item_key: item.item_key.clone(),
                    decision: if name_taken {
                        Decision::Conflict
                    } else {
                        Decision::CreateOrRecreate
                    },
                    channel_id: Some(channel_id),
                    generation: 1,
                    renamed: false,
                });
            }
        }
    }

    // catalog에서 빠졌는데 provenance가 남은 항목.
    for p in &provenance {
        if catalog.item(&p.item_key).is_none() {
            out.push(PreflightItem {
                item_key: p.item_key.clone(),
                decision: Decision::Retired,
                channel_id: None,
                generation: p.generation,
                renamed: false,
            });
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::fake::FakeEffects;
    use crate::effects::ChannelRef;
    use crate::provenance::{StepStates, StepStatus};

    fn done() -> StepStates {
        StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Done,
            membership: StepStatus::Done,
        }
    }

    fn provenance(item_key: &str, steps: StepStates) -> Provenance {
        Provenance {
            catalog_id: "schoolx.default".into(),
            catalog_version: 1,
            item_key: item_key.into(),
            generation: 1,
            steps,
            applied_at: "2026-07-28T09:00:00Z".into(),
        }
    }

    fn find<'a>(items: &'a [PreflightItem], key: &str) -> &'a PreflightItem {
        items
            .iter()
            .find(|i| i.item_key == key)
            .expect("item present")
    }

    /// 이미 적용된 항목을 시드한다.
    ///
    /// provenance는 채널 스코프 이벤트라 채널이 사라지면 읽을 수 없다. fake도
    /// 그렇게 동작하므로 — `fetch_provenance`가 `channels`에 살아 있는 채널의
    /// 항목만 돌려준다 — 시딩은 반드시 채널과 짝으로 해야 한다. provenance만
    /// 넣으면 preflight에는 "적용한 적 없음"으로 보인다.
    fn seed_applied(fx: &FakeEffects, item_key: &str, name: &str, steps: StepStates) -> Uuid {
        let channel_id = derive_channel_id("wss://relay.test", "schoolx.default", item_key, 1);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: name.into(),
        });
        fx.provenance
            .lock()
            .expect("lock")
            .push((channel_id, provenance(item_key, steps)));
        channel_id
    }

    #[tokio::test]
    async fn fresh_install_creates_everything() {
        let fx = FakeEffects::new();
        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(items.len(), 2);
        for item in &items {
            assert_eq!(item.decision, Decision::CreateOrRecreate);
            assert_eq!(item.generation, 1);
        }
    }

    #[tokio::test]
    async fn completed_item_is_no_change() {
        let fx = FakeEffects::new();
        seed_applied(&fx, "meeting", "메인 회의방", done());

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "meeting").decision, Decision::NoChange);
        assert_eq!(
            find(&items, "planning").decision,
            Decision::CreateOrRecreate
        );
    }

    #[tokio::test]
    async fn partial_item_resumes() {
        let fx = FakeEffects::new();
        seed_applied(
            &fx,
            "meeting",
            "메인 회의방",
            StepStates {
                channel: StepStatus::Done,
                canvas: StepStatus::Failed,
                membership: StepStatus::Pending,
            },
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "meeting").decision, Decision::Resume);
    }

    #[tokio::test]
    async fn same_name_without_provenance_is_a_conflict() {
        let fx = FakeEffects::new();
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: Uuid::new_v4(),
            name: "기획".into(),
        });

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "planning").decision, Decision::Conflict);
        assert_eq!(find(&items, "meeting").decision, Decision::CreateOrRecreate);
    }

    #[tokio::test]
    async fn rename_is_a_flag_not_a_decision() {
        let fx = FakeEffects::new();
        // catalog 이름은 "메인 회의방"인데 멤버가 바꿔 놓은 상태.
        seed_applied(
            &fx,
            "meeting",
            "2026 전체회의",
            StepStates {
                channel: StepStatus::Done,
                canvas: StepStatus::Failed,
                membership: StepStatus::Pending,
            },
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let meeting = find(&items, "meeting");
        // 이름을 바꿨어도 미완료면 재시도 대상이다.
        assert_eq!(meeting.decision, Decision::Resume);
        assert!(meeting.renamed);
    }

    #[tokio::test]
    async fn item_dropped_from_catalog_is_retired() {
        let fx = FakeEffects::new();
        // catalog에 없는 항목이지만 예전 버전에서 적용된 채로 남아 있다.
        seed_applied(&fx, "finance", "재무", done());

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "finance").decision, Decision::Retired);
    }
}
