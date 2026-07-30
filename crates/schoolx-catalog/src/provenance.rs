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
    /// 실행하지 않기로 했다 — 그 자리에 이미 지켜야 할 사용자 내용이 있었다.
    ///
    /// `Done`과 반드시 구별한다. 둘 다 "이 단계는 끝났다"이지만 `Done`은
    /// catalog 값이 그 방에 들어가 있다는 뜻이고 `Skipped`는 들어가 있지
    /// **않다**는 뜻이다. 같은 값으로 적으면 ledger가 하지 않은 쓰기를
    /// 보고하고, 사용자는 자기 내용이 남았다는 사실을 읽을 방법이 없다.
    ///
    /// 지금은 캔버스 단계만 이 값을 낸다.
    Skipped,
}

impl StepStatus {
    /// 이 단계가 끝났는가 — 재시도가 다시 실행하지 않는다.
    ///
    /// `Done`과 `Skipped` 둘 다다. `Skipped`를 미완료로 세면 그 항목은 영원히
    /// `partial`로 보고되고, 매 실행이 같은 결론(쓰지 않는다)에 다시 도달한다
    /// — 사용자에게는 끝나지 않는 실패로 보인다.
    pub fn is_settled(self) -> bool {
        matches!(self, Self::Done | Self::Skipped)
    }
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

    /// 세 단계가 모두 끝났는가.
    ///
    /// `Skipped`도 끝난 것으로 센다 — 이유는 [`StepStatus::is_settled`].
    pub fn is_complete(&self) -> bool {
        self.steps.channel.is_settled()
            && self.steps.canvas.is_settled()
            && self.steps.membership.is_settled()
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

    // Compile-time: catches a value-only drift between this crate's
    // KIND_WORKSPACE_PROVENANCE and buzz-core's independently declared copy.
    // `just clippy` compiles this test target, so a deleted or renamed
    // constant already fails the build — but changing one constant's
    // *value* alone still compiles fine and would only be caught by the
    // runtime test below, which needs `cargo test -p schoolx-catalog` to
    // actually run. This assert runs on every build that compiles the test
    // target, closing that gap.
    const _: () = assert!(KIND_WORKSPACE_PROVENANCE == buzz_core::kind::KIND_WORKSPACE_PROVENANCE);

    #[test]
    fn kind_matches_buzz_core_declaration() {
        // buzz-core는 같은 kind 번호를 `crates/buzz-core/src/kind.rs`에
        // 독립적으로 선언한다 — buzz-core가 schoolx-catalog에 의존할 수
        // 없으므로 두 상수는 단일 소스를 공유하지 못하고 일부러 중복된다.
        // 이 테스트가 그 중복이 조용히 벌어지는 것을 막는 유일한 장치다.
        // 둘 중 하나만 바뀌면 relay의 scope/h-tag 처리(buzz-relay)와 이
        // 크레이트의 provenance 인코딩이 서로 다른 kind 번호를 쓰게 되어,
        // provenance 발행이 relay에서 전부 거부되거나 잘못 라우팅된다.
        // buzz-core는 이 assert만을 위한 dev-dependency이며, 프로덕션
        // 의존성 그래프에 순환을 만들지 않는다.
        assert_eq!(
            KIND_WORKSPACE_PROVENANCE,
            buzz_core::kind::KIND_WORKSPACE_PROVENANCE
        );
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

    /// 내용이 있어 건너뛴 캔버스 단계는 **끝난** 것이다.
    ///
    /// 미완료로 세면 그 항목은 매 실행마다 `resume`으로 다시 들어와 영원히
    /// `partial`을 보고한다 — 재시도가 도달할 수 있는 결론은 "쓰지 않는다"
    /// 하나뿐인데도.
    #[test]
    fn skipped_canvas_counts_as_complete() {
        let mut p = sample();
        p.steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Skipped,
            membership: StepStatus::Done,
        };
        assert!(p.is_complete());
    }

    #[test]
    fn only_done_and_skipped_are_settled() {
        assert!(StepStatus::Done.is_settled());
        assert!(StepStatus::Skipped.is_settled());
        assert!(!StepStatus::Pending.is_settled());
        assert!(!StepStatus::Failed.is_settled());
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

    /// `golden_json_matches_known_wire_format` 위에 얹는 짝. 그쪽 리터럴은
    /// `done`·`failed`·`pending` 셋만 지나가므로 `skipped` 철자는 어디에도
    /// 고정되지 않는다 — 그런데 이 값도 relay에 실려 나가고 다음 실행이 다시
    /// 읽는다. 철자가 바뀌면 이미 발행된 `skipped` 증명서가 파싱되지 않고,
    /// 그 항목은 "적용한 적 없음"으로 보여 지켜 둔 사용자 캔버스 위로 시작
    /// 캔버스가 다시 내려간다. 위 golden과 같은 규칙이다 — 실패를 없애려고
    /// 이 리터럴을 고치지 말 것.
    #[test]
    fn golden_json_pins_the_skipped_spelling() {
        let mut p = sample();
        p.steps = StepStates {
            channel: StepStatus::Done,
            canvas: StepStatus::Skipped,
            membership: StepStatus::Done,
        };

        let actual = serde_json::to_value(&p).expect("serialize to Value");
        assert_eq!(
            actual["steps"],
            serde_json::json!({
                "channel": "done",
                "canvas": "skipped",
                "membership": "done"
            }),
            "kind 39500의 `skipped` 철자가 바뀌었다 — 이미 발행된 증명서가 \
             파싱되지 않고 지켜 둔 캔버스가 다시 덮어써진다"
        );

        // 되읽기까지 확인한다. 직렬화만 보면 `Deserialize` 쪽이 이 철자를
        // 받지 못해도 통과한다 — 그런데 이 값을 실제로 읽는 것은 **다음
        // 실행의 preflight**다.
        let back: Provenance = serde_json::from_value(actual).expect("skipped가 되읽어져야 한다");
        assert_eq!(back.steps.canvas, StepStatus::Skipped);
    }
}
