use std::io::Read;
#[cfg(any(not(target_os = "macos"), test))]
use std::process::Child;
use std::process::ExitStatus;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(all(target_os = "macos", not(test)))]
use crate::code_workspace::macos_git_xpc::MacGitChild;

use super::GitCommandOutput;

const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn capture_macos_child(
    mut child: MacGitChild,
    accepts_one: bool,
) -> Result<GitCommandOutput, String> {
    let stdout = child.take_stdout()?;
    let stderr = child.take_stderr()?;
    let stdout_reader = thread::spawn(move || read_bounded(stdout));
    let stderr_reader = thread::spawn(move || read_bounded(stderr));
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                return match child.terminate() {
                    Ok(()) => {
                        let _ = stdout_reader.join();
                        let _ = stderr_reader.join();
                        Err(error)
                    }
                    Err(cleanup_error) => {
                        // An unproved child may still own the pipe writers. Do
                        // not turn a bounded XPC failure into an unbounded
                        // reader join; the poisoned session retains authority.
                        drop(stdout_reader);
                        drop(stderr_reader);
                        Err(format!(
                            "{error}; additionally failed to prove typed Git child cleanup: {cleanup_error}"
                        ))
                    }
                };
            }
        }
        if Instant::now() >= deadline {
            return match child.terminate() {
                Ok(()) => {
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    Err(
                        "typed Git command timed out and its process group was terminated"
                            .to_string(),
                    )
                }
                Err(cleanup_error) => {
                    // See the wait-error branch above: cleanup ambiguity is a
                    // fail-closed authority state, not permission to wait for
                    // EOF from a potentially live writer forever.
                    drop(stdout_reader);
                    drop(stderr_reader);
                    Err(format!(
                        "typed Git command timed out and child cleanup was not proven: {cleanup_error}"
                    ))
                }
            };
        }
        thread::sleep(POLL_INTERVAL);
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "Git stdout reader panicked".to_string())??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "Git stderr reader panicked".to_string())??;
    if stdout.overflow || stderr.overflow {
        return Err(format!(
            "typed Git command output exceeded the {MAX_CAPTURE_BYTES}-byte limit"
        ));
    }
    let stderr =
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(stderr.bytes);
    validate_exit(status, &stderr, accepts_one)?;
    Ok(GitCommandOutput {
        code: status.code().unwrap_or(-1),
        stdout: stdout.bytes,
    })
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn capture_child(
    mut child: Child,
    accepts_one: bool,
) -> Result<GitCommandOutput, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Git write helper stdout was unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Git write helper stderr was unavailable".to_string())?;
    let stdout_reader = thread::spawn(move || read_bounded(stdout));
    let stderr_reader = thread::spawn(move || read_bounded(stderr));
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll Git write helper: {error}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            kill_process_group(child.id());
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(
                "typed Git command timed out and its process group was terminated".to_string(),
            );
        }
        thread::sleep(POLL_INTERVAL);
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "Git stdout reader panicked".to_string())??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "Git stderr reader panicked".to_string())??;
    if stdout.overflow || stderr.overflow {
        return Err(format!(
            "typed Git command output exceeded the {MAX_CAPTURE_BYTES}-byte limit"
        ));
    }
    let stdout = strip_test_harness_prefix(stdout.bytes);
    let stderr =
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(stderr.bytes);
    validate_exit(status, &stderr, accepts_one)?;
    Ok(GitCommandOutput {
        code: status.code().unwrap_or(-1),
        stdout,
    })
}

struct BoundedCapture {
    bytes: Vec<u8>,
    overflow: bool,
}

fn read_bounded(mut reader: impl Read) -> Result<BoundedCapture, String> {
    let mut bytes = Vec::new();
    let mut overflow = false;
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let count = reader
            .read(&mut chunk)
            .map_err(|error| format!("failed to capture typed Git output: {error}"))?;
        if count == 0 {
            break;
        }
        let remaining = MAX_CAPTURE_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..count.min(remaining)]);
        overflow |= count > remaining;
    }
    Ok(BoundedCapture { bytes, overflow })
}

fn validate_exit(status: ExitStatus, stderr: &[u8], accepts_one: bool) -> Result<(), String> {
    if status.success() || (accepts_one && status.code() == Some(1) && stderr.is_empty()) {
        return Ok(());
    }
    let diagnostic = String::from_utf8_lossy(stderr);
    let diagnostic = diagnostic.trim();
    let diagnostic = diagnostic.chars().take(1024).collect::<String>();
    Err(format!("typed Git command failed: {diagnostic}"))
}

#[cfg(all(unix, any(not(target_os = "macos"), test)))]
fn kill_process_group(raw_pid: u32) {
    if let Some(pid) = rustix::process::Pid::from_raw(raw_pid as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn strip_test_harness_prefix(output: Vec<u8>) -> Vec<u8> {
    #[cfg(test)]
    {
        output
            .strip_prefix(b"\nrunning 1 test\n")
            .unwrap_or(&output)
            .to_vec()
    }
    #[cfg(not(test))]
    output
}
