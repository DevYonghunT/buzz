//! Cross-gate admission tests for exact-bound Git writes.

#![cfg(unix)]

use super::*;
use crate::code_workspace::{
    code_thread_source, prepare_execution_root, CodeExecutionMode, CodeGitChangeSet,
    CodeGitIndexMutationInput, CodeGitStatus, CodeGitStatusInput, CodeTerminalOpenInput,
    CodeThreadBinding, CodeThreadBindingLookupInput, CodeThreadBindingScope, CodeThreadForkInput,
    CodeThreadLifecycleInput, CodeThreadLifecycleStatus, CodeTurnStartInput, CodeTurnSteerInput,
    CodeWorktreePrepareInput, CodeWorktreeRemoveInput,
};
use crate::commands::code_workspace::tests::{
    create_test_repository, method_count, persisted_local_binding, stateful_fake_codex, FakeCodex,
    TestRepository,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

const THREAD_ID: &str = "thread-git-gate";
const PREPARATION_ID: &str = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";

#[derive(Clone)]
struct StageCoordinate {
    write_generation: u64,
    snapshot_id: String,
    file_id: String,
}

struct GateFixture {
    _app_data: tempfile::TempDir,
    _nest: tempfile::TempDir,
    _repository: TestRepository,
    app_data: PathBuf,
    nest: PathBuf,
    scope: CodeThreadBindingScope,
    binding: CodeThreadBinding,
    stage: StageCoordinate,
    git_state: crate::code_workspace::CodeGitWriteState,
    runtime: crate::code_workspace::CodeRuntime,
    terminals: crate::code_workspace::CodeTerminalManager,
    binding_lock: Arc<Mutex<()>>,
    lifecycle_authority: Arc<AtomicBool>,
    fake: FakeCodex,
}

impl GateFixture {
    fn new() -> Result<Self, String> {
        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let repository = create_test_repository()?;
        let (scope, local_binding) = persisted_local_binding(&repository, "local-probe")?;
        let worktree = prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: local_binding.execution_root,
                base_ref: local_binding.base_ref,
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;
        let binding = CodeThreadBinding {
            community_id: scope.community_id.clone(),
            project_dtag: scope.project_dtag.clone(),
            repository_identity: worktree.descriptor.repository_identity.clone(),
            codex_thread_id: THREAD_ID.to_string(),
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: worktree.descriptor.execution_root.clone(),
            base_ref: worktree.descriptor.base_ref.clone(),
            worktree_id: worktree.descriptor.worktree_id.clone(),
        };
        let scope = CodeThreadBindingScope {
            repository_identity: binding.repository_identity.clone(),
            ..scope
        };
        let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
        store.upsert(binding.clone())?;
        fs::write(
            Path::new(&binding.execution_root).join("README.md"),
            b"gate change\n",
        )
        .map_err(|error| error.to_string())?;

        let source = code_thread_source(PREPARATION_ID)?;
        let fake = stateful_fake_codex(&binding.execution_root, &source, THREAD_ID, false)?;
        fake.mark_created()?;
        let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
        runtime.start(Arc::new(|_| {}))?;
        let lifecycle_authority = Arc::new(AtomicBool::new(false));
        crate::commands::code_thread_lifecycle::reconcile_all_thread_lifecycles(
            &store,
            &runtime,
            &lifecycle_authority,
        )?;
        if !lifecycle_authority.load(Ordering::Acquire) {
            return Err("fixture lifecycle authority was not established".to_string());
        }

        let git_state = crate::code_workspace::CodeGitWriteState::default();
        let git_status = crate::code_workspace::git_write::status(
            &git_state,
            CodeGitStatusInput {
                scope: scope.clone(),
                thread_id: THREAD_ID.to_string(),
            },
            crate::code_workspace::git_write::GitWriteContext {
                app_data_dir: app_data.path().to_path_buf(),
                binding: binding.clone(),
                runtime_generation: runtime.status()?.generation,
                task: CodeGitChangeSet {
                    files: Vec::new(),
                    total_files: 0,
                    files_truncated: false,
                    additions: 0,
                    deletions: 0,
                },
                activity_blocker: None,
            },
        )?;
        let stage = match git_status {
            CodeGitStatus::Ready {
                write_generation,
                snapshot_id,
                unstaged,
                ..
            } => StageCoordinate {
                write_generation,
                snapshot_id,
                file_id: unstaged
                    .files
                    .iter()
                    .find(|file| file.path == "README.md")
                    .ok_or_else(|| "fixture did not expose README.md as unstaged".to_string())?
                    .file_id
                    .clone(),
            },
            status => return Err(format!("fixture Git status was not ready: {status:?}")),
        };

        Ok(Self {
            app_data: app_data.path().to_path_buf(),
            nest: nest.path().to_path_buf(),
            _app_data: app_data,
            _nest: nest,
            _repository: repository,
            scope,
            binding,
            stage,
            git_state,
            runtime,
            terminals: crate::code_workspace::CodeTerminalManager::new(),
            binding_lock: Arc::new(Mutex::new(())),
            lifecycle_authority,
            fake,
        })
    }

    fn stage_input(&self) -> CodeGitIndexMutationInput {
        CodeGitIndexMutationInput {
            scope: self.scope.clone(),
            thread_id: THREAD_ID.to_string(),
            write_generation: self.stage.write_generation,
            snapshot_id: self.stage.snapshot_id.clone(),
            file_id: self.stage.file_id.clone(),
        }
    }

    fn stage_with_clearance(&self) -> Result<(), String> {
        let input = self.stage_input();
        with_mutation_clearance_for_test(
            &self.app_data,
            &self.nest,
            (&self.runtime, &self.terminals, &self.binding_lock),
            &self.scope,
            THREAD_ID,
            |app_data, binding| {
                crate::code_workspace::git_write::stage(&self.git_state, app_data, binding, input)
                    .map(|_| ())
            },
        )
    }

    fn create_durable_blocker(&self) -> Result<(), String> {
        crate::code_workspace::git_write::stage(
            &self.git_state,
            &self.app_data,
            &self.binding,
            self.stage_input(),
        )
        .map(|_| ())
    }

    fn evidence(&self) -> Result<MutationEvidence, String> {
        MutationEvidence::capture(&self.app_data, Path::new(&self.binding.execution_root))
    }

    fn turn_start_input(&self) -> CodeTurnStartInput {
        CodeTurnStartInput {
            scope: self.scope.clone(),
            thread_id: THREAD_ID.to_string(),
            prompt: "Exercise the admission gate".to_string(),
            model: None,
            effort: None,
        }
    }
}

impl Drop for GateFixture {
    fn drop(&mut self) {
        let _ = self.terminals.shutdown();
        let _ = self.runtime.stop();
    }
}

#[derive(Debug, Eq, PartialEq)]
struct MutationEvidence {
    journal: Option<Vec<u8>>,
    common_git: BTreeMap<PathBuf, TreeEntry>,
    worktree: BTreeMap<PathBuf, TreeEntry>,
}

#[derive(Debug, Eq, PartialEq)]
enum TreeEntry {
    Directory,
    File { len: u64, sha256: String },
    Symlink(PathBuf),
}

impl MutationEvidence {
    fn capture(app_data: &Path, root: &Path) -> Result<Self, String> {
        let common = absolute_git_path(root, &git_line(root, &["rev-parse", "--git-common-dir"])?)?;
        Ok(Self {
            journal: read_optional(&app_data.join("code").join("git-operations.json"))?,
            common_git: snapshot_tree(&common)?,
            worktree: snapshot_tree(root)?,
        })
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

fn snapshot_tree(root: &Path) -> Result<BTreeMap<PathBuf, TreeEntry>, String> {
    fn visit(
        root: &Path,
        current: &Path,
        entries: &mut BTreeMap<PathBuf, TreeEntry>,
    ) -> Result<(), String> {
        let mut children = fs::read_dir(current)
            .map_err(|error| format!("failed to read {}: {error}", current.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            let path = child.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_path_buf();
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink() {
                entries.insert(
                    relative,
                    TreeEntry::Symlink(fs::read_link(&path).map_err(|error| error.to_string())?),
                );
            } else if metadata.is_dir() {
                entries.insert(relative, TreeEntry::Directory);
                visit(root, &path, entries)?;
            } else if metadata.is_file() {
                let bytes = fs::read(&path).map_err(|error| error.to_string())?;
                entries.insert(
                    relative,
                    TreeEntry::File {
                        len: metadata.len(),
                        sha256: hex_digest(&bytes),
                    },
                );
            } else {
                return Err(format!("unsupported fixture entry {}", path.display()));
            }
        }
        Ok(())
    }

    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries)?;
    Ok(entries)
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn absolute_git_path(root: &Path, value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    path.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize Git fixture path {}: {error}",
            path.display()
        )
    })
}

fn git_line(root: &Path, args: &[&str]) -> Result<String, String> {
    let executable = crate::managed_agents::resolve_command("git")
        .ok_or_else(|| "git executable was not found".to_string())?;
    let output = Command::new(executable)
        .arg("--no-pager")
        .args(args)
        .current_dir(root)
        .env_remove("GIT_NO_REPLACE_OBJECTS")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| format!("failed to run test git: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
        .map_err(|error| error.to_string())
}

fn assert_git_unchanged(fixture: &GateFixture, before: &MutationEvidence) -> Result<(), String> {
    assert_eq!(&fixture.evidence()?, before);
    Ok(())
}

fn wait_for_file(path: &Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while !path.is_file() {
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for {}", path.display()));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

#[test]
fn durable_git_blocker_precedes_turn_terminal_fork_archive_and_remove_mutations(
) -> Result<(), String> {
    let fixture = GateFixture::new()?;
    fixture.create_durable_blocker()?;
    let git_before = fixture.evidence()?;
    let requests_before = fixture.fake.recorded_requests()?;
    let store = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let binding_before = fs::read(store.store_path()).map_err(|error| error.to_string())?;

    let turn_error = crate::commands::code_workspace::start_turn_for_test(
        fixture.turn_start_input(),
        &fixture.app_data,
        &fixture.nest,
        &fixture.runtime,
        &fixture.binding_lock,
        &fixture.lifecycle_authority,
    )
    .expect_err("durable Git blocker must reject turn/start");
    assert!(turn_error.contains("Git operation"), "{turn_error}");

    let steer_error = crate::commands::code_workspace::steer_turn_for_test(
        CodeTurnSteerInput {
            scope: fixture.scope.clone(),
            thread_id: THREAD_ID.to_string(),
            expected_turn_id: "turn-git-gate".to_string(),
            prompt: "Steer through the gate".to_string(),
        },
        &fixture.app_data,
        &fixture.nest,
        &fixture.runtime,
        &fixture.binding_lock,
        &fixture.lifecycle_authority,
    )
    .expect_err("durable Git blocker must reject turn/steer");
    assert!(steer_error.contains("Git operation"), "{steer_error}");

    let terminal_error = crate::commands::code_terminal::open_terminal_for_test(
        CodeTerminalOpenInput {
            scope: fixture.scope.clone(),
            thread_id: THREAD_ID.to_string(),
            cols: 80,
            rows: 24,
        },
        Channel::new(|_| Ok(())),
        &fixture.app_data,
        &fixture.nest,
        (
            &fixture.runtime,
            &fixture.terminals,
            &fixture.binding_lock,
            &fixture.lifecycle_authority,
        ),
    )
    .expect_err("durable Git blocker must reject PTY open");
    assert!(terminal_error.contains("Git operation"), "{terminal_error}");
    fixture
        .terminals
        .ensure_owner_absent(&fixture.scope, THREAD_ID)?;

    let fork_error = crate::commands::code_thread_fork::fork_thread_for_test(
        CodeThreadForkInput {
            scope: fixture.scope.clone(),
            thread_id: THREAD_ID.to_string(),
        },
        &fixture.app_data,
        &fixture.nest,
        &fixture.runtime,
        &fixture.terminals,
        &fixture.binding_lock,
        &fixture.lifecycle_authority,
    )
    .expect_err("durable Git blocker must reject fork");
    assert_eq!(fork_error.code, "sourceGitRecoveryRequired");

    let archive_error = crate::commands::code_thread_lifecycle::archive_thread_for_test(
        CodeThreadLifecycleInput {
            scope: fixture.scope.clone(),
            thread_id: THREAD_ID.to_string(),
        },
        &fixture.app_data,
        &fixture.nest,
        &fixture.runtime,
        &fixture.terminals,
        &fixture.binding_lock,
        &fixture.lifecycle_authority,
    )
    .expect_err("durable Git blocker must reject archive");
    assert!(archive_error.contains("Git operation"), "{archive_error}");

    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        binding_before,
        "pre-removal gates must not alter the binding store"
    );
    let lookup = CodeThreadBindingLookupInput {
        scope: fixture.scope.clone(),
        codex_thread_id: THREAD_ID.to_string(),
    };
    let claim = store.begin_archive(&lookup)?;
    store.complete_lifecycle_transition(&claim)?;
    let archived_before_remove = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let remove_error = crate::commands::code_workspace::remove_worktree_for_test(
        CodeWorktreeRemoveInput {
            scope: fixture.scope.clone(),
            thread_id: THREAD_ID.to_string(),
        },
        &fixture.app_data,
        &fixture.nest,
        &fixture.binding_lock,
        (
            &fixture.runtime,
            &fixture.terminals,
            &fixture.lifecycle_authority,
            &AtomicBool::new(false),
        ),
    )
    .expect_err("durable Git blocker must reject physical removal");
    assert!(remove_error.contains("Git operation"), "{remove_error}");
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        archived_before_remove,
        "remove gate must fail before a removal claim"
    );
    assert_eq!(
        store
            .lookup_with_lifecycle(&lookup)?
            .ok_or_else(|| "archived fixture binding disappeared".to_string())?
            .status,
        CodeThreadLifecycleStatus::Archived
    );
    assert!(Path::new(&fixture.binding.execution_root).is_dir());
    assert_eq!(fixture.fake.recorded_requests()?, requests_before);
    assert_git_unchanged(&fixture, &git_before)
}

#[test]
fn active_turn_blocks_git_before_journal_or_repository_mutation() -> Result<(), String> {
    let fixture = GateFixture::new()?;
    fixture
        .runtime
        .turn_start_at(fixture.turn_start_input(), &fixture.binding.execution_root)?;
    let before = fixture.evidence()?;

    let error = fixture
        .stage_with_clearance()
        .expect_err("active turn must reject Git mutation");
    assert!(error.contains("active, starting, or uncertain"), "{error}");
    assert_git_unchanged(&fixture, &before)
}

#[test]
fn starting_turn_blocks_git_before_journal_or_repository_mutation() -> Result<(), String> {
    let fixture = GateFixture::new()?;
    fixture.fake.block_turn_start()?;
    let runtime = fixture.runtime.clone();
    let input = fixture.turn_start_input();
    let execution_root = fixture.binding.execution_root.clone();
    let turn = std::thread::spawn(move || runtime.turn_start_at(input, &execution_root));
    let admitted = wait_for_file(
        &fixture.fake.turn_start_admitted_marker(),
        Duration::from_secs(5),
    );
    if let Err(error) = admitted {
        fixture.fake.release_turn_start()?;
        let _ = turn.join();
        return Err(error);
    }
    let before = fixture.evidence()?;
    let mutation = fixture.stage_with_clearance();
    fixture.fake.release_turn_start()?;
    turn.join()
        .map_err(|_| "turn/start fixture thread panicked".to_string())??;

    let error = mutation.expect_err("starting turn must reject Git mutation");
    assert!(error.contains("active, starting, or uncertain"), "{error}");
    assert_git_unchanged(&fixture, &before)
}

#[test]
fn uncertain_turn_blocks_git_before_journal_or_repository_mutation() -> Result<(), String> {
    let fixture = GateFixture::new()?;
    fixture.fake.fail_turn_start()?;
    fixture
        .runtime
        .turn_start_at(fixture.turn_start_input(), &fixture.binding.execution_root)
        .expect_err("fixture turn/start must become uncertain");
    let before = fixture.evidence()?;

    let error = fixture
        .stage_with_clearance()
        .expect_err("uncertain turn must reject Git mutation");
    assert!(error.contains("active, starting, or uncertain"), "{error}");
    assert_git_unchanged(&fixture, &before)
}

#[test]
fn pending_approval_blocks_git_before_journal_or_repository_mutation() -> Result<(), String> {
    let fixture = GateFixture::new()?;
    let generation = fixture.runtime.status()?.generation;
    fixture
        .runtime
        .insert_pending_approval_for_test(generation, "approval-git-gate", THREAD_ID)?;
    let before = fixture.evidence()?;

    let error = fixture
        .stage_with_clearance()
        .expect_err("pending approval must reject Git mutation");
    assert!(error.contains("pending approval"), "{error}");
    assert!(fixture.runtime.has_pending_approval(THREAD_ID)?);
    assert_git_unchanged(&fixture, &before)
}

#[test]
fn pty_owner_blocks_git_before_journal_or_repository_mutation() -> Result<(), String> {
    let fixture = GateFixture::new()?;
    let drain_marker = fixture.app_data.join("terminal-drained-unexpectedly");
    fixture
        .terminals
        .install_test_owner(&fixture.scope, THREAD_ID, drain_marker.clone())?;
    let before = fixture.evidence()?;

    let error = fixture
        .stage_with_clearance()
        .expect_err("PTY owner must reject Git mutation");
    assert!(error.contains("terminal"), "{error}");
    assert!(
        !drain_marker.exists(),
        "Git admission must not close the PTY"
    );
    assert!(
        fixture
            .terminals
            .ensure_owner_absent(&fixture.scope, THREAD_ID)
            .is_err(),
        "the blocking PTY owner must remain registered"
    );
    assert_git_unchanged(&fixture, &before)
}

#[test]
fn concurrent_git_admission_wins_before_turn_without_deadlock() -> Result<(), String> {
    let fixture = GateFixture::new()?;
    let (git_entered_tx, git_entered_rx) = mpsc::sync_channel(1);
    let (release_git_tx, release_git_rx) = mpsc::sync_channel(1);
    let (git_done_tx, git_done_rx) = mpsc::sync_channel(1);

    let git_thread = {
        let app_data = fixture.app_data.clone();
        let nest = fixture.nest.clone();
        let runtime = fixture.runtime.clone();
        let terminals = fixture.terminals.clone();
        let binding_lock = Arc::clone(&fixture.binding_lock);
        let scope = fixture.scope.clone();
        let git_state = fixture.git_state.clone();
        let input = fixture.stage_input();
        std::thread::spawn(move || {
            let result = with_mutation_clearance_for_test(
                &app_data,
                &nest,
                (&runtime, &terminals, &binding_lock),
                &scope,
                THREAD_ID,
                |app_data, binding| {
                    git_entered_tx
                        .send(())
                        .map_err(|_| "Git admission handoff receiver disappeared".to_string())?;
                    release_git_rx
                        .recv_timeout(Duration::from_secs(10))
                        .map_err(|_| "Git admission release timed out".to_string())?;
                    crate::code_workspace::git_write::stage(&git_state, app_data, binding, input)
                        .map(|_| ())
                },
            );
            let _ = git_done_tx.send(result);
        })
    };
    git_entered_rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| {
            "Git admission did not retain the binding/runtime gate (possible deadlock)".to_string()
        })?;

    let (turn_attempted_tx, turn_attempted_rx) = mpsc::sync_channel(1);
    let (turn_done_tx, turn_done_rx) = mpsc::sync_channel(1);
    let turn_thread = {
        let app_data = fixture.app_data.clone();
        let nest = fixture.nest.clone();
        let runtime = fixture.runtime.clone();
        let binding_lock = Arc::clone(&fixture.binding_lock);
        let lifecycle_authority = Arc::clone(&fixture.lifecycle_authority);
        let input = fixture.turn_start_input();
        std::thread::spawn(move || {
            let _ = turn_attempted_tx.send(());
            let result = crate::commands::code_workspace::start_turn_for_test(
                input,
                &app_data,
                &nest,
                &runtime,
                &binding_lock,
                &lifecycle_authority,
            )
            .map(|_| ());
            let _ = turn_done_tx.send(result);
        })
    };

    turn_attempted_rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "turn admission did not start".to_string())?;
    release_git_tx
        .send(())
        .map_err(|_| "Git admission release receiver disappeared".to_string())?;
    let git_result = git_done_rx
        .recv_timeout(Duration::from_secs(20))
        .map_err(|_| "Git admission timed out (possible deadlock)".to_string())?;
    let turn_result = turn_done_rx
        .recv_timeout(Duration::from_secs(20))
        .map_err(|_| "turn admission timed out (possible deadlock)".to_string())?;
    git_thread
        .join()
        .map_err(|_| "Git admission thread panicked".to_string())?;
    turn_thread
        .join()
        .map_err(|_| "turn admission thread panicked".to_string())?;

    assert!(
        git_result.is_ok(),
        "Git must win after retaining binding/runtime admission: {git_result:?}"
    );
    assert!(
        turn_result
            .as_ref()
            .is_err_and(|error| error.contains("Git operation")),
        "the concurrently admitted turn must lose to the durable Git blocker: {turn_result:?}"
    );
    assert!(fixture.evidence()?.journal.is_some());
    assert_eq!(
        method_count(&fixture.fake.recorded_requests()?, "turn/start"),
        0
    );
    Ok(())
}
