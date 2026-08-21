use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

mod validation;

use super::engine::{
    digest_bytes, digest_text, identity_digest, random_hex, status_from_code, validate_path,
    RawChange, RepoProjection, Snapshot, SnapshotFile, MAX_ACTION_FILES, MAX_COMMIT_MESSAGE_BYTES,
    MAX_PATCH_BYTES,
};
use super::git_command::{
    pin_directory, pin_input_file, verify_named_directory_identity, FileIdentity, GitCommandOutput,
    GitConfigKey, GitConfigScope, GitOperationMarker, GitWriteCommand, IndexUpdate,
    PinnedGitWriteRepository,
};
use super::private_artifact::PrivateArtifactStore;
use super::protocol::*;
use crate::code_workspace::CodeThreadBinding;
use validation::{
    read_no_follow, reject_alternate_object_store, reject_ambiguous_paths, reject_existing_lock,
    validate_autocrlf, validate_local_config, validate_oid, validate_path_attributes,
    validate_stage_entries, validate_worktree_config,
};

pub(super) fn inspect_repository(binding: &CodeThreadBinding) -> Result<RepoProjection, String> {
    if binding.execution_mode != crate::code_workspace::CodeExecutionMode::Worktree {
        return Err("Git writes are available only for managed worktrees".to_string());
    }
    let root = PathBuf::from(&binding.execution_root)
        .canonicalize()
        .map_err(|error| format!("failed to resolve managed worktree: {error}"))?;
    let runner = PinnedGitWriteRepository::pin(&root)?;
    let top_level_output = runner.run(GitWriteCommand::TopLevel)?;
    let top_level = output_line(&top_level_output)?;
    if PathBuf::from(top_level)
        .canonicalize()
        .map_err(|error| error.to_string())?
        != root
    {
        return Err("Managed worktree root changed repository identity".to_string());
    }
    if runner.run(GitWriteCommand::SymbolicHead)?.code == 0 {
        return Err("Git writes require a detached managed worktree HEAD".to_string());
    }
    let head = output_line(&runner.run(GitWriteCommand::HeadCommit)?)?.to_string();
    validate_oid(&head)?;
    let admin_output = runner.run(GitWriteCommand::GitDir)?;
    let admin_raw = output_line(&admin_output)?;
    let admin = if Path::new(admin_raw).is_absolute() {
        PathBuf::from(admin_raw)
    } else {
        root.join(admin_raw)
    }
    .canonicalize()
    .map_err(|error| format!("failed to resolve managed worktree Git directory: {error}"))?;
    let common_output = runner.run(GitWriteCommand::CommonDir)?;
    let common_raw = output_line(&common_output)?;
    let common = if Path::new(common_raw).is_absolute() {
        PathBuf::from(common_raw)
    } else {
        root.join(common_raw)
    }
    .canonicalize()
    .map_err(|error| format!("failed to resolve Git common directory: {error}"))?;
    if admin == common || admin.parent() != Some(common.join("worktrees").as_path()) {
        return Err("Git writes require a linked managed worktree admin directory".to_string());
    }
    if crate::code_workspace::repository_identity(&common)? != binding.repository_identity {
        return Err("Managed worktree common directory changed repository identity".to_string());
    }
    let admin_identity = pin_directory(&admin)?;
    let common_identity = pin_directory(&common)?;
    let object_database_identity = pin_directory(&common.join("objects"))?;
    let worktree_git_file = pin_worktree_git_file(&root, &admin)?;
    verify_named_directory_identity(&admin_identity)?;
    verify_named_directory_identity(&common_identity)?;
    verify_named_directory_identity(&object_database_identity)?;
    let runner = runner.bind_repository_authority(
        worktree_git_file.clone(),
        admin_identity.clone(),
        common_identity.clone(),
        object_database_identity.clone(),
    )?;
    revalidate_discovered_repository(&runner, &root, &admin, &common, &head)?;
    reject_operation_markers(&runner, &root)?;
    reject_existing_lock(&admin.join("index.lock"))?;
    reject_existing_lock(&admin.join("HEAD.lock"))?;
    reject_alternate_object_store(&common)?;
    let ref_format_output = runner.run(GitWriteCommand::RefFormat)?;
    let ref_format = output_line(&ref_format_output)?;
    if ref_format != "files" {
        return Err("Only the loose-files Git ref backend is writable in this release".to_string());
    }
    let shared_index_output = runner.run(GitWriteCommand::SharedIndexPath)?;
    if !output_line_allow_empty(&shared_index_output)?.is_empty() {
        return Err("Split/shared Git indexes are read-only in this release".to_string());
    }
    let index = admin.join("index");
    let index_file = pin_input_file(&index, 64 * 1024 * 1024)?;
    let index_digest = index_file.digest.clone();
    let head_path = admin.join("HEAD");
    let head_file = pin_input_file(&head_path, 4096)?;
    let expected_head = format!("{head}\n");
    if read_no_follow(&head_path, 4096)? != expected_head.as_bytes() {
        return Err("Detached HEAD file did not contain the exact resolved commit".to_string());
    }
    let head_file_digest = head_file.digest.clone();
    let local_config = runner.run(GitWriteCommand::ConfigList {
        scope: GitConfigScope::Local,
    })?;
    let worktree_config_enabled = validate_local_config(&local_config.stdout)?;
    let worktree_config = if worktree_config_enabled {
        runner
            .run(GitWriteCommand::ConfigList {
                scope: GitConfigScope::Worktree,
            })?
            .stdout
    } else {
        Vec::new()
    };
    validate_worktree_config(&worktree_config)?;
    validate_autocrlf(&runner, worktree_config_enabled)?;
    let config_digest = digest_bytes(&[local_config.stdout.as_slice(), &worktree_config].concat());
    let status = runner.run(GitWriteCommand::Status)?.stdout;
    let (staged, unstaged, has_conflicts) = parse_status(&runner, &status)?;
    reject_ambiguous_paths(&staged, &unstaged)?;
    for change in staged.iter().chain(&unstaged) {
        validate_path_attributes(&runner, &change.path)?;
    }
    let stage_entries = runner.run(GitWriteCommand::ListStageEntries { index: None })?;
    validate_stage_entries(&stage_entries.stdout)?;
    let index_semantic_digest = digest_bytes(&stage_entries.stdout);
    let identity = read_identity(&runner)?;
    let worktree_digest = changed_worktree_digest(&runner, &unstaged)?;
    let preimage = digest_text(&format!(
        "{}\0{}\0{}\0{}\0{}\0{}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}",
        head,
        index_digest,
        index_semantic_digest,
        digest_bytes(&status),
        identity_digest(identity.as_ref()),
        worktree_digest,
        runner.root_identity(),
        admin_identity,
        common_identity,
        object_database_identity,
        worktree_git_file,
        runner.git_identity(),
    ));
    Ok(RepoProjection {
        repository_identity: binding.repository_identity.clone(),
        root,
        admin,
        common,
        head,
        index_digest,
        index_semantic_digest,
        head_file_digest,
        config_digest,
        preimage,
        identity,
        staged,
        unstaged,
        has_conflicts,
        root_identity: runner.root_identity().clone(),
        admin_identity,
        common_identity,
        object_database_identity,
        worktree_git_file,
        git_identity: runner.git_identity().clone(),
        index_file,
        head_file,
        runner,
    })
}

fn reject_operation_markers(runner: &PinnedGitWriteRepository, root: &Path) -> Result<(), String> {
    for marker in [
        GitOperationMarker::MergeHead,
        GitOperationMarker::CherryPickHead,
        GitOperationMarker::RevertHead,
        GitOperationMarker::BisectLog,
        GitOperationMarker::RebaseMerge,
        GitOperationMarker::RebaseApply,
        GitOperationMarker::Sequencer,
    ] {
        let output = runner.run(GitWriteCommand::GitPath { marker })?;
        let path = PathBuf::from(output_line(&output)?);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err("An in-progress Git operation blocks SchoolX Code writes".to_string())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("failed to inspect Git operation marker: {error}")),
        }
    }
    Ok(())
}

fn pin_worktree_git_file(root: &Path, admin: &Path) -> Result<FileIdentity, String> {
    let path = root.join(".git");
    let evidence = pin_input_file(&path, 32 * 1024)?;
    if evidence.link_count != 1 {
        return Err("Managed worktree .git file must be singly linked".to_string());
    }
    let bytes = read_no_follow(&path, 32 * 1024)?;
    if digest_bytes(&bytes) != evidence.digest {
        return Err("Managed worktree .git file changed while it was frozen".to_string());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "Managed worktree .git file is not UTF-8".to_string())?;
    let value = text
        .strip_prefix("gitdir: ")
        .and_then(|value| value.strip_suffix('\n'))
        .ok_or_else(|| "Managed worktree .git file is not canonical".to_string())?;
    if value.is_empty() || value.contains(['\r', '\n', '\0']) {
        return Err("Managed worktree .git file is ambiguous".to_string());
    }
    let target = Path::new(value);
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    }
    .canonicalize()
    .map_err(|error| format!("failed to resolve managed worktree .git target: {error}"))?;
    if target != admin {
        return Err("Managed worktree .git file changed its admin authority".to_string());
    }
    Ok(evidence)
}

fn revalidate_discovered_repository(
    runner: &PinnedGitWriteRepository,
    root: &Path,
    admin: &Path,
    common: &Path,
    head: &str,
) -> Result<(), String> {
    let top = runner.run(GitWriteCommand::TopLevel)?;
    if PathBuf::from(output_line(&top)?)
        .canonicalize()
        .map_err(|error| format!("failed to re-resolve Git top-level: {error}"))?
        != root
    {
        return Err("Pinned Git top-level changed during discovery".to_string());
    }
    let git_dir = runner.run(GitWriteCommand::GitDir)?;
    if canonical_git_output(root, output_line(&git_dir)?)? != admin {
        return Err("Pinned Git admin directory changed during discovery".to_string());
    }
    let common_dir = runner.run(GitWriteCommand::CommonDir)?;
    if canonical_git_output(root, output_line(&common_dir)?)? != common {
        return Err("Pinned Git common directory changed during discovery".to_string());
    }
    if runner.run(GitWriteCommand::SymbolicHead)?.code == 0 {
        return Err("Pinned Git authority stopped using detached HEAD".to_string());
    }
    if output_line(&runner.run(GitWriteCommand::HeadCommit)?)? != head {
        return Err("Pinned Git HEAD changed during authority binding".to_string());
    }
    Ok(())
}

fn canonical_git_output(root: &Path, value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    path.canonicalize()
        .map_err(|error| format!("failed to resolve pinned Git path: {error}"))
}

fn changed_worktree_digest(
    runner: &PinnedGitWriteRepository,
    changes: &[RawChange],
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    for change in changes {
        hasher.update(change.path.as_bytes());
        hasher.update([0]);
        match runner.read_worktree_file(&change.path, 64 * 1024 * 1024)? {
            Some(file) => {
                hasher.update((file.bytes.len() as u64).to_le_bytes());
                hasher.update(if file.executable { b"x" } else { b"-" });
                hasher.update(file.bytes);
            }
            None => {
                hasher.update(b"deleted");
            }
        }
        hasher.update([0]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn output_line(output: &GitCommandOutput) -> Result<&str, String> {
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| "Git returned non-UTF-8 output".to_string())?;
    let line = text.trim_end_matches(['\r', '\n']);
    if line.is_empty() || line.contains(['\r', '\n', '\0']) {
        return Err("Git returned an ambiguous single-line value".to_string());
    }
    Ok(line)
}

pub(super) fn output_line_for_transaction(output: &GitCommandOutput) -> Result<&str, String> {
    output_line(output)
}

fn output_line_allow_empty(output: &GitCommandOutput) -> Result<&str, String> {
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| "Git returned non-UTF-8 output".to_string())?;
    let line = text.trim_end_matches(['\r', '\n']);
    if line.contains(['\r', '\n', '\0']) {
        return Err("Git returned an ambiguous single-line value".to_string());
    }
    Ok(line)
}

fn parse_status(
    runner: &PinnedGitWriteRepository,
    bytes: &[u8],
) -> Result<(Vec<RawChange>, Vec<RawChange>, bool), String> {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut has_conflicts = false;
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if record.len() < 4 || record[2] != b' ' {
            return Err("Git returned an ambiguous status record".to_string());
        }
        let x = record[0] as char;
        let y = record[1] as char;
        let path = std::str::from_utf8(&record[3..])
            .map_err(|_| "Non-UTF-8 Git paths are read-only in this release".to_string())?
            .to_string();
        validate_path(&path)?;
        if matches!(
            (x, y),
            ('D', 'D')
                | ('A', 'U')
                | ('U', 'D')
                | ('U', 'A')
                | ('D', 'U')
                | ('A', 'A')
                | ('U', 'U')
        ) {
            has_conflicts = true;
        }
        if x != ' ' && x != '?' {
            staged.push(read_change(
                runner,
                &path,
                status_from_code(x, false)?,
                true,
            )?);
        }
        if y != ' ' || (x == '?' && y == '?') {
            let untracked = x == '?' && y == '?';
            unstaged.push(read_change(
                runner,
                &path,
                status_from_code(if untracked { '?' } else { y }, untracked)?,
                false,
            )?);
        }
    }
    staged.sort_by(|left, right| left.path.cmp(&right.path));
    unstaged.sort_by(|left, right| left.path.cmp(&right.path));
    if staged.len().max(unstaged.len()) > MAX_ACTION_FILES {
        return Err(format!(
            "Git write manifest exceeds the {MAX_ACTION_FILES}-file limit"
        ));
    }
    Ok((staged, unstaged, has_conflicts))
}

fn read_change(
    runner: &PinnedGitWriteRepository,
    path: &str,
    status: CodeGitChangeStatus,
    staged: bool,
) -> Result<RawChange, String> {
    let numstat = runner
        .run(GitWriteCommand::DiffNumstat {
            staged,
            path: path.to_string(),
        })?
        .stdout;
    let mut additions = 0;
    let mut deletions = 0;
    let mut binary = false;
    if status == CodeGitChangeStatus::Untracked {
        let data = runner
            .read_worktree_file(path, 64 * 1024 * 1024)?
            .ok_or_else(|| "Untracked file changed while Git status was inspected".to_string())?
            .bytes;
        binary = data.contains(&0);
        if !binary {
            additions = String::from_utf8_lossy(&data).lines().count();
        }
    } else if let Some(line) = String::from_utf8_lossy(&numstat).lines().next() {
        let columns = line.split('\t').collect::<Vec<_>>();
        if columns.len() >= 2 {
            binary = columns[0] == "-" || columns[1] == "-";
            additions = columns[0].parse().unwrap_or(0);
            deletions = columns[1].parse().unwrap_or(0);
        }
    }
    let patch_bytes = if status == CodeGitChangeStatus::Untracked || binary {
        Vec::new()
    } else {
        runner
            .run(GitWriteCommand::DiffPatch {
                staged,
                path: path.to_string(),
            })?
            .stdout
    };
    let truncated = patch_bytes.len() > MAX_PATCH_BYTES;
    let patch =
        String::from_utf8_lossy(&patch_bytes[..patch_bytes.len().min(MAX_PATCH_BYTES)]).to_string();
    Ok(RawChange {
        path: path.to_string(),
        status,
        binary,
        additions,
        deletions,
        patch,
        truncated,
    })
}

pub(super) struct PreparedIndexCandidate {
    pub(super) candidate: FileIdentity,
    pub(super) source: Option<FileIdentity>,
    pub(super) expected_index_digest: String,
    pub(super) expected_semantic_digest: String,
    pub(super) selected_mode: String,
    pub(super) selected_blob_oid: String,
}

pub(super) struct PreparedCommitInputs {
    pub(super) candidate: FileIdentity,
    pub(super) message: FileIdentity,
}

pub(super) fn prepare_index_candidate(
    store: &PrivateArtifactStore,
    projection: &RepoProjection,
    path: &str,
    stage: bool,
) -> Result<PreparedIndexCandidate, String> {
    validate_path(path)?;
    let live_index = freeze_live_index(projection)?;
    let candidate = store.create("candidate-index", &live_index)?;
    let before_entries = projection
        .runner
        .run(GitWriteCommand::ListStageEntries { index: None })?
        .stdout;
    let mut expected = stage_entry_map(&before_entries)?;

    let (source, selected_mode, selected_blob_oid, update) = if stage {
        match projection
            .runner
            .read_worktree_file(path, 64 * 1024 * 1024)?
        {
            Some(frozen) => {
                let source = store.create("stage-source", &frozen.bytes)?;
                let oid_output = projection.runner.run(GitWriteCommand::HashObject {
                    write: false,
                    source: source.clone(),
                })?;
                let oid = output_line(&oid_output)?.to_string();
                validate_oid(&oid)?;
                let mode = if frozen.executable {
                    "100755"
                } else {
                    "100644"
                }
                .to_string();
                let update = IndexUpdate::Upsert {
                    mode: mode.clone(),
                    oid: oid.clone(),
                    path: path.to_string(),
                };
                (Some(source), mode, oid, update)
            }
            None => {
                let oid = "0".repeat(projection.head.len());
                (
                    None,
                    "000000".to_string(),
                    oid,
                    IndexUpdate::Remove {
                        path: path.to_string(),
                    },
                )
            }
        }
    } else {
        let head_entry = projection.runner.run(GitWriteCommand::HeadEntry {
            path: path.to_string(),
        })?;
        if head_entry.stdout.is_empty() {
            let oid = "0".repeat(projection.head.len());
            (
                None,
                "000000".to_string(),
                oid,
                IndexUpdate::Remove {
                    path: path.to_string(),
                },
            )
        } else {
            let record = head_entry
                .stdout
                .split(|byte| *byte == 0)
                .next()
                .ok_or_else(|| "HEAD entry was malformed".to_string())?;
            let text =
                std::str::from_utf8(record).map_err(|_| "HEAD entry was non-UTF-8".to_string())?;
            let (metadata, entry_path) = text
                .split_once('\t')
                .ok_or_else(|| "HEAD entry was malformed".to_string())?;
            let fields = metadata.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3
                || fields[1] != "blob"
                || entry_path != path
                || !matches!(fields[0], "100644" | "100755")
            {
                return Err("Only regular HEAD entries can be unstaged in this release".to_string());
            }
            validate_oid(fields[2])?;
            (
                None,
                fields[0].to_string(),
                fields[2].to_string(),
                IndexUpdate::Upsert {
                    mode: fields[0].to_string(),
                    oid: fields[2].to_string(),
                    path: path.to_string(),
                },
            )
        }
    };
    match &update {
        IndexUpdate::Upsert { mode, oid, .. } => {
            expected.insert(path.to_string(), (mode.clone(), oid.clone()));
        }
        IndexUpdate::Remove { .. } => {
            expected.remove(path);
        }
    }
    projection.runner.run(GitWriteCommand::UpdateIndex {
        index: candidate.clone(),
        update,
    })?;
    let candidate = store.secure_after_mutation(Path::new(&candidate.path))?;
    let candidate_entries = projection.runner.run(GitWriteCommand::ListStageEntries {
        index: Some(candidate.clone()),
    })?;
    let observed = stage_entry_map(&candidate_entries.stdout)?;
    if observed != expected {
        return Err("Candidate index changed entries outside the selected whole file".to_string());
    }
    Ok(PreparedIndexCandidate {
        expected_index_digest: candidate.digest.clone(),
        expected_semantic_digest: digest_bytes(&candidate_entries.stdout),
        candidate,
        source,
        selected_mode,
        selected_blob_oid,
    })
}

pub(super) fn prepare_commit_inputs(
    store: &PrivateArtifactStore,
    projection: &RepoProjection,
    message: &str,
) -> Result<PreparedCommitInputs, String> {
    let index = freeze_live_index(projection)?;
    let candidate = store.create("commit-index", &index)?;
    let message = store.create("commit-message", message.as_bytes())?;
    Ok(PreparedCommitInputs { candidate, message })
}

fn freeze_live_index(projection: &RepoProjection) -> Result<Vec<u8>, String> {
    let path = projection.admin.join("index");
    let bytes = read_no_follow(&path, 64 * 1024 * 1024)?;
    let evidence = pin_input_file(&path, 64 * 1024 * 1024)?;
    if evidence != projection.index_file || digest_bytes(&bytes) != projection.index_digest {
        return Err("Git index changed while it was frozen".to_string());
    }
    Ok(bytes)
}

fn stage_entry_map(bytes: &[u8]) -> Result<BTreeMap<String, (String, String)>, String> {
    validate_stage_entries(bytes)?;
    let mut entries = BTreeMap::new();
    for raw in bytes.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()) {
        let text = std::str::from_utf8(raw)
            .map_err(|_| "Git index contains a non-UTF-8 path".to_string())?;
        let (metadata, path) = text
            .split_once('\t')
            .ok_or_else(|| "Git index returned an ambiguous entry".to_string())?;
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        entries.insert(
            path.to_string(),
            (fields[1].to_string(), fields[2].to_string()),
        );
    }
    Ok(entries)
}

pub(super) fn revalidate_projection_evidence(projection: &RepoProjection) -> Result<(), String> {
    projection.runner.revalidate()?;
    if projection.runner.root_identity() != &projection.root_identity
        || projection.runner.git_identity() != &projection.git_identity
    {
        return Err("Pinned Git root or executable identity changed".to_string());
    }
    verify_named_directory_identity(&projection.admin_identity)?;
    verify_named_directory_identity(&projection.common_identity)?;
    verify_named_directory_identity(&projection.object_database_identity)?;
    if pin_input_file(&projection.root.join(".git"), 32 * 1024)? != projection.worktree_git_file {
        return Err("Managed worktree .git authority changed after the claim".to_string());
    }
    if crate::code_workspace::repository_identity(&projection.common)?
        != projection.repository_identity
    {
        return Err("Managed worktree repository identity changed after the claim".to_string());
    }
    reject_operation_markers(&projection.runner, &projection.root)?;
    let index = pin_input_file(&projection.admin.join("index"), 64 * 1024 * 1024)?;
    let head_file = pin_input_file(&projection.admin.join("HEAD"), 4096)?;
    if index != projection.index_file || head_file != projection.head_file {
        return Err("Git index or detached HEAD changed after the claim".to_string());
    }
    if read_no_follow(&projection.admin.join("HEAD"), 4096)?
        != format!("{}\n", projection.head).as_bytes()
    {
        return Err("Detached HEAD bytes changed after the claim".to_string());
    }
    let head_output = projection.runner.run(GitWriteCommand::HeadCommit)?;
    if output_line(&head_output)? != projection.head {
        return Err("Resolved Git HEAD changed after the claim".to_string());
    }
    let local = projection.runner.run(GitWriteCommand::ConfigList {
        scope: GitConfigScope::Local,
    })?;
    let worktree_enabled = validate_local_config(&local.stdout)?;
    let worktree = if worktree_enabled {
        projection
            .runner
            .run(GitWriteCommand::ConfigList {
                scope: GitConfigScope::Worktree,
            })?
            .stdout
    } else {
        Vec::new()
    };
    validate_worktree_config(&worktree)?;
    validate_autocrlf(&projection.runner, worktree_enabled)?;
    if digest_bytes(&[local.stdout.as_slice(), worktree.as_slice()].concat())
        != projection.config_digest
    {
        return Err("Repository config changed after the reviewed snapshot".to_string());
    }
    let entries = projection
        .runner
        .run(GitWriteCommand::ListStageEntries { index: None })?
        .stdout;
    validate_stage_entries(&entries)?;
    if digest_bytes(&entries) != projection.index_semantic_digest {
        return Err("Git index semantics changed after the reviewed snapshot".to_string());
    }
    Ok(())
}

pub(super) fn read_live_index_digest(admin: &Path) -> Result<String, String> {
    pin_input_file(&admin.join("index"), 64 * 1024 * 1024).map(|value| value.digest)
}

pub(super) fn read_live_head(admin: &Path) -> Result<String, String> {
    let bytes = read_no_follow(&admin.join("HEAD"), 4096)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| "Detached HEAD file is not UTF-8".to_string())?;
    let oid = text
        .strip_suffix('\n')
        .ok_or_else(|| "Detached HEAD file is not canonical".to_string())?;
    if oid.contains(['\r', '\n']) {
        return Err("Detached HEAD file is not canonical".to_string());
    }
    validate_oid(oid)?;
    Ok(oid.to_string())
}

pub(super) fn issue_snapshot(
    write_generation: u64,
    sequence: u64,
    projection: &RepoProjection,
) -> Result<Snapshot, String> {
    let mut path_ids = BTreeMap::new();
    for change in projection.staged.iter().chain(&projection.unstaged) {
        if !path_ids.contains_key(&change.path) {
            path_ids.insert(change.path.clone(), random_hex()?);
        }
    }
    let mut files = HashMap::new();
    for (path, file_id) in path_ids {
        files.insert(
            file_id,
            SnapshotFile {
                staged: projection.staged.iter().any(|change| change.path == path),
                unstaged: projection.unstaged.iter().any(|change| change.path == path),
                path,
            },
        );
    }
    Ok(Snapshot {
        id: random_hex()?,
        write_generation,
        sequence,
        preimage: projection.preimage.clone(),
        head: projection.head.clone(),
        root: projection.root.clone(),
        admin: projection.admin.clone(),
        identity: projection.identity.clone(),
        files,
    })
}

pub(super) fn changes_with_ids(
    changes: &[RawChange],
    snapshot: &Snapshot,
) -> Result<CodeGitChangeSet, String> {
    let files = changes
        .iter()
        .map(|change| {
            let file_id = snapshot
                .files
                .iter()
                .find_map(|(id, file)| (file.path == change.path).then(|| id.clone()))
                .ok_or_else(|| "Git snapshot is missing a file coordinate".to_string())?;
            Ok(CodeGitChangeFile {
                file_id,
                path: change.path.clone(),
                status: change.status,
                binary: change.binary,
                additions: change.additions,
                deletions: change.deletions,
                patch: change.patch.clone(),
                truncated: change.truncated,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(CodeGitChangeSet {
        total_files: files.len(),
        files_truncated: false,
        additions: files.iter().map(|file| file.additions).sum(),
        deletions: files.iter().map(|file| file.deletions).sum(),
        files,
    })
}

pub(super) fn task_with_snapshot_ids(
    task: &CodeGitChangeSet,
    snapshot: &Snapshot,
) -> Result<CodeGitChangeSet, String> {
    let mut task = task.clone();
    for file in &mut task.files {
        file.file_id = snapshot
            .files
            .iter()
            .find_map(|(id, coordinate)| (coordinate.path == file.path).then(|| id.clone()))
            .map(Ok)
            .unwrap_or_else(random_hex)?;
    }
    Ok(task)
}

fn read_identity(
    runner: &PinnedGitWriteRepository,
) -> Result<Option<CodeGitCommitIdentity>, String> {
    let name = config_value(runner, GitConfigKey::UserName)?;
    let email = config_value(runner, GitConfigKey::UserEmail)?;
    match (name, email) {
        (Some(name), Some(email)) if valid_identity(&name, &email) => {
            Ok(Some(CodeGitCommitIdentity { name, email }))
        }
        _ => Ok(None),
    }
}

fn config_value(
    runner: &PinnedGitWriteRepository,
    key: GitConfigKey,
) -> Result<Option<String>, String> {
    let output = runner.run(GitWriteCommand::ConfigValue {
        scope: GitConfigScope::Local,
        key,
    })?;
    if output.code == 1 {
        return Ok(None);
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "Repository identity is not UTF-8".to_string())?;
    let values = text.lines().collect::<Vec<_>>();
    if values.len() != 1 {
        return Ok(None);
    }
    Ok(Some(values[0].to_string()))
}

fn valid_identity(name: &str, email: &str) -> bool {
    name == name.trim()
        && email == email.trim()
        && !name.is_empty()
        && !email.is_empty()
        && !name
            .chars()
            .any(|value| value.is_control() || matches!(value, '<' | '>'))
        && !email
            .chars()
            .any(|value| value.is_control() || value.is_whitespace() || matches!(value, '<' | '>'))
        && email.matches('@').count() == 1
        && !email.starts_with('@')
        && !email.ends_with('@')
}

pub(super) fn validate_commit_message(message: &str) -> Result<String, String> {
    let normalized = message.replace("\r\n", "\n");
    if normalized != message || message != message.trim() || message.is_empty() {
        return Err(
            "Commit message must be non-empty, LF-normalized, and have no surrounding whitespace"
                .to_string(),
        );
    }
    if message.chars().any(|value| {
        value == '\0' || value == '\r' || (value.is_control() && value != '\n' && value != '\t')
    }) {
        return Err("Commit message contains an unsupported control character".to_string());
    }
    let lower = message.to_ascii_lowercase();
    if lower
        .lines()
        .any(|line| line.starts_with("co-authored-by:") || line.starts_with("signed-off-by:"))
    {
        return Err(
            "Commit message must not supply Co-authored-by or Signed-off-by trailers".to_string(),
        );
    }
    Ok(message.to_string())
}

pub(super) fn canonical_commit_message(
    message: &str,
    identity: &CodeGitCommitIdentity,
) -> Result<String, String> {
    let canonical = format!(
        "{}\n\nCo-authored-by: {} <{}>\nSigned-off-by: {} <{}>\n",
        message.trim_end_matches('\n'),
        identity.name,
        identity.email,
        identity.name,
        identity.email
    );
    if canonical.len() > MAX_COMMIT_MESSAGE_BYTES {
        return Err(format!(
            "Canonical commit message exceeds {MAX_COMMIT_MESSAGE_BYTES} bytes"
        ));
    }
    Ok(canonical)
}
