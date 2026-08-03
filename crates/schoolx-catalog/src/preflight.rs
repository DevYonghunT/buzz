//! 적용 전 항목별 판정.
//!
//! 판정 근거는 이름이 아니라 provenance다. 이름은 `Conflict` 감지에만 쓴다.

use crate::catalog::Catalog;
use crate::channel_id::derive_channel_id;
use crate::effects::{CatalogEffects, EffectError, ProvenanceRecord};
use crate::provenance::{Provenance, StepStates};
use serde::Serialize;
use std::collections::HashSet;
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
    /// catalog가 정한 표시 이름 — 사람에게 보여줄 값이다.
    ///
    /// UI는 이 값을 쓰고 `item_key`를 쓰지 않는다. `item_key`는 내부
    /// 식별자라 화면에 그대로 나가면 사용자는 `메인 회의방` 자리에서
    /// `meeting`을 보게 된다.
    ///
    /// **`Retired`면 `None`이다.** 증명서는 남았는데 catalog 항목이 사라진
    /// 경우라 이름을 알 수 있는 곳이 어디에도 없다 — 이름은 catalog에만
    /// 있고 증명서에는 실리지 않는다. 그 자리에 `item_key`를 대신 넣지
    /// 않는 이유는, 그러면 "이게 그 방의 이름"과 "이름을 모른다"가 같은
    /// 값으로 보여 UI가 둘을 구별할 수 없기 때문이다. `None`을 어떻게
    /// 보여줄지는 UI가 정한다.
    ///
    /// 이 방의 **현재** 이름이 아니다. 사용자가 방 이름을 바꿨으면 이 값과
    /// 다르고, 그 사실은 [`PreflightItem::renamed`]가 말한다.
    pub name: Option<String>,
    /// 판정.
    pub decision: Decision,
    /// 알려진 채널 ID. `CreateOrRecreate`면 앞으로 쓸 ID다.
    pub channel_id: Option<Uuid>,
    /// `channel_id`가 지금 접근 가능 목록에 있는가.
    ///
    /// saga는 이 값으로 §7의 `deleted` 조건 두 절 중 두 번째 —
    /// "접근 불가" — 를 판정한다. `duplicate`만으로는 "예전에 만들었다가
    /// 삭제됨"과 "만들어졌는데 provenance를 쓰기 전에 죽음"을 구분할 수
    /// 없다. `channel_id`가 `None`인 `Retired`에서는 항상 `false`다.
    pub channel_present: bool,
    /// 채널 ID 도출에 쓸 세대.
    pub generation: u32,
    /// provenance에 기록된 단계별 상태. provenance가 없으면 전부 `Pending`.
    ///
    /// saga는 이 값을 그대로 이어받는다. saga가 provenance를 다시 읽지
    /// 않게 하려고 여기에 싣는다 — 두 번째 읽기가 실패하면 멀쩡한 채널이
    /// "삭제됨"으로 오판되기 때문이다.
    pub steps: StepStates,
    /// 사용자가 이름을 바꿨는가. 판정과 무관한 표시용 플래그다.
    pub renamed: bool,
}

/// 이 레코드가 §5의 도출식이 예측하는 바로 그 채널에 실려 있는가.
///
/// 이 함수가 이 크레이트의 신뢰 경계다. `channel_id`는 relay가 알려 준 사실
/// (그 이벤트의 `h` 태그)이고, 오른쪽은 우리가 계산한 값이다. 둘이 같아야만
/// 그 레코드가 이 catalog가 남긴 것이다.
fn record_sits_in_its_derived_channel(
    relay_scope: &str,
    catalog_id: &str,
    channel_id: Uuid,
    provenance: &Provenance,
) -> bool {
    channel_id
        == derive_channel_id(
            relay_scope,
            catalog_id,
            &provenance.item_key,
            provenance.generation,
        )
}

/// catalog 전체를 판정한다.
pub async fn preflight(
    catalog: &Catalog,
    effects: &dyn CatalogEffects,
) -> Result<Vec<PreflightItem>, EffectError> {
    let relay_scope = effects.relay_scope().await;
    let channels = effects.list_channels().await?;
    let fetched = effects.fetch_provenance(&catalog.catalog_id).await?;

    // **도출된 채널에 실려 있지 않은 레코드는 버린다.**
    //
    // relay의 읽기 ACL은 "이 사용자가 접근할 수 있는 채널의 이벤트"까지만
    // 좁히고, 그 집합에는 커뮤니티의 모든 `open` 채널이 들어간다. 인증된
    // 사용자라면 누구나 open 채널을 만들어 거기에 kind 39500을 발행할 수
    // 있다 — 자기 채널이라 쓰기도 정당하게 승인된다. 학교라면 학생 아무나가
    // 그 사용자다. 그러므로 레코드가 **읽혔다**는 사실도, 레코드가 스스로
    // 적어 둔 `item_key`·`generation`도 그 레코드의 출처를 조금도 증명하지
    // 않는다.
    //
    // 위조할 수 없는 결합은 하나뿐이다: 그 레코드가 실려 있는 채널이 §5의
    // 도출식 `derive_channel_id(relay_scope, catalog_id, item_key,
    // generation)`이 예측하는 채널과 같아야 한다. 그 채널에 발행하려면 그
    // 채널의 쓰기 권한이 필요하므로, 남의 채널에 레코드를 아무리 쌓아도
    // 판정은 움직이지 않는다.
    //
    // **이 검사 혼자서는 막지 못하는 것도, 아래 §5의 owner 검사를 더해도
    // 여전히 막지 못하는 것도 적어 둔다: 선점.** 채널 ID는 클라이언트가
    // 정하는 값이라, 도출식을 계산해 그 ID로 **채널을 먼저 만들어 버린**
    // 공격자는 자기 채널 안에서 이 검사를 통과하는 레코드를 만들 수 있다.
    // 이벤트 하나를 발행하는 것과는 값이 다르다 — 항목마다 그 ID를 영구히
    // 태워야 한다.
    //
    // 아래 서명자 검사를 더해도 이 선점 경로는 닫히지 않는다 — 선점자는
    // 자기가 먼저 만든 그 채널의 **진짜 생성자**다(relay가 그렇게 기록한다).
    // 그러므로 자기 손으로 서명해 남긴 레코드는 그 검사도 그대로 통과한다.
    // 「내가 만들고 내가 서명했다」는 참인 진술이라 어느 검사로도 거짓으로
    // 만들 수 없다. 선점을 막는 것은 이 크레이트가 아니다 — saga가 쓰기
    // 직전에 묻는 게이트(§7·§8, `saga.rs`)가 `not_owned`로 막는다.
    //
    // **읽힘 자체는 방어가 아니다.** 예전 주석은 여기에 「관리자가 그 채널의
    // 멤버가 아니면 relay의 읽기 ACL이 애초에 그 레코드를 넘기지 않는다」를
    // 함께 적어 두었는데, 그건 틀렸다: `open` 채널은 비멤버도 읽는다. relay의
    // 채널 스코프 ACL은 경계가 아니라 1차 필터이고, 그 필터가 통과시키는
    // 집합에는 커뮤니티의 모든 open 채널이 들어간다. 그러므로 방어선은
    // 위의 saga 게이트 하나로 세어야 한다. 자세한 근거는
    // `docs/schoolx-2/CATALOG_SECURITY.md` §5(선점 채널 문단)·§7을 보라.
    //
    // 그러면 아래 서명자 검사는 실제로 무엇을 닫는가: **선점되지 않은 진짜
    // 채널 안에서, 생성자가 아닌 멤버가 발행한 레코드.** kind 39500 쓰기는
    // relay에서 채널 멤버십만 요구하고 owner십을 요구하지 않으므로
    // (`crates/buzz-relay/src/handlers/ingest.rs`의 `requires_h_channel_scope`
    // + 일반 멤버십 게이트), 진짜 `#회의` 채널의 멤버 아무나(학교라면 학생
    // 아무나)가 그 채널 안에 완료로 위조한 레코드를 발행할 수 있다. 채널
    // 결합만 보면 그 레코드는 진짜 채널에 있으므로 통과한다 — 아래 owner
    // 검사가 서명자 불일치로 그 레코드를 버린다.
    //
    // 검사에 걸린 레코드는 "우리와 무관한 레코드"가 아니라 **위조이거나
    // 버그**다. 그런데도 오류로 올리지 않고 애초에 없었던 것처럼 버리는
    // 이유는, 오류로 올리면 아무나 발행할 수 있는 이벤트 하나로 관리자의
    // preflight 전체를 영구히 막을 수 있기 때문이다. 없는 것으로 보면 그
    // 항목은 `create_or_recreate`로 떨어져 정상 경로를 그대로 탄다 — 그
    // ID의 채널이 이미 선점돼 있다면 saga의 owner 게이트(§8)가 그 방에 쓰는
    // 것을 막고 `not_owned`로 사용자에게 넘긴다.
    let provenance: Vec<ProvenanceRecord> = fetched
        .into_iter()
        .filter(|record| {
            record_sits_in_its_derived_channel(
                &relay_scope,
                &catalog.catalog_id,
                record.channel_id,
                &record.provenance,
            )
        })
        .collect();

    // §5: 증명서는 (1) 도출식이 예측하는 채널에 실려 있고 (2) 그 채널의
    // 현재 owner가 서명한 것만 인정한다. 위에서 적었듯 (2)는 (1)이 놓치는
    // 선점 경로를 닫지 않는다 — 그것을 닫는 것은 이 owner 검사가 아니라
    // relay ACL과 saga의 owner 게이트다. (2)가 실제로 닫는 것은 선점되지
    // 않은 진짜 채널에서 owner가 아닌 멤버가 발행한 레코드다.
    //
    // owner를 특정할 수 없으면 버린다. 검증할 수 없는 것을 통과시키지
    // 않는다. `channel_owner`가 `Err`를 돌려줘도 마찬가지로 버린다 —
    // `?`로 올리지 않는다. 이 루프는 공격자가 통제할 수 있는 channel_id를
    // 순회하므로, 조회 실패를 위로 전파하면 위 문단과 같은 이유로 그
    // 하나가 관리자의 preflight 전체를 영구히 막는 지렛대가 된다. 버려진
    // 레코드가 진짜였다면 그 항목은 "적용한 적 없음"으로 보일 뿐이고,
    // 재적용은 §8의 캔버스 가드를 그대로 지나 기존 내용을 지키며 한 번
    // 만에 스스로 복구된다.
    let mut honoured: Vec<&ProvenanceRecord> = Vec::new();
    for record in &provenance {
        match effects.channel_owner(record.channel_id).await {
            Ok(Some(owner)) if owner == record.signer => honoured.push(record),
            _ => {}
        }
    }

    let mut out = Vec::with_capacity(catalog.items.len());

    for item in &catalog.items {
        let known: Option<&Provenance> = honoured
            .iter()
            .find(|record| record.provenance.item_key == item.item_key)
            .map(|record| &record.provenance);

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
                    name: Some(item.name.clone()),
                    decision: if p.is_complete() {
                        Decision::NoChange
                    } else {
                        Decision::Resume
                    },
                    channel_id: Some(channel_id),
                    channel_present: live.is_some(),
                    generation: p.generation,
                    steps: p.steps,
                    renamed: live.is_some_and(|c| c.name != item.name),
                });
            }
            None => {
                let channel_id =
                    derive_channel_id(&relay_scope, &catalog.catalog_id, &item.item_key, 1);
                let name_taken = channels.iter().any(|c| c.name == item.name);
                out.push(PreflightItem {
                    item_key: item.item_key.clone(),
                    name: Some(item.name.clone()),
                    decision: if name_taken {
                        Decision::Conflict
                    } else {
                        Decision::CreateOrRecreate
                    },
                    channel_id: Some(channel_id),
                    // provenance는 없지만 ID는 살아 있을 수 있다 — relay가
                    // 채널을 만든 직후 클라이언트가 provenance를 쓰기 전에
                    // 죽은 경우다. saga가 `duplicate`를 받았을 때 이 값으로
                    // 그 경우와 "삭제됨"을 가른다.
                    channel_present: channels.iter().any(|c| c.id == channel_id),
                    generation: 1,
                    steps: StepStates::default(),
                    renamed: false,
                });
            }
        }
    }

    // catalog에서 빠졌는데 provenance가 남은 항목.
    //
    // **`item_key`당 한 줄이다.** kind 39500은 `(kind, pubkey, d 태그)`별로
    // addressable이라 NIP-33 LWW는 신원 안에서만 적용된다 — 한 항목을 적용한
    // 신원 수만큼 레코드가 쌓인다. 관리자 둘이 같은 항목을 적용했고 그 항목이
    // 나중에 catalog에서 빠지면 레코드가 둘이고, 그대로 밀면 `item_key`가 같은
    // 줄이 두 개 나간다. 위 catalog 항목 루프는 `find`로 첫 레코드만 쓰므로
    // 이 문제가 없고, 여기만 레코드를 그대로 훑는다.
    //
    // 첫 레코드를 쓴다. 이 줄은 정보 표시용이고(`Retired`는 아무것도 하지
    // 않는다) 레코드마다 다를 수 있는 값은 `steps`·`generation`뿐이라 어느
    // 것을 고르든 동작은 같다 — 지어내지 않고 실재하는 레코드 하나를 그대로
    // 싣는다는 것만이 중요하다.
    let mut retired_keys: HashSet<&str> = HashSet::new();
    for record in &honoured {
        let p = &record.provenance;
        if catalog.item(&p.item_key).is_none() && retired_keys.insert(p.item_key.as_str()) {
            out.push(PreflightItem {
                item_key: p.item_key.clone(),
                // 이 분기의 정의가 곧 "catalog에 이 항목이 없다"이므로 이름을
                // 가져올 곳이 없다. 지어내지 않는다.
                name: None,
                decision: Decision::Retired,
                channel_id: None,
                channel_present: false,
                generation: p.generation,
                // 실제 상태를 그대로 싣는다. 미완료인 채로 catalog에서 빠질
                // 수 있으므로 여기서 완료를 지어내면 ledger가 거짓말을 한다.
                steps: p.steps,
                renamed: false,
            });
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::fake::{FakeEffects, FAKE_ME};
    use crate::effects::ChannelRef;
    use crate::provenance::{StepStates, StepStatus};

    fn done() -> StepStates {
        StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Done,
            membership: StepStatus::Done,
        }
    }

    /// `preflight`가 `derive_channel_id`에 하드코딩된 1이 아니라 provenance의
    /// 실제 `generation`을 넘기는지 확인할 수 있도록 세대를 지정해 만든다.
    fn provenance_with_generation(
        item_key: &str,
        generation: u32,
        steps: StepStates,
    ) -> Provenance {
        Provenance {
            catalog_id: "schoolx.default".into(),
            catalog_version: 1,
            item_key: item_key.into(),
            generation,
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

    /// 그 키로 나온 줄의 개수. `find`는 첫 줄만 보므로 "정확히 한 줄"을
    /// 확인하려면 이쪽이 필요하다.
    fn count(items: &[PreflightItem], key: &str) -> usize {
        items.iter().filter(|i| i.item_key == key).count()
    }

    /// 공격자가 직접 만든 `open` 채널의 ID.
    ///
    /// 학교 워크스페이스의 아무 인증 사용자나(= 학생 누구나) 이런 채널을 만들
    /// 수 있고, 자기 채널이므로 거기에 kind 39500을 발행하는 것도 relay가
    /// 정당하게 승인한다. relay의 읽기 ACL은 `open` 채널을 모두 통과시키므로
    /// 그 레코드는 모든 관리자의 preflight에 그대로 도착한다.
    fn attacker_channel() -> Uuid {
        Uuid::parse_str("11111111-2222-4333-8444-555555555555").expect("고정 UUID")
    }

    /// 이미 적용된 항목을 세대 1로 시드한다.
    ///
    /// provenance는 채널 스코프 이벤트라 채널이 사라지면 읽을 수 없다. fake도
    /// 그렇게 동작하므로 — `fetch_provenance`가 `channels`에 살아 있는 채널의
    /// 항목만 돌려준다 — 시딩은 반드시 채널과 짝으로 해야 한다. provenance만
    /// 넣으면 preflight에는 "적용한 적 없음"으로 보인다.
    ///
    /// owner도 `FAKE_ME`로 심는다 — 「내가 만들고 내가 남긴」 상태이므로
    /// 서명자(`FAKE_ME`)와 채널 owner가 같아야 §5의 owner 검사를 통과한다.
    fn seed_applied(fx: &FakeEffects, item_key: &str, name: &str, steps: StepStates) -> Uuid {
        seed_applied_with_generation(fx, item_key, name, steps, 1)
    }

    /// `seed_applied`와 같지만 세대를 지정한다.
    ///
    /// 채널 ID와 provenance를 같은 세대로 도출/기록해야 `fetch_provenance`가
    /// 돌려준 provenance의 `generation`과 그 provenance가 실제로 실린 채널의
    /// ID가 서로 어긋나지 않는다.
    fn seed_applied_with_generation(
        fx: &FakeEffects,
        item_key: &str,
        name: &str,
        steps: StepStates,
        generation: u32,
    ) -> Uuid {
        let channel_id =
            derive_channel_id("wss://relay.test", "schoolx.default", item_key, generation);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: name.into(),
        });
        fx.set_channel_creator(channel_id, FAKE_ME);
        fx.seed_provenance(
            channel_id,
            FAKE_ME,
            provenance_with_generation(item_key, generation, steps),
        );
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
            // 설정 화면이 이 값을 방 이름으로 보여준다. 없으면 `item_key`가
            // 그대로 나가 사용자는 `메인 회의방` 자리에서 `meeting`을 본다.
            assert_eq!(
                item.name.as_deref(),
                Some(
                    crate::builtin()
                        .item(&item.item_key)
                        .expect("catalog item")
                        .name
                        .as_str()
                ),
                "{}의 catalog 표시 이름이 실리지 않았다",
                item.item_key
            );
            // provenance가 없으면 단계는 전부 미실행이고, 도출된 ID를 쓰는
            // 채널도 아직 없다.
            assert_eq!(item.steps, StepStates::default());
            assert!(!item.channel_present);
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
        let steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        seed_applied(&fx, "meeting", "메인 회의방", steps);

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let meeting = find(&items, "meeting");
        assert_eq!(meeting.decision, Decision::Resume);
        // saga는 provenance를 다시 읽지 않고 이 값을 그대로 쓴다. 여기서
        // 실제 상태가 실려 나가지 않으면 saga가 완료된 단계를 다시 실행한다.
        assert_eq!(meeting.steps, steps);
        assert!(meeting.channel_present);
    }

    #[tokio::test]
    async fn same_name_without_provenance_is_a_conflict() {
        let fx = FakeEffects::new();
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: Uuid::new_v4(),
            name: "기획".into(),
        });

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let planning = find(&items, "planning");
        assert_eq!(planning.decision, Decision::Conflict);
        // 동명 채널은 우리가 도출한 ID가 아니다 — 이름이 같다고 우리 것으로
        // 세지 않는다.
        assert!(!planning.channel_present);
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
    async fn renamed_complete_item_is_no_change() {
        let fx = FakeEffects::new();
        // catalog 이름은 "메인 회의방"인데 멤버가 바꿔 놓은 상태 — 이번엔 완료.
        seed_applied(&fx, "meeting", "2026 전체회의", done());

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let meeting = find(&items, "meeting");
        // `rename_is_a_flag_not_a_decision`은 미완료 항목이라 "renamed면
        // 무조건 Resume"인 버그와 구분되지 않는다. 완료 항목에서는 갈린다 —
        // renamed는 표시용 플래그일 뿐이라 완료면 이름이 바뀌었어도
        // NoChange여야 한다.
        assert_eq!(meeting.decision, Decision::NoChange);
        assert!(meeting.renamed);
    }

    #[tokio::test]
    async fn item_dropped_from_catalog_is_retired() {
        let fx = FakeEffects::new();
        // catalog에 없는 항목이지만 예전 버전에서 적용된 채로 남아 있다.
        seed_applied(&fx, "finance", "재무", done());

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let finance = find(&items, "finance");
        assert_eq!(finance.decision, Decision::Retired);
        // 이름은 catalog에만 있고 증명서에는 실리지 않는다 — catalog에서
        // 빠진 항목의 이름은 알 방법이 없다. `item_key`로 메우면 UI가
        // `finance`를 방 이름으로 보여주면서 그게 진짜 이름인지 모른다는
        // 뜻인지 구별할 수 없게 된다.
        assert_eq!(finance.name, None);
    }

    #[tokio::test]
    async fn incomplete_item_dropped_from_catalog_is_retired() {
        let fx = FakeEffects::new();
        // catalog에 없는 항목이고, 게다가 적용이 끝나기도 전에 catalog에서
        // 빠졌다. 설계 문서의 Retired 조건에는 완료 절이 없으므로 이래도
        // Retired다 — 완료를 요구하는 좁은 규칙과 여기서 갈린다.
        let steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        seed_applied(&fx, "finance", "재무", steps);

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let finance = find(&items, "finance");
        assert_eq!(finance.decision, Decision::Retired);
        // Retired가 곧 완료를 뜻하지는 않는다. 실제 단계 상태가 그대로
        // 실려야 ledger가 완료를 지어내지 않는다.
        assert_eq!(finance.steps, steps);
    }

    /// **도출된 채널에 실려 있지 않은 provenance는 없는 것으로 본다.**
    ///
    /// 도달 경로는 예외적인 상황이 아니다. relay의 읽기 ACL은 커뮤니티의 모든
    /// `open` 채널을 통과시키고, 인증된 사용자라면 누구나 open 채널을 만들어
    /// 자기 채널에 kind 39500을 발행할 수 있다 — 학교에서는 학생 아무나가 그
    /// 사용자다. `d` 태그도 `content`도 발행자가 정하므로 `item_key`를
    /// `meeting`으로, `steps`를 전부 완료로 적는 데 아무 권한도 필요 없다.
    ///
    /// 그 레코드를 권위로 읽으면 `meeting`은 영원히 `no_change`가 되어
    /// 체크박스가 잠기고, 학교의 표준 방은 만들어질 수 없다. 관리자에게는
    /// 우회로가 없다: 그 이벤트는 공격자의 채널에 있어 지울 수 없고, NIP-33
    /// LWW는 `(kind, pubkey, d)`별이라 덮어쓸 수도 없다.
    #[tokio::test]
    async fn provenance_published_in_another_channel_is_ignored() {
        let fx = FakeEffects::new();
        let derived = derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1);
        assert_ne!(
            attacker_channel(),
            derived,
            "공격자 채널이 도출된 채널과 같으면 이 테스트는 아무것도 검증하지 않는다"
        );
        // 공격자가 자기 open 채널에 `meeting` 완료 증명서를 발행했다.
        fx.seed_provenance_in_channel(
            attacker_channel(),
            "학생회 잡담방",
            provenance_with_generation("meeting", 1, done()),
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let meeting = find(&items, "meeting");

        // 채택되지 않았다 — "이미 적용됨"이 아니라 "적용한 적 없음"이다.
        assert_eq!(
            meeting.decision,
            Decision::CreateOrRecreate,
            "남의 채널에 실린 증명서가 판정을 결정했다"
        );
        // 공격자가 적어 둔 단계 상태를 한 글자도 물려받지 않았다. `decision`만
        // 보면 상태만 오염시키는 구현도 통과한다 — saga는 이 `steps`를 그대로
        // 이어받아 완료된 단계를 건너뛴다.
        assert_eq!(meeting.steps, StepStates::default());
        assert_eq!(meeting.generation, 1);
        assert_eq!(meeting.channel_id, Some(derived));
        // 공격자 채널은 도출된 ID가 아니므로 우리 방으로 세지 않는다.
        assert!(!meeting.channel_present);
    }

    /// 진짜 증명서가 **함께 있어도** 위조본이 이기지 않는다.
    ///
    /// 위 테스트는 위조본 하나뿐이라, 레코드를 훑는 순서만 바꾼 구현
    /// (예: "도출된 채널의 레코드를 우선한다")도 통과한다. 여기서는 위조본을
    /// **먼저** 넣는다 — 예전 코드의 `find`는 첫 일치를 그대로 쓰므로 이
    /// 순서에서 위조본이 이긴다. 진짜 증명서는 미완료라 판정이 갈린다:
    /// 위조본이 이기면 `no_change`, 버려지면 `resume`이다.
    #[tokio::test]
    async fn a_forged_record_does_not_outrank_the_real_one() {
        let fx = FakeEffects::new();
        let real_steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        // 공격자 레코드를 먼저 심는다. 세대까지 위조해 두었다 — 세대는 채널
        // ID 도출 입력이고 이 값 말고는 아무것도 그 필드를 흔들지 않으므로,
        // 채택되면 관리자는 존재하지 않는 방을 가리키게 된다.
        fx.seed_provenance_in_channel(
            attacker_channel(),
            "학생회 잡담방",
            provenance_with_generation("meeting", 9, done()),
        );
        let real_channel = seed_applied(&fx, "meeting", "메인 회의방", real_steps);

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let meeting = find(&items, "meeting");

        assert_eq!(meeting.decision, Decision::Resume);
        assert_eq!(meeting.steps, real_steps);
        assert_eq!(meeting.generation, 1, "위조된 세대가 채택됐다");
        assert_eq!(meeting.channel_id, Some(real_channel));
        assert!(meeting.channel_present);
    }

    /// 위조본은 `Retired` 줄도 만들어 내지 못한다.
    ///
    /// `Retired`는 catalog에 없는 `item_key`로 성립하므로, 공격자가 아무
    /// 문자열이나 `d` 태그에 적으면 모든 관리자의 화면에 유령 항목이 하나씩
    /// 늘어난다. 이 루프는 catalog 항목 루프와 달리 레코드를 그대로 훑기
    /// 때문에 결합 검사를 따로 통과해야 한다.
    #[tokio::test]
    async fn a_foreign_channel_record_does_not_create_a_retired_row() {
        let fx = FakeEffects::new();
        fx.seed_provenance_in_channel(
            attacker_channel(),
            "학생회 잡담방",
            provenance_with_generation("존재하지-않는-항목", 1, done()),
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(
            count(&items, "존재하지-않는-항목"),
            0,
            "남의 채널에 실린 증명서가 유령 항목을 만들었다"
        );
        assert_eq!(items.len(), crate::builtin().items.len());
    }

    /// 같은 `item_key`가 서로 다른 **세대**로 두 번 honoured돼도 `retired`
    /// 줄은 하나다.
    ///
    /// (이 테스트는 owner 검사 도입 전에는 "같은 채널·같은 세대, 서로 다른
    /// 서명자"로 dedup을 시험했다. 이제는 그 모양이 성립하지 않는다: 한
    /// 채널의 owner는 하나뿐이므로, owner가 아닌 서명자의 레코드는
    /// `honoured`에 들어가기 전에 §5의 owner 검사가 이미 버린다 — 즉 같은
    /// 채널·같은 세대에서 동시에 honoured되는 레코드는 최대 하나다. 이
    /// dedup이 실제로 시험받는 유일한 경로는 **세대**가 다른 경우다: 세대가
    /// 다르면 도출되는 채널도 다르므로(§5) 채널마다 별도의 owner가 각자
    /// 자기 레코드에 서명하면 둘 다 honoured될 수 있다.)
    ///
    /// kind 39500은 `(kind, pubkey, d 태그)`별로 addressable이라 NIP-33
    /// LWW는 신원 안에서만 적용된다 — 그래서 세대 1과 세대 2가 나란히
    /// 남을 수 있다(§4). 그 항목이 나중에 catalog에서 빠지면 이 루프가
    /// 레코드마다 한 줄씩 밀어 `item_key`가 같은 줄이 두 개 나가고, UI에는
    /// 같은 방이 두 번 보인다.
    #[tokio::test]
    async fn a_retired_item_at_two_generations_is_one_row() {
        let fx = FakeEffects::new();
        // 세대 1 — 채널·owner·서명자를 `seed_applied`가 함께 만들고
        // honoured 목록에 먼저 들어간다(삽입 순서를 그대로 따른다).
        seed_applied(&fx, "finance", "재무", done());
        // 세대 2 — 도출되는 채널이 다르므로 owner도 다른 사람("admin-b")
        // 일 수 있고, 그 사람이 자기 레코드에 서명하면 이 레코드도
        // honoured된다. steps를 세대 1과 다르게 두어, 아래 assert가
        // "먼저 심은 세대 1이 그대로 실렸다"를 실제로 구별하게 한다 — 둘을
        // 합치거나 뒤바꾸는 구현이었다면 여기서 갈린다.
        let gen2_channel = derive_channel_id("wss://relay.test", "schoolx.default", "finance", 2);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: gen2_channel,
            name: "재무 (구)".into(),
        });
        fx.set_channel_creator(gen2_channel, "admin-b");
        fx.seed_provenance(
            gen2_channel,
            "admin-b",
            provenance_with_generation(
                "finance",
                2,
                StepStates {
                    channel: StepStatus::Done,
                    canvas: StepStatus::Pending,
                    membership: StepStatus::Pending,
                },
            ),
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(
            count(&items, "finance"),
            1,
            "세대마다 한 줄씩 나갔다 — UI에 같은 방이 두 번 보인다"
        );
        let finance = find(&items, "finance");
        assert_eq!(finance.decision, Decision::Retired);
        // 먼저 만난 레코드(세대 1)를 그대로 실었다 — 두 레코드를 합치거나
        // 지어내지 않았다.
        assert_eq!(finance.steps, done());
        assert_eq!(finance.generation, 1);
    }

    #[tokio::test]
    async fn provenance_generation_is_reported_and_derives_channel_id() {
        let fx = FakeEffects::new();
        // provenance의 generation이 내장 catalog가 실제로 내는 값(1)보다
        // 크다 — 하드코딩된 1과 진짜 값을 구분하기 위해서다.
        seed_applied_with_generation(&fx, "meeting", "메인 회의방", done(), 3);

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        let meeting = find(&items, "meeting");
        assert_eq!(meeting.generation, 3);
        assert_eq!(
            meeting.channel_id,
            Some(derive_channel_id(
                "wss://relay.test",
                "schoolx.default",
                "meeting",
                3
            ))
        );
    }

    /// §5의 두 번째 조건: 도출된 채널에 실려 있어도 서명자가 그 채널의
    /// 생성자와 **다르면** 버린다 — 생성자를 몰라서가 아니라 생성자를 알고,
    /// 그 값이 서명자와 다르기 때문이다.
    ///
    /// 생성자를 `FAKE_ME`로 **등록해 둔다.** 등록하지 않으면 `channel_owner`가
    /// `Ok(None)`을 돌려주고, 그 레코드는 생성자 **불명** 분기에서 버려진다
    /// — 그건 이 테스트가 아니라 `provenance_is_ignored_when_the_owner_is_unknown`이
    /// 다루는 별개의 경로다.
    ///
    /// 첫 번째 조건(채널 결합)만으로는 도출된 ID를 먼저 선점한 공격자가
    /// 자기 채널 안에서 발행한 증명서를 거르지 못한다 — 정말 그 채널에
    /// 있기 때문이다. 채널 이름이 catalog 값과 같으므로, 증명서가 버려지면
    /// "적용한 적 없음 + 동명 채널 있음"이 되어 `Conflict`로 떨어진다.
    #[tokio::test]
    async fn provenance_signed_by_a_non_owner_is_ignored() {
        let fx = FakeEffects::new();
        let channel_id = derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: "메인 회의방".into(),
        });
        // 이 방은 내가 만들었다 — `is_owner`와 `channel_owner`가 둘 다
        // 이 하나에서 답을 낸다.
        fx.set_channel_creator(channel_id, FAKE_ME);
        // 그런데 증명서는 다른 사람이 서명했다 — owner는 알려져 있고,
        // 서명자와 **다르다**.
        fx.seed_provenance(
            channel_id,
            "someone-else",
            provenance_with_generation("meeting", 1, done()),
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        // 그 증명서는 없는 것으로 친다 — 이름이 catalog 값 그대로이므로
        // 동명 충돌로 떨어진다.
        assert_eq!(find(&items, "meeting").decision, Decision::Conflict);
    }

    /// §5의 owner 검사에는 서명자 불일치 말고 또 하나의 버림 사유가 있다:
    /// owner를 아예 특정할 수 없는 경우(`Ok(None)`). 위
    /// `provenance_signed_by_a_non_owner_is_ignored`와 대비된다 — 저기는
    /// owner가 `FAKE_ME`로 **알려져 있고** 서명자가 그와 다른 경우(불일치
    /// 분기)였다. 여기는 owner를 아예 등록하지 않는다(불명 분기).
    ///
    /// 서명자를 일부러 `FAKE_ME`로 둔다 — "내가 서명했다"는 사실조차 owner를
    /// 특정할 수 없으면 레코드를 구하지 못한다는 것을 보이기 위해서다. 이
    /// 분기를 `Some(owner) if owner == signer`만으로 시험하면 우연히도
    /// `None == signer`가 성립하지 않는다는 사실 하나로 통과해 버려서 정말
    /// "불명이라 버렸는지"를 증명하지 못한다 — 서명자를 아무 값으로 둬도
    /// 결과가 같다는 것 자체가 이 테스트의 핵심이다.
    #[tokio::test]
    async fn provenance_is_ignored_when_the_owner_is_unknown() {
        let fx = FakeEffects::new();
        let channel_id = derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: "메인 회의방".into(),
        });
        // owner를 등록하지 않는다 — `channel_owner`는 `Ok(None)`을 돌려준다.
        fx.seed_provenance(
            channel_id,
            FAKE_ME,
            provenance_with_generation("meeting", 1, done()),
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "meeting").decision, Decision::Conflict);
    }

    /// owner가 직접 서명한 증명서는 그대로 인정된다.
    #[tokio::test]
    async fn provenance_signed_by_the_owner_is_honoured() {
        let fx = FakeEffects::new();
        let channel_id = derive_channel_id("wss://relay.test", "schoolx.default", "meeting", 1);
        fx.channels.lock().expect("lock").push(ChannelRef {
            id: channel_id,
            name: "메인 회의방".into(),
        });
        fx.set_channel_creator(channel_id, "owner-pubkey");
        fx.seed_provenance(
            channel_id,
            "owner-pubkey",
            provenance_with_generation("meeting", 1, done()),
        );

        let items = preflight(crate::builtin(), &fx).await.expect("preflight");
        assert_eq!(find(&items, "meeting").decision, Decision::NoChange);
    }

    /// `channel_owner` 조회 자체가 실패해도 `preflight` 전체가 `Err`로
    /// 막히지 않는다 — 그 레코드만 버려진다.
    ///
    /// 이 루프는 공격자가 통제할 수 있는 channel_id를 순회한다. `?`로
    /// 올리면 아무나 자기 채널의 `channel_owner` 조회 하나를 실패하게 만드는
    /// 것만으로 관리자의 preflight 전체를 영구히 막을 수 있다 — 25줄 위
    /// 채널 결합 검사가 명시적으로 피하는 바로 그 지렛대다. `Ok(None)`과
    /// 같은 방향으로 버리는 것이 안전한 실패다: 진짜였을 레코드가 버려져도
    /// 그 항목은 "적용한 적 없음"으로 보일 뿐이고, 재적용은 §8의 캔버스
    /// 가드를 지나 기존 내용을 지키며 한 번 만에 스스로 복구된다.
    #[tokio::test]
    async fn channel_owner_lookup_failure_discards_the_record_without_erroring() {
        let fx = FakeEffects::new();
        seed_applied(&fx, "meeting", "메인 회의방", done());
        fx.fail_next("channel_owner");

        let items = preflight(crate::builtin(), &fx)
            .await
            .expect("channel_owner 조회 실패가 preflight 전체를 막았다");
        // 버려졌다 — "적용한 적 없음"인데 이름이 catalog 값 그대로인 채널이
        // 있으니 동명 충돌이다.
        assert_eq!(find(&items, "meeting").decision, Decision::Conflict);
    }
}
