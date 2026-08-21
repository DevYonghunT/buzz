use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use serde_json::{json, Value};

use super::*;
use crate::code_workspace::bindings::{
    CodeThreadBindingScope, CodeThreadLifecycleStatus, CodeThreadPreparationState,
};
use crate::code_workspace::worktrees::CodeWorktreeDescriptor;

const WORKTREE_ID: &str = "11111111-1111-4111-8111-111111111111";
const PREPARATION_ID: &str = "22222222-2222-4222-8222-222222222222";
const REMOVAL_ID: &str = "33333333-3333-4333-8333-333333333333";
const SECOND_REMOVAL_ID: &str = "44444444-4444-4444-8444-444444444444";

struct Seed {
    scope: CodeThreadBindingScope,
    lookup: CodeThreadBindingLookupInput,
    binding: CodeThreadBinding,
    descriptor: CodeWorktreeDescriptor,
    claim: CodeWorktreeRemovalClaimInput,
    external_root: PathBuf,
}

fn seed_active() -> Result<(tempfile::TempDir, CodeThreadBindingStore, Seed), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let app_data = directory.path().join("app-data");
    fs::create_dir(&app_data).map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(&app_data)?;

    let external_root = directory.path().join("external-state");
    let managed_parent = external_root.join("WORKTREES").join("a".repeat(64));
    let managed_root = managed_parent.join(WORKTREE_ID);
    let git_admin_parent = external_root.join("repository.git").join("worktrees");
    let git_admin_entry = format!("admin-{WORKTREE_ID}");
    fs::create_dir_all(&managed_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(git_admin_parent.join(&git_admin_entry))
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(external_root.join("transcripts")).map_err(|error| error.to_string())?;
    fs::create_dir_all(external_root.join("sibling-worktree"))
        .map_err(|error| error.to_string())?;
    fs::write(managed_root.join(".git"), b"gitdir: admin\n").map_err(|error| error.to_string())?;
    fs::write(
        managed_root.join("tracked.txt"),
        b"preserve worktree bytes\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        git_admin_parent.join(&git_admin_entry).join("gitdir"),
        managed_root.to_string_lossy().as_bytes(),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        external_root
            .join("transcripts")
            .join("thread-removal.jsonl"),
        b"preserve transcript\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        external_root.join("sibling-worktree").join("sentinel"),
        b"preserve sibling\n",
    )
    .map_err(|error| error.to_string())?;

    let execution_root = managed_root
        .canonicalize()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let git_admin_parent = git_admin_parent
        .canonicalize()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let scope = CodeThreadBindingScope {
        community_id: "community-removal".to_string(),
        project_dtag: "project-removal".to_string(),
        repository_identity: "a".repeat(64),
    };
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: scope.repository_identity.clone(),
        execution_root,
        base_ref: "0".repeat(40),
        worktree_id: Some(WORKTREE_ID.to_string()),
    };
    store.create_preparation_with_merge_target(
        PREPARATION_ID.to_string(),
        scope.clone(),
        &descriptor,
        Some("refs/heads/main".to_string()),
    )?;
    let starting = store.claim_preparation_for_start(&scope, PREPARATION_ID, Vec::new())?;
    assert_eq!(starting.state, CodeThreadPreparationState::Starting);
    let binding = store.commit_preparation_binding(&scope, PREPARATION_ID, "thread-removal")?;
    let lookup = CodeThreadBindingLookupInput {
        scope: scope.clone(),
        codex_thread_id: binding.codex_thread_id.clone(),
    };
    let claim = CodeWorktreeRemovalClaimInput {
        lookup: lookup.clone(),
        merge_proof: CodeMergeProofReceipt {
            repository_identity: scope.repository_identity.clone(),
            worktree_id: WORKTREE_ID.to_string(),
            head_commit: "1".repeat(40),
            target_ref: "refs/heads/main".to_string(),
            target_commit: "2".repeat(40),
        },
        physical_manifest_digest: "d".repeat(64),
        git_admin_parent,
        git_admin_entry,
    };
    Ok((
        directory,
        store,
        Seed {
            scope,
            lookup,
            binding,
            descriptor,
            claim,
            external_root,
        },
    ))
}

fn archive(
    store: &CodeThreadBindingStore,
    lookup: &CodeThreadBindingLookupInput,
) -> Result<(), String> {
    let claim = store.begin_archive(lookup)?;
    let archived = store.complete_lifecycle_transition(&claim)?;
    if archived.status != CodeThreadLifecycleStatus::Archived {
        return Err("test binding did not reach Archived".to_string());
    }
    Ok(())
}

fn claim_with_id(
    store: &CodeThreadBindingStore,
    input: &CodeWorktreeRemovalClaimInput,
    removal_id: &str,
) -> Result<CodeWorktreeRemovalRecord, String> {
    store.get_or_claim_worktree_removal_with_save(input, removal_id.to_string(), |index| {
        store.save(index)
    })
}

fn write_json(path: &Path, value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    fs::write(path, &bytes).map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

fn object_keys(value: &Value) -> Result<BTreeSet<String>, String> {
    value
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .ok_or_else(|| "expected JSON object".to_string())
}

fn json_with_duplicate_removals(value: &Value) -> Result<Vec<u8>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "binding index must be a JSON object".to_string())?;
    let removals = object
        .get("removals")
        .ok_or_else(|| "binding index is missing removals".to_string())?;
    let mut members = object
        .iter()
        .filter(|(key, _)| key.as_str() != "removals")
        .map(|(key, value)| {
            Ok(format!(
                "{}:{}",
                serde_json::to_string(key).map_err(|error| error.to_string())?,
                serde_json::to_string(value).map_err(|error| error.to_string())?
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let removals = serde_json::to_string(removals).map_err(|error| error.to_string())?;
    members.push(format!("\"removals\":{removals}"));
    members.push(format!("\"removals\":{removals}"));
    Ok(format!("{{{}}}\n", members.join(",")).into_bytes())
}

fn snapshot_tree(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    fn visit(
        root: &Path,
        current: &Path,
        output: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<(), String> {
        let mut entries = fs::read_dir(current)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .into_owned();
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink() {
                output.insert(
                    format!("symlink:{relative}"),
                    fs::read_link(&path)
                        .map_err(|error| error.to_string())?
                        .to_string_lossy()
                        .into_owned()
                        .into_bytes(),
                );
            } else if metadata.is_dir() {
                output.insert(format!("dir:{relative}"), Vec::new());
                visit(root, &path, output)?;
            } else if metadata.is_file() {
                output.insert(
                    format!("file:{relative}"),
                    fs::read(&path).map_err(|error| error.to_string())?,
                );
            } else {
                output.insert(format!("special:{relative}"), Vec::new());
            }
        }
        Ok(())
    }

    let mut output = BTreeMap::new();
    visit(root, root, &mut output)?;
    Ok(output)
}

#[test]
fn v4_removal_wire_is_strict_tagged_and_preserves_malformed_bytes() -> Result<(), String> {
    let (_directory, store, seed) = seed_active()?;
    archive(&store, &seed.lookup)?;
    let claimed = claim_with_id(&store, &seed.claim, REMOVAL_ID)?;
    assert!(matches!(claimed, CodeWorktreeRemovalRecord::Claimed(_)));

    let valid_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let valid: Value = serde_json::from_slice(&valid_bytes).map_err(|error| error.to_string())?;
    let removal = &valid["removals"][0];
    assert_eq!(
        object_keys(removal)?,
        BTreeSet::from([
            "binding".to_string(),
            "executionDisposition".to_string(),
            "mergeProof".to_string(),
            "physical".to_string(),
            "physicalManifestDigest".to_string(),
            "removalId".to_string(),
            "state".to_string(),
            "threadLifecycleAtClaim".to_string(),
            "transcriptDisposition".to_string(),
        ])
    );
    assert_eq!(removal["state"], "claimed");
    assert_eq!(removal["threadLifecycleAtClaim"], "archived");
    assert_eq!(removal["transcriptDisposition"], "preserved");
    assert_eq!(removal["executionDisposition"], "removed");
    assert_eq!(
        removal["binding"].as_object().map(|value| value.len()),
        Some(8)
    );
    assert_eq!(
        object_keys(&removal["mergeProof"])?,
        BTreeSet::from([
            "headCommit".to_string(),
            "repositoryIdentity".to_string(),
            "targetCommit".to_string(),
            "targetRef".to_string(),
            "worktreeId".to_string(),
        ])
    );
    assert_eq!(
        object_keys(&removal["physical"])?,
        BTreeSet::from([
            "gitAdminEntry".to_string(),
            "gitAdminParent".to_string(),
            "managedRoot".to_string(),
            "managedRootParent".to_string(),
            "quarantineName".to_string(),
        ])
    );
    assert_eq!(store.load()?.removals, vec![claimed]);

    let mut fixtures = Vec::new();
    let mut unknown_top = valid.clone();
    unknown_top["removals"][0]["unexpected"] = json!(true);
    fixtures.push(unknown_top);
    let mut unknown_state = valid.clone();
    unknown_state["removals"][0]["state"] = json!("paused");
    fixtures.push(unknown_state);
    let mut missing_state = valid.clone();
    missing_state["removals"][0]
        .as_object_mut()
        .ok_or_else(|| "removal must be an object".to_string())?
        .remove("state");
    fixtures.push(missing_state);
    let mut missing_binding = valid.clone();
    missing_binding["removals"][0]
        .as_object_mut()
        .ok_or_else(|| "removal must be an object".to_string())?
        .remove("binding");
    fixtures.push(missing_binding);
    let mut bad_id = valid.clone();
    bad_id["removals"][0]["removalId"] = json!("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA");
    fixtures.push(bad_id);
    let mut canonical_non_v4_id = valid.clone();
    canonical_non_v4_id["removals"][0]["removalId"] = json!("aaaaaaaa-aaaa-1aaa-8aaa-aaaaaaaaaaaa");
    fixtures.push(canonical_non_v4_id);
    let mut bad_lifecycle = valid.clone();
    bad_lifecycle["removals"][0]["threadLifecycleAtClaim"] = json!("active");
    fixtures.push(bad_lifecycle);
    let mut bad_transcript = valid.clone();
    bad_transcript["removals"][0]["transcriptDisposition"] = json!("deleted");
    fixtures.push(bad_transcript);
    let mut bad_execution = valid.clone();
    bad_execution["removals"][0]["executionDisposition"] = json!("available");
    fixtures.push(bad_execution);
    let mut bad_digest = valid.clone();
    bad_digest["removals"][0]["physicalManifestDigest"] = json!("not-a-digest");
    fixtures.push(bad_digest);
    let mut bad_ref = valid.clone();
    bad_ref["removals"][0]["mergeProof"]["targetRef"] = json!("origin/main");
    fixtures.push(bad_ref);
    let mut mixed_object_format = valid.clone();
    mixed_object_format["removals"][0]["mergeProof"]["targetCommit"] = json!("2".repeat(64));
    fixtures.push(mixed_object_format);
    let mut bad_quarantine = valid.clone();
    bad_quarantine["removals"][0]["physical"]["quarantineName"] = json!("other");
    fixtures.push(bad_quarantine);
    let mut duplicate = valid.clone();
    let duplicate_record = duplicate["removals"][0].clone();
    duplicate["removals"]
        .as_array_mut()
        .ok_or_else(|| "removals must be an array".to_string())?
        .push(duplicate_record);
    fixtures.push(duplicate);
    let mut active_pending = valid.clone();
    active_pending["lifecycles"][0]["lifecycle"] = json!({ "state": "active" });
    fixtures.push(active_pending);
    let mut missing_live = valid.clone();
    missing_live["bindings"] = json!([]);
    missing_live["lifecycles"] = json!([]);
    missing_live["mergeTargets"] = json!([]);
    fixtures.push(missing_live);

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

    let valid_text = String::from_utf8(valid_bytes).map_err(|error| error.to_string())?;
    let duplicate_members = [
        valid_text.replacen(
            "\"state\": \"claimed\",",
            "\"state\": \"claimed\",\n      \"state\": \"claimed\",",
            1,
        ),
        valid_text.replacen(
            &format!("\"headCommit\": \"{}\",", "1".repeat(40)),
            &format!(
                "\"headCommit\": \"{}\",\n        \"headCommit\": \"{}\",",
                "1".repeat(40),
                "1".repeat(40)
            ),
            1,
        ),
    ];
    let mut duplicate_members = duplicate_members
        .map(String::into_bytes)
        .into_iter()
        .collect::<Vec<_>>();
    duplicate_members.push(json_with_duplicate_removals(&valid)?);
    for bytes in duplicate_members {
        fs::write(store.store_path(), &bytes).map_err(|error| error.to_string())?;
        assert!(store.load().is_err());
        assert_eq!(
            fs::read(store.store_path()).map_err(|error| error.to_string())?,
            bytes
        );
    }
    Ok(())
}

#[test]
fn claim_requires_archived_exact_authority_and_retry_never_retargets() -> Result<(), String> {
    let (_directory, store, seed) = seed_active()?;
    let before = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    assert!(claim_with_id(&store, &seed.claim, REMOVAL_ID).is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        before
    );

    archive(&store, &seed.lookup)?;
    let archived_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let mut wrong_authority = seed.claim.clone();
    wrong_authority.merge_proof.target_ref = "refs/heads/other".to_string();
    assert!(claim_with_id(&store, &wrong_authority, REMOVAL_ID).is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        archived_bytes
    );
    let claimed = store.get_or_claim_worktree_removal(&seed.claim)?;
    let removal_id = &claimed.authority().removal_id;
    let parsed_id = uuid::Uuid::parse_str(removal_id).map_err(|error| error.to_string())?;
    assert_eq!(parsed_id.get_version(), Some(uuid::Version::Random));
    assert_eq!(parsed_id.hyphenated().to_string(), *removal_id);
    let claimed_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let save_called = Cell::new(false);
    let retry = store.get_or_claim_worktree_removal_with_save(
        &seed.claim,
        SECOND_REMOVAL_ID.to_string(),
        |_| {
            save_called.set(true);
            Err("retry must not save".to_string())
        },
    )?;
    assert_eq!(retry, claimed);
    assert!(!save_called.get());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        claimed_bytes
    );
    assert!(store.begin_unarchive(&seed.lookup).is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        claimed_bytes
    );

    let mut changed = seed.claim.clone();
    changed.merge_proof.target_ref = "refs/heads/replacement".to_string();
    changed.physical_manifest_digest = "invalid-after-existing-claim".to_string();
    assert_eq!(
        store.get_or_claim_worktree_removal(&changed)?,
        claimed,
        "existing journal must remain authoritative instead of retargeting"
    );
    assert_eq!(store.list_pending_worktree_removals()?, vec![claimed]);
    Ok(())
}

#[test]
fn public_admission_fails_closed_for_shutdown_and_unreconciled_lifecycle() -> Result<(), String> {
    let (_directory, store, seed) = seed_active()?;
    archive(&store, &seed.lookup)?;
    let runtime = crate::code_workspace::CodeRuntime::new();
    let terminals = crate::code_workspace::CodeTerminalManager::new();
    let binding_lock = Mutex::new(());
    let input = CodeWorktreeRemoveInput {
        scope: seed.scope.clone(),
        thread_id: seed.lookup.codex_thread_id.clone(),
    };
    let before = fs::read(store.store_path()).map_err(|error| error.to_string())?;

    let shutdown_error = remove_archived_worktree(
        &store,
        binding_lock
            .lock()
            .map_err(|_| "test binding lock is unavailable".to_string())?,
        input.clone(),
        &seed.external_root,
        CodeWorktreeRemovalContext {
            runtime: &runtime,
            terminals: &terminals,
            lifecycle_authority_ready: &AtomicBool::new(true),
            shutdown_started: &AtomicBool::new(true),
        },
    )
    .expect_err("shutdown must reject a new physical removal");
    assert!(shutdown_error.contains("cannot start during app shutdown"));

    let lifecycle_error = remove_archived_worktree(
        &store,
        binding_lock
            .lock()
            .map_err(|_| "test binding lock is unavailable".to_string())?,
        input,
        &seed.external_root,
        CodeWorktreeRemovalContext {
            runtime: &runtime,
            terminals: &terminals,
            lifecycle_authority_ready: &AtomicBool::new(false),
            shutdown_started: &AtomicBool::new(false),
        },
    )
    .expect_err("unreconciled lifecycle authority must reject removal");
    assert!(lifecycle_error.contains("lifecycle authority is not ready"));
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        before
    );
    assert!(store.lookup_worktree_removal(&seed.lookup)?.is_none());
    Ok(())
}

#[test]
fn save_failure_and_response_loss_converge_at_claim_and_removing_boundaries() -> Result<(), String>
{
    let (_directory, store, seed) = seed_active()?;
    archive(&store, &seed.lookup)?;
    let external_before = snapshot_tree(&seed.external_root)?;
    let bytes_before = fs::read(store.store_path()).map_err(|error| error.to_string())?;

    let failed =
        store.get_or_claim_worktree_removal_with_save(&seed.claim, REMOVAL_ID.to_string(), |_| {
            Err("injected claim save failure".to_string())
        });
    assert!(failed.is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        bytes_before
    );
    assert!(store.lookup_worktree_removal(&seed.lookup)?.is_none());

    let response_lost = store.get_or_claim_worktree_removal_with_save(
        &seed.claim,
        REMOVAL_ID.to_string(),
        |index| {
            store.save(index)?;
            Err("claim response lost after commit".to_string())
        },
    );
    assert!(response_lost.is_err());
    let claimed = store
        .lookup_worktree_removal(&seed.lookup)?
        .ok_or_else(|| "committed claim is missing".to_string())?;
    assert_eq!(claimed.authority().removal_id, REMOVAL_ID);
    let claimed_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    assert_eq!(store.get_or_claim_worktree_removal(&seed.claim)?, claimed);
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        claimed_bytes
    );

    let failed_transition = store.mark_worktree_removal_removing_with_save(&claimed, |_| {
        Err("injected removing save failure".to_string())
    });
    assert!(failed_transition.is_err());
    assert_eq!(
        store.lookup_worktree_removal(&seed.lookup)?,
        Some(claimed.clone())
    );

    let response_lost = store.mark_worktree_removal_removing_with_save(&claimed, |index| {
        store.save(index)?;
        Err("removing response lost after commit".to_string())
    });
    assert!(response_lost.is_err());
    let removing = store
        .lookup_worktree_removal(&seed.lookup)?
        .ok_or_else(|| "committed removing state is missing".to_string())?;
    assert!(matches!(removing, CodeWorktreeRemovalRecord::Removing(_)));
    let save_called = Cell::new(false);
    assert_eq!(
        store.mark_worktree_removal_removing_with_save(&claimed, |_| {
            save_called.set(true);
            Err("idempotent retry must not save".to_string())
        })?,
        removing
    );
    assert!(!save_called.get());
    assert_eq!(snapshot_tree(&seed.external_root)?, external_before);
    Ok(())
}

#[test]
fn claimed_cancellation_is_exact_while_removing_is_sticky() -> Result<(), String> {
    let (_directory, store, seed) = seed_active()?;
    archive(&store, &seed.lookup)?;
    let claimed = claim_with_id(&store, &seed.claim, REMOVAL_ID)?;
    let before = fs::read(store.store_path()).map_err(|error| error.to_string())?;

    let mut forged_authority = claimed.authority().clone();
    forged_authority.merge_proof.target_commit = "3".repeat(40);
    let forged = CodeWorktreeRemovalRecord::Claimed(forged_authority);
    assert!(store
        .cancel_claimed_worktree_removal_definitely_not_started(&forged)
        .is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        before
    );

    let failed = store.cancel_claimed_worktree_removal_with_save(&claimed, |_| {
        Err("injected cancel save failure".to_string())
    });
    assert!(failed.is_err());
    assert_eq!(
        store.lookup_worktree_removal(&seed.lookup)?,
        Some(claimed.clone())
    );
    let response_lost = store.cancel_claimed_worktree_removal_with_save(&claimed, |index| {
        store.save(index)?;
        Err("cancel response lost after commit".to_string())
    });
    assert!(response_lost.is_err());
    store.cancel_claimed_worktree_removal_definitely_not_started(&claimed)?;
    assert!(store.lookup_worktree_removal(&seed.lookup)?.is_none());
    assert_eq!(store.lookup(&seed.lookup)?, Some(seed.binding.clone()));
    assert_eq!(
        store
            .lookup_with_lifecycle(&seed.lookup)?
            .map(|snapshot| snapshot.status),
        Some(CodeThreadLifecycleStatus::Archived)
    );

    let replacement = claim_with_id(&store, &seed.claim, SECOND_REMOVAL_ID)?;
    assert_ne!(
        replacement.authority().removal_id,
        claimed.authority().removal_id
    );
    assert!(store
        .cancel_claimed_worktree_removal_definitely_not_started(&claimed)
        .is_err());
    let removing = store.mark_worktree_removal_removing(&replacement)?;
    let sticky_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    assert!(store
        .cancel_claimed_worktree_removal_definitely_not_started(&removing)
        .is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        sticky_bytes
    );
    assert!(matches!(
        store.lookup_worktree_removal(&seed.lookup)?,
        Some(CodeWorktreeRemovalRecord::Removing(_))
    ));
    Ok(())
}

fn assert_unsupported_pending_recovery_is_zero_mutation(
    recover: impl Fn(&CodeThreadBindingStore, &Path) -> Result<(), String>,
) -> Result<(), String> {
    for removing in [false, true] {
        let (directory, store, seed) = seed_active()?;
        archive(&store, &seed.lookup)?;
        let claimed = claim_with_id(&store, &seed.claim, REMOVAL_ID)?;
        let pending = if removing {
            store.mark_worktree_removal_removing(&claimed)?
        } else {
            claimed
        };
        let untouched_nest = directory
            .path()
            .join("unsupported-nest-must-not-be-created");
        let store_before = fs::read(store.store_path()).map_err(|error| error.to_string())?;
        let store_modified_before = fs::metadata(store.store_path())
            .and_then(|metadata| metadata.modified())
            .map_err(|error| error.to_string())?;
        let fixture_before = snapshot_tree(directory.path())?;

        let error = recover(&store, &untouched_nest)
            .expect_err("unsupported-platform recovery must reject a pending removal");
        assert_eq!(
            error,
            "SchoolX Code pinned worktree removal is unsupported on this platform"
        );
        assert_eq!(
            fs::read(store.store_path()).map_err(|error| error.to_string())?,
            store_before,
            "unsupported recovery changed the binding store for removing={removing}"
        );
        assert_eq!(
            fs::metadata(store.store_path())
                .and_then(|metadata| metadata.modified())
                .map_err(|error| error.to_string())?,
            store_modified_before,
            "unsupported recovery rewrote the binding store for removing={removing}"
        );
        assert_eq!(
            snapshot_tree(directory.path())?,
            fixture_before,
            "unsupported recovery changed the fixture for removing={removing}"
        );
        assert_eq!(
            store.lookup_worktree_removal(&seed.lookup)?,
            Some(pending),
            "unsupported recovery changed the pending journal for removing={removing}"
        );
    }
    Ok(())
}

#[test]
fn unsupported_pending_recovery_helper_is_zero_mutation() -> Result<(), String> {
    assert_unsupported_pending_recovery_is_zero_mutation(|store, _nest_root| {
        super::physical::reject_unsupported_pending_worktree_removals(store)
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn unsupported_platform_pending_removal_recovery_is_zero_mutation() -> Result<(), String> {
    assert_unsupported_pending_recovery_is_zero_mutation(
        super::physical::recover_pending_worktree_removals,
    )
}

#[test]
fn final_save_faults_retain_or_atomically_retire_the_complete_live_join() -> Result<(), String> {
    let (_directory, store, seed) = seed_active()?;
    archive(&store, &seed.lookup)?;
    let external_before = snapshot_tree(&seed.external_root)?;
    let claimed = claim_with_id(&store, &seed.claim, REMOVAL_ID)?;
    let removing = store.mark_worktree_removal_removing(&claimed)?;
    let removing_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;

    let failed = store.finalize_worktree_removal_with_save(&removing, |_| {
        Err("injected final save failure".to_string())
    });
    assert!(failed.is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        removing_bytes
    );
    let before_retry = store.load()?;
    assert_eq!(before_retry.bindings, vec![seed.binding.clone()]);
    assert_eq!(before_retry.lifecycles.len(), 1);
    assert_eq!(before_retry.merge_targets.len(), 1);
    assert_eq!(before_retry.removals, vec![removing.clone()]);

    let response_lost = store.finalize_worktree_removal_with_save(&removing, |index| {
        store.save(index)?;
        Err("final response lost after commit".to_string())
    });
    assert!(response_lost.is_err());
    let removed = store
        .lookup_worktree_removal(&seed.lookup)?
        .ok_or_else(|| "removed tombstone is missing".to_string())?;
    assert!(matches!(removed, CodeWorktreeRemovalRecord::Removed(_)));
    assert_eq!(removed.authority(), removing.authority());
    let final_index = store.load()?;
    assert!(final_index.bindings.is_empty());
    assert!(final_index.lifecycles.is_empty());
    assert!(final_index.merge_targets.is_empty());
    assert_eq!(final_index.removals, vec![removed.clone()]);
    assert!(store.list_pending_worktree_removals()?.is_empty());

    let save_called = Cell::new(false);
    assert_eq!(
        store.finalize_worktree_removal_with_save(&removing, |_| {
            save_called.set(true);
            Err("removed retry must not save".to_string())
        })?,
        removed
    );
    assert!(!save_called.get());
    assert_eq!(snapshot_tree(&seed.external_root)?, external_before);
    Ok(())
}

#[test]
fn removed_tombstone_retry_returns_the_same_exact_receipt_without_runtime_or_cleanup(
) -> Result<(), String> {
    let (_directory, store, seed) = seed_active()?;
    archive(&store, &seed.lookup)?;
    let claimed = claim_with_id(&store, &seed.claim, REMOVAL_ID)?;
    let pending_inventory = crate::code_workspace::worktree_inventory::list_worktree_inventory(
        &store,
        &seed.external_root,
        &seed.scope,
    )?;
    assert_eq!(pending_inventory.len(), 1);
    assert!(pending_inventory[0].preserved);
    assert!(!pending_inventory[0].can_remove);
    assert!(pending_inventory[0].blockers.contains(
        &crate::code_workspace::worktree_inventory::CodeWorktreeInventoryBlocker::MergeProofUnavailable
    ));
    let removing = store.mark_worktree_removal_removing(&claimed)?;
    store.finalize_worktree_removal_after_test_verified_absence(&removing)?;
    fs::remove_file(
        seed.external_root
            .join("transcripts")
            .join("thread-removal.jsonl"),
    )
    .map_err(|error| error.to_string())?;
    let cleanup_marker = store
        .store_path()
        .parent()
        .ok_or_else(|| "binding store has no parent".to_string())?
        .join("removal-manifests-v1")
        .join(format!("{}.json", "d".repeat(64)));
    fs::create_dir(
        cleanup_marker
            .parent()
            .ok_or_else(|| "cleanup marker has no parent".to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::write(&cleanup_marker, b"invalid cleanup marker must remain\n")
        .map_err(|error| error.to_string())?;
    let cleanup_marker_before = fs::read(&cleanup_marker).map_err(|error| error.to_string())?;

    let runtime = crate::code_workspace::CodeRuntime::new();
    let terminals = crate::code_workspace::CodeTerminalManager::new();
    let binding_lock = Mutex::new(());
    let input = CodeWorktreeRemoveInput {
        scope: seed.scope.clone(),
        thread_id: seed.lookup.codex_thread_id.clone(),
    };
    let retry = |shutdown_started| {
        remove_archived_worktree(
            &store,
            binding_lock
                .lock()
                .map_err(|_| "test binding lock is unavailable".to_string())?,
            input.clone(),
            &seed.external_root,
            CodeWorktreeRemovalContext {
                runtime: &runtime,
                terminals: &terminals,
                lifecycle_authority_ready: &AtomicBool::new(false),
                shutdown_started: &AtomicBool::new(shutdown_started),
            },
        )
    };
    let first = retry(false)?;
    let second = retry(true)?;
    assert_eq!(first, second);
    assert_eq!(
        fs::read(&cleanup_marker).map_err(|error| error.to_string())?,
        cleanup_marker_before,
        "public tombstone receipt retry must not attempt physical cleanup"
    );
    assert_eq!(first.removal_id, REMOVAL_ID);
    assert_eq!(first.scope, seed.scope);
    assert_eq!(first.thread_id, seed.lookup.codex_thread_id);
    assert_eq!(first.worktree_id, WORKTREE_ID);
    assert_eq!(first.head_commit, "1".repeat(40));
    assert_eq!(first.merged_into_ref, "refs/heads/main");
    assert_eq!(first.merged_into_commit, "2".repeat(40));
    assert_eq!(
        first.transcript_disposition,
        CodeWorktreeTranscriptDisposition::Preserved
    );
    assert_eq!(
        first.execution_disposition,
        CodeWorktreeExecutionDisposition::Removed
    );
    assert_eq!(
        object_keys(&serde_json::to_value(first).map_err(|error| error.to_string())?)?,
        [
            "executionDisposition",
            "headCommit",
            "mergedIntoCommit",
            "mergedIntoRef",
            "removalId",
            "scope",
            "threadId",
            "transcriptDisposition",
            "worktreeId",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );
    Ok(())
}

#[test]
fn test_absence_seam_preserves_the_opaque_finalization_cas_contract() -> Result<(), String> {
    let (_directory, store, seed) = seed_active()?;
    archive(&store, &seed.lookup)?;
    let claimed = claim_with_id(&store, &seed.claim, REMOVAL_ID)?;
    let claimed_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;

    assert!(store
        .finalize_worktree_removal_after_test_verified_absence(&claimed)
        .is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        claimed_bytes
    );

    let removing = store.mark_worktree_removal_removing(&claimed)?;
    let removing_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let mut forged_authority = removing.authority().clone();
    forged_authority.merge_proof.target_commit = "3".repeat(40);
    let forged_removing = CodeWorktreeRemovalRecord::Removing(forged_authority);
    assert!(store
        .finalize_worktree_removal_after_test_verified_absence(&forged_removing)
        .is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        removing_bytes
    );

    let removed = store.finalize_worktree_removal_after_test_verified_absence(&removing)?;
    assert!(matches!(removed, CodeWorktreeRemovalRecord::Removed(_)));
    assert_eq!(removed.authority(), removing.authority());

    let removed_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    assert_eq!(
        store.finalize_worktree_removal_after_test_verified_absence(&removing)?,
        removed,
        "a lost finalization response must converge on the durable tombstone"
    );
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        removed_bytes
    );

    let index = store.load()?;
    assert!(index.bindings.is_empty());
    assert!(index.lifecycles.is_empty());
    assert!(index.merge_targets.is_empty());
    assert_eq!(index.removals, vec![removed]);
    Ok(())
}

#[test]
fn removed_tombstone_is_non_executable_and_permanently_reserves_all_identities(
) -> Result<(), String> {
    let (_directory, store, seed) = seed_active()?;
    archive(&store, &seed.lookup)?;
    let claimed = claim_with_id(&store, &seed.claim, REMOVAL_ID)?;
    let removing = store.mark_worktree_removal_removing(&claimed)?;
    let removed = store.finalize_worktree_removal_after_test_verified_absence(&removing)?;
    assert!(matches!(removed, CodeWorktreeRemovalRecord::Removed(_)));
    assert!(
        crate::code_workspace::worktree_inventory::list_worktree_inventory(
            &store,
            &seed.external_root,
            &seed.scope,
        )?
        .is_empty()
    );

    assert!(store.lookup(&seed.lookup)?.is_none());
    assert!(store.lookup_with_lifecycle(&seed.lookup)?.is_none());
    assert!(store.require_active_binding(&seed.lookup).is_err());
    assert!(store
        .ensure_thread_unbound(&seed.lookup.codex_thread_id)
        .is_err());
    assert!(store
        .ensure_fork_source_available(&seed.scope, &seed.lookup.codex_thread_id)
        .is_err());
    assert!(store
        .load()?
        .reserved_thread_ids()
        .contains(&seed.lookup.codex_thread_id));
    assert!(store.upsert(seed.binding.clone()).is_err());
    assert!(store
        .create_preparation(
            "55555555-5555-4555-8555-555555555555".to_string(),
            seed.scope.clone(),
            &seed.descriptor,
        )
        .is_err());
    let local_reuse = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Local,
        repository_identity: seed.scope.repository_identity.clone(),
        execution_root: seed.descriptor.execution_root.clone(),
        base_ref: seed.descriptor.base_ref.clone(),
        worktree_id: None,
    };
    assert!(store
        .create_preparation(
            "88888888-8888-4888-8888-888888888888".to_string(),
            seed.scope.clone(),
            &local_reuse,
        )
        .is_err());
    assert!(store
        .ensure_execution_available(&super::super::CodeExecutionAvailabilityInput {
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: seed.descriptor.execution_root.clone(),
            worktree_id: seed.descriptor.worktree_id.clone(),
        })
        .is_err());

    let new_worktree_id = "66666666-6666-4666-8666-666666666666";
    let new_root = seed
        .external_root
        .join("WORKTREES")
        .join(&seed.scope.repository_identity)
        .join(new_worktree_id);
    fs::create_dir(&new_root).map_err(|error| error.to_string())?;
    let new_descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: seed.scope.repository_identity.clone(),
        execution_root: new_root
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned(),
        base_ref: "0".repeat(40),
        worktree_id: Some(new_worktree_id.to_string()),
    };
    let new_preparation_id = "77777777-7777-4777-8777-777777777777";
    assert!(store
        .create_fork_preparation(
            "99999999-9999-4999-8999-999999999999".to_string(),
            seed.scope.clone(),
            seed.lookup.codex_thread_id.clone(),
            &new_descriptor,
        )
        .is_err());
    store.create_preparation(
        new_preparation_id.to_string(),
        seed.scope.clone(),
        &new_descriptor,
    )?;
    store.claim_preparation_for_start(&seed.scope, new_preparation_id, Vec::new())?;
    assert!(store
        .commit_preparation_binding(
            &seed.scope,
            new_preparation_id,
            &seed.lookup.codex_thread_id,
        )
        .is_err());
    assert_eq!(
        store.preparation(&seed.scope, new_preparation_id)?.state,
        CodeThreadPreparationState::Starting
    );

    let forged_live = {
        let mut value = read_json(store.store_path())?;
        value["preparations"] = json!([]);
        value["bindings"] = json!([seed.binding]);
        let mut lifecycle = json!({
            "communityId": seed.scope.community_id,
            "projectDtag": seed.scope.project_dtag,
            "repositoryIdentity": seed.scope.repository_identity,
            "codexThreadId": seed.lookup.codex_thread_id,
            "lifecycle": { "state": "archived" }
        });
        value["lifecycles"] = json!([lifecycle.take()]);
        value
    };
    let bytes = write_json(store.store_path(), &forged_live)?;
    assert!(store.load().is_err());
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        bytes
    );
    Ok(())
}
