//! Repository and configuration evidence validation for Git write recovery.

use std::fs;
use std::path::{Path, PathBuf};

use super::super::super::engine::digest_bytes;
use super::super::super::git_command::{
    pin_directory, pin_input_file, DirectoryIdentity, FileIdentity, GitCommandOutput, GitConfigKey,
    GitConfigScope, GitOperationMarker, GitWriteCommand, PinnedGitWriteRepository,
};
use super::super::super::journal::{
    GitJournalFileIdentity, GitJournalPathIdentity, GitJournalRecord,
};
use super::super::super::owned_lock::{OwnedDirectoryIdentity, PinnedAdminDirectory};
use super::super::super::repository::{
    output_line_for_transaction, read_live_head, read_live_index_digest,
};
use crate::code_workspace::CodeThreadBinding;

pub(super) struct RecoveryRepository {
    pub(super) runner: PinnedGitWriteRepository,
    pub(super) admin: PinnedAdminDirectory,
    pub(super) index_digest: String,
    pub(super) head: String,
}

impl RecoveryRepository {
    pub(super) fn open(
        binding: &CodeThreadBinding,
        record: &GitJournalRecord,
    ) -> Result<Self, String> {
        let root = Path::new(&record.repository.root.exact_path);
        let discovery = PinnedGitWriteRepository::pin(root)?;
        if !directory_matches(discovery.root_identity(), &record.repository.root)
            || discovery.git_identity() != &git_identity(&record.repository.git_executable)
        {
            return Err("Pinned root or Git executable differs from journal evidence".to_string());
        }
        let admin_identity = pin_directory(Path::new(&record.repository.admin.exact_path))?;
        let common_identity = pin_directory(Path::new(&record.repository.common_dir.exact_path))?;
        let object_database_identity =
            pin_directory(Path::new(&record.repository.object_database.exact_path))?;
        let worktree_git_file = git_identity(&record.repository.worktree_git_file);
        if !directory_matches(&admin_identity, &record.repository.admin)
            || !directory_matches(&common_identity, &record.repository.common_dir)
            || !directory_matches(
                &object_database_identity,
                &record.repository.object_database,
            )
            || pin_input_file(Path::new(&worktree_git_file.path), 32 * 1024)? != worktree_git_file
        {
            return Err("Git repository authority differs from journal evidence".to_string());
        }
        let runner = discovery.bind_repository_authority(
            worktree_git_file,
            admin_identity.clone(),
            common_identity,
            object_database_identity,
        )?;
        if crate::code_workspace::repository_identity(Path::new(
            &record.repository.common_dir.exact_path,
        ))? != binding.repository_identity
        {
            return Err("Recovery repository identity changed".to_string());
        }
        let admin = PinnedAdminDirectory::pin(Path::new(&record.repository.admin.exact_path))?;
        require_owned_admin(&admin, &record.repository.admin)?;
        verify_repository_paths(&runner, record)?;
        let index_digest = read_live_index_digest(Path::new(&record.repository.admin.exact_path))?;
        let head = read_live_head(Path::new(&record.repository.admin.exact_path))?;
        let resolved = runner.run(GitWriteCommand::HeadCommit)?;
        if output_line_for_transaction(&resolved)? != head {
            return Err("Resolved HEAD differs from its detached file".to_string());
        }
        Ok(Self {
            runner,
            admin,
            index_digest,
            head,
        })
    }

    pub(super) fn revalidate_for_resume(&self, record: &GitJournalRecord) -> Result<(), String> {
        self.runner.revalidate()?;
        self.admin.revalidate()?;
        verify_repository_paths(&self.runner, record)?;
        validate_repository_safety(&self.runner, record)?;
        if read_live_index_digest(Path::new(&record.repository.admin.exact_path))?
            != record.repository.before_index_digest
            || read_live_head(Path::new(&record.repository.admin.exact_path))?
                != record.repository.previous_head
        {
            return Err("Git index or HEAD changed before recovery publish".to_string());
        }
        let resolved = self.runner.run(GitWriteCommand::HeadCommit)?;
        if output_line_for_transaction(&resolved)? != record.repository.previous_head {
            return Err("Resolved HEAD changed before recovery publish".to_string());
        }
        Ok(())
    }

    pub(super) fn verify_object(&self, oid: &str, expected: &str) -> Result<(), String> {
        let output = self.runner.run(GitWriteCommand::ObjectType {
            oid: oid.to_string(),
        })?;
        if output_line_for_transaction(&output)? != expected {
            return Err(format!("Recovered Git object {oid} is not a {expected}"));
        }
        Ok(())
    }
}

fn verify_repository_paths(
    runner: &PinnedGitWriteRepository,
    record: &GitJournalRecord,
) -> Result<(), String> {
    let root = Path::new(&record.repository.root.exact_path);
    let top = runner.run(GitWriteCommand::TopLevel)?;
    if canonical_output(root, output_line_for_transaction(&top)?)? != root {
        return Err("Recovery top-level path changed".to_string());
    }
    let admin = runner.run(GitWriteCommand::GitDir)?;
    if canonical_output(root, output_line_for_transaction(&admin)?)?
        != Path::new(&record.repository.admin.exact_path)
    {
        return Err("Recovery Git admin path changed".to_string());
    }
    let common = runner.run(GitWriteCommand::CommonDir)?;
    if canonical_output(root, output_line_for_transaction(&common)?)?
        != Path::new(&record.repository.common_dir.exact_path)
    {
        return Err("Recovery Git common directory changed".to_string());
    }
    if runner.run(GitWriteCommand::SymbolicHead)?.code == 0 {
        return Err("Recovery requires the original detached HEAD".to_string());
    }
    Ok(())
}

fn validate_repository_safety(
    runner: &PinnedGitWriteRepository,
    record: &GitJournalRecord,
) -> Result<(), String> {
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
        let path = resolve_output(
            Path::new(&record.repository.root.exact_path),
            output_line_for_transaction(&output)?,
        );
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Err("An in-progress Git operation blocks recovery".to_string()),
            Err(error) => return Err(format!("failed to inspect recovery marker: {error}")),
        }
    }
    let refs = runner.run(GitWriteCommand::RefFormat)?;
    if output_line_for_transaction(&refs)? != "files" {
        return Err("Recovery requires the loose-files ref backend".to_string());
    }
    let shared = runner.run(GitWriteCommand::SharedIndexPath)?;
    if !output_allow_empty(&shared)?.is_empty() {
        return Err("Recovery refuses a split/shared index".to_string());
    }
    let objects = Path::new(&record.repository.common_dir.exact_path).join("objects");
    let _ = pin_directory(&objects)?;
    match fs::symlink_metadata(objects.join("info").join("alternates")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => return Err("Recovery refuses an alternate object database".to_string()),
        Err(error) => return Err(format!("failed to inspect recovery alternates: {error}")),
    }
    validate_config(runner, &record.repository.before_config_digest)
}

fn validate_config(runner: &PinnedGitWriteRepository, expected: &str) -> Result<(), String> {
    let local = runner.run(GitWriteCommand::ConfigList {
        scope: GitConfigScope::Local,
    })?;
    if local.code != 0 {
        return Err("Failed to read repository-local recovery config".to_string());
    }
    let records = parse_config_records(&local.stdout)?;
    validate_config_records(&records)?;
    let worktree_enabled = records
        .iter()
        .filter(|(key, _)| key == "extensions.worktreeconfig")
        .try_fold(false, |_, (_, value)| parse_git_bool(value))?;
    let worktree = if worktree_enabled {
        let output = runner.run(GitWriteCommand::ConfigList {
            scope: GitConfigScope::Worktree,
        })?;
        if output.code != 0 {
            return Err("Failed to read worktree recovery config".to_string());
        }
        let records = parse_config_records(&output.stdout)?;
        validate_config_records(&records)?;
        output.stdout
    } else {
        Vec::new()
    };
    validate_autocrlf(runner, worktree_enabled)?;
    if digest_bytes(&[local.stdout.as_slice(), worktree.as_slice()].concat()) != expected {
        return Err("Repository config changed after the prepared claim".to_string());
    }
    Ok(())
}

fn parse_config_records(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            let text = std::str::from_utf8(raw)
                .map_err(|_| "Repository config contains non-UTF-8 data".to_string())?;
            let (key, value) = text
                .split_once('\n')
                .or_else(|| text.split_once('='))
                .ok_or_else(|| "Repository config returned an ambiguous record".to_string())?;
            if key.is_empty() || key.chars().any(char::is_control) || value.contains('\0') {
                return Err("Repository config returned an unsafe record".to_string());
            }
            Ok((key.to_ascii_lowercase(), value.to_string()))
        })
        .collect()
}

fn validate_config_records(records: &[(String, String)]) -> Result<(), String> {
    for (key, value) in records {
        let executable = key.starts_with("filter.")
            || key == "diff.external"
            || (key.starts_with("diff.")
                && (key.ends_with(".command") || key.ends_with(".textconv")))
            || matches!(
                key.as_str(),
                "core.fsmonitor"
                    | "core.hookspath"
                    | "core.alternaterefscommand"
                    | "core.attributesfile"
                    | "core.worktree"
                    | "extensions.partialclone"
            )
            || key.starts_with("include.")
            || key.starts_with("includeif.");
        if executable {
            return Err(format!("Recovery config {key} is outside the safe subset"));
        }
        if matches!(
            key.as_str(),
            "core.sparsecheckout" | "core.sparsecheckoutcone" | "index.sparse"
        ) && parse_git_bool(value)?
        {
            return Err("Recovery refuses sparse Git state".to_string());
        }
        if key == "extensions.objectformat" && !matches!(value.as_str(), "sha1" | "sha256") {
            return Err("Recovery found an unsupported object format".to_string());
        }
        if key == "extensions.refstorage" && value != "files" {
            return Err("Recovery found an unsupported ref backend".to_string());
        }
    }
    Ok(())
}

fn validate_autocrlf(
    runner: &PinnedGitWriteRepository,
    worktree_enabled: bool,
) -> Result<(), String> {
    let scopes = if worktree_enabled {
        &[GitConfigScope::Local, GitConfigScope::Worktree][..]
    } else {
        &[GitConfigScope::Local][..]
    };
    for scope in scopes {
        let output = runner.run(GitWriteCommand::ConfigValue {
            scope: *scope,
            key: GitConfigKey::CoreAutocrlf,
        })?;
        if output.code == 0 && parse_git_bool(output_line_for_transaction(&output)?)? {
            return Err("core.autocrlf changed to an unsafe value".to_string());
        }
    }
    Ok(())
}

fn directory_matches(actual: &DirectoryIdentity, expected: &GitJournalPathIdentity) -> bool {
    actual.path == expected.exact_path
        && actual.device == expected.device
        && actual.inode == expected.inode
        && actual.owner == expected.owner
        && actual.mode == expected.mode
}

fn git_identity(evidence: &GitJournalFileIdentity) -> FileIdentity {
    FileIdentity {
        path: evidence.exact_path.clone(),
        device: evidence.device,
        inode: evidence.inode,
        owner: evidence.owner,
        mode: evidence.mode,
        link_count: evidence.link_count,
        size: evidence.size,
        digest: evidence.sha256.clone(),
    }
}

pub(super) fn require_owned_admin(
    admin: &PinnedAdminDirectory,
    expected: &GitJournalPathIdentity,
) -> Result<(), String> {
    let actual: &OwnedDirectoryIdentity = admin.identity();
    if actual.path != expected.exact_path
        || actual.device != expected.device
        || actual.inode != expected.inode
        || actual.owner != expected.owner
        || actual.mode != expected.mode
    {
        return Err("Pinned recovery admin differs from journal evidence".to_string());
    }
    Ok(())
}

fn parse_git_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" | "" => Ok(false),
        _ => Err("Repository config contains an invalid boolean".to_string()),
    }
}

fn output_allow_empty(output: &GitCommandOutput) -> Result<&str, String> {
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| "Git returned non-UTF-8 recovery output".to_string())?;
    let line = text.trim_end_matches(['\r', '\n']);
    if line.contains(['\r', '\n', '\0']) {
        return Err("Git returned ambiguous recovery output".to_string());
    }
    Ok(line)
}

fn canonical_output(root: &Path, value: &str) -> Result<PathBuf, String> {
    resolve_output(root, value)
        .canonicalize()
        .map_err(|error| format!("failed to resolve recovery Git path: {error}"))
}

fn resolve_output(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}
