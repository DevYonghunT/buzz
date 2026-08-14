use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::bindings::CodeThreadBindingScope;
use super::protocol::{redact_protocol_value, validate_id, CodeRequestId, CodeWorkspaceEventDraft};

const MAX_PENDING_APPROVALS: usize = 128;

/// Decisions supported by stable command and file-change approval responses.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeApprovalDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

/// Lifetime of an explicitly granted permission subset.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodePermissionScope {
    Turn,
    Session,
}

/// Normalized frontend response for either a decision or permission grant.
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
        permissions: Value,
        scope: CodePermissionScope,
        #[serde(default)]
        strict_auto_review: bool,
    },
}

impl CodeApprovalResponse {
    /// Whether this response would grant the requested operation rather than
    /// decline or cancel it.
    pub(crate) fn approves_execution(&self) -> bool {
        matches!(
            self,
            Self::Decision {
                decision: CodeApprovalDecision::Accept | CodeApprovalDecision::AcceptForSession
            } | Self::Permissions { .. }
        )
    }
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
        if kind == ApprovalKind::Permissions
            && !params.get("permissions").is_some_and(Value::is_object)
        {
            return Err("Codex permission approval request has invalid permissions".to_string());
        }
        Ok(Some(Self {
            generation,
            request_id,
            thread_id,
            turn_id,
            item_id,
            method: method.to_string(),
            kind,
            params,
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
                "request": redact_protocol_value(self.params.clone())
            }),
        })
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
            (
                ApprovalKind::Permissions,
                CodeApprovalResponse::Permissions {
                    permissions,
                    scope,
                    strict_auto_review,
                },
            ) => {
                if !permissions.is_object() {
                    return Err("granted Codex permissions must be an object".to_string());
                }
                let requested = self.params.get("permissions").ok_or_else(|| {
                    "pending Codex permission request has no permissions".to_string()
                })?;
                if !is_json_subset(permissions, requested) {
                    return Err("granted Codex permissions exceed the pending request".to_string());
                }
                Ok(json!({
                    "permissions": permissions,
                    "scope": scope,
                    "strictAutoReview": strict_auto_review
                }))
            }
            (ApprovalKind::Permissions, CodeApprovalResponse::Decision { .. }) => Err(
                "permission approvals require an explicit granted permission subset".to_string(),
            ),
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

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|state| state.pending.len() + state.reserved.len())
            .unwrap_or_default()
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

fn is_json_subset(granted: &Value, requested: &Value) -> bool {
    match (granted, requested) {
        (Value::Object(granted), Value::Object(requested)) => granted.iter().all(|(key, value)| {
            requested
                .get(key)
                .is_some_and(|requested| is_json_subset(value, requested))
        }),
        (Value::Array(granted), Value::Array(requested)) => granted
            .iter()
            .all(|value| requested.iter().any(|candidate| candidate == value)),
        _ => granted == requested,
    }
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
            "permissions": {
                "network": { "enabled": true },
                "fileSystem": { "read": ["/tmp/read"], "write": ["/tmp/write"] }
            }
        })
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
    fn permission_grants_cannot_exceed_the_requested_subset() -> Result<(), String> {
        let store = PendingApprovalStore::default();
        store.reset(1);
        store.insert_request(
            1,
            json!(9),
            "item/permissions/requestApproval",
            Some(permission_request()),
        )?;

        let response = CodeApprovalResponseInput {
            runtime_generation: 1,
            request_id: CodeRequestId::Number(9),
            scope: binding_scope(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            response: CodeApprovalResponse::Permissions {
                permissions: json!({ "fileSystem": { "write": ["/tmp/not-requested"] } }),
                scope: CodePermissionScope::Turn,
                strict_auto_review: false,
            },
        };
        assert!(store.reserve_response(&response).is_err());
        assert_eq!(store.len(), 1);
        Ok(())
    }

    #[test]
    fn approval_json_is_strict_and_maps_strict_auto_review() -> Result<(), String> {
        let input: CodeApprovalResponseInput = serde_json::from_value(json!({
            "runtimeGeneration": 7,
            "requestId": "approval-1",
            "scope": binding_scope(),
            "threadId": "thread-1",
            "turnId": "turn-1",
            "response": {
                "type": "permissions",
                "permissions": { "network": { "enabled": true } },
                "scope": "turn",
                "strictAutoReview": true
            }
        }))
        .map_err(|error| error.to_string())?;

        assert!(input.approves_execution());
        match input.response {
            CodeApprovalResponse::Permissions {
                strict_auto_review, ..
            } => assert!(strict_auto_review),
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
            "strict_auto_review": true
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
        assert!(CodeApprovalResponse::Permissions {
            permissions: json!({}),
            scope: CodePermissionScope::Turn,
            strict_auto_review: false,
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
