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
use crate::relay::{relay_api_base_url_with_override, submit_event_with_keys};

/// `commands::channels::is_duplicate_channel_rejection`와 같은 판정을 한다
/// (그쪽은 모듈 비공개라 재사용할 수 없어 그대로 반복한다). `submit_event_with_keys`는
/// relay가 거부한 이벤트를 `"relay rejected event: <message>"`로 감싸고,
/// 채널 UUID 중복 거부의 메시지는 정확히 `"duplicate: channel already exists"`다.
fn is_duplicate_channel_rejection(error: &str) -> bool {
    error.contains("relay rejected event:") && error.contains("duplicate: channel already exists")
}

/// `commands::canvas::get_canvas`의 JSON 응답을 `CatalogEffects::read_canvas`의
/// 계약으로 옮긴다.
///
/// `get_canvas`는 캔버스 이벤트가 하나도 없으면 오류가 아니라
/// `{"content": "", "event_id": null, ...}`을 돌려준다 — "채널은 있는데
/// 캔버스가 없다"는 정상 상태이므로 `Ok(None)`이다. `event_id`가 있는데
/// `content`를 문자열로 읽을 수 없으면 그건 "비어 있다"가 **아니라**
/// 모르겠다이다. 빈 문자열로 뭉개면 saga가 그 방을 비었다고 보고 사용자
/// 내용 위에 시작 캔버스를 쓴다 — 그래서 오류로 돌린다.
fn canvas_content_from_response(
    response: &serde_json::Value,
) -> Result<Option<String>, EffectError> {
    if response
        .get("event_id")
        .is_none_or(serde_json::Value::is_null)
    {
        return Ok(None);
    }

    response
        .get("content")
        .and_then(serde_json::Value::as_str)
        .map(|content| Some(content.to_string()))
        .ok_or_else(|| EffectError("캔버스 응답에 content 문자열이 없습니다".to_string()))
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
        // 그래서 개수 상한이 아니라 `commands::channels::query_relay_all`의
        // `(until, before_id)` 커서 페이징을 그대로 재사용한다 — 매직 넘버
        // `limit` 없이 몇 신원이 쌓아 놨든 끝까지 읽는다.
        //
        // 필터는 반드시 `{"kinds": [KIND_WORKSPACE_PROVENANCE]}`뿐이어야 한다.
        // 절대로 이걸 현재 catalog 항목으로, 예를 들어
        // `schoolx_catalog_pkg::builtin().items`에서 만든 `#d` 태그 목록으로
        // 좁히지 마라. `Decision::Retired`(`preflight.rs`)는 provenance는
        // 있는데 그 `item_key`가 지금 catalog에는 없는 항목을 찾아서 성립하는
        // 판정이다 — 예전 catalog 버전이 만든 방인데 그 항목이 이후
        // `catalog.json`에서 빠져도 사용자에게 계속 보여주기 위한 것이다.
        // 이 조회를 현재 catalog 항목으로 좁히면 relay가 돌려줄 수 있는 모든
        // 레코드가 이미 catalog에 있는 항목으로 보장되어 버려서, catalog에서
        // 항목이 하나라도 빠지는 순간부터 Retired는 영영 성립할 수 없다 —
        // 상황에 따라 가끔이 아니라 매번, 결정적으로. `crates/schoolx-catalog`의
        // `FakeEffects::fetch_provenance`는 `catalog_id`와 살아 있는 채널로만
        // 걸러 `#d` 스코핑 개념이 아예 없으므로, 크레이트 테스트
        // (`item_dropped_from_catalog_is_retired`,
        // `incomplete_item_dropped_from_catalog_is_retired`)는 이 회귀를 잡지
        // 못한다 — 여기서 다시 좁히면 데스크톱 빌드에서만 조용히 재발한다.
        let events = crate::commands::channels::query_relay_all(
            &self.state,
            serde_json::json!({
                "kinds": [KIND_WORKSPACE_PROVENANCE],
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

    async fn read_canvas(&self, channel_id: Uuid) -> Result<Option<String>, EffectError> {
        // 새 relay 경로를 만들지 않는다 — 기존 캔버스 읽기 명령을 그대로
        // 쓴다. 매핑은 `canvas_content_from_response`가 한다.
        let response =
            crate::commands::canvas::get_canvas(channel_id.to_string(), self.state.clone())
                .await
                .map_err(EffectError)?;
        canvas_content_from_response(&response)
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

    /// `commands::canvas::get_canvas`가 캔버스 이벤트가 하나도 없을 때 실제로
    /// 내는 응답 그대로다. 채널은 있는데 캔버스가 없는 것은 오류가 아니므로
    /// `Ok(None)`이어야 한다 — 오류로 돌리면 새 방마다 캔버스 단계가
    /// `failed`가 되어 시작 캔버스가 영영 들어가지 않는다.
    #[test]
    fn missing_canvas_event_reads_as_no_canvas() {
        let response = serde_json::json!({
            "content": "",
            "event_id": null,
            "updated_at": null,
            "author": null,
        });
        assert_eq!(canvas_content_from_response(&response), Ok(None));
    }

    #[test]
    fn existing_canvas_reads_as_its_content() {
        let response = serde_json::json!({
            "content": "팀이 직접 정리한 회의 규칙",
            "event_id": "abc123",
            "updated_at": 1_770_000_000_u64,
            "author": "deadbeef",
        });
        assert_eq!(
            canvas_content_from_response(&response),
            Ok(Some("팀이 직접 정리한 회의 규칙".to_string()))
        );
    }

    /// 본문이 빈 캔버스 **이벤트**는 이벤트가 없는 것과 다르다. 여기서는
    /// 있는 그대로 `Some("")`로 옮기고, "지켜야 할 내용인가"는 saga가
    /// 판단한다 (`schoolx-catalog`의 캔버스 단계). 어댑터가 미리 `None`으로
    /// 뭉개면 두 상태가 saga에 도착하기도 전에 사라진다.
    #[test]
    fn empty_content_with_an_event_is_reported_as_empty_not_missing() {
        let response = serde_json::json!({
            "content": "",
            "event_id": "abc123",
            "updated_at": 1_770_000_000_u64,
            "author": "deadbeef",
        });
        assert_eq!(
            canvas_content_from_response(&response),
            Ok(Some(String::new()))
        );
    }

    /// 캔버스 이벤트는 있는데 본문을 문자열로 읽을 수 없다 — 응답 모양이
    /// 우리가 아는 것과 다르다. 이건 "비어 있다"가 아니라 **모르겠다**이므로
    /// 오류여야 한다. `unwrap_or_default()`로 빈 문자열을 지어내면 saga가 그
    /// 방을 비었다고 보고 사용자 캔버스 위에 시작 캔버스를 덮어쓴다.
    #[test]
    fn unreadable_content_is_an_error_not_an_empty_canvas() {
        let response = serde_json::json!({
            "content": 42,
            "event_id": "abc123",
        });
        assert!(canvas_content_from_response(&response).is_err());
    }
}
