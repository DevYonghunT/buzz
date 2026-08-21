use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::code_workspace::git_write::journal::{
    GitJournalPhase, GitJournalRecord, GitOperationJournalStore,
};
use crate::code_workspace::git_write::{
    acknowledge, commit, recover_startup_journals, stage, status, unstage, CodeGitAcknowledgeInput,
    CodeGitAcknowledgeReceipt, CodeGitChangeSet, CodeGitCommitInput, CodeGitIndexMutationInput,
    CodeGitMutationReceipt, CodeGitOperation, CodeGitStatus, CodeGitStatusInput, CodeGitWriteState,
    GitWriteContext,
};
use crate::code_workspace::{
    CodeExecutionMode, CodeThreadBinding, CodeThreadBindingScope, CodeThreadBindingStore,
};

pub(super) const RAW_COMMIT_MESSAGE: &str = "Crash-safe commit";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum CrashOperation {
    Stage,
    Unstage,
    Commit,
    AcknowledgeStage,
}

impl CrashOperation {
    pub(super) fn journal_operation(self) -> CodeGitOperation {
        match self {
            Self::Stage | Self::AcknowledgeStage => CodeGitOperation::Stage,
            Self::Unstage => CodeGitOperation::Unstage,
            Self::Commit => CodeGitOperation::Commit,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CrashRequest {
    pub(super) version: u32,
    pub(super) app_data: String,
    pub(super) nest_root: String,
    pub(super) binding: CodeThreadBinding,
    pub(super) operation: CrashOperation,
    pub(super) boundary: super::super::fault::TransactionFaultBoundary,
}

pub(super) struct CrashFixture {
    _repository: tempfile::TempDir,
    _nest: tempfile::TempDir,
    app_data: tempfile::TempDir,
    pub(super) source_root: PathBuf,
    pub(super) managed_root: PathBuf,
    pub(super) sibling_root: PathBuf,
    pub(super) binding: CodeThreadBinding,
    pub(super) base_head: String,
    source_refs: String,
    source_status: String,
    sibling_status: String,
}

impl CrashFixture {
    pub(super) fn prepare(operation: CrashOperation) -> Result<Self, String> {
        let repository = tempfile::tempdir().map_err(|error| error.to_string())?;
        run(repository.path(), &["init", "-q"])?;
        run(
            repository.path(),
            &["config", "--local", "user.name", "Crash Test"],
        )?;
        run(
            repository.path(),
            &["config", "--local", "user.email", "crash@example.com"],
        )?;
        fs::write(repository.path().join("tracked.txt"), b"base\n")
            .map_err(|error| error.to_string())?;
        fs::write(repository.path().join("untouched.txt"), b"untouched\n")
            .map_err(|error| error.to_string())?;
        run(repository.path(), &["add", "tracked.txt", "untouched.txt"])?;
        run(repository.path(), &["commit", "-q", "-m", "base"])?;
        let base_head = run(repository.path(), &["rev-parse", "HEAD"])?
            .trim()
            .to_string();

        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let managed_root = nest.path().join("managed");
        let sibling_root = nest.path().join("sibling");
        add_detached_worktree(repository.path(), &managed_root)?;
        add_detached_worktree(repository.path(), &sibling_root)?;
        let managed_root = managed_root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let sibling_root = sibling_root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let common = resolve_git_path(&managed_root, "--git-common-dir")?;
        let scope = CodeThreadBindingScope {
            community_id: "git-crash-community".to_string(),
            project_dtag: "git-crash-project".to_string(),
            repository_identity: crate::code_workspace::repository_identity(&common)?,
        };
        let binding = CodeThreadBinding {
            community_id: scope.community_id.clone(),
            project_dtag: scope.project_dtag.clone(),
            repository_identity: scope.repository_identity.clone(),
            codex_thread_id: "git-crash-thread".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: path_text(&managed_root, "managed worktree")?,
            base_ref: base_head.clone(),
            worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
        };
        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        fs::set_permissions(app_data.path(), fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        CodeThreadBindingStore::for_app_data(app_data.path())?.upsert(binding.clone())?;

        fs::write(managed_root.join("tracked.txt"), b"staged version\n")
            .map_err(|error| error.to_string())?;
        if matches!(operation, CrashOperation::Unstage | CrashOperation::Commit) {
            run(&managed_root, &["add", "tracked.txt"])?;
        }
        if operation == CrashOperation::Commit {
            fs::write(managed_root.join("tracked.txt"), b"unstaged version\n")
                .map_err(|error| error.to_string())?;
        }

        let source_refs = run(
            repository.path(),
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )?;
        let source_status = run(repository.path(), &["status", "--porcelain=v1", "-z"])?;
        let sibling_status = run(&sibling_root, &["status", "--porcelain=v1", "-z"])?;
        Ok(Self {
            source_root: repository.path().to_path_buf(),
            _repository: repository,
            _nest: nest,
            app_data,
            managed_root,
            sibling_root,
            binding,
            base_head,
            source_refs,
            source_status,
            sibling_status,
        })
    }

    pub(super) fn request(
        &self,
        operation: CrashOperation,
        boundary: super::super::fault::TransactionFaultBoundary,
    ) -> Result<CrashRequest, String> {
        let app_data = self
            .app_data
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let nest_root = self
            ._nest
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        Ok(CrashRequest {
            version: 1,
            app_data: path_text(&app_data, "app data")?,
            nest_root: path_text(&nest_root, "nest root")?,
            binding: self.binding.clone(),
            operation,
            boundary,
        })
    }

    pub(super) fn admin(&self) -> Result<PathBuf, String> {
        resolve_git_path(&self.managed_root, "--git-dir")
    }

    pub(super) fn load_record(&self) -> Result<GitJournalRecord, String> {
        let records = GitOperationJournalStore::for_app_data(self.app_data.path())?
            .load()?
            .records;
        if records.len() != 1 {
            return Err(format!(
                "crash fixture expected one journal record, found {}",
                records.len()
            ));
        }
        records
            .into_iter()
            .next()
            .ok_or_else(|| "crash fixture journal was empty".to_string())
    }

    pub(super) fn recover_startup(&self) -> Result<(), String> {
        let store = CodeThreadBindingStore::for_app_data(self.app_data.path())?;
        recover_startup_journals(&store, self.app_data.path(), self._nest.path())
    }

    pub(super) fn exact_retry(
        &self,
        operation: CrashOperation,
        record: &GitJournalRecord,
    ) -> Result<CodeGitMutationReceipt, String> {
        let state = CodeGitWriteState::default();
        match operation {
            CrashOperation::Stage | CrashOperation::Unstage | CrashOperation::AcknowledgeStage => {
                let input = CodeGitIndexMutationInput {
                    scope: record.key.scope.clone(),
                    thread_id: record.key.thread_id.clone(),
                    write_generation: record.request.write_generation,
                    snapshot_id: record.request.snapshot_id.clone(),
                    file_id: record
                        .request
                        .file_id
                        .clone()
                        .ok_or_else(|| "index crash record lost its file id".to_string())?,
                };
                let receipt = if operation == CrashOperation::Unstage {
                    unstage(&state, self.app_data.path(), &self.binding, input)?
                } else {
                    stage(&state, self.app_data.path(), &self.binding, input)?
                };
                Ok(CodeGitMutationReceipt::Index(receipt))
            }
            CrashOperation::Commit => Ok(CodeGitMutationReceipt::Commit(commit(
                &state,
                self.app_data.path(),
                &self.binding,
                CodeGitCommitInput {
                    scope: record.key.scope.clone(),
                    thread_id: record.key.thread_id.clone(),
                    write_generation: record.request.write_generation,
                    snapshot_id: record.request.snapshot_id.clone(),
                    message: RAW_COMMIT_MESSAGE.to_string(),
                },
            )?)),
        }
    }

    pub(super) fn retry_acknowledgement(
        &self,
        record: &GitJournalRecord,
    ) -> Result<CodeGitAcknowledgeReceipt, String> {
        let coordinate = record
            .acknowledgement
            .as_ref()
            .ok_or_else(|| "acknowledgement crash lost its coordinate".to_string())?;
        acknowledge(
            &CodeGitWriteState::default(),
            self.app_data.path(),
            CodeGitAcknowledgeInput {
                scope: record.key.scope.clone(),
                thread_id: record.key.thread_id.clone(),
                operation_id: coordinate.operation_id.clone(),
                write_generation: coordinate.write_generation,
                snapshot_id: coordinate.snapshot_id.clone(),
            },
        )
    }

    pub(super) fn assert_preserved(&self) -> Result<(), String> {
        if run(
            &self.source_root,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )? != self.source_refs
        {
            return Err("source repository refs changed during crash recovery".to_string());
        }
        if run(&self.source_root, &["status", "--porcelain=v1", "-z"])? != self.source_status {
            return Err("source checkout changed during crash recovery".to_string());
        }
        if run(&self.sibling_root, &["status", "--porcelain=v1", "-z"])? != self.sibling_status
            || run(&self.sibling_root, &["rev-parse", "HEAD"])?.trim() != self.base_head
            || fs::read(self.sibling_root.join("tracked.txt")).map_err(|error| error.to_string())?
                != b"base\n"
        {
            return Err("sibling worktree changed during crash recovery".to_string());
        }
        Ok(())
    }

    pub(super) fn assert_completed(
        &self,
        operation: CrashOperation,
        record: &GitJournalRecord,
    ) -> Result<(), String> {
        if record.phase != GitJournalPhase::CompletedAwaitingAck
            || !record.cleanup_complete
            || record.operation != operation.journal_operation()
        {
            return Err(format!(
                "crash recovery did not complete exact cleanup: {:?}",
                record.phase
            ));
        }
        let admin = self.admin()?;
        if admin.join("index.lock").exists() || admin.join("HEAD.lock").exists() {
            return Err("owned standard Git lock remained after recovery".to_string());
        }
        for artifact in [
            Some(&record.artifacts.candidate_index),
            record.artifacts.source.as_ref(),
            record.artifacts.message.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if Path::new(&artifact.parent_path)
                .join(&artifact.name)
                .exists()
            {
                return Err(format!(
                    "private Git artifact remained after recovery: {}",
                    artifact.name
                ));
            }
        }
        for name in [
            &record.artifacts.index_artifact_name,
            &record.artifacts.head_artifact_name,
        ] {
            if admin.join(name).exists() {
                return Err(format!(
                    "owned Git artifact remained after recovery: {name}"
                ));
            }
        }
        match record
            .receipt
            .as_ref()
            .ok_or_else(|| "completed crash record lost its receipt".to_string())?
        {
            CodeGitMutationReceipt::Index(_) if operation == CrashOperation::Stage => {
                if run(&self.managed_root, &["show", ":tracked.txt"])? != "staged version\n"
                    || run(&self.managed_root, &["rev-parse", "HEAD"])?.trim() != self.base_head
                {
                    return Err("stage recovery published the wrong index state".to_string());
                }
            }
            CodeGitMutationReceipt::Index(_) if operation == CrashOperation::Unstage => {
                if run(&self.managed_root, &["show", ":tracked.txt"])? != "base\n"
                    || fs::read(self.managed_root.join("tracked.txt"))
                        .map_err(|error| error.to_string())?
                        != b"staged version\n"
                {
                    return Err("unstage recovery changed the wrong Git state".to_string());
                }
            }
            CodeGitMutationReceipt::Index(_) if operation == CrashOperation::AcknowledgeStage => {
                return Err("acknowledgement crash should finish as acknowledged".to_string());
            }
            CodeGitMutationReceipt::Commit(receipt) if operation == CrashOperation::Commit => {
                if run(&self.managed_root, &["rev-parse", "HEAD"])?.trim() != receipt.commit
                    || run(&self.managed_root, &["show", "HEAD:tracked.txt"])? != "staged version\n"
                    || fs::read(self.managed_root.join("tracked.txt"))
                        .map_err(|error| error.to_string())?
                        != b"unstaged version\n"
                    || run(
                        &self.managed_root,
                        &["rev-list", "--count", &format!("{}..HEAD", self.base_head)],
                    )?
                    .trim()
                        != "1"
                {
                    return Err("commit recovery was not exact-once staged-only".to_string());
                }
            }
            other => {
                return Err(format!(
                    "crash operation and receipt disagreed: {operation:?} {other:?}"
                ));
            }
        }
        if run(&self.managed_root, &["show", ":untouched.txt"])? != "untouched\n" {
            return Err("unselected index entry changed during recovery".to_string());
        }
        self.assert_preserved()
    }
}

pub(super) fn execute_request(request: &CrashRequest) -> Result<(), String> {
    let app_data = canonical_exact(&request.app_data, "app data")?;
    let root = canonical_exact(&request.binding.execution_root, "managed root")?;
    let nest = canonical_exact(&request.nest_root, "nest root")?;
    if !root.starts_with(&nest) {
        return Err("crash request managed root escaped its nest".to_string());
    }
    let state = CodeGitWriteState::default();
    let initial = ready(status(
        &state,
        CodeGitStatusInput {
            scope: request.binding.scope(),
            thread_id: request.binding.codex_thread_id.clone(),
        },
        context(&app_data, &request.binding),
    )?)?;
    let file = match request.operation {
        CrashOperation::Stage | CrashOperation::AcknowledgeStage => initial
            .unstaged
            .files
            .iter()
            .find(|file| file.path == "tracked.txt"),
        CrashOperation::Unstage => initial
            .staged
            .files
            .iter()
            .find(|file| file.path == "tracked.txt"),
        CrashOperation::Commit => None,
    };
    match request.operation {
        CrashOperation::Stage | CrashOperation::Unstage | CrashOperation::AcknowledgeStage => {
            let input = CodeGitIndexMutationInput {
                scope: request.binding.scope(),
                thread_id: request.binding.codex_thread_id.clone(),
                write_generation: initial.write_generation,
                snapshot_id: initial.snapshot_id,
                file_id: file
                    .ok_or_else(|| "crash action did not expose tracked.txt".to_string())?
                    .file_id
                    .clone(),
            };
            let receipt = if request.operation == CrashOperation::Unstage {
                unstage(&state, &app_data, &request.binding, input)?
            } else {
                stage(&state, &app_data, &request.binding, input)?
            };
            if request.operation == CrashOperation::AcknowledgeStage {
                let post = ready(status(
                    &state,
                    CodeGitStatusInput {
                        scope: request.binding.scope(),
                        thread_id: request.binding.codex_thread_id.clone(),
                    },
                    context(&app_data, &request.binding),
                )?)?;
                acknowledge(
                    &state,
                    &app_data,
                    CodeGitAcknowledgeInput {
                        scope: request.binding.scope(),
                        thread_id: request.binding.codex_thread_id.clone(),
                        operation_id: receipt.operation_id,
                        write_generation: post.write_generation,
                        snapshot_id: post.snapshot_id,
                    },
                )?;
            }
        }
        CrashOperation::Commit => {
            commit(
                &state,
                &app_data,
                &request.binding,
                CodeGitCommitInput {
                    scope: request.binding.scope(),
                    thread_id: request.binding.codex_thread_id.clone(),
                    write_generation: initial.write_generation,
                    snapshot_id: initial.snapshot_id,
                    message: RAW_COMMIT_MESSAGE.to_string(),
                },
            )?;
        }
    }
    Ok(())
}

struct ReadySnapshot {
    write_generation: u64,
    snapshot_id: String,
    staged: CodeGitChangeSet,
    unstaged: CodeGitChangeSet,
}

fn ready(status: CodeGitStatus) -> Result<ReadySnapshot, String> {
    match status {
        CodeGitStatus::Ready {
            write_generation,
            snapshot_id,
            staged,
            unstaged,
            ..
        } => Ok(ReadySnapshot {
            write_generation,
            snapshot_id,
            staged: *staged,
            unstaged: *unstaged,
        }),
        other => Err(format!(
            "crash fixture expected ready status, got {other:?}"
        )),
    }
}

fn context(app_data: &Path, binding: &CodeThreadBinding) -> GitWriteContext {
    GitWriteContext {
        app_data_dir: app_data.to_path_buf(),
        binding: binding.clone(),
        runtime_generation: 7,
        task: CodeGitChangeSet {
            files: Vec::new(),
            total_files: 0,
            files_truncated: false,
            additions: 0,
            deletions: 0,
        },
        activity_blocker: None,
    }
}

fn add_detached_worktree(repository: &Path, target: &Path) -> Result<(), String> {
    let target = path_text(target, "worktree target")?;
    run(
        repository,
        &["worktree", "add", "-q", "--detach", target.as_str(), "HEAD"],
    )?;
    Ok(())
}

pub(super) fn resolve_git_path(root: &Path, argument: &str) -> Result<PathBuf, String> {
    let value = PathBuf::from(run(root, &["rev-parse", argument])?.trim());
    let value = if value.is_absolute() {
        value
    } else {
        root.join(value)
    };
    value.canonicalize().map_err(|error| error.to_string())
}

pub(super) fn run(root: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn canonical_exact(value: &str, label: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("crash request {label} is not absolute"));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to resolve crash request {label}: {error}"))?;
    if canonical != path {
        return Err(format!("crash request {label} is not canonical"));
    }
    Ok(canonical)
}

fn path_text(path: &Path, label: &str) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{label} path is not UTF-8"))
}
