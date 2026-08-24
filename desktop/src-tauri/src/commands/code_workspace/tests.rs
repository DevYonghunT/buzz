use super::*;
use crate::code_workspace::bindings::{CodeExecutionMode, CodeThreadPreparationState};
use crate::code_workspace::{
    CodeRuntimeActiveTurnCheckpoint, CodeRuntimeApprovalCheckpoint, CodeRuntimeEventCheckpoint,
};
use serde_json::json;
use std::fs;
use std::process::Command;

fn scope() -> CodeThreadBindingScope {
    CodeThreadBindingScope {
        community_id: "community-a".to_string(),
        project_dtag: "project-a".to_string(),
        repository_identity: "a".repeat(64),
    }
}

fn runtime_event(thread_id: Option<&str>, sequence: u64) -> CodeRuntimeEvent {
    CodeRuntimeEvent {
        runtime_generation: 4,
        sequence,
        thread_id: thread_id.map(str::to_string),
        turn_id: Some("turn-a".to_string()),
        item_id: None,
        kind: "turn/started".to_string(),
        payload: json!({ "sequence": sequence }),
    }
}

fn local_binding(execution_root: &Path, thread_id: &str) -> CodeThreadBinding {
    let scope = scope();
    let execution_root = execution_root
        .canonicalize()
        .unwrap_or_else(|_| execution_root.to_path_buf());
    CodeThreadBinding {
        community_id: scope.community_id,
        project_dtag: scope.project_dtag,
        repository_identity: scope.repository_identity,
        codex_thread_id: thread_id.to_string(),
        execution_mode: CodeExecutionMode::Local,
        execution_root: execution_root.to_string_lossy().into_owned(),
        base_ref: "b".repeat(40),
        worktree_id: None,
    }
}

#[test]
fn current_diff_mapping_preserves_status_binary_and_completeness_contract() {
    let statuses = [
        CurrentRepoChangeStatus::Added,
        CurrentRepoChangeStatus::Modified,
        CurrentRepoChangeStatus::Deleted,
        CurrentRepoChangeStatus::TypeChanged,
        CurrentRepoChangeStatus::Unmerged,
        CurrentRepoChangeStatus::Untracked,
    ];
    let diff = CurrentRepoDiffInfo {
        files: statuses
            .into_iter()
            .enumerate()
            .map(
                |(index, status)| crate::commands::project_git_diff::CurrentRepoDiffFileInfo {
                    path: format!("file-{index}"),
                    status,
                    binary: index == 1,
                    additions: index,
                    deletions: index.saturating_add(1),
                    patch: format!("patch-{index}"),
                    truncated: index == 2,
                },
            )
            .collect(),
        total_files: 9,
        files_truncated: true,
        additions: 15,
        deletions: 21,
    };

    let mapped = code_thread_changes_from_current_diff(diff);
    assert_eq!(mapped.total_files, 9);
    assert!(mapped.files_truncated);
    assert_eq!(mapped.additions, 15);
    assert_eq!(mapped.deletions, 21);
    assert!(mapped.commit_body.is_none());
    assert_eq!(
        mapped
            .files
            .iter()
            .map(|file| file.status)
            .collect::<Vec<_>>(),
        vec![
            CodeThreadChangeStatus::Added,
            CodeThreadChangeStatus::Modified,
            CodeThreadChangeStatus::Deleted,
            CodeThreadChangeStatus::TypeChanged,
            CodeThreadChangeStatus::Unmerged,
            CodeThreadChangeStatus::Untracked,
        ]
    );
    assert!(mapped.files[1].binary);
    assert!(mapped.files[2].truncated);
}

fn recovery_preparation(
    execution_root: &str,
    baseline: Option<Vec<&str>>,
) -> CodeThreadPreparation {
    let scope = scope();
    CodeThreadPreparation {
        preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
        community_id: scope.community_id,
        project_dtag: scope.project_dtag,
        repository_identity: scope.repository_identity,
        execution_mode: CodeExecutionMode::Local,
        execution_root: execution_root.to_string(),
        base_ref: "b".repeat(40),
        worktree_id: None,
        operation: crate::code_workspace::CodeThreadPreparationOperation::Start,
        source_thread_id: None,
        state: crate::code_workspace::bindings::CodeThreadPreparationState::Starting,
        recovery_thread_baseline: baseline
            .map(|thread_ids| thread_ids.into_iter().map(str::to_string).collect()),
        merge_target_ref: None,
    }
}

fn recovery_candidate(
    thread_id: &str,
    execution_root: Option<&str>,
    thread_source: Option<&str>,
) -> CodeRecoveryThread {
    CodeRecoveryThread {
        thread: crate::code_workspace::CodeThreadSummary {
            id: thread_id.to_string(),
            session_id: None,
            forked_from_id: None,
            parent_thread_id: None,
            preview: None,
            ephemeral: false,
            model_provider: None,
            created_at: None,
            updated_at: None,
            cwd: execution_root.map(str::to_string),
            name: None,
            status: None,
            turns: Vec::new(),
        },
        thread_source: thread_source.map(str::to_string),
        session_source: None,
        ephemeral_present: false,
    }
}

#[cfg(unix)]
pub(crate) struct FakeCodex {
    _directory: tempfile::TempDir,
    pub(crate) executable: PathBuf,
}

#[cfg(unix)]
impl FakeCodex {
    pub(crate) fn started_marker(&self) -> PathBuf {
        self.executable.with_file_name("codex.started")
    }

    pub(crate) fn created_marker(&self) -> PathBuf {
        self.executable.with_file_name("codex.created")
    }

    pub(crate) fn terminal_drained_marker(&self) -> PathBuf {
        self.executable.with_file_name("codex.terminal-drained")
    }

    pub(crate) fn mark_created(&self) -> Result<(), String> {
        fs::write(self.created_marker(), b"created").map_err(|error| error.to_string())
    }

    pub(crate) fn request_approval_on_read(&self) -> Result<(), String> {
        fs::write(
            self.executable.with_file_name("codex.request-approval"),
            b"pending",
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn block_turn_start(&self) -> Result<(), String> {
        fs::write(
            self.executable.with_file_name("codex.block-turn-start"),
            b"block",
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn turn_start_admitted_marker(&self) -> PathBuf {
        self.executable.with_file_name("codex.turn-start-admitted")
    }

    pub(crate) fn release_turn_start(&self) -> Result<(), String> {
        fs::write(
            self.executable.with_file_name("codex.release-turn-start"),
            b"release",
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn fail_turn_start(&self) -> Result<(), String> {
        fs::write(
            self.executable.with_file_name("codex.fail-turn-start"),
            b"fail",
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn spawn_descendant_after_terminal_drain(&self) -> Result<(), String> {
        fs::write(
            self.executable
                .with_file_name("codex.spawn-descendant-after-terminal"),
            b"spawn",
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn fail_archive_response(&self) -> Result<(), String> {
        fs::write(
            self.executable.with_file_name("codex.fail-archive"),
            b"fail",
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn fail_archive_commit(&self, code_dir: &Path) -> Result<(), String> {
        fs::write(
            self.executable.with_file_name("codex.fail-archive-commit"),
            code_dir.to_string_lossy().as_bytes(),
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn recorded_requests(&self) -> Result<Vec<serde_json::Value>, String> {
        let contents = fs::read_to_string(self.executable.with_file_name("codex.requests"))
            .map_err(|error| error.to_string())?;
        contents
            .lines()
            .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
            .collect()
    }
}

#[cfg(unix)]
fn shell_double_quoted_json(value: &serde_json::Value) -> Result<String, String> {
    serde_json::to_string(value)
        .map_err(|error| error.to_string())
        .map(|value| {
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
                .replace('`', "\\`")
        })
}

#[cfg(unix)]
pub(crate) fn stateful_fake_codex(
    execution_root: &str,
    thread_source: &str,
    thread_id: &str,
    uncertain_start: bool,
) -> Result<FakeCodex, String> {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = directory.path().join("codex");
    let thread = |id: &str| {
        json!({
            "id": id,
            "sessionId": "session-phase1c",
            "cliVersion": "0.145.0",
            "preview": "Phase 1C fixture",
            "ephemeral": false,
            "modelProvider": "openai",
            "createdAt": 1723600000,
            "updatedAt": 1723600001,
            "cwd": execution_root,
            "source": "appServer",
            "status": { "type": "idle" },
            "threadSource": thread_source,
            "turns": []
        })
    };
    let baseline_page = shell_double_quoted_json(&json!({
        "data": [thread("thread-before")],
        "nextCursor": null
    }))?;
    let recovery_page = shell_double_quoted_json(&json!({
        "data": [thread("thread-before"), thread(thread_id)],
        "nextCursor": null
    }))?;
    let descendant_page = shell_double_quoted_json(&json!({
        "data": [
            thread("thread-before"),
            thread(thread_id),
            {
                "id": "thread-descendant",
                "sessionId": "session-descendant",
                "cliVersion": "0.145.0",
                "preview": "Late descendant fixture",
                "ephemeral": false,
                "modelProvider": "openai",
                "createdAt": 1723600002,
                "updatedAt": 1723600003,
                "cwd": execution_root,
                "source": { "subAgent": "review" },
                "status": { "type": "idle" },
                "parentThreadId": thread_id,
                "turns": []
            }
        ],
        "nextCursor": null
    }))?;
    let empty_page = shell_double_quoted_json(&json!({
        "data": [],
        "nextCursor": null,
        "backwardsCursor": null
    }))?;
    let archived_page = shell_double_quoted_json(&json!({
        "data": [thread(thread_id)],
        "nextCursor": null,
        "backwardsCursor": null
    }))?;
    let empty_loaded = shell_double_quoted_json(&json!({
        "data": [],
        "nextCursor": null
    }))?;
    let opened = shell_double_quoted_json(&json!({
        "thread": thread(thread_id),
        "instructionSources": [],
        "model": "gpt-test",
        "reasoningEffort": "high"
    }))?;
    let read = shell_double_quoted_json(&json!({ "thread": thread(thread_id) }))?;
    let archived_notification = shell_double_quoted_json(&json!({
        "method": "thread/archived",
        "params": { "threadId": thread_id }
    }))?;
    let unarchived_notification = shell_double_quoted_json(&json!({
        "method": "thread/unarchived",
        "params": { "threadId": thread_id }
    }))?;
    let approval_request = shell_double_quoted_json(&json!({
        "id": "approval-command",
        "method": "item/commandExecution/requestApproval",
        "params": {
            "threadId": thread_id,
            "turnId": "turn-approval",
            "itemId": "item-command",
            "startedAtMs": 1723600011000_u64,
            "command": "cargo test",
            "cwd": execution_root,
            "reason": "Run the focused tests"
        }
    }))?;
    let turn = shell_double_quoted_json(&json!({
        "turn": { "id": "turn-phase1c", "status": "inProgress" }
    }))?;
    let start_reply = if uncertain_start {
        "printf '%s\\n' \"{\\\"id\\\":$request_id,\\\"error\\\":{\\\"code\\\":-32000,\\\"message\\\":\\\"simulated uncertain start\\\"}}\""
                .to_string()
    } else {
        format!("printf '%s\\n' \"{{\\\"id\\\":$request_id,\\\"result\\\":{opened}}}\"")
    };
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
: > "$0.started"
IFS= read -r initialize
printf '%s\n' '{{"id":1,"result":{{"userAgent":"codex-phase1c","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}}}'
IFS= read -r initialized
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
        *'"method":"thread/list"'*)
      case "$line" in
        *'"archived":true'*)
          if [ -f "$0.archived" ]; then
            printf '%s\n' "{{\"id\":$request_id,\"result\":{archived_page}}}"
          else
            printf '%s\n' "{{\"id\":$request_id,\"result\":{empty_page}}}"
          fi
          ;;
        *)
          if [ -f "$0.archived" ]; then
            printf '%s\n' "{{\"id\":$request_id,\"result\":{baseline_page}}}"
          elif [ -f "$0.created" ]; then
            if [ -f "$0.spawn-descendant-after-terminal" ] && [ -f "$0.terminal-drained" ]; then
              printf '%s\n' "{{\"id\":$request_id,\"result\":{descendant_page}}}"
            else
              printf '%s\n' "{{\"id\":$request_id,\"result\":{recovery_page}}}"
            fi
          else
            printf '%s\n' "{{\"id\":$request_id,\"result\":{baseline_page}}}"
          fi
          ;;
      esac
      ;;
    *'"method":"thread/loaded/list"'*)
      printf '%s\n' "{{\"id\":$request_id,\"result\":{empty_loaded}}}"
      ;;
    *'"method":"thread/start"'*)
      : > "$0.created"
      {start_reply}
      ;;
    *'"method":"thread/read"'*)
      if [ -f "$0.request-approval" ]; then
        rm -f "$0.request-approval"
        printf '%s\n' "{approval_request}"
      fi
      printf '%s\n' "{{\"id\":$request_id,\"result\":{read}}}"
      ;;
    *'"method":"thread/archive"'*)
      if [ ! -f "$0.terminal-drained" ]; then
        printf '%s\n' "{{\"id\":$request_id,\"error\":{{\"code\":-32001,\"message\":\"terminal was not drained before archive\"}}}}"
      elif [ -f "$0.fail-archive" ]; then
        printf '%s\n' "{{\"id\":$request_id,\"error\":{{\"code\":-32002,\"message\":\"simulated uncertain archive\"}}}}"
      else
        : > "$0.archived"
        if [ -f "$0.fail-archive-commit" ]; then
          commit_dir=$(cat "$0.fail-archive-commit")
          chmod 500 "$commit_dir"
        fi
        printf '%s\n' "{archived_notification}"
        printf '%s\n' "{{\"id\":$request_id,\"result\":{{}}}}"
      fi
      ;;
    *'"method":"thread/unarchive"'*)
      rm -f "$0.archived"
      printf '%s\n' "{unarchived_notification}"
      printf '%s\n' "{{\"id\":$request_id,\"result\":{read}}}"
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' "{{\"id\":$request_id,\"result\":{opened}}}"
      ;;
    *'"method":"turn/start"'*)
      if [ -f "$0.block-turn-start" ]; then
        : > "$0.turn-start-admitted"
        while [ ! -f "$0.release-turn-start" ]; do
          sleep 0.01
        done
      fi
      if [ -f "$0.fail-turn-start" ]; then
        printf '%s\n' "{{\"id\":$request_id,\"error\":{{\"code\":-32003,\"message\":\"simulated uncertain turn start\"}}}}"
      else
        printf '%s\n' "{{\"id\":$request_id,\"result\":{turn}}}"
      fi
      ;;
    *)
      printf '%s\n' "{{\"id\":$request_id,\"error\":{{\"code\":-32601,\"message\":\"unexpected method\"}}}}"
      ;;
  esac
done
"#
    );
    fs::write(&executable, script).map_err(|error| error.to_string())?;
    let mut permissions = fs::metadata(&executable)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).map_err(|error| error.to_string())?;
    Ok(FakeCodex {
        _directory: directory,
        executable,
    })
}

pub(crate) struct TestRepository {
    _directory: tempfile::TempDir,
    root: PathBuf,
}

fn test_git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let executable = crate::managed_agents::resolve_command("git")
        .ok_or_else(|| "git executable was not found".to_string())?;
    let output = Command::new(executable)
        .arg("--no-pager")
        .args(args)
        .current_dir(cwd)
        .env_remove("GIT_NO_REPLACE_OBJECTS")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| format!("failed to run test git: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn test_git_line(cwd: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(test_git(cwd, args)?)
        .map(|output| output.trim_end_matches(['\r', '\n']).to_string())
        .map_err(|error| error.to_string())
}

pub(crate) fn create_test_repository() -> Result<TestRepository, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = directory.path().join("repository");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    test_git(&root, &["init", "--initial-branch=main"])?;
    fs::write(root.join("README.md"), "phase 1c\n").map_err(|error| error.to_string())?;
    test_git(&root, &["add", "README.md"])?;
    test_git(
        &root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "phase 1c fixture",
        ],
    )?;
    Ok(TestRepository {
        _directory: directory,
        root,
    })
}

pub(crate) fn phase1c_scope(repository_identity: String) -> CodeThreadBindingScope {
    CodeThreadBindingScope {
        community_id: "community-phase1c".to_string(),
        project_dtag: "project-phase1c".to_string(),
        repository_identity,
    }
}

pub(crate) fn persisted_local_binding(
    repository: &TestRepository,
    thread_id: &str,
) -> Result<(CodeThreadBindingScope, CodeThreadBinding), String> {
    let descriptor = preflight_execution_root(&repository.root.to_string_lossy(), "HEAD")?;
    let base_ref = String::from_utf8(test_git(&repository.root, &["rev-parse", "HEAD"])?)
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();
    let scope = phase1c_scope(descriptor.repository_identity);
    let binding = CodeThreadBinding {
        community_id: scope.community_id.clone(),
        project_dtag: scope.project_dtag.clone(),
        repository_identity: scope.repository_identity.clone(),
        codex_thread_id: thread_id.to_string(),
        execution_mode: CodeExecutionMode::Local,
        execution_root: descriptor.repository_root,
        base_ref,
        worktree_id: None,
    };
    Ok((scope, binding))
}

pub(crate) fn method_count(requests: &[serde_json::Value], method: &str) -> usize {
    requests
        .iter()
        .filter(|request| request["method"] == method)
        .count()
}

mod changes_security_tests;
mod changes_tests;
mod event_recovery_tests;
mod thread_flow_tests;
