//! 적용 실행의 machine-readable 결과. UI와 CLI가 같은 것을 읽는다.

use crate::preflight::Decision;
use crate::provenance::StepStates;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 항목 하나의 최종 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// 이번 실행에서 무언가 바꿨고 전 단계가 끝났다.
    Applied,
    /// 이미 끝나 있어 아무것도 하지 않았다.
    Unchanged,
    /// 일부 단계가 실패했다. 재시도가 이어서 한다.
    Partial,
    /// 사용자 조치 없이는 진행할 수 없다.
    Blocked,
}

/// 사람이 결정해야 하는 것.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserAction {
    /// 예전에 만들었다가 삭제된 항목이다. 다시 만들지 물어본다.
    ConfirmRecreate,
    /// provenance 없는 동명 채널이 있다. 어떻게 할지 물어본다.
    ResolveConflict,
}

/// 항목 하나의 결과.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerItem {
    /// catalog 항목 키.
    pub item_key: String,
    /// preflight 판정을 문자열로 남긴 것.
    pub decision: String,
    /// 관련 채널 ID.
    pub channel_id: Option<Uuid>,
    /// 쓰인 세대.
    pub generation: u32,
    /// 단계별 최종 상태.
    pub steps: StepStates,
    /// 최종 상태.
    pub outcome: Outcome,
    /// 필요한 사용자 조치.
    pub user_action: Option<UserAction>,
    /// 실패 사유.
    pub error: Option<String>,
}

/// 적용 실행 하나의 결과 전체.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    /// 적용에 쓰인 catalog.
    pub catalog_id: String,
    /// 적용에 쓰인 catalog 버전.
    pub catalog_version: u32,
    /// 항목별 결과.
    pub items: Vec<LedgerItem>,
}

/// `LedgerItem::decision`의 여섯 번째 값. `Decision`에는 없다 — 생성이
/// `duplicate`로 거부되고 그 채널이 접근 불가일 때 saga만 만들어 낸다.
///
/// saga가 문자열 리터럴을 직접 쓰지 않고 이 상수를 쓰기 때문에, ledger의
/// `decision` 어휘 전체가 이 모듈 한 곳에 모인다.
pub const DELETED_DECISION: &str = "deleted";

/// `Decision`의 안정적인 문자열 표현. UI와 CLI가 이 값을 읽는다.
pub fn decision_label(decision: Decision) -> &'static str {
    match decision {
        Decision::CreateOrRecreate => "create_or_recreate",
        Decision::Resume => "resume",
        Decision::NoChange => "no_change",
        Decision::Conflict => "conflict",
        Decision::Retired => "retired",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_DECISIONS: [Decision; 5] = [
        Decision::CreateOrRecreate,
        Decision::Resume,
        Decision::NoChange,
        Decision::Conflict,
        Decision::Retired,
    ];

    /// `decision_label`은 `Decision`의 serde derive가 이미 만들어 내는
    /// 문자열을 손으로 한 번 더 쓴 것이다. 두 표현은 같은 값을 서로 다른
    /// 소비자에게 보낸다 — ledger의 `decision` 필드는 `decision_label`을
    /// 거치고, Task 8의 Tauri command는 `Decision`을 그대로 반환해 serde
    /// 철자를 내보낸다. 둘이 어긋나면 같은 판정이 미리보기에서는 한 철자로,
    /// 결과에서는 다른 철자로 보인다. 이 테스트만이 그 둘을 묶어 둔다.
    #[test]
    fn decision_label_matches_serde_spelling() {
        for decision in ALL_DECISIONS {
            let serde_spelling = serde_json::to_value(decision).expect("serialize");
            assert_eq!(
                serde_spelling,
                serde_json::json!(decision_label(decision)),
                "{decision:?}의 serde 철자와 decision_label이 어긋났다 — \
                 미리보기(Decision)와 ledger(decision_label)가 다른 값을 낸다"
            );
        }
    }

    /// `LedgerItem::decision`이 가질 수 있는 값 전부를 한 곳에 고정한다.
    ///
    /// 다섯 개는 `Decision`에서, 여섯 번째(`deleted`)는 saga에서 온다.
    /// UI와 CLI가 이 문자열로 분기하므로 철자를 바꾸는 것은 wire format
    /// 변경이다. 실패를 없애려고 이 리터럴을 고치지 말 것.
    #[test]
    fn ledger_decision_vocabulary_is_pinned() {
        let mut vocabulary: Vec<&str> = ALL_DECISIONS.iter().map(|d| decision_label(*d)).collect();
        vocabulary.push(DELETED_DECISION);
        assert_eq!(
            vocabulary,
            vec![
                "create_or_recreate",
                "resume",
                "no_change",
                "conflict",
                "retired",
                "deleted",
            ]
        );
    }
}
