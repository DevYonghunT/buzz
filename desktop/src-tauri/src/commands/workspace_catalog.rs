//! SchoolX 워크스페이스 catalog 적용.
//!
//! `schoolx-catalog` 크레이트의 saga를 실제 relay에 연결한다. 판정과 순서는
//! 전부 크레이트 쪽에 있고 여기에는 I/O만 있다 — 이 파일이 하는 일은
//! `CatalogEffects`의 각 메서드를 이 데스크톱 백엔드의 기존 relay 헬퍼로
//! 옮기는 것뿐이다.

use schoolx_catalog_pkg::effects::{
    CatalogEffects, ChannelRef, ChannelSpec, CreateOutcome, EffectError, ProvenanceRecord,
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

/// relay가 돌려준 kind 39500 이벤트를 `CatalogEffects::fetch_provenance`의
/// 계약 — [`ProvenanceRecord`] (채널 + 서명자 + 레코드 셋) — 으로 옮긴다.
///
/// **`h` 태그와 서명자를 반드시 같이 낸다.** 채널을 버리고 `content`만 내면
/// `preflight`가 도출된 채널 결합을 검사할 수 없고, 그러면 아무 인증
/// 사용자나(학교에서는 학생 누구나) 자기 open 채널을 만들어 거기에 kind
/// 39500을 발행하는 것만으로 관리자의 판정을 대신 정하게 된다 — 자기
/// 채널이라 그 쓰기는 relay에서 정당하게 승인된다. 서명자를 함께 내지 않으면
/// `preflight`는 §5의 owner 검사(선점된 채널 안에서 발행된 위조 증명서를
/// 걸러내는 검사)를 할 재료가 없다. 어느 쌍이 유효한가는 `preflight`가
/// 판정하고, 여기서는 relay가 말한 사실을 잃지 않는 것까지만 한다.
///
/// 서명자는 `ev.pubkey.to_hex()` — 소문자 hex(NIP-01). `nostr::PublicKey::to_hex()`는
/// 내부적으로 `hex::encode`를 쓰고, relay가 kind:39002 `p` 태그에 채널 owner를
/// 적을 때도(`crates/buzz-relay/src/handlers/side_effects.rs::emit_group_discovery_events`)
/// 같은 `hex::encode`를 쓴다 — 그래서 이 값은 `channel_owner`가 돌려주는 값과
/// 인코딩이 맞는다. `preflight`는 이 값을 그 값과 정확한 문자열(`==`)로 비교하므로
/// (`ProvenanceRecord::signer`의 문서 참고), 여기서 다른 인코딩으로 바꾸면 안 된다.
///
/// 세 경우에 이벤트를 버린다. 셋 다 "이 이벤트는 우리가 읽을 수 있는 레코드가
/// 아니다"이지 오류가 아니다.
///
/// - `h` 태그가 없거나 UUID로 파싱되지 않는다 — relay는 kind 39500에 `h`
///   스코프를 강제하므로(`requires_h_channel_scope`) 이건 일어날 수 없는
///   모양이다. 일어났다면 우리가 모르는 무언가이므로 채널을 지어내지 않는다.
/// - `content`가 `Provenance`로 파싱되지 않는다 — 남의 이벤트이거나, 더 새
///   버전이 쓴 레코드다(§4의 리더-우선 순서가 이 경우를 다룬다).
/// - 다른 catalog의 레코드다.
fn provenance_records_from_events(
    events: &[nostr::Event],
    catalog_id: &str,
) -> Vec<ProvenanceRecord> {
    events
        .iter()
        .filter_map(|ev| {
            let h_tag = ev.tags.iter().find_map(|t| {
                let s = t.as_slice();
                if s.len() >= 2 && s[0] == "h" {
                    Some(s[1].clone())
                } else {
                    None
                }
            })?;
            let channel_id = Uuid::parse_str(&h_tag).ok()?;
            let provenance = serde_json::from_str::<Provenance>(&ev.content).ok()?;
            (provenance.catalog_id == catalog_id).then_some(ProvenanceRecord {
                channel_id,
                signer: ev.pubkey.to_hex(),
                provenance,
            })
        })
        .collect()
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

    async fn fetch_provenance(
        &self,
        catalog_id: &str,
    ) -> Result<Vec<ProvenanceRecord>, EffectError> {
        // relay의 채널 스코프 ACL은 **경계가 아니라 1차 필터다.** 그 ACL이
        // 통과시키는 집합에는 커뮤니티의 모든 `open` 채널이 들어가고, 인증된
        // 사용자라면 누구나 open 채널을 만들어 자기 채널에 kind 39500을
        // 발행할 수 있다. 그래서 여기서는 relay가 돌려준 것을 이 catalog_id로
        // 골라내되 **각 레코드가 실려 있던 채널(`h` 태그)을 함께** 낸다 —
        // 진짜 판정은 `preflight`가 도출된 채널 ID와 대조해서 한다.
        // 그 이상 거르거나 중복을 지우거나 없는 값을 지어내지 않는다.
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

        Ok(provenance_records_from_events(&events, catalog_id))
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
        Ok(self.channel_owner(channel_id).await? == Some(me))
    }

    /// 채널의 **생성자**를 돌려준다 — 역할 테이블의 `owner`가 아니다.
    ///
    /// 채택이 묻는 것은 「여기서 쓸 수 있는가」가 아니라 「이 방이 우리 것인가」다.
    /// 역할로는 그 답을 낼 수 없다: `owner`는 상위 등급이 남에게 **줄 수 있는
    /// 값**이고(`buzz-db/src/channel.rs`의 부여 검사는 「주는 쪽이 elevated인가」만
    /// 본다) 개수 상한도 없다 — 하한 가드("마지막 owner를 못 뺀다")만 있다.
    /// 그래서 도출 ID를 선점한 공격자가 피해자에게 `admin`이 아니라 **`owner`**를
    /// 주면, 역할 기반 판정은 피해자에게 「이 방은 우리 것」이라고 답한다.
    /// `admin`만 떼는 것으로는 닫히지 않는다.
    ///
    /// `channels.created_by`는 생성 시 한 번 쓰이고 갱신되지 않으며 relay의 어떤
    /// 경로도 그 컬럼을 다시 쓰지 않는다. 그래서 선점자가 무슨 역할을 뿌리든
    /// 생성자는 바뀌지 않는다. 설계 근거: docs/schoolx-2/CATALOG_SECURITY.md §6.
    ///
    /// 값은 relay가 kind:39000의 `created_by` 태그에 `hex::encode`로 적은 소문자
    /// hex다(`side_effects.rs::emit_group_discovery_events`, 그리고 backfill 경로인
    /// `buzz-admin`의 reconcile도 같은 태그를 낸다). `ProvenanceRecord::signer`
    /// (`ev.pubkey.to_hex()`)도 같은 `hex::encode` 경로라 두 값의 인코딩이 맞는다 —
    /// `preflight`가 이 값을 `signer`와 정확한 문자열(`==`)로 비교한다.
    ///
    /// kind:39000은 relay만 저작할 수 있다(`is_relay_only_kind`, 회귀 테스트
    /// `e2e_relay.rs::test_client_submitted_nip29_group_metadata_and_admins_are_rejected`).
    /// 그 불변식이 없으면 이 값도 위조 가능해져 §6이 다시 무너진다.
    ///
    /// 태그가 비어 있으면 `Ok(None)` — 그 태그가 생기기 전에 쓰인 이벤트다.
    /// 「모른다」이지 「아무나」가 아니므로, `preflight`는 이 채널의 증명서를 전부
    /// 버리고 saga는 채택을 거부한다. 검증할 수 없는 것을 통과시키지 않는다.
    async fn channel_owner(&self, channel_id: Uuid) -> Result<Option<String>, EffectError> {
        let detail = crate::commands::channels::get_channel_details(
            channel_id.to_string(),
            self.state.clone(),
        )
        .await
        .map_err(EffectError)?;
        Ok(Some(detail.created_by).filter(|creator| !creator.is_empty()))
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

/// `get_my_relay_membership`의 응답에서 내 커뮤니티 역할을 꺼낸다.
///
/// 응답은 `{"member": {"pubkey": ..., "role": ...}}` 또는 `{"member": null}`이다
/// (`commands::relay_members::get_my_relay_membership`). 비멤버는 후자이고, 그때는
/// `None`이다 — 「역할을 모른다」와 「멤버가 아니다」를 여기서 구별하지 않는다.
/// `role_may_apply`가 둘 다 거부하므로 구별할 이유가 없다.
///
/// relay I/O에서 떼어 놓은 이유는 `role_may_apply`와 같다: 이 게이트에서
/// 손으로 쓴 파싱은 여기뿐이라 따로 시험할 수 있어야 한다.
fn membership_role(membership: &serde_json::Value) -> Option<String> {
    membership
        .get("member")?
        .get("role")?
        .as_str()
        .map(str::to_string)
}

/// 이 커뮤니티 역할이 catalog를 적용할 수 있는가.
///
/// relay I/O에서 분리해 두어 판정만 단위 테스트할 수 있게 한다. 모르는
/// 역할은 거부한다 — 나중에 역할이 추가되어도 자동으로 권한을 얻지 않아야
/// 한다.
fn role_may_apply(role: Option<&str>) -> bool {
    matches!(role, Some("owner") | Some("admin"))
}

/// 커뮤니티 역할이 없어 거부했다.
///
/// 이 값들은 문구가 아니라 **식별자**다. 사용자에게 보일 문장은 프론트엔드가
/// 지역화한다(`features/workspace-catalog/catalogError.ts`) — 어댑터가 한국어를
/// 하드코딩하면 영어 로케일 사용자에게 그대로 샌다.
const CATALOG_ADMIN_REQUIRED: &str = "catalog-admin-required";

/// 이 relay에 커뮤니티 역할이라는 개념 자체가 없어 판정할 수 없다.
///
/// `catalog-admin-required`와 **구별해서** 낸다. 두 상태는 사용자가 할 일이
/// 다르다: 전자는 관리자에게 요청하면 되고, 후자는 요청할 관리자가 없다
/// (릴레이 설정 문제다). 하나로 뭉개면 오픈 릴레이에서 커뮤니티 소유자
/// 본인에게 "관리자에게 요청하세요"라고 말하게 된다.
const CATALOG_MEMBERSHIP_UNAVAILABLE: &str = "catalog-membership-unavailable";

/// 게이트의 판정 전부. relay I/O에서 떼어 놓아 네 갈래를 전부 시험한다.
///
/// `requires_membership`가 `false`면 `membership`은 보지 않는다 — 그 릴레이는
/// kind 13534를 애초에 발행하지 않으므로 조회할 것이 없다.
///
/// **오픈 릴레이를 통과시키지 않는다.** 「명부가 없다」를 「제한 없음」으로
/// 읽으면 게이트가 그 릴레이에서 no-op이 되고, `CATALOG_SECURITY.md` §1의
/// 선점 공격 앞에 데스크톱 경로가 무방비가 된다. 판정할 수 없으면 거부하되,
/// 무엇이 문제인지 구별되는 식별자로 말한다.
fn gate_decision(requires_membership: bool, membership: &serde_json::Value) -> Result<(), String> {
    if !requires_membership {
        return Err(CATALOG_MEMBERSHIP_UNAVAILABLE.to_string());
    }
    if role_may_apply(membership_role(membership).as_deref()) {
        Ok(())
    } else {
        Err(CATALOG_ADMIN_REQUIRED.to_string())
    }
}

/// catalog 적용은 커뮤니티 관리자만 할 수 있다.
///
/// 근거가 되는 역할은 relay가 서명한 kind 13534 목록에서 온다 —
/// 클라이언트가 만드는 값이 아니라 위조할 수 없다. `is_relay_only_kind`가
/// 그 kind의 클라이언트 제출을 ingest에서 거부하고
/// (`buzz-core/src/kind.rs`, 회귀 테스트 `e2e_relay.rs`의
/// `test_client_submitted_nip43_membership_snapshots_are_rejected`), 그래서
/// 이 게이트의 근거는 채널 멤버십(kind 39002)과 달리 위조 가능한 값이 아니다.
/// 채널 레벨 역할이 아니라 커뮤니티 레벨 역할을 본다: 이 동작은 커뮤니티
/// 전체에 기본 업무방을 만드는 일이기 때문이다.
///
/// **그 kind는 오픈 릴레이에는 없다.** `require_relay_membership`는 기본값이
/// `false`이고(`buzz-relay/src/config.rs`), 그때 relay는 NIP-43을 광고하지도
/// 발행하지도 않는다(`nip11.rs`의 `advertise_nip43`). `.env.example`도
/// `just test-e2e`도 그 값을 켜지 않으므로 기본 개발 릴레이가 그 상태다.
/// 그래서 명부를 조회하기 전에 릴레이가 NIP-43을 광고하는지부터 묻는다 —
/// 명부의 부재를 거부로 읽으면 커뮤니티 소유자 본인이 잠긴다. 프론트엔드도
/// 같은 규칙을 명문화하고 있다(`shared/api/relayMembers.ts`의
/// `snapshotFound` doc: "absence of this snapshot must not be treated as a
/// denial").
///
/// preflight도 막는다. 미리보기만으로도 어떤 항목이 이미 적용됐는지가
/// 드러나고 그것은 private 채널의 존재 정보다.
///
/// 이 게이트는 클라이언트 측이다. 직접 relay에 채널 생성 이벤트를 쏘는
/// 것은 막지 못하며 막으려는 대상도 아니다 — 채널을 만드는 것은 모든
/// 구성원의 정상 권한이고, 여기서 막는 것은 「catalog 적용으로 기본 업무방
/// 일습을 만드는 것」이다. 설계 근거: docs/schoolx-2/CATALOG_SECURITY.md §3·§4.
async fn require_community_admin(state: &State<'_, AppState>) -> Result<(), String> {
    let requires_membership =
        crate::commands::relay_members::relay_requires_membership(state.clone()).await?;
    // 명부가 존재할 수 있을 때만 조회한다. 오픈 릴레이에서는 왕복이 낭비고,
    // 응답도 언제나 `{"member": null}`이라 판정에 보탤 것이 없다.
    let membership = if requires_membership {
        crate::commands::relay_members::get_my_relay_membership(state.clone()).await?
    } else {
        serde_json::Value::Null
    };
    gate_decision(requires_membership, &membership)
}

/// catalog 적용 전 항목별 판정을 돌려준다.
#[tauri::command]
pub async fn preflight_workspace_catalog(
    state: State<'_, AppState>,
) -> Result<Vec<PreflightItem>, String> {
    require_community_admin(&state).await?;
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
    require_community_admin(&state).await?;
    let effects = RelayEffects { state };
    schoolx_catalog_pkg::saga::apply(schoolx_catalog_pkg::builtin(), &effects, &selected)
        .await
        .map_err(|e| e.0)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use schoolx_catalog_pkg::provenance::{StepStates, StepStatus};

    /// 공격자가 만든 `open` 채널의 ID — 도출된 채널이 아니다. 인증된 사용자면
    /// 누구나 이런 채널을 만들고 거기에 kind 39500을 발행할 수 있다.
    const FOREIGN_CHANNEL: &str = "11111111-2222-4333-8444-555555555555";

    fn sample_provenance(catalog_id: &str) -> Provenance {
        Provenance {
            catalog_id: catalog_id.to_string(),
            catalog_version: 1,
            item_key: "meeting".into(),
            generation: 1,
            steps: StepStates {
                channel: StepStatus::Done,
                canvas: StepStatus::Done,
                membership: StepStatus::Done,
            },
            applied_at: "2026-07-28T09:00:00Z".into(),
        }
    }

    /// relay가 돌려주는 모양 그대로의 kind 39500 이벤트를 만든다. `h`가
    /// `None`이면 태그를 아예 달지 않는다.
    fn provenance_event(h: Option<&str>, content: &str) -> nostr::Event {
        let keys = nostr::Keys::generate();
        let mut tags =
            vec![nostr::Tag::parse(["d", "schoolx.default:meeting"]).expect("d 태그가 만들어진다")];
        if let Some(h) = h {
            tags.push(nostr::Tag::parse(["h", h]).expect("h 태그가 만들어진다"));
        }
        nostr::EventBuilder::new(
            nostr::Kind::Custom(KIND_WORKSPACE_PROVENANCE as u16),
            content,
        )
        .tags(tags)
        .sign_with_keys(&keys)
        .expect("테스트 이벤트가 서명된다")
    }

    fn json(provenance: &Provenance) -> String {
        serde_json::to_string(provenance).expect("직렬화")
    }

    /// `h` 태그를 그대로 실어 낸다 — 그것도 **도출된 채널이 아닌** 값을.
    ///
    /// 이 어댑터는 어느 채널이 옳은지 판정하지 않는다. 판정은 `preflight`가
    /// 도출식으로 하고, 여기서 채널을 버리면 그쪽이 판정할 재료 자체가
    /// 사라진다 — 그 상태에서는 아무 학생이나 자기 open 채널에 발행한
    /// 레코드가 관리자의 판정을 대신 정한다. 그래서 "남의 채널이면 여기서
    /// 걸러내면 되지 않나"의 답이 아니라, **잃지 않고 넘긴다**가 이 함수의
    /// 계약이다.
    #[test]
    fn the_h_tag_travels_with_the_record() {
        let provenance = sample_provenance("schoolx.default");
        let events = vec![provenance_event(Some(FOREIGN_CHANNEL), &json(&provenance))];
        let signer = events[0].pubkey.to_hex();

        let records = provenance_records_from_events(&events, "schoolx.default");
        assert_eq!(
            records,
            vec![ProvenanceRecord {
                channel_id: Uuid::parse_str(FOREIGN_CHANNEL).expect("고정 UUID"),
                signer,
                provenance,
            }]
        );
    }

    /// `ProvenanceRecord::signer`는 그 이벤트에 실제로 서명한 pubkey를 소문자
    /// hex로 나른다(`ev.pubkey.to_hex()`) — `preflight`가 `channel_owner`와
    /// 정확한 문자열로 비교하는 값이라(§5), 여기서 잃거나 인덱스와 뒤섞이면
    /// 안 된다. 이벤트마다 서로 다른 무작위 키로 서명해, 우연히 같은 문자열이
    /// 나와 이 검증이 무력화되지 않게 한다.
    #[test]
    fn the_signer_travels_with_the_record() {
        let mine = sample_provenance("schoolx.default");
        let other_channel = "99999999-8888-4777-8666-555555555555";
        let events = vec![
            provenance_event(Some(FOREIGN_CHANNEL), &json(&mine)),
            provenance_event(Some(other_channel), &json(&mine)),
        ];
        let expected_signers: Vec<String> = events.iter().map(|e| e.pubkey.to_hex()).collect();
        assert_ne!(
            expected_signers[0], expected_signers[1],
            "테스트 이벤트는 서로 다른 무작위 키로 서명돼야 이 검증이 의미가 있다"
        );

        let records = provenance_records_from_events(&events, "schoolx.default");
        let actual_signers: Vec<String> = records.iter().map(|r| r.signer.clone()).collect();
        assert_eq!(actual_signers, expected_signers);
    }

    /// `h` 태그가 없는 kind 39500은 relay가 애초에 받지 않는다
    /// (`requires_h_channel_scope`). 그래도 왔다면 우리가 모르는 무언가이므로
    /// 채널을 지어내지 않고 버린다 — 채널 없이 통과시키면 `preflight`가
    /// 결합을 검사할 방법이 없다.
    #[test]
    fn an_event_without_an_h_tag_is_dropped() {
        let events = vec![provenance_event(
            None,
            &json(&sample_provenance("schoolx.default")),
        )];
        assert!(provenance_records_from_events(&events, "schoolx.default").is_empty());
    }

    #[test]
    fn an_unparseable_h_tag_is_dropped() {
        let events = vec![provenance_event(
            Some("채널이-아니다"),
            &json(&sample_provenance("schoolx.default")),
        )];
        assert!(provenance_records_from_events(&events, "schoolx.default").is_empty());
    }

    /// 다른 catalog의 레코드는 이 catalog의 판정에 끼어들지 않는다. `h` 태그를
    /// 나르기 시작했다고 이 필터가 사라지면, 다른 catalog가 같은 `item_key`를
    /// 쓰는 것만으로 판정이 섞인다.
    #[test]
    fn another_catalogs_record_is_dropped() {
        let events = vec![provenance_event(
            Some(FOREIGN_CHANNEL),
            &json(&sample_provenance("someone.else")),
        )];
        assert!(provenance_records_from_events(&events, "schoolx.default").is_empty());
    }

    /// 파싱되지 않는 content는 버린다 — 더 새 버전이 쓴 레코드이거나 남의
    /// 이벤트다 (§4의 리더-우선 순서가 이 경우를 다룬다).
    #[test]
    fn an_unparseable_content_is_dropped() {
        let events = vec![provenance_event(Some(FOREIGN_CHANNEL), "not json")];
        assert!(provenance_records_from_events(&events, "schoolx.default").is_empty());
    }

    /// 여러 이벤트가 섞여 와도 살아남을 것만 살아남고, 그 순서와 채널 결합이
    /// 보존된다. 한 건짜리 테스트만으로는 `filter_map`이 첫 실패에서 멈추는
    /// 구현과 구별되지 않는다.
    #[test]
    fn a_mixed_batch_keeps_only_the_readable_records() {
        let mine = sample_provenance("schoolx.default");
        let other_channel = "99999999-8888-4777-8666-555555555555";
        let events = vec![
            provenance_event(None, &json(&mine)),
            provenance_event(Some(FOREIGN_CHANNEL), &json(&mine)),
            provenance_event(Some("채널이-아니다"), &json(&mine)),
            provenance_event(Some(other_channel), "not json"),
            provenance_event(Some(other_channel), &json(&mine)),
        ];
        let signer_for_foreign = events[1].pubkey.to_hex();
        let signer_for_other = events[4].pubkey.to_hex();

        let records = provenance_records_from_events(&events, "schoolx.default");
        assert_eq!(
            records,
            vec![
                ProvenanceRecord {
                    channel_id: Uuid::parse_str(FOREIGN_CHANNEL).expect("고정 UUID"),
                    signer: signer_for_foreign,
                    provenance: mine.clone(),
                },
                ProvenanceRecord {
                    channel_id: Uuid::parse_str(other_channel).expect("고정 UUID"),
                    signer: signer_for_other,
                    provenance: mine,
                },
            ]
        );
    }

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

    /// 커뮤니티 소유자와 관리자만 catalog를 적용한다. 모르는 역할은 거부한다 —
    /// 나중에 역할이 추가되어도 자동으로 권한을 얻으면 안 된다.
    #[test]
    fn only_community_owner_and_admin_may_apply() {
        assert!(role_may_apply(Some("owner")));
        assert!(role_may_apply(Some("admin")));
        assert!(!role_may_apply(Some("member")));
        assert!(!role_may_apply(None));
        assert!(!role_may_apply(Some("guest")));
    }

    /// `relay_members_from_event`는 역할 태그가 비었으면 `"member"`로 떨어뜨리고
    /// (`nostr_convert.rs:516-520`), 멤버가 아니면 `get_my_relay_membership`이
    /// `{"member": null}`을 돌려준다. 그 두 모양을 `role_may_apply`가 받는
    /// `Option<&str>`로 옮기는 부분이 이 게이트에서 유일하게 손으로 쓴 파싱이라
    /// 따로 고정한다 — 여기서 `None`이 잘못 나오면 게이트가 조용히 전원 거부가
    /// 되고, 잘못 `Some("admin")`이 나오면 게이트가 아무나 통과시킨다.
    #[test]
    fn membership_role_is_read_from_the_relay_signed_shape() {
        assert_eq!(
            membership_role(&serde_json::json!({ "member": { "pubkey": "ab", "role": "admin" } })),
            Some("admin".to_string())
        );
        // 비멤버 — relay가 나를 목록에서 못 찾은 모양.
        assert_eq!(
            membership_role(&serde_json::json!({ "member": null })),
            None
        );
        // 역할 없는 멤버는 있을 수 없는 모양이지만(`relay_members_from_event`가
        // 항상 채운다), 왔다면 권한을 지어내지 않는다.
        assert_eq!(
            membership_role(&serde_json::json!({ "member": { "pubkey": "ab" } })),
            None
        );
    }

    /// 게이트가 내는 세 결과를 전부 고정한다.
    ///
    /// 오픈 릴레이 갈래가 특히 중요하다. `require_relay_membership`는 기본값이
    /// `false`라(`buzz-relay/src/config.rs`) `.env.example`로 띄운 개발 릴레이와
    /// `just test-e2e`가 전부 그 상태다. 이 갈래를 `Ok`로 만들면 게이트가 거기서
    /// 조용히 no-op이 되고, `catalog-admin-required`로 뭉개면 커뮤니티 소유자
    /// 본인에게 "관리자에게 요청하세요"라고 말하게 된다 — 요청할 관리자가
    /// 없는데도.
    #[test]
    fn a_relay_without_community_roles_is_refused_but_says_so_differently() {
        assert_eq!(
            gate_decision(false, &serde_json::Value::Null),
            Err("catalog-membership-unavailable".to_string())
        );
        // 명부가 있다고 주장하는 릴레이에서도 역할이 없으면 거부한다. 다만
        // 이쪽은 사용자가 관리자에게 요청하면 풀린다.
        assert_eq!(
            gate_decision(true, &serde_json::json!({ "member": null })),
            Err("catalog-admin-required".to_string())
        );
        assert_eq!(
            gate_decision(true, &serde_json::json!({ "member": { "role": "member" } })),
            Err("catalog-admin-required".to_string())
        );
        assert_eq!(
            gate_decision(true, &serde_json::json!({ "member": { "role": "admin" } })),
            Ok(())
        );
        assert_eq!(
            gate_decision(true, &serde_json::json!({ "member": { "role": "owner" } })),
            Ok(())
        );
    }

    /// 오픈 릴레이에서는 역할을 보지 않는다 — 봐서도 안 된다. 그 릴레이가
    /// 어떤 이유로 `owner`가 담긴 명부를 돌려주더라도 통과시키지 않는다:
    /// NIP-43을 광고하지 않는 릴레이의 kind 13534는 relay-only 불변식
    /// (`is_relay_only_kind`)이 지켜 준다는 보장이 없기 때문이다.
    #[test]
    fn an_open_relay_is_refused_even_if_it_claims_a_role() {
        assert_eq!(
            gate_decision(false, &serde_json::json!({ "member": { "role": "owner" } })),
            Err("catalog-membership-unavailable".to_string())
        );
    }
}
