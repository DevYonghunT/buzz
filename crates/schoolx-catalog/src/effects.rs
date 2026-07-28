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
    async fn fetch_provenance(&self, catalog_id: &str) -> Result<Vec<Provenance>, EffectError>;

    /// 채널을 만든다. 이미 점유된 ID면 `Duplicate`.
    async fn create_channel(&self, spec: ChannelSpec) -> Result<CreateOutcome, EffectError>;

    /// 시작 캔버스를 적용한다.
    async fn set_canvas(&self, channel_id: Uuid, content: &str) -> Result<(), EffectError>;

    /// 현재 사용자가 이 채널의 owner인가.
    async fn is_owner(&self, channel_id: Uuid) -> Result<bool, EffectError>;

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
        /// `(channel_id, provenance)` 쌍. 실제 relay에서 provenance
        /// 이벤트는 채널 스코프다 — 자기 채널에 실려서 저장되고, 그 채널이
        /// soft delete되면 함께 읽을 수 없게 된다. channel_id 없이는 이
        /// 상태를 표현할 수조차 없어야 하므로, `channels`와의 연결을 필드
        /// 타입 자체에 새긴다. 가시성 필터링은 `fetch_provenance`가 한다.
        pub provenance: Mutex<Vec<(Uuid, Provenance)>>,
        /// 이미 점유된 채널 UUID — soft delete된 것 포함.
        pub burned_ids: Mutex<HashSet<Uuid>>,
        pub canvases: Mutex<HashMap<Uuid, String>>,
        /// 이 이름의 연산을 한 번 실패시킨다.
        pub fail_once: Mutex<HashSet<String>>,
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

        fn take_failure(&self, op: &str) -> Result<(), EffectError> {
            let mut guard = self.fail_once.lock().expect("lock");
            if guard.remove(op) {
                return Err(EffectError(format!("injected failure: {op}")));
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
        async fn fetch_provenance(&self, catalog_id: &str) -> Result<Vec<Provenance>, EffectError> {
            self.take_failure("fetch_provenance")?;
            let live_channels: HashSet<Uuid> = self
                .channels
                .lock()
                .expect("lock")
                .iter()
                .map(|c| c.id)
                .collect();
            Ok(self
                .provenance
                .lock()
                .expect("lock")
                .iter()
                .filter(|(channel_id, p)| {
                    p.catalog_id == catalog_id && live_channels.contains(channel_id)
                })
                .map(|(_, p)| p.clone())
                .collect())
        }

        async fn create_channel(&self, spec: ChannelSpec) -> Result<CreateOutcome, EffectError> {
            self.take_failure("create_channel")?;
            if !self.burned_ids.lock().expect("lock").insert(spec.id) {
                return Ok(CreateOutcome::Duplicate);
            }
            self.channels.lock().expect("lock").push(ChannelRef {
                id: spec.id,
                name: spec.name,
            });
            self.owned.lock().expect("lock").insert(spec.id);
            Ok(CreateOutcome::Created)
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
            let mut store = self.provenance.lock().expect("lock");
            store.retain(|(_, p)| p.d_tag() != provenance.d_tag());
            store.push((channel_id, provenance.clone()));
            Ok(())
        }

        async fn now_rfc3339(&self) -> String {
            "2026-07-28T09:00:00Z".into()
        }
    }
}
