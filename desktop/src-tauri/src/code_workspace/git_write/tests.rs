use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::engine::*;
use super::protocol::*;
use super::repository::inspect_repository;
use crate::code_workspace::{CodeExecutionMode, CodeThreadBinding, CodeThreadBindingScope};

#[test]
fn detached_primary_checkout_cannot_masquerade_as_a_managed_worktree() -> Result<(), String> {
    let repository = tempfile::tempdir().map_err(|error| error.to_string())?;
    run(repository.path(), &["init", "-q"])?;
    fs::write(repository.path().join("tracked.txt"), "base\n")
        .map_err(|error| error.to_string())?;
    run(repository.path(), &["add", "tracked.txt"])?;
    run(
        repository.path(),
        &[
            "-c",
            "user.name=Test User",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "-m",
            "base",
        ],
    )?;
    run(repository.path(), &["switch", "-q", "--detach"])?;
    let common = repository
        .path()
        .join(".git")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let binding = CodeThreadBinding {
        community_id: "community-primary".to_string(),
        project_dtag: "project-primary".to_string(),
        repository_identity: crate::code_workspace::repository_identity(&common)?,
        codex_thread_id: "thread-primary".to_string(),
        execution_mode: CodeExecutionMode::Worktree,
        execution_root: repository
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .to_string(),
        base_ref: run(repository.path(), &["rev-parse", "HEAD"])?
            .trim()
            .to_string(),
        worktree_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
    };

    let error = inspect_repository(&binding)
        .err()
        .ok_or_else(|| "primary checkout unexpectedly passed write preflight".to_string())?;
    assert!(error.contains("linked managed worktree"), "{error}");
    Ok(())
}

#[test]
fn fake_admin_outside_common_worktrees_is_blocked_without_mutation() -> Result<(), String> {
    let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
    let main = sandbox.path().join("main");
    let managed_root = sandbox.path().join("managed");
    fs::create_dir(&main).map_err(|error| error.to_string())?;
    run(&main, &["init", "-q"])?;
    run(&main, &["config", "user.name", "SchoolX Test"])?;
    run(&main, &["config", "user.email", "schoolx@example.test"])?;
    fs::write(main.join("tracked.txt"), "base\n").map_err(|error| error.to_string())?;
    run(&main, &["add", "tracked.txt"])?;
    run(&main, &["commit", "-q", "-m", "base"])?;
    let managed_text = managed_root
        .to_str()
        .ok_or_else(|| "managed worktree path is not UTF-8".to_string())?;
    run(
        &main,
        &["worktree", "add", "-q", "--detach", managed_text, "HEAD"],
    )?;

    let head = run(&managed_root, &["rev-parse", "HEAD"])?
        .trim()
        .to_string();
    let real_admin = git_path(
        &managed_root,
        &run(&managed_root, &["rev-parse", "--git-dir"])?,
    )?;
    let common = git_path(
        &managed_root,
        &run(&managed_root, &["rev-parse", "--git-common-dir"])?,
    )?;
    let fake_admin = sandbox
        .path()
        .join("foreign")
        .join("worktrees")
        .join("fake");
    fs::create_dir_all(&fake_admin).map_err(|error| error.to_string())?;
    fs::copy(real_admin.join("HEAD"), fake_admin.join("HEAD"))
        .map_err(|error| error.to_string())?;
    fs::copy(real_admin.join("index"), fake_admin.join("index"))
        .map_err(|error| error.to_string())?;
    fs::write(
        fake_admin.join("gitdir"),
        format!("{}\n", managed_root.join(".git").display()),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        fake_admin.join("commondir"),
        format!("{}\n", common.display()),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        managed_root.join(".git"),
        format!("gitdir: {}\n", fake_admin.display()),
    )
    .map_err(|error| error.to_string())?;

    let before_objects = run(&main, &["count-objects", "-v"])?;
    let before_real_head = fs::read(real_admin.join("HEAD")).map_err(|error| error.to_string())?;
    let before_real_index =
        fs::read(real_admin.join("index")).map_err(|error| error.to_string())?;
    let before_fake_head = fs::read(fake_admin.join("HEAD")).map_err(|error| error.to_string())?;
    let before_fake_index =
        fs::read(fake_admin.join("index")).map_err(|error| error.to_string())?;
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let scope = CodeThreadBindingScope {
        community_id: "community-fake-admin".to_string(),
        project_dtag: "project-fake-admin".to_string(),
        repository_identity: crate::code_workspace::repository_identity(&common)?,
    };
    let binding = CodeThreadBinding {
        community_id: scope.community_id.clone(),
        project_dtag: scope.project_dtag.clone(),
        repository_identity: scope.repository_identity.clone(),
        codex_thread_id: "thread-fake-admin".to_string(),
        execution_mode: CodeExecutionMode::Worktree,
        execution_root: managed_root
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .to_string(),
        base_ref: head,
        worktree_id: Some("33333333-3333-4333-8333-333333333333".to_string()),
    };
    let blocked = status(
        &CodeGitWriteState::default(),
        CodeGitStatusInput {
            scope,
            thread_id: binding.codex_thread_id.clone(),
        },
        context(app_data.path(), &binding),
    )?;
    match blocked {
        CodeGitStatus::Blocked { reason, .. } => {
            assert!(reason.contains("linked managed worktree"), "{reason}");
        }
        other => {
            return Err(format!(
                "fake Git admin unexpectedly passed preflight: {other:?}"
            ))
        }
    }

    assert_eq!(run(&main, &["count-objects", "-v"])?, before_objects);
    assert_eq!(
        fs::read(real_admin.join("HEAD")).map_err(|error| error.to_string())?,
        before_real_head
    );
    assert_eq!(
        fs::read(real_admin.join("index")).map_err(|error| error.to_string())?,
        before_real_index
    );
    assert_eq!(
        fs::read(fake_admin.join("HEAD")).map_err(|error| error.to_string())?,
        before_fake_head
    );
    assert_eq!(
        fs::read(fake_admin.join("index")).map_err(|error| error.to_string())?,
        before_fake_index
    );
    assert!(!app_data.path().join("code").exists());
    Ok(())
}

#[test]
fn magic_pathspec_filename_is_diffed_without_sibling_aggregation() -> Result<(), String> {
    let repository = tempfile::tempdir().map_err(|error| error.to_string())?;
    run(repository.path(), &["init", "-q"])?;
    run(repository.path(), &["config", "user.name", "SchoolX Test"])?;
    run(
        repository.path(),
        &["config", "user.email", "schoolx@example.test"],
    )?;
    let magic_path = ":(glob)*";
    let sibling_path = "sibling.txt";
    fs::write(repository.path().join(magic_path), "base\n").map_err(|error| error.to_string())?;
    fs::write(repository.path().join(sibling_path), "base\n").map_err(|error| error.to_string())?;
    run(
        repository.path(),
        &["--literal-pathspecs", "add", "--", magic_path, sibling_path],
    )?;
    run(repository.path(), &["commit", "-q", "-m", "base"])?;

    let worktree_parent = tempfile::tempdir().map_err(|error| error.to_string())?;
    let managed_root = worktree_parent.path().join("managed");
    let managed_text = managed_root
        .to_str()
        .ok_or_else(|| "managed worktree path is not UTF-8".to_string())?;
    run(
        repository.path(),
        &["worktree", "add", "-q", "--detach", managed_text, "HEAD"],
    )?;
    fs::write(managed_root.join(magic_path), "target\n").map_err(|error| error.to_string())?;
    fs::write(
        managed_root.join(sibling_path),
        "sibling one\nsibling two\n",
    )
    .map_err(|error| error.to_string())?;

    let common = git_path(
        &managed_root,
        &run(&managed_root, &["rev-parse", "--git-common-dir"])?,
    )?;
    let scope = CodeThreadBindingScope {
        community_id: "community-literal-path".to_string(),
        project_dtag: "project-literal-path".to_string(),
        repository_identity: crate::code_workspace::repository_identity(&common)?,
    };
    let binding = CodeThreadBinding {
        community_id: scope.community_id.clone(),
        project_dtag: scope.project_dtag.clone(),
        repository_identity: scope.repository_identity.clone(),
        codex_thread_id: "thread-literal-path".to_string(),
        execution_mode: CodeExecutionMode::Worktree,
        execution_root: managed_root
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .to_string(),
        base_ref: run(&managed_root, &["rev-parse", "HEAD"])?
            .trim()
            .to_string(),
        worktree_id: Some("44444444-4444-4444-8444-444444444444".to_string()),
    };
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let ready = ready(status(
        &CodeGitWriteState::default(),
        CodeGitStatusInput {
            scope,
            thread_id: binding.codex_thread_id.clone(),
        },
        context(app_data.path(), &binding),
    )?)?;

    assert_eq!(ready.unstaged.total_files, 2);
    assert_eq!(ready.unstaged.additions, 3);
    assert_eq!(ready.unstaged.deletions, 2);
    let magic = ready
        .unstaged
        .files
        .iter()
        .find(|file| file.path == magic_path)
        .ok_or_else(|| "literal pathspec fixture row is missing".to_string())?;
    assert_eq!((magic.additions, magic.deletions), (1, 1));
    assert!(!magic.patch.contains("sibling two"));
    let sibling = ready
        .unstaged
        .files
        .iter()
        .find(|file| file.path == sibling_path)
        .ok_or_else(|| "sibling fixture row is missing".to_string())?;
    assert_eq!((sibling.additions, sibling.deletions), (2, 1));
    Ok(())
}

#[test]
fn whole_file_stage_ack_and_staged_only_commit_preserve_unstaged_bytes() -> Result<(), String> {
    let repository = tempfile::tempdir().map_err(|error| error.to_string())?;
    run(repository.path(), &["init", "-q"])?;
    run(
        repository.path(),
        &["config", "--local", "user.name", "Human Name"],
    )?;
    run(
        repository.path(),
        &["config", "--local", "user.email", "human@example.com"],
    )?;
    fs::write(repository.path().join("tracked.txt"), "base\n")
        .map_err(|error| error.to_string())?;
    run(repository.path(), &["add", "tracked.txt"])?;
    run(repository.path(), &["commit", "-q", "-m", "base"])?;
    let worktree_parent = tempfile::tempdir().map_err(|error| error.to_string())?;
    let managed_root = worktree_parent.path().join("managed");
    let managed_text = managed_root
        .to_str()
        .ok_or_else(|| "managed worktree path is not UTF-8".to_string())?;
    run(
        repository.path(),
        &["worktree", "add", "-q", "--detach", managed_text, "HEAD"],
    )?;
    fs::write(managed_root.join("tracked.txt"), "staged version\n")
        .map_err(|error| error.to_string())?;

    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    create_private_directory(&app_data.path().join("code"))?;
    let common = PathBuf::from(run(&managed_root, &["rev-parse", "--git-common-dir"])?.trim());
    let common = if common.is_absolute() {
        common
    } else {
        managed_root.join(common)
    }
    .canonicalize()
    .map_err(|error| error.to_string())?;
    let scope = CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: crate::code_workspace::repository_identity(&common)?,
    };
    let binding = CodeThreadBinding {
        community_id: scope.community_id.clone(),
        project_dtag: scope.project_dtag.clone(),
        repository_identity: scope.repository_identity.clone(),
        codex_thread_id: "thread-1".to_string(),
        execution_mode: CodeExecutionMode::Worktree,
        execution_root: managed_root
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .to_string(),
        base_ref: run(&managed_root, &["rev-parse", "HEAD"])?
            .trim()
            .to_string(),
        worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
    };
    let state = CodeGitWriteState::default();
    let input = CodeGitStatusInput {
        scope: scope.clone(),
        thread_id: "thread-1".to_string(),
    };
    let initial = ready(status(
        &state,
        input.clone(),
        context(app_data.path(), &binding),
    )?)?;
    let file = initial
        .unstaged
        .files
        .first()
        .ok_or_else(|| "fixture did not expose an unstaged file".to_string())?;
    let stage_receipt = stage(
        &state,
        app_data.path(),
        &binding,
        CodeGitIndexMutationInput {
            scope: scope.clone(),
            thread_id: input.thread_id.clone(),
            write_generation: initial.write_generation,
            snapshot_id: initial.snapshot_id,
            file_id: file.file_id.clone(),
        },
    )
    .map_err(|error| format!("stage mutation failed: {error}"))?;
    assert_eq!(
        run(&managed_root, &["diff", "--cached", "--name-only"])?.trim(),
        "tracked.txt"
    );

    let after_stage = ready(status(
        &state,
        input.clone(),
        context(app_data.path(), &binding),
    )?)?;
    assert_eq!(
        after_stage
            .blocking_receipt
            .as_ref()
            .map(CodeGitMutationReceipt::operation_id),
        Some(stage_receipt.operation_id.as_str())
    );
    acknowledge(
        &state,
        app_data.path(),
        CodeGitAcknowledgeInput {
            scope: scope.clone(),
            thread_id: input.thread_id.clone(),
            operation_id: stage_receipt.operation_id,
            write_generation: after_stage.write_generation,
            snapshot_id: after_stage.snapshot_id,
        },
    )?;

    fs::write(managed_root.join("tracked.txt"), "unstaged version\n")
        .map_err(|error| error.to_string())?;
    let before_commit = ready(status(&state, input, context(app_data.path(), &binding))?)?;
    assert_eq!(before_commit.staged.total_files, 1);
    assert_eq!(before_commit.unstaged.total_files, 1);
    let commit_receipt = commit(
        &state,
        app_data.path(),
        &binding,
        CodeGitCommitInput {
            scope,
            thread_id: "thread-1".to_string(),
            write_generation: before_commit.write_generation,
            snapshot_id: before_commit.snapshot_id,
            message: "Commit staged version".to_string(),
        },
    )
    .map_err(|error| format!("commit mutation failed: {error}"))?;
    assert_eq!(
        run(&managed_root, &["show", "HEAD:tracked.txt"])?,
        "staged version\n"
    );
    assert_eq!(
        fs::read_to_string(managed_root.join("tracked.txt")).map_err(|error| error.to_string())?,
        "unstaged version\n"
    );
    assert_eq!(
        run(&managed_root, &["rev-parse", "HEAD"])?.trim(),
        commit_receipt.commit
    );
    Ok(())
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt as _;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(path).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn create_private_directory(_path: &Path) -> Result<(), String> {
    Err("Git write integration test requires Unix".to_string())
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

fn ready(status: CodeGitStatus) -> Result<ReadyView, String> {
    match status {
        CodeGitStatus::Ready {
            write_generation,
            snapshot_id,
            staged,
            unstaged,
            blocking_receipt,
            ..
        } => Ok(ReadyView {
            write_generation,
            snapshot_id,
            staged: *staged,
            unstaged: *unstaged,
            blocking_receipt: blocking_receipt.map(|receipt| *receipt),
        }),
        other => Err(format!("expected ready Git status, got {other:?}")),
    }
}

struct ReadyView {
    write_generation: u64,
    snapshot_id: String,
    staged: CodeGitChangeSet,
    unstaged: CodeGitChangeSet,
    blocking_receipt: Option<CodeGitMutationReceipt>,
}

fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn git_path(root: &Path, value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value.trim());
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    path.canonicalize().map_err(|error| error.to_string())
}
