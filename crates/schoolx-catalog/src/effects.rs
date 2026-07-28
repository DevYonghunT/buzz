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
// Task 6/7가 이 fake를 실제 테스트에서 쓰기 전까지는 아무것도 호출되지
// 않는다 — 이 task는 경계만 놓는다는 브리프대로다. `-D warnings` 아래
// dead_code를 허용해 두면 이후 task가 이 struct를 쓰기 시작하는 순간
// 자연히 무해해진다.
#[allow(dead_code)]
pub(crate) mod fake {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    /// 실패를 주입할 수 있는 인메모리 구현.
    #[derive(Default)]
    pub(crate) struct FakeEffects {
        pub channels: Mutex<Vec<ChannelRef>>,
        pub provenance: Mutex<Vec<Provenance>>,
        /// 이미 점유된 채널 UUID — soft delete된 것 포함.
        pub burned_ids: Mutex<HashSet<Uuid>>,
        pub canvases: Mutex<HashMap<Uuid, String>>,
        /// 이 이름의 연산을 한 번 실패시킨다.
        pub fail_once: Mutex<HashSet<String>>,
        pub published: Mutex<Vec<(Uuid, Provenance)>>,
    }

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

        async fn fetch_provenance(&self, catalog_id: &str) -> Result<Vec<Provenance>, EffectError> {
            self.take_failure("fetch_provenance")?;
            Ok(self
                .provenance
                .lock()
                .expect("lock")
                .iter()
                .filter(|p| p.catalog_id == catalog_id)
                .cloned()
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
            Ok(self
                .channels
                .lock()
                .expect("lock")
                .iter()
                .any(|c| c.id == channel_id))
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
            // NIP-33 LWW: 같은 d 태그는 교체된다.
            let mut store = self.provenance.lock().expect("lock");
            store.retain(|p| p.d_tag() != provenance.d_tag());
            store.push(provenance.clone());
            Ok(())
        }

        async fn now_rfc3339(&self) -> String {
            "2026-07-28T09:00:00Z".into()
        }
    }
}
