use super::*;

pub(super) fn remaining_git_timeout(deadline: Option<Instant>) -> Result<Duration, String> {
    let timeout = deadline
        .map(|deadline| deadline.saturating_duration_since(Instant::now()))
        .unwrap_or(GIT_TIMEOUT)
        .min(GIT_TIMEOUT);
    if timeout.is_zero() {
        return Err("SchoolX Code worktree inspection budget was exhausted".to_string());
    }
    Ok(timeout)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn capture_child(
    mut child: Child,
    label: &str,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let captured = capture_child_status(&mut child, label, timeout)?;
    if !captured.status.success() {
        return Err(captured_child_error(label, &captured));
    }
    if captured.stdout.truncated {
        return Err(format!(
            "{label} output exceeded the {GIT_OUTPUT_LIMIT}-byte SchoolX Code limit"
        ));
    }
    Ok(captured.stdout.bytes)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn capture_child_status(
    child: &mut Child,
    label: &str,
    timeout: Duration,
) -> Result<CapturedChild, String> {
    let stdout_thread = spawn_pipe_reader(child.stdout.take());
    let stderr_thread = spawn_pipe_reader(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = crate::managed_agents::terminate_process(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe(stdout_thread);
                let _ = join_pipe(stderr_thread);
                return Err(format!(
                    "{label} operation timed out after {} seconds",
                    timeout.as_secs_f64()
                ));
            }
            Err(error) => {
                let _ = crate::managed_agents::terminate_process(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe(stdout_thread);
                let _ = join_pipe(stderr_thread);
                return Err(format!("failed to wait for {label}: {error}"));
            }
        }
    };

    Ok(CapturedChild {
        status,
        stdout: join_pipe(stdout_thread)?,
        stderr: join_pipe(stderr_thread)?,
    })
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn capture_macos_pinned_child(
    mut child: MacGitChild,
    label: &str,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let captured = capture_macos_pinned_child_status(&mut child, label, timeout)?;
    if !captured.status.success() {
        return Err(captured_child_error(label, &captured));
    }
    if captured.stdout.truncated {
        return Err(format!(
            "{label} output exceeded the {GIT_OUTPUT_LIMIT}-byte SchoolX Code limit"
        ));
    }
    Ok(captured.stdout.bytes)
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn capture_macos_pinned_child_status(
    child: &mut MacGitChild,
    label: &str,
    timeout: Duration,
) -> Result<CapturedChild, String> {
    let stdout_thread = spawn_pipe_reader(Some(child.take_stdout()?));
    let stderr_thread = spawn_pipe_reader(Some(child.take_stderr()?));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let error = format!(
                    "{label} operation timed out after {} seconds",
                    timeout.as_secs_f64()
                );
                return Err(finish_failed_macos_capture(
                    child,
                    stdout_thread,
                    stderr_thread,
                    error,
                ));
            }
            Err(error) => {
                return Err(finish_failed_macos_capture(
                    child,
                    stdout_thread,
                    stderr_thread,
                    format!("failed to wait for {label}: {error}"),
                ));
            }
        }
    };
    Ok(CapturedChild {
        status,
        stdout: join_pipe(stdout_thread)?,
        stderr: join_pipe(stderr_thread)?,
    })
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn finish_failed_macos_capture(
    child: &mut MacGitChild,
    stdout_thread: JoinHandle<CapturedPipe>,
    stderr_thread: JoinHandle<CapturedPipe>,
    primary_error: String,
) -> String {
    finish_failed_capture_threads(stdout_thread, stderr_thread, primary_error, || {
        child.terminate()
    })
}

#[cfg(any(all(target_os = "macos", not(test)), test))]
pub(super) fn finish_failed_capture_threads<F>(
    stdout_thread: JoinHandle<CapturedPipe>,
    stderr_thread: JoinHandle<CapturedPipe>,
    primary_error: String,
    terminate: F,
) -> String
where
    F: FnOnce() -> Result<(), String>,
{
    match terminate() {
        Ok(()) => {
            // A successful cancellation is the signed service's proof that
            // the child was killed, reaped, and its process group vanished.
            // Only that proof makes waiting for both pipe EOFs safe.
            let stdout_error = join_pipe(stdout_thread).err();
            let stderr_error = join_pipe(stderr_thread).err();
            [Some(primary_error), stdout_error, stderr_error]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; ")
        }
        Err(termination_error) => {
            // An invalidated helper or ambiguous cleanup may leave a writer
            // alive indefinitely. Dropping JoinHandles detaches the readers;
            // session poisoning retains authority and this call stays bounded.
            drop(stdout_thread);
            drop(stderr_thread);
            format!("{primary_error}; {termination_error}")
        }
    }
}

pub(super) fn captured_child_error(label: &str, captured: &CapturedChild) -> String {
    let message = String::from_utf8_lossy(&captured.stderr.bytes);
    let message = message.trim();
    let suffix = if captured.stderr.truncated {
        " [output truncated]"
    } else {
        ""
    };
    if message.is_empty() {
        format!("{label} exited with status {}{suffix}", captured.status)
    } else {
        format!("{message}{suffix}")
    }
}

pub(super) fn configure_git_environment(
    command: &mut Command,
    operation: GitOperation,
    disabled_filter_keys: &[String],
) {
    let inherited = [
        "PATH",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
    ]
    .into_iter()
    .filter_map(|name| std::env::var_os(name).map(|value| (name, value)))
    .collect::<Vec<_>>();
    command.env_clear();
    for (name, value) in inherited {
        command.env(name, value);
    }
    command.env("GIT_NO_REPLACE_OBJECTS", "1");
    command.env("GIT_NO_LAZY_FETCH", "1");
    #[cfg(unix)]
    command.env("GIT_GRAFT_FILE", "/dev/null");
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env("GIT_CONFIG_SYSTEM", "/dev/null");
    command.env("GIT_CONFIG_GLOBAL", "/dev/null");
    command.env("GIT_ATTR_NOSYSTEM", "1");
    command.env(
        "GIT_OPTIONAL_LOCKS",
        match operation {
            GitOperation::ReadOnly | GitOperation::WorkingTreeRead => "0",
            GitOperation::Mutating => "1",
        },
    );

    let config = [
        ("credential.helper", ""),
        ("advice.graftFileDeprecated", "false"),
        ("core.hooksPath", "/dev/null"),
        ("core.fsmonitor", "false"),
        ("protocol.allow", "never"),
    ];
    let static_config_len = config.len();
    command.env(
        "GIT_CONFIG_COUNT",
        (static_config_len + disabled_filter_keys.len()).to_string(),
    );
    for (index, (key, value)) in config.into_iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    for (offset, key) in disabled_filter_keys.iter().enumerate() {
        let index = static_config_len + offset;
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(
            format!("GIT_CONFIG_VALUE_{index}"),
            if key.ends_with(".required") {
                "false"
            } else {
                ""
            },
        );
    }
}

pub(super) fn spawn_pipe_reader<R>(pipe: Option<R>) -> JoinHandle<CapturedPipe>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || read_pipe_capped(pipe))
}

pub(super) fn read_pipe_capped<R>(pipe: Option<R>) -> CapturedPipe
where
    R: Read,
{
    let Some(mut pipe) = pipe else {
        return CapturedPipe {
            bytes: Vec::new(),
            truncated: false,
        };
    };
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let remaining = GIT_OUTPUT_LIMIT.saturating_sub(bytes.len());
        let retained = remaining.min(read);
        bytes.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
    }
    CapturedPipe { bytes, truncated }
}

pub(super) fn join_pipe(handle: JoinHandle<CapturedPipe>) -> Result<CapturedPipe, String> {
    handle
        .join()
        .map_err(|_| "git output reader stopped unexpectedly".to_string())
}
