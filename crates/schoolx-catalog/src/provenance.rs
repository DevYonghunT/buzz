//! kind 39500 — 워크스페이스 template provenance manifest.
//!
//! 채널 스코프 addressable 이벤트다. `d` 태그가 `<catalog_id>:<item_key>`라
//! 항목당 정확히 하나이고, NIP-33 LWW가 적용되어 재시도해도 이벤트가 쌓이지
//! 않는다. `h` 태그가 채널 ID라 private 채널 ACL이 그대로 걸린다.
//!
//! relay가 kind 39000을 DB 컬럼에서만 재구성하므로 채널 생성 이벤트에 실은
//! provenance 태그는 보존되지 않는다. 그래서 별도 이벤트가 필요하다.

use serde::{Deserialize, Serialize};

/// 워크스페이스 template provenance manifest.
///
/// SchoolX 예약 대역 39500–39599의 첫 번째 kind. 예약 대역을 두는 이유는 SQL
/// 마이그레이션 `9001+`와 같다 — upstream이 같은 번호를 쓰면 조용히 충돌하고,
/// 충돌은 컴파일 타임에 잡히지 않는다.
pub const KIND_WORKSPACE_PROVENANCE: u32 = 39500;

/// saga 단계 하나의 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    /// 아직 실행되지 않았다.
    Pending,
    /// 성공했다. 재시도는 이 단계를 건너뛴다.
    Done,
    /// 실행했고 실패했다. 재시도가 다시 시도한다.
    Failed,
}

/// 세 단계의 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepStates {
    /// 채널 생성.
    pub channel: StepStatus,
    /// 시작 캔버스 적용.
    pub canvas: StepStatus,
    /// 적용자가 owner로 들어갔는지 확인.
    pub membership: StepStatus,
}

impl Default for StepStates {
    fn default() -> Self {
        Self {
            channel: StepStatus::Pending,
            canvas: StepStatus::Pending,
            membership: StepStatus::Pending,
        }
    }
}

/// kind 39500 이벤트의 content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// 적용에 쓰인 catalog의 안정 식별자.
    pub catalog_id: String,
    /// 적용에 쓰인 catalog 버전.
    pub catalog_version: u32,
    /// 적용된 항목의 안정 키.
    pub item_key: String,
    /// 채널 ID 도출에 쓰인 세대. 명시적 재생성에서만 증가한다.
    pub generation: u32,
    /// 단계별 상태.
    pub steps: StepStates,
    /// 마지막 갱신 시각 (RFC 3339).
    pub applied_at: String,
}

impl Provenance {
    /// 이 항목의 addressable `d` 태그 값.
    pub fn d_tag(&self) -> String {
        d_tag(&self.catalog_id, &self.item_key)
    }

    /// 세 단계가 모두 `Done`인가.
    pub fn is_complete(&self) -> bool {
        self.steps.channel == StepStatus::Done
            && self.steps.canvas == StepStatus::Done
            && self.steps.membership == StepStatus::Done
    }
}

/// `<catalog_id>:<item_key>` — addressable `d` 태그 값.
pub fn d_tag(catalog_id: &str, item_key: &str) -> String {
    format!("{catalog_id}:{item_key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Provenance {
        Provenance {
            catalog_id: "schoolx.default".into(),
            catalog_version: 1,
            item_key: "meeting".into(),
            generation: 1,
            steps: StepStates::default(),
            applied_at: "2026-07-28T09:00:00Z".into(),
        }
    }

    #[test]
    fn kind_is_in_the_reserved_schoolx_band() {
        assert!((39500..=39599).contains(&KIND_WORKSPACE_PROVENANCE));
    }

    #[test]
    fn kind_is_addressable() {
        // NIP-33 parameterized replaceable 범위여야 LWW가 적용된다.
        assert!((30000..=39999).contains(&KIND_WORKSPACE_PROVENANCE));
    }

    #[test]
    fn d_tag_pairs_catalog_and_item() {
        assert_eq!(sample().d_tag(), "schoolx.default:meeting");
    }

    #[test]
    fn fresh_provenance_is_not_complete() {
        assert!(!sample().is_complete());
    }

    #[test]
    fn all_done_is_complete() {
        let mut p = sample();
        p.steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Done,
            membership: StepStatus::Done,
        };
        assert!(p.is_complete());
    }

    #[test]
    fn partial_steps_are_not_complete() {
        let mut p = sample();
        p.steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };
        assert!(!p.is_complete());
    }

    #[test]
    fn round_trips_through_json() {
        let p = sample();
        let json = serde_json::to_string(&p).expect("serialize");
        let back: Provenance = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, back);
    }

    /// Golden-value regression test. `round_trips_through_json` above is
    /// symmetric — it serializes then deserializes and compares the Rust
    /// values, so it stays green even if a field were renamed via
    /// `#[serde(rename = "...")]` or `StepStatus`'s
    /// `#[serde(rename_all = "lowercase")]` were changed to another casing.
    /// This test pins the actual JSON bytes instead: the exact field names
    /// and the exact enum spellings, matching the kind 39500 content shape
    /// documented in `docs/schoolx-2/WORKSPACE_CATALOG.md` §4.
    ///
    /// Changing this expected literal is a breaking wire-format change: it
    /// orphans every already-published provenance event, which relies on
    /// these exact key names and enum spellings to be parsed by later app
    /// versions. Do not update this literal to make a failing test pass —
    /// treat a failure here as a regression to fix, not a value to re-pin.
    #[test]
    fn golden_json_matches_known_wire_format() {
        let mut p = sample();
        p.steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Failed,
            membership: StepStatus::Pending,
        };

        let actual = serde_json::to_value(&p).expect("serialize to Value");
        let expected = serde_json::json!({
            "catalog_id": "schoolx.default",
            "catalog_version": 1,
            "item_key": "meeting",
            "generation": 1,
            "steps": {
                "channel": "done",
                "canvas": "failed",
                "membership": "pending"
            },
            "applied_at": "2026-07-28T09:00:00Z"
        });

        assert_eq!(
            actual, expected,
            "kind 39500 wire format changed (field names or enum spellings) — \
             this orphans already-published provenance events, see doc comment above"
        );
    }
}
