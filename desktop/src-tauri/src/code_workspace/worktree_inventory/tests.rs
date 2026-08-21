use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use super::*;
use crate::code_workspace::bindings::{
    CodeExecutionMode, CodeThreadBinding, CodeThreadBindingLookupInput, CodeThreadBindingScope,
    CodeThreadBindingStore, CodeThreadLifecycleStatus, CodeThreadPreparationOperation,
    CodeThreadPreparationState,
};
use crate::code_workspace::worktrees::{
    preflight_execution_root, prepare_execution_root, prepare_execution_root_with_merge_target,
    CodeWorktreeDescriptor, CodeWorktreePrepareInput, CodeWorktreeStatus,
};

struct TestRepository {
    _directory: tempfile::TempDir,
    root: PathBuf,
    base_commit: String,
    identity: String,
    common_dir: PathBuf,
}

#[derive(Debug, Eq, PartialEq)]
struct TreeEntrySnapshot {
    kind: &'static str,
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    mode: u32,
    bytes: Vec<u8>,
}

fn run_git(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| format!("failed to start test git: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn git_line(root: &Path, arguments: &[&str]) -> Result<String, String> {
    String::from_utf8(run_git(root, arguments)?)
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("test git output was not UTF-8: {error}"))
}

fn repository() -> Result<TestRepository, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = directory.path().join("repository");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    run_git(&root, &["init", "--initial-branch=main"])?;
    run_git(&root, &["config", "user.name", "SchoolX Test"])?;
    run_git(&root, &["config", "user.email", "schoolx@example.invalid"])?;
    fs::write(root.join("tracked.txt"), b"base\n").map_err(|error| error.to_string())?;
    run_git(&root, &["add", "tracked.txt"])?;
    run_git(&root, &["commit", "-m", "base"])?;
    let base_commit = git_line(&root, &["rev-parse", "HEAD"])?;
    let descriptor = preflight_execution_root(
        root.to_str()
            .ok_or_else(|| "test repository path was not UTF-8".to_string())?,
        &base_commit,
    )?;
    Ok(TestRepository {
        common_dir: PathBuf::from(descriptor.git_common_dir),
        identity: descriptor.repository_identity,
        _directory: directory,
        root: PathBuf::from(descriptor.repository_root),
        base_commit,
    })
}

fn scope(repository: &TestRepository, community: &str) -> CodeThreadBindingScope {
    CodeThreadBindingScope {
        community_id: community.to_string(),
        project_dtag: "project-a".to_string(),
        repository_identity: repository.identity.clone(),
    }
}

fn managed_descriptor(
    repository: &TestRepository,
    nest_root: &Path,
) -> Result<CodeWorktreeDescriptor, String> {
    Ok(prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: repository.root.to_string_lossy().into_owned(),
            base_ref: repository.base_commit.clone(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest_root,
    )?
    .descriptor)
}

fn binding(
    scope: &CodeThreadBindingScope,
    thread_id: &str,
    descriptor: &CodeWorktreeDescriptor,
) -> CodeThreadBinding {
    CodeThreadBinding {
        community_id: scope.community_id.clone(),
        project_dtag: scope.project_dtag.clone(),
        repository_identity: scope.repository_identity.clone(),
        codex_thread_id: thread_id.to_string(),
        execution_mode: descriptor.execution_mode,
        execution_root: descriptor.execution_root.clone(),
        base_ref: descriptor.base_ref.clone(),
        worktree_id: descriptor.worktree_id.clone(),
    }
}

fn local_descriptor(repository: &TestRepository) -> CodeWorktreeDescriptor {
    CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Local,
        repository_identity: repository.identity.clone(),
        execution_root: repository.root.to_string_lossy().into_owned(),
        base_ref: repository.base_commit.clone(),
        worktree_id: None,
    }
}

fn archive_binding(
    store: &CodeThreadBindingStore,
    scope: &CodeThreadBindingScope,
    thread_id: &str,
) -> Result<(), String> {
    let lookup = CodeThreadBindingLookupInput {
        scope: scope.clone(),
        codex_thread_id: thread_id.to_string(),
    };
    let claim = store.begin_archive(&lookup)?;
    store.complete_lifecycle_transition(&claim)?;
    Ok(())
}

fn snapshot_tree(root: &Path) -> Result<BTreeMap<PathBuf, TreeEntrySnapshot>, String> {
    fn visit(
        root: &Path,
        current: &Path,
        entries: &mut BTreeMap<PathBuf, TreeEntrySnapshot>,
    ) -> Result<(), String> {
        let metadata = fs::symlink_metadata(current).map_err(|error| {
            format!(
                "failed to inspect snapshot path {}: {error}",
                current.display()
            )
        })?;
        let relative = current
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_path_buf();
        let (kind, bytes) = if metadata.file_type().is_symlink() {
            (
                "symlink",
                fs::read_link(current)
                    .map_err(|error| error.to_string())?
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec(),
            )
        } else if metadata.is_file() {
            (
                "file",
                fs::read(current).map_err(|error| error.to_string())?,
            )
        } else if metadata.is_dir() {
            ("directory", Vec::new())
        } else {
            ("other", Vec::new())
        };
        entries.insert(
            relative,
            TreeEntrySnapshot {
                kind,
                len: metadata.len(),
                modified: metadata.modified().ok(),
                #[cfg(unix)]
                mode: {
                    use std::os::unix::fs::MetadataExt as _;
                    metadata.mode()
                },
                bytes,
            },
        );
        if metadata.is_dir() {
            let mut children = fs::read_dir(current)
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            children.sort_by_key(fs::DirEntry::file_name);
            for child in children {
                visit(root, &child.path(), entries)?;
            }
        }
        Ok(())
    }

    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries)?;
    Ok(entries)
}

fn available_status(
    descriptor: CodeWorktreeDescriptor,
    head_commit: &str,
    branch: Option<&str>,
    dirty: bool,
) -> CodeWorktreeStatus {
    CodeWorktreeStatus {
        descriptor,
        head_commit: head_commit.to_string(),
        branch: branch.map(str::to_string),
        dirty,
    }
}

#[test]
fn worktree_inventory_projects_closed_blockers_without_removal_authority() -> Result<(), String> {
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: "a".repeat(64),
        execution_root: "/native/managed".to_string(),
        base_ref: "b".repeat(40),
        worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
    };
    let owner = CodeThreadBindingScope {
        community_id: "community-a".to_string(),
        project_dtag: "project-a".to_string(),
        repository_identity: descriptor.repository_identity.clone(),
    };
    let bound = binding(&owner, "thread-a", &descriptor);
    let clean = available_status(descriptor.clone(), &descriptor.base_ref, None, false);

    for lifecycle in [
        CodeThreadLifecycleStatus::Archiving,
        CodeThreadLifecycleStatus::Unarchiving,
        CodeThreadLifecycleStatus::Unknown,
    ] {
        let row = project_binding_row(
            bound.clone(),
            lifecycle,
            Ok(clean.clone()),
            InventoryMergeProof::NotRequired,
        );
        assert_eq!(
            row.blockers,
            vec![CodeWorktreeInventoryBlocker::LifecycleUnsettled]
        );
        assert!(row.preserved);
        assert!(!row.can_remove);
    }

    let active = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Active,
        Ok(clean.clone()),
        InventoryMergeProof::NotRequired,
    );
    assert_eq!(
        active.blockers,
        vec![CodeWorktreeInventoryBlocker::ActiveBinding]
    );

    let archived = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Ok(clean.clone()),
        InventoryMergeProof::Unavailable,
    );
    assert_eq!(
        archived.blockers,
        vec![CodeWorktreeInventoryBlocker::MergeProofUnavailable]
    );
    assert!(!archived.can_remove);

    let eligible = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Ok(clean.clone()),
        InventoryMergeProof::Proven,
    );
    assert!(eligible.blockers.is_empty());
    assert!(eligible.preserved);
    assert!(eligible.can_remove);

    let merged_head_drift = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Ok(available_status(
            descriptor.clone(),
            &"c".repeat(40),
            None,
            false,
        )),
        InventoryMergeProof::Proven,
    );
    assert!(merged_head_drift.blockers.is_empty());
    assert!(merged_head_drift.can_remove);

    let changed = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Ok(available_status(
            descriptor.clone(),
            &"c".repeat(40),
            Some("topic"),
            true,
        )),
        InventoryMergeProof::Unavailable,
    );
    assert_eq!(
        changed.blockers,
        vec![
            CodeWorktreeInventoryBlocker::DirtyRoot,
            CodeWorktreeInventoryBlocker::BranchAttached,
            CodeWorktreeInventoryBlocker::HeadDrift,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ]
    );

    let unavailable = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Err("missing managed root".to_string()),
        InventoryMergeProof::Unavailable,
    );
    assert_eq!(
        unavailable.blockers,
        vec![
            CodeWorktreeInventoryBlocker::UnavailableRoot,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ]
    );
    assert!(matches!(
        unavailable.inspection,
        CodeWorktreeInspection::Unavailable { .. }
    ));

    let oversized = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Err("가".repeat(MAX_INVENTORY_ERROR_BYTES)),
        InventoryMergeProof::Unavailable,
    );
    let CodeWorktreeInspection::Unavailable { error } = oversized.inspection else {
        return Err("oversized inspection error remained available".to_string());
    };
    assert!(error.len() <= MAX_INVENTORY_ERROR_BYTES);
    assert!(error.ends_with(TRUNCATED_ERROR_SUFFIX));

    let mut local = bound;
    local.execution_mode = CodeExecutionMode::Local;
    local.worktree_id = None;
    let local_status = available_status(
        CodeWorktreeDescriptor {
            execution_mode: CodeExecutionMode::Local,
            repository_identity: local.repository_identity.clone(),
            execution_root: local.execution_root.clone(),
            base_ref: local.base_ref.clone(),
            worktree_id: None,
        },
        &local.base_ref,
        Some("main"),
        false,
    );
    let local = project_binding_row(
        local,
        CodeThreadLifecycleStatus::Archived,
        Ok(local_status),
        InventoryMergeProof::Unavailable,
    );
    assert!(local
        .blockers
        .contains(&CodeWorktreeInventoryBlocker::LocalCheckout));
    Ok(())
}

#[test]
fn worktree_inventory_lists_only_exact_scope_managed_authority() -> Result<(), String> {
    let repository = repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let owner = scope(&repository, "community-a");
    let foreign = scope(&repository, "community-b");

    let source = managed_descriptor(&repository, nest.path())?;
    store.upsert(binding(&owner, "thread-source", &source))?;
    let prepared = managed_descriptor(&repository, nest.path())?;
    store.create_preparation(
        "11111111-1111-4111-8111-111111111111".to_string(),
        owner.clone(),
        &prepared,
    )?;
    let forked = managed_descriptor(&repository, nest.path())?;
    store.create_fork_preparation(
        "22222222-2222-4222-8222-222222222222".to_string(),
        owner.clone(),
        "thread-source".to_string(),
        &forked,
    )?;
    let foreign_root = managed_descriptor(&repository, nest.path())?;
    store.upsert(binding(&foreign, "thread-foreign", &foreign_root))?;
    let local = local_descriptor(&repository);
    store.upsert(binding(&owner, "thread-local", &local))?;
    store.create_preparation(
        "33333333-3333-4333-8333-333333333333".to_string(),
        owner.clone(),
        &local,
    )?;
    let _unbound_managed_root = managed_descriptor(&repository, nest.path())?;
    let arbitrary_linked = nest.path().join("arbitrary-linked");
    run_git(
        &repository.root,
        &[
            "worktree",
            "add",
            "--detach",
            arbitrary_linked
                .to_str()
                .ok_or_else(|| "linked path was not UTF-8".to_string())?,
            "HEAD",
        ],
    )?;

    let rows = list_worktree_inventory(&store, nest.path(), &owner)?;
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| {
        row.scope == owner
            && row.descriptor.execution_mode == CodeExecutionMode::Worktree
            && row.descriptor.worktree_id.is_some()
            && row.preserved
            && !row.can_remove
    }));
    let authorities = rows
        .iter()
        .map(|row| match &row.authority {
            CodeWorktreeInventoryAuthority::Binding { thread_id, .. } => {
                format!("binding:{thread_id}")
            }
            CodeWorktreeInventoryAuthority::Preparation {
                preparation_id,
                operation,
                ..
            } => format!("preparation:{operation:?}:{preparation_id}"),
        })
        .collect::<Vec<_>>();
    assert!(authorities
        .iter()
        .any(|value| value == "binding:thread-source"));
    assert!(authorities
        .iter()
        .any(|value| value.contains("Start:11111111-1111-4111-8111-111111111111")));
    assert!(authorities
        .iter()
        .any(|value| value.contains("Fork:22222222-2222-4222-8222-222222222222")));
    assert!(rows.iter().all(|row| {
        row.descriptor.execution_root != local.execution_root
            && row.descriptor.execution_root != arbitrary_linked.to_string_lossy()
    }));

    let foreign_rows = list_worktree_inventory(&store, nest.path(), &foreign)?;
    assert_eq!(foreign_rows.len(), 1);
    assert!(matches!(
        &foreign_rows[0].authority,
        CodeWorktreeInventoryAuthority::Binding { thread_id, .. }
            if thread_id == "thread-foreign"
    ));
    Ok(())
}

#[test]
fn worktree_inventory_reads_persisted_five_state_and_starting_authority() -> Result<(), String> {
    let repository = repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let owner = scope(&repository, "community-a");

    let active = managed_descriptor(&repository, nest.path())?;
    store.upsert(binding(&owner, "thread-active", &active))?;

    let archiving = managed_descriptor(&repository, nest.path())?;
    store.upsert(binding(&owner, "thread-archiving", &archiving))?;
    let archiving_lookup = CodeThreadBindingLookupInput {
        scope: owner.clone(),
        codex_thread_id: "thread-archiving".to_string(),
    };
    store.begin_archive(&archiving_lookup)?;

    let archived = managed_descriptor(&repository, nest.path())?;
    store.upsert(binding(&owner, "thread-archived", &archived))?;
    archive_binding(&store, &owner, "thread-archived")?;

    let unarchiving = managed_descriptor(&repository, nest.path())?;
    store.upsert(binding(&owner, "thread-unarchiving", &unarchiving))?;
    archive_binding(&store, &owner, "thread-unarchiving")?;
    let unarchiving_lookup = CodeThreadBindingLookupInput {
        scope: owner.clone(),
        codex_thread_id: "thread-unarchiving".to_string(),
    };
    store.begin_unarchive(&unarchiving_lookup)?;

    let unknown = managed_descriptor(&repository, nest.path())?;
    store.upsert(binding(&owner, "thread-unknown", &unknown))?;
    let unknown_lookup = CodeThreadBindingLookupInput {
        scope: owner.clone(),
        codex_thread_id: "thread-unknown".to_string(),
    };
    let unknown_claim = store.begin_archive(&unknown_lookup)?;
    store.mark_lifecycle_unknown(&unknown_claim)?;

    let starting = managed_descriptor(&repository, nest.path())?;
    let preparation_id = "44444444-4444-4444-8444-444444444444";
    store.create_preparation(preparation_id.to_string(), owner.clone(), &starting)?;
    store.claim_preparation_for_start(&owner, preparation_id, Vec::new())?;

    let rows = list_worktree_inventory(&store, nest.path(), &owner)?;
    assert_eq!(rows.len(), 6);
    let bindings = rows
        .iter()
        .filter_map(|row| match &row.authority {
            CodeWorktreeInventoryAuthority::Binding {
                thread_id,
                lifecycle,
            } => Some((thread_id.as_str(), (*lifecycle, row.blockers.as_slice()))),
            CodeWorktreeInventoryAuthority::Preparation { .. } => None,
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        bindings["thread-active"],
        (
            CodeThreadLifecycleStatus::Active,
            [CodeWorktreeInventoryBlocker::ActiveBinding].as_slice(),
        )
    );
    for (thread_id, lifecycle) in [
        ("thread-archiving", CodeThreadLifecycleStatus::Archiving),
        ("thread-unarchiving", CodeThreadLifecycleStatus::Unarchiving),
        ("thread-unknown", CodeThreadLifecycleStatus::Unknown),
    ] {
        assert_eq!(
            bindings[thread_id],
            (
                lifecycle,
                [CodeWorktreeInventoryBlocker::LifecycleUnsettled].as_slice(),
            )
        );
    }
    assert_eq!(
        bindings["thread-archived"],
        (
            CodeThreadLifecycleStatus::Archived,
            [CodeWorktreeInventoryBlocker::MergeProofUnavailable].as_slice(),
        )
    );
    let preparation = rows
        .iter()
        .find(|row| {
            matches!(
                row.authority,
                CodeWorktreeInventoryAuthority::Preparation { .. }
            )
        })
        .ok_or_else(|| "starting preparation inventory row was missing".to_string())?;
    assert!(matches!(
        preparation.authority,
        CodeWorktreeInventoryAuthority::Preparation {
            operation: CodeThreadPreparationOperation::Start,
            state: CodeThreadPreparationState::Starting,
            ..
        }
    ));
    assert_eq!(
        preparation.blockers,
        vec![CodeWorktreeInventoryBlocker::UnfinishedPreparation]
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn archived_inventory_opens_only_for_stable_native_merge_proof_and_resolves_head_drift(
) -> Result<(), String> {
    let repository = repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let owner = scope(&repository, "community-a");
    let prepared = prepare_execution_root_with_merge_target(
        CodeWorktreePrepareInput {
            repository_root: repository.root.to_string_lossy().into_owned(),
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    let descriptor = prepared.worktree.descriptor;
    let target_ref = prepared
        .merge_target_ref
        .ok_or_else(|| "test preparation did not capture native merge authority".to_string())?;
    let preparation_id = "55555555-5555-4555-8555-555555555555";
    store.create_preparation_with_merge_target(
        preparation_id.to_string(),
        owner.clone(),
        &descriptor,
        Some(target_ref),
    )?;
    store.claim_preparation_for_start(&owner, preparation_id, Vec::new())?;
    store.commit_preparation_binding(&owner, preparation_id, "thread-eligible")?;
    archive_binding(&store, &owner, "thread-eligible")?;

    let initially_eligible = list_worktree_inventory(&store, nest.path(), &owner)?;
    assert_eq!(initially_eligible.len(), 1);
    assert!(initially_eligible[0].preserved);
    assert!(initially_eligible[0].can_remove);
    assert!(initially_eligible[0].blockers.is_empty());

    fs::write(
        Path::new(&descriptor.execution_root).join("tracked.txt"),
        b"task head\n",
    )
    .map_err(|error| error.to_string())?;
    run_git(
        Path::new(&descriptor.execution_root),
        &["add", "tracked.txt"],
    )?;
    run_git(
        Path::new(&descriptor.execution_root),
        &["commit", "-m", "task head"],
    )?;
    let task_head = git_line(
        Path::new(&descriptor.execution_root),
        &["rev-parse", "HEAD"],
    )?;

    let not_merged = list_worktree_inventory(&store, nest.path(), &owner)?;
    assert_eq!(
        not_merged[0].blockers,
        vec![
            CodeWorktreeInventoryBlocker::HeadDrift,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ]
    );
    assert!(!not_merged[0].can_remove);

    run_git(
        &repository.root,
        &["merge", "--no-ff", "-m", "merge task", &task_head],
    )?;
    let merged = list_worktree_inventory(&store, nest.path(), &owner)?;
    assert!(matches!(
        &merged[0].inspection,
        CodeWorktreeInspection::Available { head_commit, .. } if head_commit == &task_head
    ));
    assert!(merged[0].blockers.is_empty());
    assert!(merged[0].preserved);
    assert!(merged[0].can_remove);
    Ok(())
}

#[cfg(unix)]
#[test]
fn worktree_inventory_localizes_missing_symlink_and_identity_drift() -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let repository = repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let owner = scope(&repository, "community-a");
    let valid = managed_descriptor(&repository, nest.path())?;
    let missing = managed_descriptor(&repository, nest.path())?;
    let escaped = managed_descriptor(&repository, nest.path())?;
    let drifted = managed_descriptor(&repository, nest.path())?;
    for (thread_id, descriptor) in [
        ("thread-valid", &valid),
        ("thread-missing", &missing),
        ("thread-escaped", &escaped),
        ("thread-drifted", &drifted),
    ] {
        store.upsert(binding(&owner, thread_id, descriptor))?;
    }

    fs::rename(
        &missing.execution_root,
        format!("{}.preserved", missing.execution_root),
    )
    .map_err(|error| error.to_string())?;
    let escaped_preserved = format!("{}.preserved", escaped.execution_root);
    fs::rename(&escaped.execution_root, &escaped_preserved).map_err(|error| error.to_string())?;
    let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
    symlink(outside.path(), &escaped.execution_root).map_err(|error| error.to_string())?;
    fs::rename(
        &drifted.execution_root,
        format!("{}.preserved", drifted.execution_root),
    )
    .map_err(|error| error.to_string())?;
    fs::create_dir(&drifted.execution_root).map_err(|error| error.to_string())?;
    run_git(
        Path::new(&drifted.execution_root),
        &["init", "--initial-branch=main"],
    )?;

    let rows = list_worktree_inventory(&store, nest.path(), &owner)?;
    assert_eq!(rows.len(), 4);
    let by_thread = rows
        .into_iter()
        .map(|row| {
            let thread_id = match &row.authority {
                CodeWorktreeInventoryAuthority::Binding { thread_id, .. } => thread_id.clone(),
                _ => return Err("expected only binding rows".to_string()),
            };
            Ok((thread_id, row))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    assert!(matches!(
        by_thread["thread-valid"].inspection,
        CodeWorktreeInspection::Available { .. }
    ));
    for thread_id in ["thread-missing", "thread-escaped", "thread-drifted"] {
        let row = &by_thread[thread_id];
        assert!(matches!(
            row.inspection,
            CodeWorktreeInspection::Unavailable { .. }
        ));
        assert!(row
            .blockers
            .contains(&CodeWorktreeInventoryBlocker::UnavailableRoot));
        assert!(row.preserved);
        assert!(!row.can_remove);
    }
    Ok(())
}

#[test]
fn worktree_inventory_has_zero_mutation() -> Result<(), String> {
    let repository = repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let owner = scope(&repository, "community-a");
    let archived = managed_descriptor(&repository, nest.path())?;
    let prepared = managed_descriptor(&repository, nest.path())?;
    store.upsert(binding(&owner, "thread-archived", &archived))?;
    archive_binding(&store, &owner, "thread-archived")?;
    store.create_preparation(
        "11111111-1111-4111-8111-111111111111".to_string(),
        owner.clone(),
        &prepared,
    )?;

    let index_before = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let index_mtime_before = fs::metadata(store.store_path())
        .and_then(|metadata| metadata.modified())
        .map_err(|error| error.to_string())?;
    let app_data_before = snapshot_tree(app_data.path())?;
    let admin_before = snapshot_tree(&repository.common_dir)?;
    let archived_before = snapshot_tree(Path::new(&archived.execution_root))?;
    let prepared_before = snapshot_tree(Path::new(&prepared.execution_root))?;
    let lifecycle_before = store.list_with_lifecycle(&owner)?;
    let preparations_before = store.list_preparations(&owner)?;

    let read_store = CodeThreadBindingStore::for_app_data_read_only(app_data.path())?
        .ok_or_else(|| "existing binding store disappeared".to_string())?;
    let rows = list_worktree_inventory(&read_store, nest.path(), &owner)?;
    assert_eq!(rows.len(), 2);
    let archived_row = rows
        .iter()
        .find(|row| {
            matches!(
                &row.authority,
                CodeWorktreeInventoryAuthority::Binding { thread_id, .. }
                    if thread_id == "thread-archived"
            )
        })
        .ok_or_else(|| "archived inventory row was missing".to_string())?;
    assert_eq!(
        archived_row.inspection,
        CodeWorktreeInspection::Available {
            head_commit: archived.base_ref.clone(),
            branch: None,
            dirty: false,
        }
    );
    assert_eq!(
        archived_row.blockers,
        vec![CodeWorktreeInventoryBlocker::MergeProofUnavailable]
    );

    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        index_before
    );
    assert_eq!(
        fs::metadata(store.store_path())
            .and_then(|metadata| metadata.modified())
            .map_err(|error| error.to_string())?,
        index_mtime_before
    );
    assert_eq!(snapshot_tree(app_data.path())?, app_data_before);
    assert_eq!(snapshot_tree(&repository.common_dir)?, admin_before);
    assert_eq!(
        snapshot_tree(Path::new(&archived.execution_root))?,
        archived_before
    );
    assert_eq!(
        snapshot_tree(Path::new(&prepared.execution_root))?,
        prepared_before
    );
    assert_eq!(store.list_with_lifecycle(&owner)?, lifecycle_before);
    assert_eq!(store.list_preparations(&owner)?, preparations_before);
    Ok(())
}

#[test]
fn archived_inventory_reads_actual_dirty_branch_and_head_drift() -> Result<(), String> {
    let repository = repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let owner = scope(&repository, "community-a");
    let dirty = managed_descriptor(&repository, nest.path())?;
    let attached = managed_descriptor(&repository, nest.path())?;
    let drifted = managed_descriptor(&repository, nest.path())?;
    for (thread_id, descriptor) in [
        ("thread-dirty", &dirty),
        ("thread-attached", &attached),
        ("thread-head-drift", &drifted),
    ] {
        store.upsert(binding(&owner, thread_id, descriptor))?;
        archive_binding(&store, &owner, thread_id)?;
    }

    fs::write(
        Path::new(&dirty.execution_root).join("untracked.txt"),
        b"dirty\n",
    )
    .map_err(|error| error.to_string())?;
    run_git(
        Path::new(&attached.execution_root),
        &["switch", "-c", "inventory-topic"],
    )?;
    fs::write(
        Path::new(&drifted.execution_root).join("tracked.txt"),
        b"new head\n",
    )
    .map_err(|error| error.to_string())?;
    run_git(Path::new(&drifted.execution_root), &["add", "tracked.txt"])?;
    run_git(
        Path::new(&drifted.execution_root),
        &["commit", "-m", "drift"],
    )?;

    let rows = list_worktree_inventory(&store, nest.path(), &owner)?;
    let by_thread = rows
        .into_iter()
        .map(|row| {
            let thread_id = match &row.authority {
                CodeWorktreeInventoryAuthority::Binding { thread_id, .. } => thread_id.clone(),
                _ => return Err("expected only binding rows".to_string()),
            };
            Ok((thread_id, row))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    assert_eq!(
        by_thread["thread-dirty"].blockers,
        vec![
            CodeWorktreeInventoryBlocker::DirtyRoot,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ]
    );
    assert_eq!(
        by_thread["thread-attached"].blockers,
        vec![
            CodeWorktreeInventoryBlocker::BranchAttached,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ]
    );
    assert_eq!(
        by_thread["thread-head-drift"].blockers,
        vec![
            CodeWorktreeInventoryBlocker::HeadDrift,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ]
    );
    Ok(())
}

#[test]
fn read_only_store_open_does_not_create_an_absent_code_directory() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let code_directory = app_data.path().join("code");
    assert!(!code_directory.exists());
    assert!(CodeThreadBindingStore::for_app_data_read_only(app_data.path())?.is_none());
    assert!(!code_directory.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn read_only_store_rejects_insecure_permissions_without_repair() -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let repository = repository()?;
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let owner = scope(&repository, "community-a");
    store.upsert(binding(
        &owner,
        "thread-local",
        &local_descriptor(&repository),
    ))?;
    let code_directory = app_data.path().join("code");
    let index_before = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let index_mtime_before = fs::metadata(store.store_path())
        .and_then(|metadata| metadata.modified())
        .map_err(|error| error.to_string())?;

    fs::set_permissions(&code_directory, fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())?;
    let directory_mode_before = fs::symlink_metadata(&code_directory)
        .map_err(|error| error.to_string())?
        .mode()
        & 0o7777;
    let directory_error = CodeThreadBindingStore::for_app_data_read_only(app_data.path())
        .expect_err("publicly accessible code directory must fail closed");
    assert!(directory_error.contains("data directory is not private"));
    assert_eq!(
        fs::symlink_metadata(&code_directory)
            .map_err(|error| error.to_string())?
            .mode()
            & 0o7777,
        directory_mode_before
    );

    fs::set_permissions(&code_directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    fs::set_permissions(store.store_path(), fs::Permissions::from_mode(0o644))
        .map_err(|error| error.to_string())?;
    let file_mode_before = fs::symlink_metadata(store.store_path())
        .map_err(|error| error.to_string())?
        .mode()
        & 0o7777;
    let file_error = CodeThreadBindingStore::for_app_data_read_only(app_data.path())
        .expect_err("publicly accessible binding index must fail closed");
    assert!(file_error.contains("binding index is not private"));
    assert_eq!(
        fs::symlink_metadata(store.store_path())
            .map_err(|error| error.to_string())?
            .mode()
            & 0o7777,
        file_mode_before
    );
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        index_before
    );
    assert_eq!(
        fs::metadata(store.store_path())
            .and_then(|metadata| metadata.modified())
            .map_err(|error| error.to_string())?,
        index_mtime_before
    );
    Ok(())
}

#[test]
fn preparation_rows_keep_start_and_fork_authority_closed() {
    let scope = CodeThreadBindingScope {
        community_id: "community-a".to_string(),
        project_dtag: "project-a".to_string(),
        repository_identity: "a".repeat(64),
    };
    for (operation, source_thread_id) in [
        (CodeThreadPreparationOperation::Start, None),
        (
            CodeThreadPreparationOperation::Fork,
            Some("thread-source".to_string()),
        ),
    ] {
        let preparation = crate::code_workspace::bindings::CodeThreadPreparation {
            preparation_id: if operation == CodeThreadPreparationOperation::Start {
                "11111111-1111-4111-8111-111111111111".to_string()
            } else {
                "22222222-2222-4222-8222-222222222222".to_string()
            },
            community_id: scope.community_id.clone(),
            project_dtag: scope.project_dtag.clone(),
            repository_identity: scope.repository_identity.clone(),
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: "/native/prepared".to_string(),
            base_ref: "b".repeat(40),
            worktree_id: Some("33333333-3333-4333-8333-333333333333".to_string()),
            operation,
            source_thread_id,
            state: CodeThreadPreparationState::Starting,
            recovery_thread_baseline: Some(Vec::new()),
            merge_target_ref: None,
        };
        let descriptor = preparation.descriptor();
        let row = project_preparation_row(
            preparation,
            Ok(available_status(
                descriptor.clone(),
                &descriptor.base_ref,
                None,
                false,
            )),
        );
        assert_eq!(
            row.blockers,
            vec![CodeWorktreeInventoryBlocker::UnfinishedPreparation]
        );
        assert!(row.preserved);
        assert!(!row.can_remove);
    }
}

#[test]
fn exhausted_inventory_budget_remains_a_row_local_unavailable_blocker() {
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: "a".repeat(64),
        execution_root: "/native/managed".to_string(),
        base_ref: "b".repeat(40),
        worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
    };
    let owner = CodeThreadBindingScope {
        community_id: "community-a".to_string(),
        project_dtag: "project-a".to_string(),
        repository_identity: descriptor.repository_identity.clone(),
    };
    let inspection = inspect_before_deadline(
        &descriptor,
        Path::new("/native"),
        Instant::now() - Duration::from_secs(1),
    );
    let row = project_binding_row(
        binding(&owner, "thread-archived", &descriptor),
        CodeThreadLifecycleStatus::Archived,
        inspection,
        InventoryMergeProof::Unavailable,
    );
    assert_eq!(
        row.blockers,
        vec![
            CodeWorktreeInventoryBlocker::UnavailableRoot,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ]
    );
    assert!(matches!(
        row.inspection,
        CodeWorktreeInspection::Unavailable { ref error }
            if error == "SchoolX Code worktree inspection budget was exhausted"
    ));
}
