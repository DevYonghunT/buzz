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
fn malformed_empty_or_inaccurately_redacted_permissions_cannot_be_granted() -> Result<(), String> {
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
fn exact_thread_lookup_includes_pending_and_reserved_current_generation() -> Result<(), String> {
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
fn resolved_notification_does_not_allow_a_reserved_response_to_resurrect() -> Result<(), String> {
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
