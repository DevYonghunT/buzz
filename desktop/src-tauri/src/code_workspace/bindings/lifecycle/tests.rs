use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::*;
use crate::code_workspace::bindings::{CodeExecutionMode, MAX_IDENTIFIER_BYTES};
use crate::code_workspace::worktrees::CodeWorktreeDescriptor;

fn repository_identity(marker: char) -> String {
    marker.to_string().repeat(64)
}

fn scope(community: &str, marker: char) -> CodeThreadBindingScope {
    CodeThreadBindingScope {
        community_id: community.to_string(),
        project_dtag: "project".to_string(),
        repository_identity: repository_identity(marker),
    }
}

fn binding(root: &Path, owner: &CodeThreadBindingScope, thread_id: &str) -> CodeThreadBinding {
    CodeThreadBinding {
        community_id: owner.community_id.clone(),
        project_dtag: owner.project_dtag.clone(),
        repository_identity: owner.repository_identity.clone(),
        codex_thread_id: thread_id.to_string(),
        execution_mode: CodeExecutionMode::Local,
        execution_root: root.to_string_lossy().into_owned(),
        base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        worktree_id: None,
    }
}

fn lookup(owner: &CodeThreadBindingScope, thread_id: &str) -> CodeThreadBindingLookupInput {
    CodeThreadBindingLookupInput {
        scope: owner.clone(),
        codex_thread_id: thread_id.to_string(),
    }
}

fn store() -> Result<(tempfile::TempDir, CodeThreadBindingStore), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(directory.path())?;
    Ok((directory, store))
}

fn seed_active(
    directory: &tempfile::TempDir,
    store: &CodeThreadBindingStore,
    owner: &CodeThreadBindingScope,
    thread_id: &str,
) -> Result<CodeThreadBindingLookupInput, String> {
    let root = directory.path().join(format!("root-{thread_id}"));
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    store.upsert(binding(&root, owner, thread_id))?;
    Ok(lookup(owner, thread_id))
}

fn status(
    store: &CodeThreadBindingStore,
    input: &CodeThreadBindingLookupInput,
) -> Result<CodeThreadLifecycleStatus, String> {
    store
        .lookup_with_lifecycle(input)?
        .map(|snapshot| snapshot.status)
        .ok_or_else(|| "missing lifecycle snapshot".to_string())
}

fn write_json(path: &Path, value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    fs::write(path, &bytes).map_err(|error| error.to_string())?;
    Ok(bytes)
}

#[test]
fn v1_migrates_in_memory_to_active_without_writing_bytes_or_mtime() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let root = directory.path().join("legacy-root");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let legacy = json!({
        "version": 1,
        "bindings": [binding(&root, &owner, "thread-1")],
        "preparations": []
    });
    let before = write_json(store.store_path(), &legacy)?;
    let before_mtime = fs::metadata(store.store_path())
        .map_err(|error| error.to_string())?
        .modified()
        .map_err(|error| error.to_string())?;

    let migrated = store.load()?;

    assert_eq!(migrated.version, CODE_THREAD_BINDING_SCHEMA_VERSION);
    assert_eq!(migrated.lifecycles.len(), 1);
    assert_eq!(
        status(&store, &lookup(&owner, "thread-1"))?,
        CodeThreadLifecycleStatus::Active
    );
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        before
    );
    assert_eq!(
        fs::metadata(store.store_path())
            .map_err(|error| error.to_string())?
            .modified()
            .map_err(|error| error.to_string())?,
        before_mtime
    );
    Ok(())
}

#[test]
fn v3_migrates_without_authority_or_removal_state_and_without_writing() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    seed_active(&directory, &store, &owner, "thread-1")?;
    let mut legacy: Value =
        serde_json::from_slice(&fs::read(store.store_path()).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    legacy["version"] = json!(3);
    legacy
        .as_object_mut()
        .ok_or_else(|| "index must be an object".to_string())?
        .remove("mergeTargets");
    legacy
        .as_object_mut()
        .ok_or_else(|| "index must be an object".to_string())?
        .remove("removals");
    let before = write_json(store.store_path(), &legacy)?;
    let before_mtime = fs::metadata(store.store_path())
        .map_err(|error| error.to_string())?
        .modified()
        .map_err(|error| error.to_string())?;

    let migrated = store.load()?;

    assert_eq!(migrated.version, CODE_THREAD_BINDING_SCHEMA_VERSION);
    assert!(migrated.merge_targets.is_empty());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        before
    );
    assert_eq!(
        fs::metadata(store.store_path())
            .map_err(|error| error.to_string())?
            .modified()
            .map_err(|error| error.to_string())?,
        before_mtime
    );

    let mut forged_preparation = legacy;
    forged_preparation["preparations"] = json!([{
        "preparationId": "11111111-1111-4111-8111-111111111111",
        "communityId": owner.community_id,
        "projectDtag": owner.project_dtag,
        "repositoryIdentity": owner.repository_identity,
        "executionMode": "local",
        "executionRoot": directory.path().to_string_lossy(),
        "baseRef": "0123456789abcdef0123456789abcdef01234567",
        "worktreeId": null,
        "operation": "start",
        "sourceThreadId": null,
        "state": "prepared",
        "mergeTargetRef": "refs/heads/main"
    }]);
    let forged_bytes = write_json(store.store_path(), &forged_preparation)?;
    assert!(store.load().is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        forged_bytes
    );
    Ok(())
}

#[test]
fn first_lifecycle_mutation_after_v1_load_persists_v4_atomically() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let root = directory.path().join("legacy-root");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    write_json(
        store.store_path(),
        &json!({
            "version": 1,
            "bindings": [binding(&root, &owner, "thread-1")],
            "preparations": []
        }),
    )?;

    store.begin_archive(&lookup(&owner, "thread-1"))?;

    let persisted: Value =
        serde_json::from_slice(&fs::read(store.store_path()).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    assert_eq!(persisted["version"], CODE_THREAD_BINDING_SCHEMA_VERSION);
    assert_eq!(
        persisted["bindings"][0]
            .as_object()
            .map(|value| value.len()),
        Some(8)
    );
    assert_eq!(
        persisted["lifecycles"][0]["lifecycle"]["state"],
        "archiving"
    );
    assert!(persisted["lifecycles"][0]["lifecycle"]["operationId"]
        .as_str()
        .is_some());
    assert_eq!(persisted["mergeTargets"], json!([]));
    assert_eq!(persisted["removals"], json!([]));
    Ok(())
}

#[test]
fn v4_strictly_rejects_schema_and_lifecycle_drift_without_rewriting() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let input = seed_active(&directory, &store, &owner, "thread-1")?;
    let valid: Value =
        serde_json::from_slice(&fs::read(store.store_path()).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    assert_eq!(status(&store, &input)?, CodeThreadLifecycleStatus::Active);

    let mut fixtures = Vec::new();
    let mut missing_lifecycle = valid.clone();
    missing_lifecycle
        .as_object_mut()
        .ok_or_else(|| "index must be an object".to_string())?
        .remove("lifecycles");
    fixtures.push(missing_lifecycle);
    let mut future = valid.clone();
    future["version"] = json!(5);
    fixtures.push(future);
    let mut zero = valid.clone();
    zero["version"] = json!(0);
    fixtures.push(zero);
    let mut unknown_top_level = valid.clone();
    unknown_top_level["unexpected"] = json!(true);
    fixtures.push(unknown_top_level);
    let mut unknown_record_field = valid.clone();
    unknown_record_field["lifecycles"][0]["unexpected"] = json!(true);
    fixtures.push(unknown_record_field);
    let mut unknown_state = valid.clone();
    unknown_state["lifecycles"][0]["lifecycle"] = json!({ "state": "paused" });
    fixtures.push(unknown_state);
    let mut unknown_state_field = valid.clone();
    unknown_state_field["lifecycles"][0]["lifecycle"] =
        json!({ "state": "active", "operationId": "unexpected" });
    fixtures.push(unknown_state_field);
    let mut missing_operation = valid.clone();
    missing_operation["lifecycles"][0]["lifecycle"] = json!({ "state": "archiving" });
    fixtures.push(missing_operation);
    let mut malformed_operation = valid.clone();
    malformed_operation["lifecycles"][0]["lifecycle"] =
        json!({ "state": "archiving", "operationId": "not-a-uuid" });
    fixtures.push(malformed_operation);
    let mut noncanonical_operation = valid.clone();
    noncanonical_operation["lifecycles"][0]["lifecycle"] = json!({
        "state": "archiving",
        "operationId": "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"
    });
    fixtures.push(noncanonical_operation);
    let mut oversized_operation = valid.clone();
    oversized_operation["lifecycles"][0]["lifecycle"] = json!({
        "state": "archiving",
        "operationId": "a".repeat(MAX_IDENTIFIER_BYTES + 1)
    });
    fixtures.push(oversized_operation);
    let mut unknown_target = valid.clone();
    unknown_target["lifecycles"][0]["lifecycle"] = json!({
        "state": "unknown",
        "operationId": "11111111-1111-4111-8111-111111111111",
        "target": "paused"
    });
    fixtures.push(unknown_target);
    let mut missing_merge_targets = valid.clone();
    missing_merge_targets
        .as_object_mut()
        .ok_or_else(|| "index must be an object".to_string())?
        .remove("mergeTargets");
    fixtures.push(missing_merge_targets);
    let mut missing_removals = valid.clone();
    missing_removals
        .as_object_mut()
        .ok_or_else(|| "index must be an object".to_string())?
        .remove("removals");
    fixtures.push(missing_removals);
    for removal in [json!(null), json!("claim"), json!({ "state": "claimed" })] {
        let mut nonempty_removals = valid.clone();
        nonempty_removals["removals"] = json!([removal]);
        fixtures.push(nonempty_removals);
    }

    for fixture in fixtures {
        let bytes = write_json(store.store_path(), &fixture)?;
        assert!(
            store.load().is_err(),
            "fixture unexpectedly loaded: {fixture}"
        );
        assert_eq!(
            fs::read(store.store_path()).map_err(|error| error.to_string())?,
            bytes
        );
    }
    Ok(())
}

#[test]
fn v1_unknown_fields_are_strictly_rejected() -> Result<(), String> {
    let (_directory, store) = store()?;
    let fixture = json!({
        "version": 1,
        "bindings": [],
        "preparations": [],
        "lifecycles": []
    });
    let bytes = write_json(store.store_path(), &fixture)?;
    assert!(store.load().is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        bytes
    );
    Ok(())
}

#[test]
fn v2_requires_exactly_one_non_orphan_lifecycle_per_binding() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    seed_active(&directory, &store, &owner, "thread-1")?;
    let valid: Value =
        serde_json::from_slice(&fs::read(store.store_path()).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;

    let mut missing = valid.clone();
    missing["lifecycles"] = json!([]);
    let mut duplicate = valid.clone();
    let lifecycle = duplicate["lifecycles"][0].clone();
    duplicate["lifecycles"]
        .as_array_mut()
        .ok_or_else(|| "lifecycles must be an array".to_string())?
        .push(lifecycle);
    let mut orphan = valid.clone();
    orphan["lifecycles"][0]["codexThreadId"] = json!("thread-orphan");
    let mut wrong_scope = valid.clone();
    wrong_scope["lifecycles"][0]["communityId"] = json!("community-other");

    for fixture in [missing, duplicate, orphan, wrong_scope] {
        write_json(store.store_path(), &fixture)?;
        assert!(store.load().is_err(), "join fixture unexpectedly loaded");
    }
    Ok(())
}

#[test]
fn archive_unarchive_claims_commit_and_reload_exact_stable_targets() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let input = seed_active(&directory, &store, &owner, "thread-1")?;
    assert!(store.require_active_binding(&input).is_ok());

    let archive = store.begin_archive(&input)?;
    assert_eq!(
        status(&store, &input)?,
        CodeThreadLifecycleStatus::Archiving
    );
    let archived = store.complete_lifecycle_transition(&archive)?;
    assert_eq!(archived.status, CodeThreadLifecycleStatus::Archived);
    assert!(store.require_active_binding(&input).is_err());

    let reloaded = CodeThreadBindingStore::for_app_data(directory.path())?;
    assert_eq!(
        status(&reloaded, &input)?,
        CodeThreadLifecycleStatus::Archived
    );
    let unarchive = reloaded.begin_unarchive(&input)?;
    assert_eq!(
        reloaded.complete_lifecycle_transition(&unarchive)?.status,
        CodeThreadLifecycleStatus::Active
    );
    assert!(reloaded.require_active_binding(&input).is_ok());
    Ok(())
}

#[test]
fn exact_claim_rolls_back_only_definitely_unsent_and_keeps_uncertain_sticky() -> Result<(), String>
{
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let input = seed_active(&directory, &store, &owner, "thread-1")?;

    let rollback_claim = store.begin_archive(&input)?;
    assert_eq!(
        store
            .rollback_lifecycle_after_unsent(&rollback_claim)?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    assert!(store
        .complete_lifecycle_transition(&rollback_claim)
        .is_err());

    let uncertain_claim = store.begin_archive(&input)?;
    let unknown = store.mark_lifecycle_unknown(&uncertain_claim)?;
    assert_eq!(unknown.status, CodeThreadLifecycleStatus::Unknown);
    let before_idempotent = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    assert_eq!(
        store.mark_lifecycle_unknown(&uncertain_claim)?.status,
        CodeThreadLifecycleStatus::Unknown
    );
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        before_idempotent
    );
    assert!(store
        .rollback_lifecycle_after_unsent(&uncertain_claim)
        .is_err());
    assert_eq!(status(&store, &input)?, CodeThreadLifecycleStatus::Unknown);
    Ok(())
}

#[test]
fn forged_or_wrong_scope_claims_have_zero_durable_mutation() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let input = seed_active(&directory, &store, &owner, "thread-1")?;
    let before_wrong_scope = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let wrong = lookup(&scope("community-b", 'a'), "thread-1");
    assert!(store.begin_archive(&wrong).is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        before_wrong_scope
    );

    let claim = store.begin_archive(&input)?;
    let mut forged = claim.clone();
    forged.operation_id = "11111111-1111-4111-8111-111111111111".to_string();
    let before_forged = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    assert!(store.complete_lifecycle_transition(&forged).is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        before_forged
    );
    Ok(())
}

#[test]
fn reconciliation_marks_stable_drift_unknown_before_settling_next_pass() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let input = seed_active(&directory, &store, &owner, "thread-1")?;
    let matching_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, true, false)?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        matching_bytes
    );

    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, false, true)?
            .status,
        CodeThreadLifecycleStatus::Unknown
    );
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, false, true)?
            .status,
        CodeThreadLifecycleStatus::Archived
    );
    let archived_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, false, true)?
            .status,
        CodeThreadLifecycleStatus::Archived
    );
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        archived_bytes
    );

    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, true, false)?
            .status,
        CodeThreadLifecycleStatus::Unknown
    );
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, true, false)?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, true, true)?
            .status,
        CodeThreadLifecycleStatus::Unknown
    );
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, true, false)?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, false, false)?
            .status,
        CodeThreadLifecycleStatus::Unknown
    );
    Ok(())
}

#[test]
fn reconciliation_settles_inflight_exact_membership_and_keeps_ambiguity_unknown(
) -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let input = seed_active(&directory, &store, &owner, "thread-1")?;

    store.begin_archive(&input)?;
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, true, false)?
            .status,
        CodeThreadLifecycleStatus::Active
    );

    store.begin_archive(&input)?;
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, true, true)?
            .status,
        CodeThreadLifecycleStatus::Unknown
    );
    let ambiguous = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, false, false)?
            .status,
        CodeThreadLifecycleStatus::Unknown
    );
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        ambiguous
    );
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, false, true)?
            .status,
        CodeThreadLifecycleStatus::Archived
    );

    let unarchive = store.begin_unarchive(&input)?;
    assert_eq!(
        store
            .reconcile_lifecycle_membership(&input, true, false)?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    assert!(store.complete_lifecycle_transition(&unarchive).is_err());
    Ok(())
}

#[test]
fn graph_failure_marks_only_stable_state_unknown_with_preserved_target() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let input = seed_active(&directory, &store, &owner, "thread-1")?;
    assert_eq!(
        store.mark_stable_lifecycle_unknown(&input)?.status,
        CodeThreadLifecycleStatus::Unknown
    );
    let index = store.load()?;
    assert!(matches!(
        exact_lifecycle(&index, &input)?.lifecycle,
        CodeThreadLifecycle::Unknown {
            target: CodeThreadLifecycleTarget::Active,
            ..
        }
    ));
    assert!(store.mark_stable_lifecycle_unknown(&input).is_err());
    Ok(())
}

#[test]
fn injected_save_failure_leaves_prior_index_bytes_and_active_state() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let input = seed_active(&directory, &store, &owner, "thread-1")?;
    let before = fs::read(store.store_path()).map_err(|error| error.to_string())?;

    let result =
        store.begin_transition_with_save(&input, CodeThreadLifecycleTarget::Archived, |_| {
            Err("injected save failure".to_string())
        });

    assert!(result.is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        before
    );
    assert_eq!(status(&store, &input)?, CodeThreadLifecycleStatus::Active);
    Ok(())
}

#[test]
fn committed_preparation_and_test_upsert_both_create_active_join_records() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let root = directory.path().join("prepared-root");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Local,
        repository_identity: owner.repository_identity.clone(),
        execution_root: root.to_string_lossy().into_owned(),
        base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        worktree_id: None,
    };
    let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
    store.create_preparation(preparation_id.to_string(), owner.clone(), &descriptor)?;
    store.claim_preparation_for_start(&owner, preparation_id, Vec::new())?;
    store.commit_preparation_binding(&owner, preparation_id, "thread-prepared")?;
    assert_eq!(
        status(&store, &lookup(&owner, "thread-prepared"))?,
        CodeThreadLifecycleStatus::Active
    );

    let second_root = directory.path().join("upsert-root");
    fs::create_dir(&second_root).map_err(|error| error.to_string())?;
    let second_root = second_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    store.upsert(binding(&second_root, &owner, "thread-upsert"))?;
    assert_eq!(store.load()?.lifecycles.len(), 2);
    Ok(())
}

#[test]
fn archived_managed_binding_continues_to_reserve_its_worktree() -> Result<(), String> {
    let (directory, store) = store()?;
    let owner = scope("community-a", 'a');
    let root = directory.path().join("managed-root");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let worktree_id = "11111111-1111-4111-8111-111111111111";
    let mut first = binding(&root, &owner, "thread-1");
    first.execution_mode = CodeExecutionMode::Worktree;
    first.worktree_id = Some(worktree_id.to_string());
    store.upsert(first)?;
    let input = lookup(&owner, "thread-1");
    let claim = store.begin_archive(&input)?;
    store.complete_lifecycle_transition(&claim)?;

    let mut second = binding(&root, &owner, "thread-2");
    second.execution_mode = CodeExecutionMode::Worktree;
    second.worktree_id = Some(worktree_id.to_string());
    assert!(store.upsert(second).is_err());
    assert_eq!(status(&store, &input)?, CodeThreadLifecycleStatus::Archived);
    Ok(())
}
