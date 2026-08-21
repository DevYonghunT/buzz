use super::*;
use serde_json::json;

#[cfg(unix)]
fn test_git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let executable = crate::managed_agents::resolve_command("git")
        .ok_or_else(|| "git executable was not found".to_string())?;
    let output = std::process::Command::new(executable)
        .arg("--no-pager")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| format!("failed to run test git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum FakeForkMode {
    Success,
    OversizedFork,
}

#[cfg(unix)]
fn fake_codex(
    source_root: &str,
    mode: FakeForkMode,
) -> Result<(tempfile::TempDir, PathBuf), String> {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = directory.path().join("codex");
    std::fs::write(executable.with_extension("source-root"), source_root)
        .map_err(|error| error.to_string())?;
    if matches!(mode, FakeForkMode::OversizedFork) {
        std::fs::write(executable.with_extension("oversized-fork"), b"1")
            .map_err(|error| error.to_string())?;
    }
    std::fs::write(
        &executable,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
source_root=$(cat "$0.source-root")
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
: > "$0.requests"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
*'"method":"thread/read"'*)
  printf '{"id":%s,"result":{"thread":{"id":"thread-source","cwd":"%s","source":"appServer","status":{"type":"idle"},"parentThreadId":null,"forkedFromId":null,"turns":[]}}}\n' "$request_id" "$source_root"
  ;;
*'"method":"thread/list"'*)
  printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request_id"
  ;;
*'"method":"thread/loaded/list"'*)
  printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request_id"
  ;;
    *'"method":"thread/fork"'*)
      cwd=$(printf '%s' "$line" | sed -n 's/.*"cwd":"\([^"]*\)".*/\1/p')
      marker=$(printf '%s' "$line" | sed -n 's/.*"threadSource":"\([^"]*\)".*/\1/p')
      if [ -f "$0.oversized-fork" ]; then
        printf '{"id":%s,"result":{"padding":"' "$request_id"
        dd if=/dev/zero bs=4300000 count=1 2>/dev/null | tr '\000' x
        printf '"}}\n'
        continue
      fi
  printf '{"id":%s,"result":{"thread":{"id":"thread-child","sessionId":"thread-child","forkedFromId":"thread-source","parentThreadId":null,"ephemeral":false,"cwd":"%s","source":"appServer","threadSource":"%s","status":{"type":"idle"},"turns":[]},"model":"gpt-test","reasoningEffort":"high","instructionSources":[],"cwd":"%s"}}\n' "$request_id" "$cwd" "$marker" "$cwd"
  ;;
*)
  printf '{"id":%s,"error":{"code":-32601,"message":"unexpected method"}}\n' "$request_id"
  ;;
  esac
done
"#,
    )
    .map_err(|error| error.to_string())?;
    let mut permissions = std::fs::metadata(&executable)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).map_err(|error| error.to_string())?;
    Ok((directory, executable))
}

#[cfg(unix)]
fn fake_recovery_codex(
    destination_root: &str,
    thread_source: &str,
) -> Result<(tempfile::TempDir, PathBuf), String> {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = directory.path().join("codex");
    std::fs::write(
        executable.with_extension("destination-root"),
        destination_root,
    )
    .map_err(|error| error.to_string())?;
    std::fs::write(executable.with_extension("thread-source"), thread_source)
        .map_err(|error| error.to_string())?;
    std::fs::write(
        &executable,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
destination_root=$(cat "$0.destination-root")
thread_source=$(cat "$0.thread-source")
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
: > "$0.requests"
thread_json() {
  printf '{"id":"thread-child","sessionId":"thread-child","forkedFromId":"thread-source","parentThreadId":null,"ephemeral":false,"cwd":"%s","source":"appServer","threadSource":"%s","status":{"type":"idle"},"turns":[]}' "$destination_root" "$thread_source"
}
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"thread/list"'*)
      printf '{"id":%s,"result":{"data":[' "$request_id"
      thread_json
      printf '],"nextCursor":null}}\n'
      ;;
    *'"method":"thread/loaded/list"'*)
      printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request_id"
      ;;
    *'"method":"thread/read"'*)
      printf '{"id":%s,"result":{"thread":' "$request_id"
      thread_json
      printf '}}\n'
      ;;
    *'"method":"thread/resume"'*)
      printf '{"id":%s,"result":{"thread":' "$request_id"
      thread_json
      printf ',"model":"gpt-test","reasoningEffort":"high","instructionSources":[],"cwd":"%s"}}\n' "$destination_root"
      ;;
    *)
      printf '{"id":%s,"error":{"code":-32601,"message":"unexpected method"}}\n' "$request_id"
      ;;
  esac
done
"#,
    )
    .map_err(|error| error.to_string())?;
    let mut permissions = std::fs::metadata(&executable)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).map_err(|error| error.to_string())?;
    Ok((directory, executable))
}

fn scope() -> crate::code_workspace::CodeThreadBindingScope {
    crate::code_workspace::CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: "a".repeat(64),
    }
}

fn summary(root: &str, thread_id: &str, source_thread_id: &str) -> CodeThreadSummary {
    CodeThreadSummary {
        id: thread_id.to_string(),
        session_id: Some(thread_id.to_string()),
        forked_from_id: Some(source_thread_id.to_string()),
        parent_thread_id: None,
        preview: None,
        ephemeral: false,
        model_provider: Some("openai".to_string()),
        created_at: Some(1),
        updated_at: Some(1),
        cwd: Some(root.to_string()),
        name: None,
        status: Some(json!({ "type": "idle" })),
        turns: Vec::new(),
    }
}

fn preparation(root: &str) -> CodeThreadPreparation {
    let owner = scope();
    CodeThreadPreparation {
        preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
        community_id: owner.community_id,
        project_dtag: owner.project_dtag,
        repository_identity: owner.repository_identity,
        execution_mode: CodeExecutionMode::Worktree,
        execution_root: root.to_string(),
        base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        worktree_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
        operation: CodeThreadPreparationOperation::Fork,
        source_thread_id: Some("thread-source".to_string()),
        state: crate::code_workspace::bindings::CodeThreadPreparationState::Starting,
        recovery_thread_baseline: Some(Vec::new()),
        merge_target_ref: None,
    }
}

fn candidate(root: &str, thread_id: &str) -> CodeRecoveryThread {
    CodeRecoveryThread {
        thread: summary(root, thread_id, "thread-source"),
        thread_source: Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string()),
        session_source: Some(json!("appServer")),
        ephemeral_present: true,
    }
}

#[cfg(unix)]
struct ForkHarness {
    _repository: tempfile::TempDir,
    nest: tempfile::TempDir,
    app_data: tempfile::TempDir,
    source: CodeWorktreeDescriptor,
    owner: crate::code_workspace::CodeThreadBindingScope,
    store: CodeThreadBindingStore,
}

#[cfg(unix)]
fn fork_harness() -> Result<ForkHarness, String> {
    let repository = tempfile::tempdir().map_err(|error| error.to_string())?;
    test_git(repository.path(), &["init", "--initial-branch=main"])?;
    std::fs::write(repository.path().join("README.md"), "first\n")
        .map_err(|error| error.to_string())?;
    test_git(repository.path(), &["add", "README.md"])?;
    test_git(
        repository.path(),
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
    )?;
    let head = test_git(repository.path(), &["rev-parse", "HEAD"])?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let source = prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: repository.path().to_string_lossy().into_owned(),
            base_ref: head.clone(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let owner = crate::code_workspace::CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: source.descriptor.repository_identity.clone(),
    };
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    store.upsert(CodeThreadBinding {
        community_id: owner.community_id.clone(),
        project_dtag: owner.project_dtag.clone(),
        repository_identity: owner.repository_identity.clone(),
        codex_thread_id: "thread-source".to_string(),
        execution_mode: CodeExecutionMode::Worktree,
        execution_root: source.descriptor.execution_root.clone(),
        base_ref: head,
        worktree_id: source.descriptor.worktree_id.clone(),
    })?;
    Ok(ForkHarness {
        _repository: repository,
        nest,
        app_data,
        source: source.descriptor,
        owner,
        store,
    })
}

#[test]
fn fork_response_requires_exact_id_ancestry_roots_marker_and_presence() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = root
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let mut opened = crate::code_workspace::CodeThreadRpcOpenResult {
        thread: summary(&root, "thread-child", "thread-source"),
        instruction_sources: Vec::new(),
        model: "gpt-test".to_string(),
        reasoning_effort: Some("high".to_string()),
        thread_source: Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string()),
        session_source: Some(json!("appServer")),
        response_cwd: Some(root.clone()),
        ephemeral_present: true,
    };
    validate_fork_opened(
        &opened,
        "thread-source",
        &root,
        "67f11a1d-0274-4d40-9b0c-e406e51c64fb",
    )?;

    opened.thread.forked_from_id = Some("thread-other".to_string());
    assert!(validate_fork_opened(
        &opened,
        "thread-source",
        &root,
        "67f11a1d-0274-4d40-9b0c-e406e51c64fb"
    )
    .is_err());
    opened.thread.forked_from_id = Some("thread-source".to_string());
    opened.ephemeral_present = false;
    assert!(validate_fork_opened(
        &opened,
        "thread-source",
        &root,
        "67f11a1d-0274-4d40-9b0c-e406e51c64fb"
    )
    .is_err());
    opened.ephemeral_present = true;
    opened.response_cwd = Some(root.clone() + "-wrong");
    assert!(validate_fork_opened(
        &opened,
        "thread-source",
        &root,
        "67f11a1d-0274-4d40-9b0c-e406e51c64fb"
    )
    .is_err());
    Ok(())
}

#[test]
fn recovery_candidate_selection_is_exact_and_unambiguous() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = root
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let preparation = preparation(&root);
    let selected = select_fork_recovery_candidate(
        &preparation,
        vec![
            candidate(&root, "thread-child"),
            CodeRecoveryThread {
                thread_source: Some("schoolx-code/wrong".to_string()),
                ..candidate(&root, "thread-foreign")
            },
        ],
        &HashSet::new(),
        &root,
    )?;
    assert_eq!(selected.thread.id, "thread-child");

    let ambiguous = select_fork_recovery_candidate(
        &preparation,
        vec![
            candidate(&root, "thread-child-a"),
            candidate(&root, "thread-child-b"),
        ],
        &HashSet::new(),
        &root,
    );
    assert!(ambiguous.is_err_and(|error| error.contains("2 possible")));

    let bound = HashSet::from(["thread-child".to_string()]);
    assert!(select_fork_recovery_candidate(
        &preparation,
        vec![candidate(&root, "thread-child")],
        &bound,
        &root,
    )
    .is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn native_fork_uses_the_clean_current_head_and_commits_a_distinct_binding() -> Result<(), String> {
    let repository = tempfile::tempdir().map_err(|error| error.to_string())?;
    test_git(repository.path(), &["init", "--initial-branch=main"])?;
    std::fs::write(repository.path().join("README.md"), "first\n")
        .map_err(|error| error.to_string())?;
    test_git(repository.path(), &["add", "README.md"])?;
    test_git(
        repository.path(),
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
    )?;
    let initial_head = test_git(repository.path(), &["rev-parse", "HEAD"])?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let source = prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: repository.path().to_string_lossy().into_owned(),
            base_ref: initial_head.clone(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    let source_root = PathBuf::from(&source.descriptor.execution_root);
    std::fs::write(source_root.join("README.md"), "second\n").map_err(|error| error.to_string())?;
    test_git(&source_root, &["add", "README.md"])?;
    test_git(
        &source_root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "source progress",
        ],
    )?;
    let current_head = test_git(&source_root, &["rev-parse", "HEAD"])?;
    assert_ne!(current_head, initial_head);

    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let owner = crate::code_workspace::CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: source.descriptor.repository_identity.clone(),
    };
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    store.upsert(CodeThreadBinding {
        community_id: owner.community_id.clone(),
        project_dtag: owner.project_dtag.clone(),
        repository_identity: owner.repository_identity.clone(),
        codex_thread_id: "thread-source".to_string(),
        execution_mode: CodeExecutionMode::Worktree,
        execution_root: source.descriptor.execution_root.clone(),
        base_ref: initial_head,
        worktree_id: source.descriptor.worktree_id.clone(),
    })?;

    let (_fake_directory, executable) =
        fake_codex(&source.descriptor.execution_root, FakeForkMode::Success)?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(executable.clone());
    runtime.start(std::sync::Arc::new(|_| {}))?;
    runtime.commit_new_thread_lifecycle("thread-source", || Ok(()))?;
    let lifecycle_authority = AtomicBool::new(true);
    let binding_lock = Mutex::new(());
    let terminal_manager = crate::code_workspace::CodeTerminalManager::new();

    let opened = fork_thread_native(
        CodeThreadForkInput {
            scope: owner.clone(),
            thread_id: "thread-source".to_string(),
        },
        app_data.path(),
        nest.path(),
        &runtime,
        &terminal_manager,
        &binding_lock,
        &lifecycle_authority,
    )
    .map_err(|error| error.message)?;
    assert_eq!(opened.thread.id, "thread-child");
    assert_eq!(
        opened.thread.forked_from_id.as_deref(),
        Some("thread-source")
    );
    assert_eq!(opened.binding.base_ref, current_head);
    assert_ne!(
        opened.binding.execution_root,
        source.descriptor.execution_root
    );
    assert_ne!(opened.binding.worktree_id, source.descriptor.worktree_id);
    let final_index = store.load()?;
    assert_eq!(final_index.bindings.len(), 2);
    assert!(final_index.preparations.is_empty());

    let requests = std::fs::read_to_string(executable.with_extension("requests"))
        .map_err(|error| error.to_string())?
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let fork_requests = requests
        .iter()
        .filter(|request| request["method"] == "thread/fork")
        .collect::<Vec<_>>();
    assert_eq!(fork_requests.len(), 1);
    let params = fork_requests[0]["params"]
        .as_object()
        .ok_or_else(|| "fork params were not an object".to_string())?;
    assert_eq!(
        params.keys().map(String::as_str).collect::<HashSet<_>>(),
        HashSet::from([
            "threadId",
            "cwd",
            "approvalPolicy",
            "sandbox",
            "threadSource",
        ])
    );
    assert_eq!(params["threadId"], "thread-source");
    assert_eq!(params["cwd"], opened.binding.execution_root);
    assert!(params["threadSource"]
        .as_str()
        .is_some_and(|source| source.starts_with("schoolx-code/")));
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn definitely_unsent_fork_restores_and_continues_the_exact_destination() -> Result<(), String> {
    let harness = fork_harness()?;
    let (_first_fake, first_executable) =
        fake_codex(&harness.source.execution_root, FakeForkMode::Success)?;
    let first_runtime =
        crate::code_workspace::CodeRuntime::with_executable(first_executable.clone());
    first_runtime.start(std::sync::Arc::new(|_| {}))?;
    first_runtime.commit_new_thread_lifecycle("thread-source", || Ok(()))?;
    first_runtime.fail_next_fork_before_write_for_test();
    let lifecycle_authority = AtomicBool::new(true);
    let binding_lock = Mutex::new(());
    let terminal_manager = crate::code_workspace::CodeTerminalManager::new();

    let error = fork_thread_native(
        CodeThreadForkInput {
            scope: harness.owner.clone(),
            thread_id: "thread-source".to_string(),
        },
        harness.app_data.path(),
        harness.nest.path(),
        &first_runtime,
        &terminal_manager,
        &binding_lock,
        &lifecycle_authority,
    )
    .expect_err("closed transport before fork admission must roll back");
    assert_eq!(error.code, "threadForkNotSent");
    let index = harness.store.load()?;
    assert_eq!(index.preparations.len(), 1);
    let prepared = index.preparations[0].clone();
    assert_eq!(
        prepared.state,
        crate::code_workspace::bindings::CodeThreadPreparationState::Prepared
    );
    let reserved_root = prepared.execution_root.clone();
    let first_requests = std::fs::read_to_string(first_executable.with_extension("requests"))
        .map_err(|error| error.to_string())?;
    assert!(!first_requests.contains("\"method\":\"thread/fork\""));
    let _ = first_runtime.stop();

    let (_second_fake, second_executable) =
        fake_codex(&harness.source.execution_root, FakeForkMode::Success)?;
    let second_runtime =
        crate::code_workspace::CodeRuntime::with_executable(second_executable.clone());
    second_runtime.start(std::sync::Arc::new(|_| {}))?;
    second_runtime.commit_new_thread_lifecycle("thread-source", || Ok(()))?;
    let continued = open_fork_preparation_locked(
        &CodeThreadBindingRecoverInput {
            scope: harness.owner.clone(),
            preparation_id: prepared.preparation_id.clone(),
            model: None,
        },
        prepared,
        &harness.store,
        harness.nest.path(),
        &second_runtime,
        &terminal_manager,
        &lifecycle_authority,
    )?;
    assert_eq!(continued.binding.execution_root, reserved_root);
    assert_eq!(continued.thread.id, "thread-child");
    let final_index = harness.store.load()?;
    assert_eq!(final_index.bindings.len(), 2);
    assert!(final_index.preparations.is_empty());
    let second_requests = std::fs::read_to_string(second_executable.with_extension("requests"))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        second_requests
            .matches("\"method\":\"thread/fork\"")
            .count(),
        1
    );
    second_runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn oversized_post_write_response_stays_starting_and_recovers_without_refork() -> Result<(), String>
{
    let harness = fork_harness()?;
    let (_first_fake, first_executable) =
        fake_codex(&harness.source.execution_root, FakeForkMode::OversizedFork)?;
    let first_runtime =
        crate::code_workspace::CodeRuntime::with_executable(first_executable.clone());
    first_runtime.start(std::sync::Arc::new(|_| {}))?;
    first_runtime.commit_new_thread_lifecycle("thread-source", || Ok(()))?;
    let lifecycle_authority = AtomicBool::new(true);
    let binding_lock = Mutex::new(());
    let terminal_manager = crate::code_workspace::CodeTerminalManager::new();

    let error = fork_thread_native(
        CodeThreadForkInput {
            scope: harness.owner.clone(),
            thread_id: "thread-source".to_string(),
        },
        harness.app_data.path(),
        harness.nest.path(),
        &first_runtime,
        &terminal_manager,
        &binding_lock,
        &lifecycle_authority,
    )
    .expect_err("oversized response after write must remain uncertain");
    assert_eq!(error.code, "threadForkUncertain");
    let index = harness.store.load()?;
    assert_eq!(index.preparations.len(), 1);
    let starting = index.preparations[0].clone();
    assert_eq!(
        starting.state,
        crate::code_workspace::bindings::CodeThreadPreparationState::Starting
    );
    let first_requests = std::fs::read_to_string(first_executable.with_extension("requests"))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        first_requests.matches("\"method\":\"thread/fork\"").count(),
        1
    );
    let _ = first_runtime.stop();

    let marker = code_thread_source(&starting.preparation_id)?;
    let (_recovery_fake, recovery_executable) =
        fake_recovery_codex(&starting.execution_root, &marker)?;
    let recovery_runtime =
        crate::code_workspace::CodeRuntime::with_executable(recovery_executable.clone());
    recovery_runtime.start(std::sync::Arc::new(|_| {}))?;
    let recovered = open_fork_preparation_locked(
        &CodeThreadBindingRecoverInput {
            scope: harness.owner.clone(),
            preparation_id: starting.preparation_id.clone(),
            model: None,
        },
        starting.clone(),
        &harness.store,
        harness.nest.path(),
        &recovery_runtime,
        &terminal_manager,
        &lifecycle_authority,
    )?;
    assert_eq!(recovered.thread.id, "thread-child");
    assert_eq!(recovered.binding.execution_root, starting.execution_root);
    let recovery_requests = std::fs::read_to_string(recovery_executable.with_extension("requests"))
        .map_err(|error| error.to_string())?;
    assert!(!recovery_requests.contains("\"method\":\"thread/fork\""));
    assert_eq!(
        recovery_requests
            .matches("\"method\":\"thread/resume\"")
            .count(),
        1
    );
    let final_index = harness.store.load()?;
    assert_eq!(final_index.bindings.len(), 2);
    assert!(final_index.preparations.is_empty());
    recovery_runtime.stop()?;
    Ok(())
}
