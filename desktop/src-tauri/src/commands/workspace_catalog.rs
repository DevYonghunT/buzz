//! SchoolX 워크스페이스 catalog 적용.
//!
//! `schoolx-catalog` 크레이트의 saga를 실제 relay에 연결한다. 판정과 순서는
//! 전부 크레이트 쪽에 있고 여기에는 I/O만 있다 — 이 파일이 하는 일은
//! `CatalogEffects`의 각 메서드를 이 데스크톱 백엔드의 기존 relay 헬퍼로
//! 옮기는 것뿐이다.

use schoolx_catalog_pkg::effects::{
    CatalogEffects, ChannelRef, ChannelSpec, CreateOutcome, EffectError,
};
use schoolx_catalog_pkg::ledger::Ledger;
use schoolx_catalog_pkg::preflight::PreflightItem;
use schoolx_catalog_pkg::provenance::{d_tag, Provenance, KIND_WORKSPACE_PROVENANCE};
use tauri::State;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::relay::{relay_api_base_url_with_override, submit_event_with_keys};

/// `commands::channels::is_duplicate_channel_rejection`와 같은 판정을 한다
/// (그쪽은 모듈 비공개라 재사용할 수 없어 그대로 반복한다). `submit_event_with_keys`는
/// relay가 거부한 이벤트를 `"relay rejected event: <message>"`로 감싸고,
/// 채널 UUID 중복 거부의 메시지는 정확히 `"duplicate: channel already exists"`다.
fn is_duplicate_channel_rejection(error: &str) -> bool {
    error.contains("relay rejected event:") && error.contains("duplicate: channel already exists")
}

/// `CatalogEffects`를 이 데스크톱 백엔드의 relay 접근으로 구현한다.
///
/// 각 메서드는 같은 `commands` 디렉터리의 기존 채널/캔버스 명령이 쓰는 헬퍼를
/// 그대로 호출한다 — 새 relay 경로를 만들지 않는다.
struct RelayEffects<'a> {
    state: State<'a, AppState>,
}

#[async_trait::async_trait]
impl CatalogEffects for RelayEffects<'_> {
    async fn relay_scope(&self) -> String {
        relay_api_base_url_with_override(&self.state)
    }

    async fn list_channels(&self) -> Result<Vec<ChannelRef>, EffectError> {
        let channels = crate::commands::channels::get_channels(self.state.clone())
            .await
            .map_err(EffectError)?;
        Ok(channels
            .into_iter()
            .filter_map(|c| {
                Uuid::parse_str(&c.id)
                    .ok()
                    .map(|id| ChannelRef { id, name: c.name })
            })
            .collect())
    }

    async fn fetch_provenance(&self, catalog_id: &str) -> Result<Vec<Provenance>, EffectError> {
        // relay의 채널 스코프 ACL이 보안 경계다 — 여기서는 relay가 실제로
        // 돌려준 것만 이 catalog_id로 골라낼 뿐, 그 이상 거르거나 중복을
        // 지우거나 없는 값을 지어내지 않는다.
        //
        // kind 39500은 kind 39000과 달리 `(kind, pubkey, d-tag)`별로
        // addressable이다 — d 태그 하나당 이벤트가 전역에 하나가 아니라, 그
        // 항목을 완료·재개·채택한 신원마다 하나씩 쌓인다(다른 관리자가
        // 이어받는 경로는 saga에서 예외가 아니라 흔한 경로다). 그래서 예전처럼
        // 고정된 `limit`을 걸면 신원 수가 늘수록 결과가 조용히 잘릴 수 있고,
        // saga는 그 잘림을 알 방법이 없다 — 잘려서 빠진 항목은 provenance가
        // 없는 것처럼 보여 `CreateOrRecreate`로 오판하고, 이미 있는 채널에
        // `create_channel`을 걸어 `duplicate`를 받아도 `channel_present`가
        // (별도의, 잘리지 않는 `list_channels()` 호출에서 나오므로) true라
        // owner 게이트를 그대로 통과해 캔버스를 catalog 기본값으로 덮어쓴다.
        //
        // 그래서 개수 상한이 아니라 `#d` 필터로 범위를 좁힌다: 이 빌드에
        // 컴파일된 catalog(`schoolx_catalog_pkg::builtin()`)의 항목 키만큼만
        // d 태그를 만들어 건다 — 이 집합은 catalog 크기로 정확히 알려진 작은
        // 집합이다. 하지만 그 필터가 돌려주는 이벤트 **개수**는 신원 수에
        // 비례해 늘 수 있어 catalog 크기만으로는 그 개수의 상한을 셀 수
        // 없으므로, 매직 넘버 `limit` 대신 `commands::channels::query_relay_all`의
        // `(until, before_id)` 커서 페이징을 그대로 재사용해 몇 신원이 쌓아
        // 놨든 끝까지 읽는다.
        let d_tags: Vec<String> = schoolx_catalog_pkg::builtin()
            .items
            .iter()
            .map(|item| d_tag(catalog_id, &item.item_key))
            .collect();

        let events = crate::commands::channels::query_relay_all(
            &self.state,
            serde_json::json!({
                "kinds": [KIND_WORKSPACE_PROVENANCE],
                "#d": d_tags,
            }),
        )
        .await
        .map_err(EffectError)?;

        Ok(events
            .iter()
            .filter_map(|ev| serde_json::from_str::<Provenance>(&ev.content).ok())
            .filter(|p| p.catalog_id == catalog_id)
            .collect())
    }

    async fn create_channel(&self, spec: ChannelSpec) -> Result<CreateOutcome, EffectError> {
        let keys = self.state.signing_keys().map_err(EffectError)?;
        let builder = crate::events::build_create_channel(
            spec.id,
            &spec.name,
            spec.visibility.as_str(),
            &spec.channel_type,
            Some(&spec.description),
            None,
        )
        .map_err(EffectError)?;

        match submit_event_with_keys(builder, &self.state, &keys, None).await {
            Ok(_) => {
                // 이 실행이 방금 만든 채널만 표시한다 — kind:39002 멤버십이
                // 비동기로 전파되는 동안 `get_channels`가 소유자로 분류할 수
                // 있도록. `Duplicate` 분기에서는 절대 이걸 부르지 않는다:
                // 우리가 만든 게 아닌 방일 수 있고, saga의 owner 게이트가
                // 아직 그걸 확인하기 전이다.
                self.state
                    .mark_pending_owned_channel(&keys.public_key().to_hex(), &spec.id.to_string());
                Ok(CreateOutcome::Created)
            }
            Err(error) if is_duplicate_channel_rejection(&error) => Ok(CreateOutcome::Duplicate),
            Err(error) => Err(EffectError(error)),
        }
    }

    async fn set_canvas(&self, channel_id: Uuid, content: &str) -> Result<(), EffectError> {
        crate::commands::canvas::set_canvas(
            channel_id.to_string(),
            content.to_string(),
            self.state.clone(),
        )
        .await
        .map(|_| ())
        .map_err(EffectError)
    }

    async fn is_owner(&self, channel_id: Uuid) -> Result<bool, EffectError> {
        let keys = self.state.signing_keys().map_err(EffectError)?;
        let me = keys.public_key().to_hex();
        let response = crate::commands::channels::get_channel_members(
            channel_id.to_string(),
            self.state.clone(),
        )
        .await
        .map_err(EffectError)?;
        // relay 전반의 권한 검사와 같은 기준이다 (예: side_effects.rs) —
        // owner와 admin 모두 채널을 대신해 쓸 수 있는 상위 등급이고,
        // "owner" 단독 역할은 소유권 이전처럼 유일성이 필요한 연산에만 쓰인다.
        Ok(response
            .members
            .iter()
            .any(|m| m.pubkey == me && (m.role == "owner" || m.role == "admin")))
    }

    async fn publish_provenance(
        &self,
        channel_id: Uuid,
        provenance: &Provenance,
    ) -> Result<(), EffectError> {
        let keys = self.state.signing_keys().map_err(EffectError)?;
        let content = serde_json::to_string(provenance)
            .map_err(|e| EffectError(format!("provenance 직렬화 실패: {e}")))?;
        let builder = nostr::EventBuilder::new(
            nostr::Kind::Custom(KIND_WORKSPACE_PROVENANCE as u16),
            content,
        )
        .tags(vec![
            nostr::Tag::parse(vec!["d", &provenance.d_tag()])
                .map_err(|e| EffectError(format!("d 태그: {e}")))?,
            nostr::Tag::parse(vec!["h", &channel_id.to_string()])
                .map_err(|e| EffectError(format!("h 태그: {e}")))?,
        ]);

        submit_event_with_keys(builder, &self.state, &keys, None)
            .await
            .map(|_| ())
            .map_err(EffectError)
    }

    async fn now_rfc3339(&self) -> String {
        crate::util::now_iso()
    }
}

/// catalog 적용 전 항목별 판정을 돌려준다.
#[tauri::command]
pub async fn preflight_workspace_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<PreflightItem>, String> {
    let effects = RelayEffects { state };
    schoolx_catalog_pkg::preflight::preflight(schoolx_catalog_pkg::builtin(), &effects)
        .await
        .map_err(|e| e.0)
}

/// 선택한 catalog 항목을 적용하고 result ledger를 돌려준다.
#[tauri::command]
pub async fn apply_workspace_catalog(
    selected: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Ledger, String> {
    let effects = RelayEffects { state };
    schoolx_catalog_pkg::saga::apply(schoolx_catalog_pkg::builtin(), &effects, &selected)
        .await
        .map_err(|e| e.0)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// `create_channel`이 `Ok(CreateOutcome::Duplicate)`로 바꿔 읽어야 하는
    /// 정확한 문자열. `crates/buzz-relay/src/handlers/ingest.rs`의
    /// `create_channel_with_id` 분기가 실제로 내는 그대로다:
    /// `message: "duplicate: channel already exists".into()`를
    /// `submit_event_with_keys`가 `"relay rejected event: {message}"`로
    /// 감싼다. 이 리터럴이 바뀌면 saga가 중복 생성을 더는 인식하지 못하고
    /// 하드 에러로 새서, 다른 관리자가 이어받는 채택 경로 전체가 막힌다.
    #[test]
    fn matches_the_exact_relay_duplicate_channel_rejection() {
        assert!(is_duplicate_channel_rejection(
            "relay rejected event: duplicate: channel already exists"
        ));
    }

    /// 같은 "relay rejected event:" 래핑에 "duplicate:"까지 들어 있지만,
    /// 채널이 아니라 kind:7 reaction 중복이다 (`ingest.rs`의
    /// `message: "duplicate: reaction already exists".into()`). 문자열에
    /// "duplicate"가 있다는 것만으로 판정하면 이 무관한 거부까지
    /// `Duplicate`로 오판해, saga가 실제로는 만들어지지 않은 채널을 이미
    /// 있는 채널인 것처럼 채택하려 든다.
    #[test]
    fn does_not_match_a_different_duplicate_kind() {
        assert!(!is_duplicate_channel_rejection(
            "relay rejected event: duplicate: reaction already exists"
        ));
    }

    /// relay 거부와 아예 무관한 오류(`relay.rs`의 `classify_request_error`가
    /// 내는 연결 실패 메시지). 이런 오류까지 `Duplicate`로 새면 진짜 실패가
    /// "이미 있는 채널"로 둔갑해 saga가 아무 것도 하지 않고 성공한 것처럼
    /// 보고한다.
    #[test]
    fn does_not_match_an_unrelated_error() {
        assert!(!is_duplicate_channel_rejection(
            "relay unreachable: could not connect to relay"
        ));
    }
}
