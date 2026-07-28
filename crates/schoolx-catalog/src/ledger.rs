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
