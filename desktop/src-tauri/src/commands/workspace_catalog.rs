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
use schoolx_catalog_pkg::provenance::{Provenance, KIND_WORKSPACE_PROVENANCE};
use tauri::State;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::relay::{query_relay, relay_api_base_url_with_override, submit_event_with_keys};

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
        let events = query_relay(
            &self.state,
            &[serde_json::json!({
                "kinds": [KIND_WORKSPACE_PROVENANCE],
                "limit": 200
            })],
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
