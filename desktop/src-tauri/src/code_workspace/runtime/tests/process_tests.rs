use super::*;

#[cfg(unix)]
#[test]
fn starts_initializes_and_stops_a_fake_app_server() -> Result<(), String> {
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r request
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
while IFS= read -r line; do :; done
"#,
    )?;
    let runtime = CodeRuntime::with_executable(executable);

    let ready = runtime.start(noop_emitter())?;
    assert_eq!(ready.phase, CodeRuntimePhase::Ready);
    assert_eq!(ready.version.as_deref(), Some("codex-cli 0.145.0"));
    assert_eq!(ready.user_agent.as_deref(), Some("codex-test"));
    assert!(ready.pid.is_some());

    let stopped = runtime.stop()?;
    assert_eq!(stopped.phase, CodeRuntimePhase::Stopped);
    assert!(stopped.pid.is_none());
    Ok(())
}

#[cfg(unix)]
#[test]
fn starts_an_audited_codex_0_149_app_server() -> Result<(), String> {
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.149.0"
  exit 0
fi
IFS= read -r request
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
while IFS= read -r line; do :; done
"#,
    )?;
    let runtime = CodeRuntime::with_executable(executable);

    let ready = runtime.start(noop_emitter())?;
    assert_eq!(ready.phase, CodeRuntimePhase::Ready);
    assert_eq!(ready.version.as_deref(), Some("codex-cli 0.149.0"));
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn failed_stop_retains_process_and_blocks_restart_until_verified_teardown() -> Result<(), String> {
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r request
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
while IFS= read -r line; do :; done
"#,
    )?;
    let runtime = CodeRuntime::with_executable(executable);
    let ready = runtime.start(noop_emitter())?;
    let original_generation = ready.generation;
    let original_pid = ready
        .pid
        .ok_or_else(|| "ready runtime did not expose its process ID".to_string())?;
    {
        let mut inner = runtime.inner.lock().map_err(|error| error.to_string())?;
        let process = inner
            .process
            .as_mut()
            .ok_or_else(|| "ready runtime did not retain its process".to_string())?;
        process.stop_failures_remaining = 2;
    }

    assert!(runtime.stop().is_err());
    let failed_stop = runtime.status()?;
    assert_eq!(failed_stop.phase, CodeRuntimePhase::Failed);
    assert_eq!(failed_stop.generation, original_generation);
    assert_eq!(failed_stop.pid, Some(original_pid));

    assert!(runtime.start(noop_emitter()).is_err());
    let blocked_restart = runtime.status()?;
    assert_eq!(blocked_restart.phase, CodeRuntimePhase::Failed);
    assert_eq!(blocked_restart.generation, original_generation);
    assert_eq!(blocked_restart.pid, Some(original_pid));

    let restarted = runtime.start(noop_emitter())?;
    assert_eq!(restarted.phase, CodeRuntimePhase::Ready);
    assert_eq!(restarted.generation, original_generation.saturating_add(1));
    assert!(restarted.pid.is_some_and(|pid| pid != original_pid));
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn leader_exit_kills_term_resistant_app_server_descendant() -> Result<(), String> {
    let pid_directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let descendant_pid_path = pid_directory.path().join("descendant.pid");
    let quoted_pid_path = shell_quote(&descendant_pid_path);
    let (_directory, executable) = fake_codex(&format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
/bin/sh -c 'trap "" TERM HUP; printf "%s\n" "$$" > "$1"; while :; do sleep 1; done' schoolx-descendant {quoted_pid_path} &
while [ ! -s {quoted_pid_path} ]; do :; done
IFS= read -r request
printf '%s\n' '{{"id":1,"result":{{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}}}'
IFS= read -r initialized
sleep 0.1
exit 23
"#
    ))?;
    let runtime = CodeRuntime::with_executable(executable);

    let ready = runtime.start(noop_emitter())?;
    let leader_pid = runtime_process_pid(
        ready
            .pid
            .ok_or_else(|| "ready runtime did not expose its process ID".to_string())?,
    )?;
    let mut cleanup_guard = ProcessGroupCleanupGuard::new(leader_pid);
    let descendant_pid = fs::read_to_string(&descendant_pid_path)
        .map_err(|error| error.to_string())?
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("invalid descendant PID: {error}"))?;
    let descendant_pid = runtime_process_pid(descendant_pid)?;

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if runtime.status()?.phase == CodeRuntimePhase::Failed {
            break;
        }
        if Instant::now() >= deadline {
            return Err("runtime did not observe the app-server leader exit".to_string());
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    wait_for_process_exit(descendant_pid, Duration::from_secs(3))?;
    wait_for_process_group_exit(leader_pid, Duration::from_secs(3))?;
    cleanup_guard.disarm();
    Ok(())
}

#[cfg(unix)]
#[test]
fn failed_initialize_does_not_leave_the_process_ready() -> Result<(), String> {
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r request
printf '%s\n' '{"id":1,"error":{"code":-32602,"message":"bad initialize"}}'
while IFS= read -r line; do :; done
"#,
    )?;
    let runtime = CodeRuntime::with_executable(executable);

    let error = runtime.start(noop_emitter()).err();
    assert!(error
        .as_deref()
        .is_some_and(|message| message.contains("bad initialize")));
    assert_eq!(runtime.status()?.phase, CodeRuntimePhase::Failed);
    Ok(())
}

#[cfg(unix)]
#[test]
fn failed_initialize_redacts_rpc_stderr_and_status_diagnostics() -> Result<(), String> {
    let rpc_canary = "sk-initialize-rpc-canary";
    let stderr_canary = "sk-initialize-stderr-canary";
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
printf '%s\n' 'stderr sk-initialize-stderr-canary' >&2
sleep 0.1
IFS= read -r request
printf '%s\n' '{"id":1,"error":{"code":-32602,"message":"bad initialize sk-initialize-rpc-canary"}}'
while IFS= read -r line; do :; done
"#,
    )?;
    let runtime = CodeRuntime::with_executable(executable);

    let error = runtime
        .start(noop_emitter())
        .err()
        .ok_or_else(|| "initialize failure unexpectedly started the runtime".to_string())?;
    assert!(!error.contains(rpc_canary));
    assert!(!error.contains(stderr_canary));
    assert!(error.contains("[REDACTED]"));

    let status = runtime.status()?;
    let last_error = status
        .last_error
        .as_deref()
        .ok_or_else(|| "failed runtime status had no last error".to_string())?;
    assert_eq!(status.phase, CodeRuntimePhase::Failed);
    assert!(!last_error.contains(rpc_canary));
    assert!(!last_error.contains(stderr_canary));
    assert!(last_error.contains("[REDACTED]"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn successful_initialize_redacts_status_metadata() -> Result<(), String> {
    let canaries = [
        "sk-initialize-user-agent-canary",
        "sk-initialize-codex-home-canary",
        "sk-initialize-platform-family-canary",
        "sk-initialize-platform-os-canary",
    ];
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r request
printf '%s\n' '{"id":1,"result":{"userAgent":"agent sk-initialize-user-agent-canary","codexHome":"/tmp/sk-initialize-codex-home-canary","platformFamily":"unix sk-initialize-platform-family-canary","platformOs":"macos sk-initialize-platform-os-canary"}}'
IFS= read -r initialized
while IFS= read -r line; do :; done
"#,
    )?;
    let runtime = CodeRuntime::with_executable(executable);

    let ready = runtime.start(noop_emitter())?;
    let status = runtime.status()?;
    for canary in canaries {
        for value in [
            ready.user_agent.as_deref(),
            ready.codex_home.as_deref(),
            ready.platform_family.as_deref(),
            ready.platform_os.as_deref(),
            status.user_agent.as_deref(),
            status.codex_home.as_deref(),
            status.platform_family.as_deref(),
            status.platform_os.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            assert!(!value.contains(canary));
        }
    }
    assert!(ready
        .user_agent
        .as_deref()
        .is_some_and(|value| value.contains("[REDACTED]")));
    assert!(status
        .codex_home
        .as_deref()
        .is_some_and(|value| value.contains("[REDACTED]")));
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn public_probe_start_and_status_redact_child_version_diagnostics() -> Result<(), String> {
    let failure_canary = "sk-runtime-probe-failure-canary";
    let (_failure_directory, failure_executable) = fake_codex(
        r#"#!/bin/sh
printf '%s\n' 'version failed sk-runtime-probe-failure-canary' >&2
exit 1
"#,
    )?;
    let failed_runtime = CodeRuntime::with_executable(failure_executable);

    let failed_probe = failed_runtime.probe();
    let probe_error = failed_probe
        .error
        .as_deref()
        .ok_or_else(|| "failed public probe returned no diagnostic".to_string())?;
    assert!(!probe_error.contains(failure_canary));
    assert!(probe_error.contains("[REDACTED]"));

    let start_error = failed_runtime
        .start(noop_emitter())
        .err()
        .ok_or_else(|| "failed version probe unexpectedly started the runtime".to_string())?;
    assert!(!start_error.contains(failure_canary));
    assert!(start_error.contains("[REDACTED]"));
    let failed_status = failed_runtime.status()?;
    let last_error = failed_status
        .last_error
        .as_deref()
        .ok_or_else(|| "failed version status had no last error".to_string())?;
    assert!(!last_error.contains(failure_canary));
    assert!(last_error.contains("[REDACTED]"));

    let version_canary = "sk-runtime-probe-version-canary";
    let (_version_directory, version_executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0 sk-runtime-probe-version-canary"
  exit 0
fi
exit 1
"#,
    )?;
    let version_runtime = CodeRuntime::with_executable(version_executable);
    let public_probe = version_runtime.probe();
    let public_version = public_probe
        .version
        .as_deref()
        .ok_or_else(|| "public probe returned no version".to_string())?;
    assert!(!public_version.contains(version_canary));
    assert!(public_version.contains("[REDACTED]"));

    let unsupported_error = version_runtime
        .start(noop_emitter())
        .err()
        .ok_or_else(|| "unsupported version unexpectedly started the runtime".to_string())?;
    assert!(!unsupported_error.contains(version_canary));
    let version_status = version_runtime.status()?;
    let status_version = version_status
        .version
        .as_deref()
        .ok_or_else(|| "unsupported version status had no version".to_string())?;
    assert!(!status_version.contains(version_canary));
    assert!(status_version.contains("[REDACTED]"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn unsupported_version_is_rejected_before_app_server_spawn() -> Result<(), String> {
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.146.0"
  exit 0
fi
printf 'spawned\n' > "$0.spawned"
exit 1
"#,
    )?;
    let spawn_marker = executable.with_file_name("codex.spawned");
    let runtime = CodeRuntime::with_executable(executable);

    let probe = runtime.probe();
    assert!(probe.available);
    assert_eq!(probe.version.as_deref(), Some("codex-cli 0.146.0"));

    let error = runtime
        .start(noop_emitter())
        .expect_err("unsupported Codex must not start");
    assert!(error
        .contains("requires codex-cli 0.145.<numeric patch> or codex-cli 0.149.<numeric patch>"));
    let status = runtime.status()?;
    assert_eq!(status.phase, CodeRuntimePhase::Failed);
    assert_eq!(status.version.as_deref(), Some("codex-cli 0.146.0"));
    assert!(status.pid.is_none());
    assert!(!spawn_marker.exists());
    Ok(())
}
