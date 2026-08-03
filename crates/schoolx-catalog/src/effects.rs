//! relay I/O 경계.
//!
//! saga가 이 trait 뒤에서만 relay와 이야기하므로, fault injection 테스트가
//! live relay 없이 돈다. 실제 구현은 데스크톱 Tauri 백엔드에 있다.

use crate::catalog::Visibility;
use crate::provenance::Provenance;
use uuid::Uuid;

/// effect 실행 실패. 메시지는 사용자에게 그대로 보일 수 있다.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct EffectError(pub String);

/// 접근 가능한 채널 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRef {
    /// 채널 UUID.
    pub id: Uuid,
    /// 현재 표시 이름. 사용자가 바꿨을 수 있다.
    pub name: String,
}

/// 채널 생성 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSpec {
    /// 결정론적으로 도출된 채널 UUID.
    pub id: Uuid,
    /// 채널 이름.
    pub name: String,
    /// 채널 설명.
    pub description: String,
    /// `stream` 또는 `forum`.
    pub channel_type: String,
    /// 공개 범위.
    pub visibility: Visibility,
}

/// 채널 생성 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateOutcome {
    /// 새로 만들어졌다.
    Created,
    /// 이 UUID가 이미 점유돼 있다. relay가 채널을 soft delete하고 ID를 계속
    /// 점유하므로, 접근 가능 목록에 없다면 예전에 만들었다가 삭제된 것이다.
    Duplicate,
}

/// relay에서 읽어 온 provenance 한 건과, 그것을 검증하는 데 필요한 맥락.
///
/// 내용만으로는 신뢰할 수 없다 — 어느 채널에 실려 있었는지(`channel_id`)와
/// 누가 서명했는지(`signer`)가 있어야 §5의 두 조건을 검사할 수 있다.
/// 튜플로 두면 세 값의 순서를 호출부가 외워야 하므로 이름을 붙인다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceRecord {
    /// 이 이벤트가 실려 있던 채널 (`h` 태그).
    pub channel_id: Uuid,
    /// 이 이벤트에 서명한 pubkey — 소문자 hex(NIP-01 표준 인코딩). 이
    /// 크레이트는 [`CatalogEffects::channel_owner`]가 돌려주는 값과 정확한
    /// 문자열(`==`)로 비교하므로, 인코딩이 어긋나면(대문자, bech32/npub 등)
    /// 실제로 같은 키라도 다른 문자열로 보여 조용히 서명자 불일치로
    /// 처리된다.
    pub signer: String,
    /// 이벤트 content.
    pub provenance: Provenance,
}

/// saga가 필요로 하는 relay 연산.
#[async_trait::async_trait]
pub trait CatalogEffects: Send + Sync {
    /// 채널 ID 도출에 쓰이는 relay 범위 문자열.
    async fn relay_scope(&self) -> String;

    /// 현재 사용자가 접근할 수 있는 채널. 삭제된 채널은 포함되지 않는다.
    async fn list_channels(&self) -> Result<Vec<ChannelRef>, EffectError>;

    /// 읽을 수 있는 이 catalog의 provenance 이벤트 전부.
    ///
    /// 채널 스코프라 비멤버인 항목은 결과에 나타나지 않는다. 이건 버그가
    /// 아니라 보안 계약이다.
    ///
    /// 각 레코드는 실려 있던 채널과 서명자를 함께 담는다. 둘 다
    /// `preflight`가 §5의 검증에 쓴다 — 채널 결합만으로는 그 채널을 선점한
    /// 사람이 발행한 증명서를 걸러내지 못한다.
    ///
    /// # 채널을 같이 나르는 이유
    ///
    /// relay의 읽기 ACL은 "이 사용자가 접근할 수 있는 채널의 이벤트"까지만
    /// 좁힌다. 그런데 그 집합에는 커뮤니티의 **모든 `open` 채널**이 들어가고,
    /// 인증된 사용자라면 누구나 open 채널을 만들어 자기 채널에 kind 39500을
    /// 발행할 수 있다 — 자기 채널이므로 쓰기도 정당하게 승인된다. 즉 "읽혔다"는
    /// 것은 "이 catalog가 쓴 레코드다"를 조금도 뜻하지 않는다.
    ///
    /// 레코드 본문(`d` 태그, `item_key`, `generation`)은 전부 발행자가 정한
    /// 값이라 어느 것도 근거가 되지 못한다. 위조할 수 없는 결합은 하나뿐이다:
    /// 그 레코드가 실려 있는 **채널**이 §5의 도출식이 예측하는 채널과 같은가.
    /// 그래서 이 메서드는 채널을 버리지 않는다 — 버리는 순간 [`preflight`]가
    /// 그 검사를 할 수 없고, 아무 학생이나 만든 레코드가 관리자의 판정을
    /// 대신 결정한다. 검사는 [`preflight`]가 하고, 그 검사가 **막지 못하는
    /// 것**(도출된 ID를 선점한 공격자)도 거기 주석에 적혀 있다.
    ///
    /// [`preflight`]: crate::preflight::preflight
    async fn fetch_provenance(
        &self,
        catalog_id: &str,
    ) -> Result<Vec<ProvenanceRecord>, EffectError>;

    /// 채널을 만든다. 이미 점유된 ID면 `Duplicate`.
    async fn create_channel(&self, spec: ChannelSpec) -> Result<CreateOutcome, EffectError>;

    /// 이 채널의 현재 캔버스 본문. 캔버스가 아직 없으면 `None`.
    ///
    /// saga가 시작 캔버스를 쓰기 **전에** 부른다. 캔버스 단계는 provenance가
    /// 미완료라고 적혀 있으면 다시 실행되는데, 부분 실패로 그 상태가 남는
    /// 것은 정상이고 그 사이 팀이 방을 쓰기 시작하는 것도 정상이다. 캔버스
    /// 쓰기는 되돌릴 수 없으므로, 지켜야 할 내용이 있는지 먼저 묻지 않으면
    /// 재시도 한 번이 팀이 써 둔 내용을 지운다.
    ///
    /// `Ok(None)`은 "채널은 있는데 캔버스가 없다"이지 오류가 아니다. 읽지
    /// 못한 것은 `Err`로만 표현한다 — 둘을 같은 값으로 뭉개면 relay 오류가
    /// "비어 있음"으로 둔갑해 saga가 그대로 덮어쓴다.
    async fn read_canvas(&self, channel_id: Uuid) -> Result<Option<String>, EffectError>;

    /// 시작 캔버스를 적용한다.
    async fn set_canvas(&self, channel_id: Uuid, content: &str) -> Result<(), EffectError>;

    /// 현재 사용자가 이 채널의 owner인가.
    async fn is_owner(&self, channel_id: Uuid) -> Result<bool, EffectError>;

    /// 이 채널의 owner pubkey — [`ProvenanceRecord::signer`]와 같은 인코딩
    /// (소문자 hex)이어야 한다. owner를 알 수 없으면 `None`.
    ///
    /// **인코딩을 반드시 맞춰야 한다.** `preflight`는 이 값을
    /// `ProvenanceRecord::signer`와 정확한 문자열(`==`)로 비교한다 —
    /// 대소문자나 bech32/npub 같은 다른 인코딩으로 돌려주면 실제로는 같은
    /// owner라도 매번 불일치로 보여 그 채널의 모든 증명서가 조용히
    /// 버려진다. 버려진 결과가 `Err`가 아니라 "적용한 적 없음"으로만
    /// 보이므로(§5) 이 실수는 로그에도 남지 않는다.
    ///
    /// `is_owner`와 다르다. 저쪽은 「내가 owner인가」이고 이쪽은 「owner가
    /// 누구인가」다. provenance 검증은 후자를 필요로 한다 — 증명서를 남긴
    /// 사람이 그 채널의 owner였는지 물어야 하기 때문이다.
    ///
    /// `Ok(None)`은 「채널은 있는데 owner를 특정할 수 없다」이지 오류가
    /// 아니다. 그 경우 그 채널의 증명서는 전부 버린다 — 검증할 수 없는 것을
    /// 통과시키지 않는다. `Err`도 같은 방향으로 다룬다 — 호출부(`preflight`)가
    /// 레코드 단위로 버리지 전체를 실패시키지 않는다.
    async fn channel_owner(&self, channel_id: Uuid) -> Result<Option<String>, EffectError>;

    /// provenance 이벤트를 발행한다 (kind 39500).
    async fn publish_provenance(
        &self,
        channel_id: Uuid,
        provenance: &Provenance,
    ) -> Result<(), EffectError>;

    /// 현재 시각 (RFC 3339). 테스트가 고정할 수 있도록 주입한다.
    async fn now_rfc3339(&self) -> String;
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    /// fake에서 「현재 사용자」의 pubkey. 실제 값은 의미 없고, 테스트가
    /// 「나」와 「남」을 구별할 수 있으면 된다.
    pub(crate) const FAKE_ME: &str = "me";

    /// 실패를 주입할 수 있는 인메모리 구현.
    ///
    /// Task 6/7이 이 fake를 실제 테스트에서 쓰기 전까지는 아래 필드 중
    /// 무엇도 값을 갖지 않는다 — 이 task는 경계만 놓는다는 브리프대로다.
    /// clippy가 실제로 dead_code로 잡는 건 이 struct 자체와 바로 아래
    /// inherent impl뿐이다(필드 하나하나가 아니라). 그래서
    /// `#[allow(dead_code)]`를 모듈 전체가 아니라 이 두 곳에만 건다 —
    /// 그러면 나중에 `mod fake`에 추가되는 **다른 최상위 항목**은 계속
    /// dead_code 검사를 받는다.
    ///
    /// 다만 struct에 건 allow는 그 struct의 **모든 필드**에 전파된다. 즉
    /// 여기에 새 필드를 넣고 아무도 읽지 않아도 clippy는 침묵한다(실측 확인).
    /// 필드가 죽었는지는 사람이 봐야 한다.
    #[allow(dead_code)]
    #[derive(Default)]
    pub(crate) struct FakeEffects {
        pub channels: Mutex<Vec<ChannelRef>>,
        /// `(channel_id, signer, provenance)`. 실제 relay에서 provenance
        /// 이벤트는 채널 스코프이고 서명자가 있다. 셋을 분리해 두면 테스트가
        /// "채널은 맞는데 서명자가 다른" 상태를 만들 수 있다 — 그게 선점
        /// 공격의 모양이다.
        pub provenance: Mutex<Vec<(Uuid, String, Provenance)>>,
        /// 이미 점유된 채널 UUID — soft delete된 것 포함.
        pub burned_ids: Mutex<HashSet<Uuid>>,
        pub canvases: Mutex<HashMap<Uuid, String>>,
        /// 성공한 `create_channel` 호출의 append-only 로그 — 요청받은
        /// `ChannelSpec` 전체를 그대로 담는다.
        ///
        /// `channels`는 `ChannelRef`(id + 이름)만 담아서 `visibility`,
        /// `description`, `channel_type`이 사라진다. 그러면 "내장 방은
        /// private으로 만들어진다"는 수용 기준을 어떤 테스트도 검증할 수
        /// 없다. 사용자가 지운 상태를 만들려고 `channels`를 비우는 테스트가
        /// 있으므로 이 로그는 일부러 분리해 둔다 — 여기 남은 기록은
        /// "이 실행이 무엇을 relay에 보냈는가"이지 "지금 무엇이 있는가"가
        /// 아니다.
        pub created: Mutex<Vec<ChannelSpec>>,
        /// 이 이름의 연산을 한 번 실패시킨다. 부작용이 일어나기 **전에**
        /// 걸린다 — 요청이 relay에 닿지도 못한 경우다.
        pub fail_once: Mutex<HashSet<String>>,
        /// 이 이름의 연산을 부작용이 커밋된 **뒤에** 한 번 실패시킨다.
        ///
        /// `fail_once`로는 "relay는 커밋했는데 클라이언트는 실패로 봤다"를
        /// 표현할 수 없다. 그런데 그게 바로 `duplicate` + 접근 가능 상태가
        /// 실제로 만들어지는 경로다 — 응답이 유실되거나 앱이 죽으면 ID는
        /// 탔는데 provenance는 없는 상태로 남는다. 이 상태를 손으로 시드하지
        /// 않고 실제 생성 경로를 통해 만들 수 있어야, 채택 분기가 진짜
        /// 시퀀스에서 검증된다.
        pub fail_after_commit: Mutex<HashSet<String>>,
        /// `(op, nth)` — 그 op의 nth번째(1-based) 호출을 실패시킨다.
        ///
        /// `fail_once`로는 "첫 호출은 되고 두 번째만 실패"를 표현할 수
        /// 없다. saga가 같은 연산을 몇 번 부르는지가 곧 회귀 대상인
        /// 경우(중복 `fetch_provenance`)에 필요하다.
        pub fail_at: Mutex<HashSet<(String, u32)>>,
        /// op 이름별 호출 횟수.
        pub calls: Mutex<HashMap<String, u32>>,
        /// 모든 `publish_provenance` 호출의 append-only 로그.
        ///
        /// `provenance`와 달리 절대 걸러내거나 지우지 않는다 — 몇 번
        /// 발행이 시도됐는지를 테스트가 손실 없이 그대로 assert할 수
        /// 있게 하는 것이 유일한 목적이며, 이건 의도적인 설계지 필터링을
        /// 깜빡한 게 아니다.
        pub published: Mutex<Vec<(Uuid, Provenance)>>,
        /// 현재 사용자가 owner인 채널 UUID 집합.
        ///
        /// `channels`(= 접근 가능한/보이는 채널)와 고의로 분리한다 —
        /// 그래야 "채널은 존재하고 접근도 되지만 내가 owner는 아니다"라는,
        /// `channels` 멤버십만으로는 표현할 수 없는 상태를 테스트가 만들 수
        /// 있다.
        pub owned: Mutex<HashSet<Uuid>>,
        /// 채널별 owner pubkey. `owned`(내가 owner인 채널)와 분리한다 —
        /// 「내가 owner다」와 「owner가 누구다」는 다른 질문이고, 선점 공격은
        /// 그 차이에서 산다.
        pub owners: Mutex<HashMap<Uuid, String>>,
    }

    #[allow(dead_code)]
    impl FakeEffects {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// 다음 `op` 호출을 한 번 실패시킨다.
        pub(crate) fn fail_next(&self, op: &str) {
            self.fail_once.lock().expect("lock").insert(op.to_string());
        }

        /// 다음 `op` 호출을 부작용이 커밋된 **뒤에** 한 번 실패시킨다.
        pub(crate) fn fail_next_after_commit(&self, op: &str) {
            self.fail_after_commit
                .lock()
                .expect("lock")
                .insert(op.to_string());
        }

        /// `op`의 `nth`번째(1-based) 호출을 실패시킨다. 그 앞뒤 호출은
        /// 정상으로 둔다.
        pub(crate) fn fail_nth(&self, op: &str, nth: u32) {
            self.fail_at
                .lock()
                .expect("lock")
                .insert((op.to_string(), nth));
        }

        /// 이번 실행 **이전부터** 그 방에 있던 캔버스 내용을 심는다.
        ///
        /// `set_canvas`를 대신 부르면 `calls`가 올라가 "saga가 쓰지
        /// 않았다"를 호출 횟수로 관측할 수 없게 된다. 그래서 저장소에 직접
        /// 넣는다 — `channels`·`owned`를 직접 채워 이전 실행의 흔적을 만드는
        /// 다른 시딩과 같은 방식이다.
        pub(crate) fn seed_canvas(&self, channel_id: Uuid, content: &str) {
            self.canvases
                .lock()
                .expect("lock")
                .insert(channel_id, content.to_string());
        }

        /// 이전 실행이 남긴 증명서를 심는다. `signer`로 「내가 남긴 것」과
        /// 「남이 남긴 것」을 구별한다.
        pub(crate) fn seed_provenance(
            &self,
            channel_id: Uuid,
            signer: &str,
            provenance: Provenance,
        ) {
            self.provenance.lock().expect("lock").push((
                channel_id,
                signer.to_string(),
                provenance,
            ));
        }

        /// provenance 레코드가 **호출자가 지정한 채널**에 실려 있는 상태를
        /// 만든다. 그 채널도 함께 접근 가능 목록에 넣는다 —
        /// `fetch_provenance`는 살아 있는 채널의 레코드만 돌려주므로 둘을
        /// 따로 심으면 아무 일도 일어나지 않는다.
        ///
        /// 각 테스트 모듈의 `seed_applied*` 헬퍼는 채널 ID를 항상
        /// `derive_channel_id`로 계산해 레코드와 **올바르게** 짝지어 준다.
        /// 그래서 그 헬퍼만으로는 "도출식이 예측하는 채널이 **아닌** 곳에
        /// 실려 있는 레코드"를 표현할 수 없다 — 그런데 그 상태는 실제 relay
        /// 에서 아무 인증 사용자나 만들 수 있다(자기 open 채널을 만들고 거기에
        /// kind 39500을 발행하면 된다). 표현할 수 없는 상태는 테스트되지
        /// 않으므로, 짝을 어긋나게 놓을 수 있는 통로를 여기 하나 둔다.
        ///
        /// 서명자는 항상 `FAKE_ME`가 아닌 고정값("attacker")이다 — 이
        /// 헬퍼가 모델링하는 레코드는 항상 「남의 채널」에 실린 것이라 「내」
        /// 서명일 수 없다.
        pub(crate) fn seed_provenance_in_channel(
            &self,
            channel_id: Uuid,
            channel_name: &str,
            provenance: Provenance,
        ) {
            self.channels.lock().expect("lock").push(ChannelRef {
                id: channel_id,
                name: channel_name.to_string(),
            });
            self.seed_provenance(channel_id, "attacker", provenance);
        }

        /// 이 채널의 owner를 지정한다.
        pub(crate) fn set_channel_owner(&self, channel_id: Uuid, pubkey: &str) {
            self.owners
                .lock()
                .expect("lock")
                .insert(channel_id, pubkey.to_string());
        }

        /// 지금까지 `op`가 호출된 횟수.
        pub(crate) fn call_count(&self, op: &str) -> u32 {
            self.calls
                .lock()
                .expect("lock")
                .get(op)
                .copied()
                .unwrap_or(0)
        }

        fn take_failure(&self, op: &str) -> Result<(), EffectError> {
            let nth = {
                let mut calls = self.calls.lock().expect("lock");
                let count = calls.entry(op.to_string()).or_insert(0);
                *count += 1;
                *count
            };
            if self
                .fail_at
                .lock()
                .expect("lock")
                .remove(&(op.to_string(), nth))
            {
                return Err(EffectError(format!("injected failure: {op} (call #{nth})")));
            }
            let mut guard = self.fail_once.lock().expect("lock");
            if guard.remove(op) {
                return Err(EffectError(format!("injected failure: {op}")));
            }
            Ok(())
        }

        /// 부작용을 이미 적용한 뒤에 호출한다. 호출 횟수는 `take_failure`가
        /// 이미 셌으므로 여기서 다시 세지 않는다.
        fn take_post_commit_failure(&self, op: &str) -> Result<(), EffectError> {
            if self.fail_after_commit.lock().expect("lock").remove(op) {
                return Err(EffectError(format!("injected failure after commit: {op}")));
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl CatalogEffects for FakeEffects {
        async fn relay_scope(&self) -> String {
            "wss://relay.test".into()
        }

        async fn list_channels(&self) -> Result<Vec<ChannelRef>, EffectError> {
            self.take_failure("list_channels")?;
            Ok(self.channels.lock().expect("lock").clone())
        }

        // 채널-존재 필터가 채널 스코프 저장을 모델링한다: provenance는 항상
        // 어떤 채널에 실려서 저장되므로, 그 채널이 `channels`에서 사라지면
        // (soft delete) provenance도 자동으로 읽을 수 없게 되어야 한다.
        // 이건 최적화가 아니라 의도된 동작이다 — 실제 relay에서 채널 스코프
        // 이벤트는 채널이 삭제되면 함께 조회 불가능해진다.
        async fn fetch_provenance(
            &self,
            catalog_id: &str,
        ) -> Result<Vec<ProvenanceRecord>, EffectError> {
            self.take_failure("fetch_provenance")?;
            let live_channels: HashSet<Uuid> = self
                .channels
                .lock()
                .expect("lock")
                .iter()
                .map(|c| c.id)
                .collect();
            // 여기서 채널과 서명자를 **버리지 않는다.** 예전에는 `(_, p)`로
            // 벗겨서 레코드만 돌려줬는데, 그러면 이 fake만 채널 결합을 알고
            // 실제 어댑터는 알 수 없는 상태가 된다 — 실제로 그렇게 벌어졌고,
            // 그 틈이 곧 남의 채널에 발행한 레코드가 판정을 가로채는 경로였다.
            // 어느 레코드가 유효한가는 `preflight`가 도출식과 서명자로 판정한다.
            Ok(self
                .provenance
                .lock()
                .expect("lock")
                .iter()
                .filter(|(channel_id, _, p)| {
                    p.catalog_id == catalog_id && live_channels.contains(channel_id)
                })
                .map(|(channel_id, signer, provenance)| ProvenanceRecord {
                    channel_id: *channel_id,
                    signer: signer.clone(),
                    provenance: provenance.clone(),
                })
                .collect())
        }

        async fn create_channel(&self, spec: ChannelSpec) -> Result<CreateOutcome, EffectError> {
            self.take_failure("create_channel")?;
            if !self.burned_ids.lock().expect("lock").insert(spec.id) {
                return Ok(CreateOutcome::Duplicate);
            }
            self.channels.lock().expect("lock").push(ChannelRef {
                id: spec.id,
                name: spec.name.clone(),
            });
            self.owned.lock().expect("lock").insert(spec.id);
            self.owners
                .lock()
                .expect("lock")
                .insert(spec.id, FAKE_ME.to_string());
            self.created.lock().expect("lock").push(spec);
            // relay는 커밋했는데 호출자는 오류를 본다. ID는 탄 채로 남고
            // 채널도 남는다 — 재시도가 `Duplicate` + 접근 가능을 만난다.
            self.take_post_commit_failure("create_channel")?;
            Ok(CreateOutcome::Created)
        }

        // `canvases`에 항목이 없는 것이 곧 "캔버스가 아직 없다"이다 —
        // 실제 relay에서 kind 40100 이벤트가 하나도 없는 상태에 대응한다.
        // 빈 문자열로 뭉개지 않는다: 그러면 "캔버스 이벤트가 없다"와 "본문이
        // 빈 캔버스 이벤트가 있다"가 fake에서 구별되지 않아, saga가 그 둘을
        // 어떻게 다루는지 테스트가 관측할 수 없다.
        async fn read_canvas(&self, channel_id: Uuid) -> Result<Option<String>, EffectError> {
            self.take_failure("read_canvas")?;
            Ok(self
                .canvases
                .lock()
                .expect("lock")
                .get(&channel_id)
                .cloned())
        }

        async fn set_canvas(&self, channel_id: Uuid, content: &str) -> Result<(), EffectError> {
            self.take_failure("set_canvas")?;
            self.canvases
                .lock()
                .expect("lock")
                .insert(channel_id, content.to_string());
            Ok(())
        }

        async fn is_owner(&self, channel_id: Uuid) -> Result<bool, EffectError> {
            self.take_failure("is_owner")?;
            Ok(self.owned.lock().expect("lock").contains(&channel_id))
        }

        async fn channel_owner(&self, channel_id: Uuid) -> Result<Option<String>, EffectError> {
            self.take_failure("channel_owner")?;
            Ok(self.owners.lock().expect("lock").get(&channel_id).cloned())
        }

        async fn publish_provenance(
            &self,
            channel_id: Uuid,
            provenance: &Provenance,
        ) -> Result<(), EffectError> {
            self.take_failure("publish_provenance")?;
            self.published
                .lock()
                .expect("lock")
                .push((channel_id, provenance.clone()));
            // NIP-33 LWW: 같은 d 태그는 교체된다 — retain-then-push라 같은
            // d_tag로 재발행해도 정확히 한 항목만 남고, 그 값은 새 값이다.
            // 서명자는 항상 fake의 「나」다 — 발행은 언제나 적용자 본인이
            // 하는 행동이다.
            let mut store = self.provenance.lock().expect("lock");
            store.retain(|(_, _, p)| p.d_tag() != provenance.d_tag());
            store.push((channel_id, FAKE_ME.to_string(), provenance.clone()));
            Ok(())
        }

        async fn now_rfc3339(&self) -> String {
            "2026-07-28T09:00:00Z".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_record_carries_channel_and_signer() {
        let record = ProvenanceRecord {
            channel_id: Uuid::nil(),
            signer: "abc123".into(),
            provenance: Provenance {
                catalog_id: "schoolx.default".into(),
                catalog_version: 1,
                item_key: "meeting".into(),
                generation: 1,
                steps: crate::provenance::StepStates::default(),
                applied_at: "2026-08-01T00:00:00Z".into(),
            },
        };
        assert_eq!(record.channel_id, Uuid::nil());
        assert_eq!(record.signer, "abc123");
        assert_eq!(record.provenance.item_key, "meeting");
    }
}
