use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::bindings::CodeThreadBindingScope;
use super::protocol::{redact_protocol_value, validate_id, CodeRequestId, CodeWorkspaceEventDraft};

mod permission_display;

use permission_display::permission_display_from_raw;

const MAX_PENDING_APPROVALS: usize = 128;
const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;

/// Decisions supported by stable command and file-change approval responses.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeApprovalDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

/// Lifetime of an explicitly granted permission request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodePermissionScope {
    Turn,
    Session,
}

/// Opaque permission response intent. Requested permissions never round-trip
/// through the frontend as authority-bearing response data.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodePermissionIntent {
    Grant,
    Decline,
}

/// Normalized frontend response for either a decision or permission intent.
#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CodeApprovalResponse {
    Decision {
        decision: CodeApprovalDecision,
    },
    Permissions {
        intent: CodePermissionIntent,
        scope: CodePermissionScope,
    },
}

impl CodeApprovalResponse {
    /// Whether this response would grant the requested operation rather than
    /// decline or cancel it.
    pub(crate) fn approves_execution(&self) -> bool {
        match self {
            Self::Decision {
                decision: CodeApprovalDecision::Accept | CodeApprovalDecision::AcceptForSession,
            } => true,
            Self::Permissions {
                intent: CodePermissionIntent::Grant,
                ..
            } => true,
            Self::Decision { .. } | Self::Permissions { .. } => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionDisplay {
    grantable: bool,
    network: Option<PermissionNetworkDisplay>,
    file_system: Option<PermissionFileSystemDisplay>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionNetworkDisplay {
    enabled: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionFileSystemDisplay {
    entries: Option<Vec<PermissionFileSystemEntryDisplay>>,
    glob_scan_max_depth: Option<u64>,
    read: Option<Vec<String>>,
    write: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionFileSystemEntryDisplay {
    access: PermissionAccessDisplay,
    path: PermissionPathDisplay,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum PermissionAccessDisplay {
    Read,
    Write,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum PermissionPathDisplay {
    Path { path: String },
    GlobPattern { pattern: String },
    Special { value: PermissionSpecialPathDisplay },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PermissionSpecialPathDisplay {
    Root,
    Minimal,
    ProjectRoots {
        subpath: Option<String>,
    },
    Tmpdir,
    SlashTmp,
    Unknown {
        path: String,
        subpath: Option<String>,
    },
}

/// Identity-bound response to one pending app-server approval request.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeApprovalResponseInput {
    pub runtime_generation: u64,
    pub request_id: CodeRequestId,
    /// Persisted community/project/repository scope that owns the thread.
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub turn_id: String,
    pub response: CodeApprovalResponse,
}

impl CodeApprovalResponseInput {
    /// Whether the response can authorize filesystem or process execution.
    pub(crate) fn approves_execution(&self) -> bool {
        self.response.approves_execution()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum ApprovalKind {
    CommandExecution,
    FileChange,
    Permissions,
}

#[derive(Clone, Debug)]
struct PendingApproval {
    generation: u64,
    request_id: CodeRequestId,
    thread_id: String,
    turn_id: String,
    item_id: String,
    method: String,
    kind: ApprovalKind,
    params: Value,
    permission_display: Option<PermissionDisplay>,
}

impl PendingApproval {
    fn from_request(
        generation: u64,
        request_id: Value,
        method: &str,
        params: Option<Value>,
    ) -> Result<Option<Self>, String> {
        let kind = match method {
            "item/commandExecution/requestApproval" => ApprovalKind::CommandExecution,
            "item/fileChange/requestApproval" => ApprovalKind::FileChange,
            "item/permissions/requestApproval" => ApprovalKind::Permissions,
            _ => return Ok(None),
        };
        let request_id = CodeRequestId::from_value(request_id)?;
        let params = params.ok_or_else(|| format!("Codex `{method}` request has no params"))?;
        let thread_id = required_string(&params, "threadId", method)?;
        let turn_id = required_string(&params, "turnId", method)?;
        let item_id = required_string(&params, "itemId", method)?;
        let permission_display = (kind == ApprovalKind::Permissions)
            .then(|| permission_display_from_raw(params.get("permissions")));
        Ok(Some(Self {
            generation,
            request_id,
            thread_id,
            turn_id,
            item_id,
            method: method.to_string(),
            kind,
            params,
            permission_display,
        }))
    }

    fn event(&self) -> Result<CodeWorkspaceEventDraft, String> {
        let approval_kind = serde_json::to_value(self.kind)
            .map_err(|error| format!("failed to encode Codex approval kind: {error}"))?;
        Ok(CodeWorkspaceEventDraft {
            thread_id: Some(self.thread_id.clone()),
            turn_id: Some(self.turn_id.clone()),
            item_id: Some(self.item_id.clone()),
            kind: self.method.clone(),
            payload: json!({
                "requestId": self.request_id,
                "approvalKind": approval_kind,
                "request": self.public_request()?
            }),
        })
    }

    fn public_request(&self) -> Result<Value, String> {
        if self.kind != ApprovalKind::Permissions {
            return Ok(redact_protocol_value(self.params.clone()));
        }
        let display = self.permission_display.as_ref().ok_or_else(|| {
            "pending Codex permission request has no display snapshot".to_string()
        })?;
        let mut request = Map::new();
        request.insert("threadId".to_string(), json!(self.thread_id));
        request.insert("turnId".to_string(), json!(self.turn_id));
        request.insert("itemId".to_string(), json!(self.item_id));
        for key in ["startedAtMs", "cwd", "environmentId", "reason"] {
            if let Some(value) = self.params.get(key) {
                request.insert(key.to_string(), redact_protocol_value(value.clone()));
            }
        }
        request.insert(
            "permissionDisplay".to_string(),
            serde_json::to_value(display)
                .map_err(|error| format!("failed to encode permission display: {error}"))?,
        );
        Ok(Value::Object(request))
    }

    fn response_value(&self, response: &CodeApprovalResponse) -> Result<Value, String> {
        match (self.kind, response) {
            (
                ApprovalKind::CommandExecution | ApprovalKind::FileChange,
                CodeApprovalResponse::Decision { decision },
            ) => {
                self.validate_available_decision(*decision)?;
                Ok(json!({ "decision": decision }))
            }
            (ApprovalKind::Permissions, CodeApprovalResponse::Permissions { intent, scope }) => {
                match intent {
                    CodePermissionIntent::Decline => Ok(json!({
                        "permissions": {},
                        "scope": CodePermissionScope::Turn,
                        "strictAutoReview": false
                    })),
                    CodePermissionIntent::Grant => {
                        if !self
                            .permission_display
                            .as_ref()
                            .is_some_and(|display| display.grantable)
                        {
                            return Err(
                            "Codex permission request cannot be granted because its display is incomplete or inaccurate"
                                .to_string(),
                        );
                        }
                        let requested =
                            self.params.get("permissions").cloned().ok_or_else(|| {
                                "pending Codex permission request has no permissions".to_string()
                            })?;
                        Ok(json!({
                            "permissions": requested,
                            "scope": scope,
                            "strictAutoReview": *scope == CodePermissionScope::Turn
                        }))
                    }
                }
            }
            (ApprovalKind::Permissions, CodeApprovalResponse::Decision { .. }) => {
                Err("permission approvals require an explicit grant or decline intent".to_string())
            }
            (_, CodeApprovalResponse::Permissions { .. }) => {
                Err("only permission requests accept a permission grant".to_string())
            }
        }
    }

    fn validate_available_decision(&self, decision: CodeApprovalDecision) -> Result<(), String> {
        let Some(available) = self.params.get("availableDecisions") else {
            return Ok(());
        };
        let available = available
            .as_array()
            .ok_or_else(|| "pending Codex approval has invalid available decisions".to_string())?;
        let decision = serde_json::to_value(decision)
            .map_err(|error| format!("failed to encode Codex approval decision: {error}"))?;
        if available.iter().any(|candidate| candidate == &decision) {
            Ok(())
        } else {
            Err("selected Codex approval decision is not available".to_string())
        }
    }
}

#[derive(Default)]
struct ApprovalState {
    generation: u64,
    next_reservation_id: u64,
    pending: HashMap<CodeRequestId, PendingApproval>,
    reserved: HashMap<CodeRequestId, ReservedApproval>,
}

struct ReservedApproval {
    reservation_id: u64,
    approval: PendingApproval,
}

/// Validated wire response reserved for one in-flight app-server write.
pub(crate) struct ApprovalResponseReservation {
    generation: u64,
    request_id: CodeRequestId,
    reservation_id: u64,
    wire_request_id: Value,
    wire_result: Value,
}

impl ApprovalResponseReservation {
    pub(crate) fn wire_response(&self) -> (Value, Value) {
        (self.wire_request_id.clone(), self.wire_result.clone())
    }
}

#[derive(Default)]
pub(crate) struct PendingApprovalStore {
    inner: Mutex<ApprovalState>,
}

/// Held across one guarded lifecycle write so an approval cannot be inserted
/// or reserved between the exact-thread check and JSON-RPC byte admission.
pub(crate) struct PendingApprovalAdmissionGuard<'a> {
    _state: MutexGuard<'a, ApprovalState>,
}

impl PendingApprovalStore {
    pub(crate) fn reset(&self, generation: u64) {
        if let Ok(mut state) = self.inner.lock() {
            state.generation = generation;
            state.next_reservation_id = 0;
            state.pending.clear();
            state.reserved.clear();
        }
    }

    pub(crate) fn clear_generation(&self, generation: u64) {
        if let Ok(mut state) = self.inner.lock() {
            if state.generation == generation {
                state.pending.clear();
                state.reserved.clear();
            }
        }
    }

    pub(crate) fn clear_turn(&self, generation: u64, thread_id: &str, turn_id: &str) {
        if let Ok(mut state) = self.inner.lock() {
            if state.generation == generation {
                state.pending.retain(|_, approval| {
                    approval.thread_id != thread_id || approval.turn_id != turn_id
                });
                state.reserved.retain(|_, reserved| {
                    reserved.approval.thread_id != thread_id || reserved.approval.turn_id != turn_id
                });
            }
        }
    }

    /// Check both pending and response-reserved approvals for one exact thread
    /// in the current runtime generation.
    pub(crate) fn has_for_thread(&self, generation: u64, thread_id: &str) -> Result<bool, String> {
        validate_id("approval thread", thread_id)?;
        let state = self.inner.lock().map_err(|error| error.to_string())?;
        if state.generation != generation {
            return Err("Codex approval lookup belongs to a stale runtime generation".to_string());
        }
        Ok(state
            .pending
            .values()
            .any(|approval| approval.thread_id == thread_id)
            || state
                .reserved
                .values()
                .any(|reserved| reserved.approval.thread_id == thread_id))
    }

    /// Lock the current approval generation and prove one exact thread has no
    /// pending or response-reserved request.
    pub(crate) fn lock_without_thread_approval(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<PendingApprovalAdmissionGuard<'_>, String> {
        validate_id("approval admission thread", thread_id)?;
        let state = self.inner.lock().map_err(|error| error.to_string())?;
        if state.generation != generation {
            return Err(
                "Codex approval admission belongs to a stale runtime generation".to_string(),
            );
        }
        if state
            .pending
            .values()
            .any(|approval| approval.thread_id == thread_id)
            || state
                .reserved
                .values()
                .any(|reserved| reserved.approval.thread_id == thread_id)
        {
            return Err("Codex thread has a pending or response-reserved approval".to_string());
        }
        Ok(PendingApprovalAdmissionGuard { _state: state })
    }

    pub(crate) fn insert_request(
        &self,
        generation: u64,
        request_id: Value,
        method: &str,
        params: Option<Value>,
    ) -> Result<Option<CodeWorkspaceEventDraft>, String> {
        let Some(approval) = PendingApproval::from_request(generation, request_id, method, params)?
        else {
            return Ok(None);
        };
        let event = approval.event()?;
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        if state.generation != generation {
            return Err("Codex approval belongs to a stale runtime generation".to_string());
        }
        if state.pending.len() + state.reserved.len() >= MAX_PENDING_APPROVALS {
            return Err(format!(
                "Codex pending approval limit of {MAX_PENDING_APPROVALS} was reached"
            ));
        }
        if state.pending.contains_key(&approval.request_id)
            || state.reserved.contains_key(&approval.request_id)
        {
            return Err("Codex sent a duplicate approval request id".to_string());
        }
        state.pending.insert(approval.request_id.clone(), approval);
        Ok(Some(event))
    }

    pub(crate) fn resolve_notification(&self, generation: u64, params: &Value) {
        let Some(request_id) = params
            .get("requestId")
            .cloned()
            .and_then(|value| CodeRequestId::from_value(value).ok())
        else {
            return;
        };
        let thread_id = params.get("threadId").and_then(Value::as_str);
        if let Ok(mut state) = self.inner.lock() {
            if state.generation != generation {
                return;
            }
            let matches_pending_thread = state
                .pending
                .get(&request_id)
                .is_some_and(|approval| thread_id == Some(approval.thread_id.as_str()));
            let matches_reserved_thread = state
                .reserved
                .get(&request_id)
                .is_some_and(|reserved| thread_id == Some(reserved.approval.thread_id.as_str()));
            if matches_pending_thread || matches_reserved_thread {
                state.pending.remove(&request_id);
                state.reserved.remove(&request_id);
            }
        }
    }

    /// Atomically validate and reserve a response before the app-server write.
    ///
    /// While reserved, concurrent callers cannot answer the same request. The
    /// caller must commit after a successful write or restore after failure.
    pub(crate) fn reserve_response(
        &self,
        input: &CodeApprovalResponseInput,
    ) -> Result<ApprovalResponseReservation, String> {
        validate_id("thread", &input.thread_id)?;
        validate_id("turn", &input.turn_id)?;
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        if state.generation != input.runtime_generation {
            return Err("Codex approval belongs to a stale runtime generation".to_string());
        }
        let (wire_request_id, wire_result) = {
            let Some(approval) = state.pending.get(&input.request_id) else {
                if state.reserved.contains_key(&input.request_id) {
                    return Err("Codex approval response is already in progress".to_string());
                }
                return Err("Codex approval is no longer pending".to_string());
            };
            if approval.generation != input.runtime_generation
                || approval.thread_id != input.thread_id
                || approval.turn_id != input.turn_id
            {
                return Err(
                    "Codex approval does not match the active generation, thread, and turn"
                        .to_string(),
                );
            }
            let result = approval.response_value(&input.response)?;
            super::jsonrpc::validate_value_size(&result)?;
            (approval.request_id.to_value(), result)
        };
        let reservation_id = state
            .next_reservation_id
            .checked_add(1)
            .ok_or_else(|| "Codex approval reservation counter was exhausted".to_string())?;
        state.next_reservation_id = reservation_id;
        let approval = state
            .pending
            .remove(&input.request_id)
            .ok_or_else(|| "Codex approval disappeared while being reserved".to_string())?;
        state.reserved.insert(
            input.request_id.clone(),
            ReservedApproval {
                reservation_id,
                approval,
            },
        );
        Ok(ApprovalResponseReservation {
            generation: input.runtime_generation,
            request_id: input.request_id.clone(),
            reservation_id,
            wire_request_id,
            wire_result,
        })
    }

    /// Permanently consume a reservation after its response was written.
    pub(crate) fn commit_response(
        &self,
        reservation: &ApprovalResponseReservation,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        if state.generation != reservation.generation {
            return Ok(());
        }
        let matches = state
            .reserved
            .get(&reservation.request_id)
            .is_some_and(|reserved| reserved.reservation_id == reservation.reservation_id);
        if matches {
            state.reserved.remove(&reservation.request_id);
        }
        Ok(())
    }

    /// Restore a reservation after the app-server write failed.
    pub(crate) fn restore_response(
        &self,
        reservation: &ApprovalResponseReservation,
    ) -> Result<(), String> {
        let mut state = self.inner.lock().map_err(|error| error.to_string())?;
        if state.generation != reservation.generation {
            return Ok(());
        }
        let matches = state
            .reserved
            .get(&reservation.request_id)
            .is_some_and(|reserved| reserved.reservation_id == reservation.reservation_id);
        if !matches {
            return Ok(());
        }
        let reserved = state
            .reserved
            .remove(&reservation.request_id)
            .ok_or_else(|| "Codex approval reservation disappeared while restoring".to_string())?;
        if state.pending.contains_key(&reservation.request_id) {
            return Err("Codex approval request id was reused while restoring".to_string());
        }
        state
            .pending
            .insert(reservation.request_id.clone(), reserved.approval);
        Ok(())
    }

    /// Snapshot every approval owned by one runtime generation while the
    /// caller holds the event watermark lock. Reserved approvals remain
    /// visible but cannot be answered a second time.
    pub(crate) fn checkpoint_events(
        &self,
        generation: u64,
    ) -> Result<Vec<(CodeWorkspaceEventDraft, bool)>, String> {
        let state = self.inner.lock().map_err(|error| error.to_string())?;
        if state.generation != generation {
            return Err(
                "Codex approval checkpoint belongs to a stale runtime generation".to_string(),
            );
        }
        let mut approvals = state
            .pending
            .values()
            .map(|approval| (approval, true))
            .chain(
                state
                    .reserved
                    .values()
                    .map(|reserved| (&reserved.approval, false)),
            )
            .collect::<Vec<_>>();
        approvals.sort_by(|(left, _), (right, _)| {
            left.thread_id
                .cmp(&right.thread_id)
                .then_with(|| left.turn_id.cmp(&right.turn_id))
                .then_with(|| left.item_id.cmp(&right.item_id))
                .then_with(|| {
                    request_id_sort_key(&left.request_id)
                        .cmp(&request_id_sort_key(&right.request_id))
                })
        });
        approvals
            .into_iter()
            .map(|(approval, respondable)| Ok((approval.event()?, respondable)))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|state| state.pending.len() + state.reserved.len())
            .unwrap_or_default()
    }
}

fn request_id_sort_key(request_id: &CodeRequestId) -> (u8, String) {
    match request_id {
        CodeRequestId::Number(value) => (0, format!("{value:020}")),
        CodeRequestId::String(value) => (1, value.clone()),
    }
}

fn required_string(value: &Value, key: &str, method: &str) -> Result<String, String> {
    let string = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Codex `{method}` request has invalid `{key}`"))?;
    validate_id(key, string)?;
    Ok(string.to_string())
}

#[cfg(test)]
mod tests;
