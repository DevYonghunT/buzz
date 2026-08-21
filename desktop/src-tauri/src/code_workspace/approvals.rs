use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::bindings::CodeThreadBindingScope;
use super::protocol::{
    redact_protocol_text, redact_protocol_value, validate_id, CodeRequestId,
    CodeWorkspaceEventDraft,
};

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

struct PermissionDisplayValidation {
    accurate: bool,
    non_empty: bool,
}

impl PermissionDisplayValidation {
    fn new() -> Self {
        Self {
            accurate: true,
            non_empty: false,
        }
    }

    fn invalidate(&mut self) {
        self.accurate = false;
    }
}

fn permission_display_from_raw(raw: Option<&Value>) -> PermissionDisplay {
    let mut validation = PermissionDisplayValidation::new();
    let Some(permissions) = raw.and_then(Value::as_object) else {
        validation.invalidate();
        return PermissionDisplay {
            grantable: false,
            network: None,
            file_system: None,
        };
    };
    if !has_only_keys(permissions, &["network", "fileSystem"]) {
        validation.invalidate();
    }
    let network = permissions
        .get("network")
        .and_then(|value| parse_network_display(value, &mut validation));
    let file_system = permissions
        .get("fileSystem")
        .and_then(|value| parse_file_system_display(value, &mut validation));
    PermissionDisplay {
        grantable: validation.accurate && validation.non_empty,
        network,
        file_system,
    }
}

fn parse_network_display(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionNetworkDisplay> {
    if value.is_null() {
        return None;
    }
    let Some(network) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    if !has_only_keys(network, &["enabled"]) {
        validation.invalidate();
    }
    let enabled = match network.get("enabled") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(enabled)) => {
            validation.non_empty |= *enabled;
            Some(*enabled)
        }
        Some(_) => {
            validation.invalidate();
            None
        }
    };
    Some(PermissionNetworkDisplay { enabled })
}

fn parse_file_system_display(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionFileSystemDisplay> {
    if value.is_null() {
        return None;
    }
    let Some(file_system) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    if !has_only_keys(
        file_system,
        &["entries", "globScanMaxDepth", "read", "write"],
    ) {
        validation.invalidate();
    }
    let entries = file_system
        .get("entries")
        .and_then(|value| parse_file_system_entries(value, validation));
    let glob_scan_max_depth = match file_system.get("globScanMaxDepth") {
        None | Some(Value::Null) => None,
        Some(value) => match value
            .as_u64()
            .filter(|depth| *depth > 0 && *depth <= MAX_SAFE_JSON_INTEGER)
        {
            Some(depth) => Some(depth),
            None => {
                validation.invalidate();
                None
            }
        },
    };
    let read = file_system
        .get("read")
        .and_then(|value| parse_permission_paths(value, validation));
    let write = file_system
        .get("write")
        .and_then(|value| parse_permission_paths(value, validation));
    validation.non_empty |= entries.as_ref().is_some_and(|entries| !entries.is_empty())
        || read.as_ref().is_some_and(|paths| !paths.is_empty())
        || write.as_ref().is_some_and(|paths| !paths.is_empty());
    Some(PermissionFileSystemDisplay {
        entries,
        glob_scan_max_depth,
        read,
        write,
    })
}

fn parse_permission_paths(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<Vec<String>> {
    if value.is_null() {
        return None;
    }
    let Some(paths) = value.as_array() else {
        validation.invalidate();
        return None;
    };
    Some(
        paths
            .iter()
            .filter_map(|path| permission_text(path, validation))
            .collect(),
    )
}

fn parse_file_system_entries(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<Vec<PermissionFileSystemEntryDisplay>> {
    if value.is_null() {
        return None;
    }
    let Some(entries) = value.as_array() else {
        validation.invalidate();
        return None;
    };
    Some(
        entries
            .iter()
            .filter_map(|entry| parse_file_system_entry(entry, validation))
            .collect(),
    )
}

fn parse_file_system_entry(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionFileSystemEntryDisplay> {
    let Some(entry) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    if !has_only_keys(entry, &["access", "path"]) {
        validation.invalidate();
    }
    let access = match entry.get("access").and_then(Value::as_str) {
        Some("read") => Some(PermissionAccessDisplay::Read),
        Some("write") => Some(PermissionAccessDisplay::Write),
        Some("deny") => Some(PermissionAccessDisplay::Deny),
        _ => {
            validation.invalidate();
            None
        }
    };
    let path = match entry.get("path") {
        Some(path) => parse_permission_path(path, validation),
        None => {
            validation.invalidate();
            None
        }
    };
    match (access, path) {
        (Some(access), Some(path)) => Some(PermissionFileSystemEntryDisplay { access, path }),
        _ => None,
    }
}

fn parse_permission_path(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionPathDisplay> {
    let Some(path) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    match path.get("type").and_then(Value::as_str) {
        Some("path") => {
            if !has_only_keys(path, &["type", "path"]) {
                validation.invalidate();
            }
            match path.get("path") {
                Some(path) => permission_text(path, validation)
                    .map(|path| PermissionPathDisplay::Path { path }),
                None => {
                    validation.invalidate();
                    None
                }
            }
        }
        Some("glob_pattern") => {
            if !has_only_keys(path, &["type", "pattern"]) {
                validation.invalidate();
            }
            match path.get("pattern") {
                Some(pattern) => permission_text(pattern, validation)
                    .map(|pattern| PermissionPathDisplay::GlobPattern { pattern }),
                None => {
                    validation.invalidate();
                    None
                }
            }
        }
        Some("special") => {
            if !has_only_keys(path, &["type", "value"]) {
                validation.invalidate();
            }
            match path.get("value") {
                Some(value) => parse_special_path(value, validation)
                    .map(|value| PermissionPathDisplay::Special { value }),
                None => {
                    validation.invalidate();
                    None
                }
            }
        }
        _ => {
            validation.invalidate();
            None
        }
    }
}

fn parse_special_path(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionSpecialPathDisplay> {
    let Some(special) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    match special.get("kind").and_then(Value::as_str) {
        Some("root") => exact_special(
            special,
            &["kind"],
            PermissionSpecialPathDisplay::Root,
            validation,
        ),
        Some("minimal") => exact_special(
            special,
            &["kind"],
            PermissionSpecialPathDisplay::Minimal,
            validation,
        ),
        Some("tmpdir") => exact_special(
            special,
            &["kind"],
            PermissionSpecialPathDisplay::Tmpdir,
            validation,
        ),
        Some("slash_tmp") => exact_special(
            special,
            &["kind"],
            PermissionSpecialPathDisplay::SlashTmp,
            validation,
        ),
        Some("project_roots") => {
            if !has_only_keys(special, &["kind", "subpath"]) {
                validation.invalidate();
            }
            optional_permission_text(special.get("subpath"), validation)
                .map(|subpath| PermissionSpecialPathDisplay::ProjectRoots { subpath })
        }
        Some("unknown") => {
            if !has_only_keys(special, &["kind", "path", "subpath"]) {
                validation.invalidate();
            }
            let path = match special.get("path") {
                Some(path) => permission_text(path, validation),
                None => {
                    validation.invalidate();
                    None
                }
            };
            let subpath = optional_permission_text(special.get("subpath"), validation);
            path.zip(subpath)
                .map(|(path, subpath)| PermissionSpecialPathDisplay::Unknown { path, subpath })
        }
        _ => {
            validation.invalidate();
            None
        }
    }
}

fn exact_special(
    special: &Map<String, Value>,
    keys: &[&str],
    display: PermissionSpecialPathDisplay,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionSpecialPathDisplay> {
    if !has_only_keys(special, keys) {
        validation.invalidate();
    }
    Some(display)
}

fn optional_permission_text(
    value: Option<&Value>,
    validation: &mut PermissionDisplayValidation,
) -> Option<Option<String>> {
    match value {
        None | Some(Value::Null) => Some(None),
        Some(value) => permission_text(value, validation).map(Some),
    }
}

fn permission_text(value: &Value, validation: &mut PermissionDisplayValidation) -> Option<String> {
    let Some(text) = value.as_str() else {
        validation.invalidate();
        return None;
    };
    if text.is_empty() {
        validation.invalidate();
    }
    let redacted = redact_protocol_text(text);
    if redacted != text {
        validation.invalidate();
    }
    Some(redacted)
}

fn has_only_keys(object: &Map<String, Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    fn binding_scope() -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: "community-1".to_string(),
            project_dtag: "project-1".to_string(),
            repository_identity: "a".repeat(64),
        }
    }

    fn decision_input(generation: u64) -> CodeApprovalResponseInput {
        CodeApprovalResponseInput {
            runtime_generation: generation,
            request_id: CodeRequestId::String("approval-1".to_string()),
            scope: binding_scope(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            response: CodeApprovalResponse::Decision {
                decision: CodeApprovalDecision::Accept,
            },
        }
    }

    fn insert_file_approval(store: &PendingApprovalStore, generation: u64) -> Result<(), String> {
        store.insert_request(
            generation,
            json!("approval-1"),
            "item/fileChange/requestApproval",
            Some(json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1"
            })),
        )?;
        Ok(())
    }

    fn permission_request() -> Value {
        json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "item-1",
            "cwd": "/tmp/project",
            "permissions": {
                "network": { "enabled": true },
                "fileSystem": {
                    "entries": [
                        {
                            "access": "write",
                            "path": { "type": "path", "path": "/tmp/generated" }
                        },
                        {
                            "access": "read",
                            "path": {
                                "type": "glob_pattern",
                                "pattern": "/tmp/project/**/*.rs"
                            }
                        },
                        {
                            "access": "deny",
                            "path": {
                                "type": "special",
                                "value": {
                                    "kind": "project_roots",
                                    "subpath": ".git"
                                }
                            }
                        }
                    ],
                    "globScanMaxDepth": 12,
                    "read": ["/tmp/read"],
                    "write": ["/tmp/write"]
                }
            }
        })
    }

    fn permission_input(
        request_id: CodeRequestId,
        intent: CodePermissionIntent,
        scope: CodePermissionScope,
    ) -> CodeApprovalResponseInput {
        CodeApprovalResponseInput {
            runtime_generation: 1,
            request_id,
            scope: binding_scope(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            response: CodeApprovalResponse::Permissions { intent, scope },
        }
    }

    #[test]
    fn generation_and_thread_turn_tuple_gate_responses() -> Result<(), String> {
        let store = PendingApprovalStore::default();
        store.reset(4);
        insert_file_approval(&store, 4)?;

        let stale = decision_input(3);
        assert!(store.reserve_response(&stale).is_err());
        assert_eq!(store.len(), 1);

        let current = CodeApprovalResponseInput {
            runtime_generation: 4,
            ..stale
        };
        let reservation = store.reserve_response(&current)?;
        assert_eq!(store.len(), 1);
        store.commit_response(&reservation)?;
        assert_eq!(store.len(), 0);
        Ok(())
    }

    #[test]
    fn permission_event_exposes_only_deterministic_redacted_display() -> Result<(), String> {
        let store = PendingApprovalStore::default();
        store.reset(1);
        let event = store
            .insert_request(
                1,
                json!(9),
                "item/permissions/requestApproval",
                Some(permission_request()),
            )?
            .ok_or_else(|| "permission request was not recognized".to_string())?;
        let request = &event.payload["request"];
        assert!(request.get("permissions").is_none());
        assert_eq!(request["permissionDisplay"]["grantable"], true);
        assert_eq!(request["permissionDisplay"]["network"]["enabled"], true);
        assert_eq!(
            request["permissionDisplay"]["fileSystem"]["entries"][0],
            json!({
                "access": "write",
                "path": { "type": "path", "path": "/tmp/generated" }
            })
        );
        assert_eq!(
            request["permissionDisplay"]["fileSystem"]["entries"][1],
            json!({
                "access": "read",
                "path": {
                    "type": "globPattern",
                    "pattern": "/tmp/project/**/*.rs"
                }
            })
        );
        assert_eq!(
            request["permissionDisplay"]["fileSystem"]["entries"][2],
            json!({
                "access": "deny",
                "path": {
                    "type": "special",
                    "value": { "kind": "project_roots", "subpath": ".git" }
                }
            })
        );
        assert_eq!(
            request["permissionDisplay"]["fileSystem"]["globScanMaxDepth"],
            12
        );
        Ok(())
    }

    #[test]
    fn permission_grant_uses_whole_raw_request_and_canonical_turn_flags() -> Result<(), String> {
        let store = PendingApprovalStore::default();
        store.reset(1);
        let request = permission_request();
        let requested_permissions = request["permissions"].clone();
        store.insert_request(
            1,
            json!(9),
            "item/permissions/requestApproval",
            Some(request),
        )?;

        let response = permission_input(
            CodeRequestId::Number(9),
            CodePermissionIntent::Grant,
            CodePermissionScope::Turn,
        );
        let reservation = store.reserve_response(&response)?;
        let (_, result) = reservation.wire_response();
        assert_eq!(result["permissions"], requested_permissions);
        assert_eq!(result["scope"], "turn");
        assert_eq!(result["strictAutoReview"], true);
        assert_eq!(store.len(), 1);
        Ok(())
    }

    #[test]
    fn permission_session_and_decline_results_are_canonical() -> Result<(), String> {
        let store = PendingApprovalStore::default();
        store.reset(1);
        let request = permission_request();
        let requested_permissions = request["permissions"].clone();
        store.insert_request(
            1,
            json!(10),
            "item/permissions/requestApproval",
            Some(request),
        )?;
        let grant = permission_input(
            CodeRequestId::Number(10),
            CodePermissionIntent::Grant,
            CodePermissionScope::Session,
        );
        let reservation = store.reserve_response(&grant)?;
        let (_, result) = reservation.wire_response();
        assert_eq!(result["permissions"], requested_permissions);
        assert_eq!(result["scope"], "session");
        assert_eq!(result["strictAutoReview"], false);
        store.commit_response(&reservation)?;

        store.insert_request(
            1,
            json!(11),
            "item/permissions/requestApproval",
            Some(permission_request()),
        )?;
        let decline = permission_input(
            CodeRequestId::Number(11),
            CodePermissionIntent::Decline,
            CodePermissionScope::Session,
        );
        let reservation = store.reserve_response(&decline)?;
        let (_, result) = reservation.wire_response();
        assert_eq!(
            result,
            json!({
                "permissions": {},
                "scope": "turn",
                "strictAutoReview": false
            })
        );
        Ok(())
    }

    #[test]
    fn malformed_empty_or_inaccurately_redacted_permissions_cannot_be_granted() -> Result<(), String>
    {
        for (index, permissions) in [
            json!({}),
            json!({ "futurePermission": { "enabled": true } }),
            json!({ "fileSystem": { "read": ["/tmp/sk-project-secret"] } }),
            json!({
                "fileSystem": {
                    "entries": [{
                        "access": "write",
                        "path": { "type": "future", "path": "/tmp/write" }
                    }]
                }
            }),
        ]
        .into_iter()
        .enumerate()
        {
            let store = PendingApprovalStore::default();
            store.reset(1);
            let request_id = CodeRequestId::Number(index as u64);
            let event = store
                .insert_request(
                    1,
                    request_id.to_value(),
                    "item/permissions/requestApproval",
                    Some(json!({
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "itemId": "item-1",
                        "permissions": permissions
                    })),
                )?
                .ok_or_else(|| "permission request was not recognized".to_string())?;
            assert_eq!(
                event.payload["request"]["permissionDisplay"]["grantable"],
                false
            );
            if index == 2 {
                let public_payload = event.payload.to_string();
                assert!(!public_payload.contains("sk-project-secret"));
                assert!(public_payload.contains("[REDACTED]"));
            }
            assert!(store
                .reserve_response(&permission_input(
                    request_id.clone(),
                    CodePermissionIntent::Grant,
                    CodePermissionScope::Turn,
                ))
                .is_err());
            let decline = store.reserve_response(&permission_input(
                request_id,
                CodePermissionIntent::Decline,
                CodePermissionScope::Session,
            ))?;
            assert_eq!(
                decline.wire_response().1,
                json!({
                    "permissions": {},
                    "scope": "turn",
                    "strictAutoReview": false
                })
            );
        }
        Ok(())
    }

    #[test]
    fn approval_json_is_strict_and_contains_only_permission_intent() -> Result<(), String> {
        let input: CodeApprovalResponseInput = serde_json::from_value(json!({
            "runtimeGeneration": 7,
            "requestId": "approval-1",
            "scope": binding_scope(),
            "threadId": "thread-1",
            "turnId": "turn-1",
            "response": {
                "type": "permissions",
                "intent": "grant",
                "scope": "turn"
            }
        }))
        .map_err(|error| error.to_string())?;

        assert!(input.approves_execution());
        match input.response {
            CodeApprovalResponse::Permissions { intent, scope } => {
                assert_eq!(intent, CodePermissionIntent::Grant);
                assert_eq!(scope, CodePermissionScope::Turn);
            }
            CodeApprovalResponse::Decision { .. } => {
                return Err("expected a permission response".to_string());
            }
        }

        assert!(serde_json::from_value::<CodeApprovalResponseInput>(json!({
            "runtimeGeneration": 7,
            "requestId": "approval-1",
            "scope": binding_scope(),
            "threadId": "thread-1",
            "turnId": "turn-1",
            "response": { "type": "decision", "decision": "accept" },
            "unexpected": true
        }))
        .is_err());
        assert!(serde_json::from_value::<CodeApprovalResponse>(json!({
            "type": "decision",
            "decision": "accept",
            "unexpected": true
        }))
        .is_err());
        assert!(serde_json::from_value::<CodeApprovalResponse>(json!({
            "type": "permissions",
            "permissions": {},
            "scope": "turn",
            "intent": "grant"
        }))
        .is_err());
        assert!(serde_json::from_value::<CodeApprovalResponse>(json!({
            "type": "permissions",
            "intent": "grant",
            "scope": "turn",
            "strictAutoReview": true
        }))
        .is_err());
        Ok(())
    }

    #[test]
    fn approval_intent_distinguishes_grants_from_rejections() {
        for decision in [
            CodeApprovalDecision::Accept,
            CodeApprovalDecision::AcceptForSession,
        ] {
            assert!(CodeApprovalResponse::Decision { decision }.approves_execution());
        }
        for decision in [CodeApprovalDecision::Decline, CodeApprovalDecision::Cancel] {
            assert!(!CodeApprovalResponse::Decision { decision }.approves_execution());
        }
        assert!(!CodeApprovalResponse::Permissions {
            intent: CodePermissionIntent::Decline,
            scope: CodePermissionScope::Turn,
        }
        .approves_execution());
        assert!(CodeApprovalResponse::Permissions {
            intent: CodePermissionIntent::Grant,
            scope: CodePermissionScope::Turn,
        }
        .approves_execution());
    }

    #[test]
    fn failed_send_restores_reservation_for_one_retry() -> Result<(), String> {
        let store = PendingApprovalStore::default();
        store.reset(1);
        insert_file_approval(&store, 1)?;
        let input = decision_input(1);

        let first = store.reserve_response(&input)?;
        assert!(store.reserve_response(&input).is_err());
        assert_eq!(store.len(), 1);
        store.restore_response(&first)?;

        let retry = store.reserve_response(&input)?;
        store.commit_response(&retry)?;
        assert_eq!(store.len(), 0);
        assert!(store.reserve_response(&input).is_err());
        Ok(())
    }

    #[test]
    fn checkpoint_marks_reserved_approval_non_respondable_until_restore() -> Result<(), String> {
        let store = PendingApprovalStore::default();
        store.reset(1);
        insert_file_approval(&store, 1)?;
        let pending = store.checkpoint_events(1)?;
        assert_eq!(pending.len(), 1);
        assert!(pending[0].1);

        let reservation = store.reserve_response(&decision_input(1))?;
        let reserved = store.checkpoint_events(1)?;
        assert_eq!(reserved.len(), 1);
        assert!(!reserved[0].1);

        store.restore_response(&reservation)?;
        let restored = store.checkpoint_events(1)?;
        assert_eq!(restored.len(), 1);
        assert!(restored[0].1);
        Ok(())
    }

    #[test]
    fn exact_thread_lookup_includes_pending_and_reserved_current_generation() -> Result<(), String>
    {
        let store = PendingApprovalStore::default();
        store.reset(1);
        drop(store.lock_without_thread_approval(1, "thread-1")?);
        insert_file_approval(&store, 1)?;
        assert!(store.has_for_thread(1, "thread-1")?);
        assert!(store.lock_without_thread_approval(1, "thread-1").is_err());
        assert!(!store.has_for_thread(1, "thread-2")?);
        assert!(store.has_for_thread(2, "thread-1").is_err());

        let reservation = store.reserve_response(&decision_input(1))?;
        assert!(store.has_for_thread(1, "thread-1")?);
        assert!(store.lock_without_thread_approval(1, "thread-1").is_err());
        store.commit_response(&reservation)?;
        assert!(!store.has_for_thread(1, "thread-1")?);
        Ok(())
    }

    #[test]
    fn resolved_notification_does_not_allow_a_reserved_response_to_resurrect() -> Result<(), String>
    {
        let store = PendingApprovalStore::default();
        store.reset(1);
        insert_file_approval(&store, 1)?;
        let reservation = store.reserve_response(&decision_input(1))?;

        store.resolve_notification(
            1,
            &json!({ "requestId": "approval-1", "threadId": "thread-1" }),
        );
        store.restore_response(&reservation)?;

        assert_eq!(store.len(), 0);
        assert!(store.reserve_response(&decision_input(1)).is_err());
        Ok(())
    }

    #[test]
    fn concurrent_responses_get_exactly_one_reservation() -> Result<(), String> {
        let store = Arc::new(PendingApprovalStore::default());
        store.reset(1);
        insert_file_approval(&store, 1)?;
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let input = decision_input(1);
                barrier.wait();
                store.reserve_response(&input)
            }));
        }

        let mut reservations = Vec::new();
        let mut failures = 0;
        for handle in handles {
            match handle
                .join()
                .map_err(|_| "approval reservation thread panicked".to_string())?
            {
                Ok(reservation) => reservations.push(reservation),
                Err(_) => failures += 1,
            }
        }
        assert_eq!(reservations.len(), 1);
        assert_eq!(failures, 1);
        store.commit_response(&reservations[0])?;
        assert_eq!(store.len(), 0);
        Ok(())
    }
}
