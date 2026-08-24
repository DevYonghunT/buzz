use super::*;

pub(super) fn remaining_timeout(deadline: Instant) -> Result<Duration, String> {
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(GIT_TIMEOUT);
    if timeout.is_zero() {
        return Err("SchoolX Code removal Git budget was exhausted".to_string());
    }
    Ok(timeout)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn capture_child(child: &mut Child, timeout: Duration) -> Result<CapturedChild, String> {
    let stdout = spawn_pipe_reader(child.stdout.take());
    let stderr = spawn_pipe_reader(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = crate::managed_agents::terminate_process(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe(stdout);
                let _ = join_pipe(stderr);
                return Err("SchoolX Code removal Git helper timed out".to_string());
            }
            Err(error) => {
                let _ = crate::managed_agents::terminate_process(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe(stdout);
                let _ = join_pipe(stderr);
                return Err(format!("failed to wait for removal Git helper: {error}"));
            }
        }
    };
    Ok(CapturedChild {
        status,
        stdout: join_pipe(stdout)?,
        stderr: join_pipe(stderr)?,
    })
}

pub(super) fn spawn_pipe_reader<R>(pipe: Option<R>) -> JoinHandle<CapturedPipe>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let Some(mut pipe) = pipe else {
            return CapturedPipe {
                bytes: Vec::new(),
                truncated: false,
            };
        };
        let mut bytes = Vec::new();
        let mut truncated = false;
        let mut buffer = [0_u8; 8192];
        loop {
            let read = match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let remaining = MAX_GIT_OUTPUT_BYTES.saturating_sub(bytes.len());
            let retained = remaining.min(read);
            bytes.extend_from_slice(&buffer[..retained]);
            truncated |= retained < read;
        }
        CapturedPipe { bytes, truncated }
    })
}

pub(super) fn join_pipe(handle: JoinHandle<CapturedPipe>) -> Result<CapturedPipe, String> {
    handle
        .join()
        .map_err(|_| "removal Git helper output reader panicked".to_string())
}

pub(super) fn require_success(captured: &CapturedChild, label: &str) -> Result<(), String> {
    if captured.status.success()
        && !captured.stdout.truncated
        && !captured.stderr.truncated
        && captured.stderr.bytes.is_empty()
    {
        Ok(())
    } else {
        Err(captured_error(label, captured))
    }
}

pub(super) fn captured_error(label: &str, captured: &CapturedChild) -> String {
    let stderr = String::from_utf8_lossy(&captured.stderr.bytes);
    let message = stderr.trim();
    if message.is_empty() {
        format!("{label} exited with status {}", captured.status)
    } else {
        format!("{label}: {message}")
    }
}

pub(super) fn one_line(bytes: &[u8], label: &str) -> Result<String, String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|error| format!("{label} was not UTF-8: {error}"))?
        .trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.contains(['\r', '\n']) {
        return Err(format!("{label} did not contain exactly one value"));
    }
    Ok(value.to_string())
}
