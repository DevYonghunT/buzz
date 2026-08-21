use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::ipc::{Channel, InvokeResponseBody};

use super::*;

fn scope(marker: char) -> CodeThreadBindingScope {
    CodeThreadBindingScope {
        community_id: "community".to_string(),
        project_dtag: "project".to_string(),
        repository_identity: marker.to_string().repeat(64),
    }
}

#[cfg(unix)]
fn captured_output(events: &Arc<Mutex<Vec<Value>>>) -> Result<Vec<u8>, String> {
    Ok(events
        .lock()
        .map_err(|_| "terminal event lock is unavailable".to_string())?
        .iter()
        .filter(|event| event["type"] == "output")
        .filter_map(|event| event["data"].as_array())
        .flatten()
        .filter_map(Value::as_u64)
        .map(|byte| byte as u8)
        .collect())
}

#[cfg(unix)]
fn wait_for_shell_ready(
    manager: &CodeTerminalManager,
    session: &CodeTerminalSession,
    events: &Arc<Mutex<Vec<Value>>>,
) -> Result<(), String> {
    let marker = "__schoolx_terminal_shell_ready__";
    manager.stdin(CodeTerminalStdinInput {
        scope: session.scope.clone(),
        thread_id: session.thread_id.clone(),
        session_id: session.session_id.clone(),
        data: format!("echo {marker}\r").into_bytes(),
    })?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = captured_output(events)?;
        if String::from_utf8_lossy(&output).matches(marker).count() >= 2 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("terminal shell readiness handshake timed out".to_string());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn terminal_event_wire_is_exact_and_byte_preserving() -> Result<(), String> {
    let event = CodeTerminalEvent::Output {
        scope: scope('a'),
        thread_id: "thread-1".to_string(),
        session_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
        sequence: 7,
        data: vec![0, 0x1b, 0xff],
    };
    let value = serde_json::to_value(event).map_err(|error| error.to_string())?;
    assert_eq!(value["type"], json!("output"));
    assert_eq!(value["threadId"], json!("thread-1"));
    assert_eq!(value["sequence"], json!(7));
    assert_eq!(value["data"], json!([0, 27, 255]));
    assert!(value.get("thread_id").is_none());
    Ok(())
}

#[test]
fn control_inputs_reject_unknown_authority_fields() {
    let decoded = serde_json::from_value::<CodeTerminalOpenInput>(json!({
        "scope": scope('b'),
        "threadId": "thread-1",
        "cols": 80,
        "rows": 24,
        "cwd": "/caller-controlled"
    }));
    assert!(decoded.is_err());
}

#[test]
fn shutdown_gate_permanently_rejects_open() -> Result<(), String> {
    let manager = CodeTerminalManager::new();
    manager.shutdown()?;
    let inner = lock_manager(&manager.inner)?;
    assert_eq!(inner.lifecycle, ManagerLifecycle::Shutdown);
    Ok(())
}

#[test]
fn dimensions_and_session_ids_are_bounded() {
    assert!(validate_dimensions(80, 24).is_ok());
    assert!(validate_dimensions(0, 24).is_err());
    assert!(validate_dimensions(80, MAX_TERMINAL_DIMENSION + 1).is_err());
    assert!(validate_session_id("not-a-session").is_err());
    assert!(validate_session_id("67f11a1d-0274-4d40-9b0c-e406e51c64fb").is_ok());
}

#[test]
fn reader_queue_discards_during_native_teardown() -> Result<(), String> {
    let (output_tx, _output_rx) = std::sync::mpsc::sync_channel(1);
    output_tx
        .try_send(ReaderEvent::Output(vec![1]))
        .map_err(|error| error.to_string())?;
    let discard_output = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let thread_discard = Arc::clone(&discard_output);
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let forwarded =
            queue_reader_event(&output_tx, &thread_discard, ReaderEvent::Output(vec![2]));
        let _ = done_tx.send(forwarded);
    });

    std::thread::sleep(Duration::from_millis(20));
    discard_output.store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(
        done_rx.recv_timeout(Duration::from_secs(1)),
        Ok(true),
        "reader must not remain blocked on a full IPC queue during PTY close"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn queued_terminate_is_acknowledged_when_natural_exit_wins() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured_events = Arc::clone(&events);
    let manager = CodeTerminalManager::new();
    let session = manager.open(
        CodeTerminalOpenInput {
            scope: scope('9'),
            thread_id: "thread-terminate-exit-race".to_string(),
            cols: 80,
            rows: 24,
        },
        directory.path(),
        Channel::new(move |body| {
            let json = match body {
                InvokeResponseBody::Json(json) => json,
                InvokeResponseBody::Raw(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            };
            let event = serde_json::from_str(&json)?;
            match captured_events.lock() {
                Ok(mut events) => events.push(event),
                Err(poisoned) => poisoned.into_inner().push(event),
            }
            Ok(())
        }),
    )?;
    wait_for_shell_ready(&manager, &session, &events)?;

    let terminate_reply = {
        // Keep unregister blocked while the shell exits, then enqueue the
        // terminate waiter through the same lock used by the public command.
        // The actor must drain it after removing the exact owner.
        let mut inner = lock_manager(&manager.inner)?;
        let entry = inner
            .sessions
            .get_mut(&session.session_id)
            .ok_or_else(|| "terminal race-test session disappeared".to_string())?;
        let (stdin_reply_tx, stdin_reply_rx) = std::sync::mpsc::sync_channel(1);
        entry
            .control_tx
            .try_send(SessionControl::Stdin {
                data: b"exit\r".to_vec(),
                reply: stdin_reply_tx,
            })
            .map_err(|_| "failed to queue race-test shell exit".to_string())?;
        stdin_reply_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| format!("race-test stdin reply failed: {error}"))??;
        std::thread::sleep(Duration::from_millis(750));

        let (terminate_reply_tx, terminate_reply_rx) = std::sync::mpsc::sync_channel(1);
        entry.closing = true;
        entry
            .terminate_tx
            .send(TerminateControl {
                reply: Some(terminate_reply_tx),
            })
            .map_err(|_| "failed to queue race-test terminate".to_string())?;
        terminate_reply_rx
    };

    terminate_reply
        .recv_timeout(Duration::from_secs(3))
        .map_err(|error| format!("queued terminate was not acknowledged: {error}"))??;
    manager.shutdown()
}

#[cfg(unix)]
#[test]
fn native_pty_uses_bound_cwd_and_streams_stdin_output_exit() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured_events = Arc::clone(&events);
    let channel = Channel::new(move |body| {
        let json = match body {
            InvokeResponseBody::Json(json) => json,
            InvokeResponseBody::Raw(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        };
        let event = serde_json::from_str(&json)?;
        match captured_events.lock() {
            Ok(mut events) => events.push(event),
            Err(poisoned) => poisoned.into_inner().push(event),
        }
        Ok(())
    });

    let manager = CodeTerminalManager::new();
    let input = CodeTerminalOpenInput {
        scope: scope('c'),
        thread_id: "thread-native-pty".to_string(),
        cols: 80,
        rows: 24,
    };
    let session = manager.open(input, directory.path(), channel)?;
    wait_for_shell_ready(&manager, &session, &events)?;
    let wrong_owner_resize = manager.resize(CodeTerminalResizeInput {
        scope: scope('d'),
        thread_id: session.thread_id.clone(),
        session_id: session.session_id.clone(),
        cols: 100,
        rows: 30,
    });
    assert!(wrong_owner_resize.is_err());
    manager.resize(CodeTerminalResizeInput {
        scope: session.scope.clone(),
        thread_id: session.thread_id.clone(),
        session_id: session.session_id.clone(),
        cols: 100,
        rows: 30,
    })?;
    let marker = "__schoolx_terminal_cwd__";
    let command = format!("printf '{marker}%s\\n' \"$PWD\"\nexit\n");
    manager.stdin(CodeTerminalStdinInput {
        scope: session.scope.clone(),
        thread_id: session.thread_id.clone(),
        session_id: session.session_id.clone(),
        data: command.into_bytes(),
    })?;

    let deadline = Instant::now() + Duration::from_secs(5);
    let observed = loop {
        let snapshot = events
            .lock()
            .map_err(|_| "terminal event lock is unavailable".to_string())?
            .clone();
        if snapshot.iter().any(|event| event["type"] == "exit") {
            break snapshot;
        }
        if Instant::now() >= deadline {
            let _ = manager.shutdown();
            return Err("native terminal did not emit exit".to_string());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    manager.shutdown()?;

    let output = observed
        .iter()
        .filter(|event| event["type"] == "output")
        .filter_map(|event| event["data"].as_array())
        .flatten()
        .filter_map(Value::as_u64)
        .map(|byte| byte as u8)
        .collect::<Vec<_>>();
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains(marker));
    assert!(output.contains(&directory.path().to_string_lossy().to_string()));
    assert_eq!(
        observed.last().and_then(|event| event["exitCode"].as_u64()),
        Some(0)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn explicit_terminate_reaps_background_shell_job() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured_events = Arc::clone(&events);
    let channel = Channel::new(move |body| {
        let json = match body {
            InvokeResponseBody::Json(json) => json,
            InvokeResponseBody::Raw(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        };
        let event = serde_json::from_str(&json)?;
        match captured_events.lock() {
            Ok(mut events) => events.push(event),
            Err(poisoned) => poisoned.into_inner().push(event),
        }
        Ok(())
    });

    let manager = CodeTerminalManager::new();
    let session = manager.open(
        CodeTerminalOpenInput {
            scope: scope('e'),
            thread_id: "thread-descendant-cleanup".to_string(),
            cols: 80,
            rows: 24,
        },
        directory.path(),
        channel,
    )?;
    wait_for_shell_ready(&manager, &session, &events)?;
    let marker = "__schoolx_terminal_child_pid__";
    manager.stdin(CodeTerminalStdinInput {
        scope: session.scope.clone(),
        thread_id: session.thread_id.clone(),
        session_id: session.session_id.clone(),
        data: format!("(trap '' HUP TERM; exec /bin/sleep 30) & echo {marker}$!\n").into_bytes(),
    })?;

    let deadline = Instant::now() + Duration::from_secs(5);
    let child_pid = loop {
        let output = events
            .lock()
            .map_err(|_| "terminal event lock is unavailable".to_string())?
            .iter()
            .filter(|event| event["type"] == "output")
            .filter_map(|event| event["data"].as_array())
            .flatten()
            .filter_map(Value::as_u64)
            .map(|byte| byte as u8)
            .collect::<Vec<_>>();
        let output = String::from_utf8_lossy(&output);
        if let Some(offset) = output.rfind(marker) {
            let digits = output[offset + marker.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            if !digits.is_empty() {
                break digits
                    .parse::<i32>()
                    .map_err(|error| format!("invalid terminal child pid: {error}"))?;
            }
        }
        if Instant::now() >= deadline {
            let _ = manager.shutdown();
            return Err(format!(
                "terminal did not report its background child pid; output={output:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    manager.terminate(CodeTerminalTerminateInput {
        scope: session.scope,
        thread_id: session.thread_id,
        session_id: session.session_id,
    })?;
    manager.shutdown()?;

    let child_pid = rustix::process::Pid::from_raw(child_pid)
        .ok_or_else(|| "terminal reported an invalid background child pid".to_string())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match rustix::process::test_kill_process(child_pid) {
            Err(error) if error == rustix::io::Errno::SRCH => break,
            Ok(()) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(()) => {
                let _ = rustix::process::kill_process(child_pid, rustix::process::Signal::KILL);
                return Err("background terminal child survived explicit terminate".to_string());
            }
            Err(error) => {
                return Err(format!("failed to inspect terminal child cleanup: {error}"));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn observed_leader_exit_reaps_reparented_session_job() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured_events = Arc::clone(&events);
    let channel = Channel::new(move |body| {
        let json = match body {
            InvokeResponseBody::Json(json) => json,
            InvokeResponseBody::Raw(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        };
        let event = serde_json::from_str(&json)?;
        match captured_events.lock() {
            Ok(mut events) => events.push(event),
            Err(poisoned) => poisoned.into_inner().push(event),
        }
        Ok(())
    });

    let manager = CodeTerminalManager::new();
    let session = manager.open(
        CodeTerminalOpenInput {
            scope: scope('f'),
            thread_id: "thread-natural-descendant-cleanup".to_string(),
            cols: 80,
            rows: 24,
        },
        directory.path(),
        channel,
    )?;
    wait_for_shell_ready(&manager, &session, &events)?;
    let marker = "__schoolx_terminal_orphan_pid__";
    manager.stdin(CodeTerminalStdinInput {
        scope: session.scope.clone(),
        thread_id: session.thread_id.clone(),
        session_id: session.session_id.clone(),
        data: format!("(trap '' HUP TERM; exec /bin/sleep 30) & echo {marker}$!\n").into_bytes(),
    })?;

    let deadline = Instant::now() + Duration::from_secs(5);
    let child_pid = loop {
        let output = events
            .lock()
            .map_err(|_| "terminal event lock is unavailable".to_string())?
            .iter()
            .filter(|event| event["type"] == "output")
            .filter_map(|event| event["data"].as_array())
            .flatten()
            .filter_map(Value::as_u64)
            .map(|byte| byte as u8)
            .collect::<Vec<_>>();
        let output = String::from_utf8_lossy(&output);
        if let Some(offset) = output.rfind(marker) {
            let digits = output[offset + marker.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            if !digits.is_empty() {
                break digits
                    .parse::<i32>()
                    .map_err(|error| format!("invalid terminal orphan pid: {error}"))?;
            }
        }
        if Instant::now() >= deadline {
            let _ = manager.shutdown();
            return Err("terminal did not report its potential orphan pid".to_string());
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    // Kill only the shell leader. The ignored-signal background process is
    // immediately reparented but retains the PTY leader's POSIX session ID.
    manager.stdin(CodeTerminalStdinInput {
        scope: session.scope,
        thread_id: session.thread_id,
        session_id: session.session_id,
        data: b"kill -KILL $$\n".to_vec(),
    })?;
    let exit_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if events
            .lock()
            .map_err(|_| "terminal event lock is unavailable".to_string())?
            .iter()
            .any(|event| event["type"] == "exit")
        {
            break;
        }
        if Instant::now() >= exit_deadline {
            let _ = manager.shutdown();
            return Err("terminal did not observe its shell leader exit".to_string());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    manager.shutdown()?;

    let child_pid = rustix::process::Pid::from_raw(child_pid)
        .ok_or_else(|| "terminal reported an invalid orphan pid".to_string())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match rustix::process::test_kill_process(child_pid) {
            Err(error) if error == rustix::io::Errno::SRCH => return Ok(()),
            Ok(()) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(()) => {
                let _ = rustix::process::kill_process(child_pid, rustix::process::Signal::KILL);
                return Err("reparented terminal session child survived leader exit".to_string());
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect reparented child cleanup: {error}"
                ));
            }
        }
    }
}

impl CodeTerminalManager {
    pub(crate) fn install_test_owner(
        &self,
        scope: &CodeThreadBindingScope,
        thread_id: &str,
        drained_marker: std::path::PathBuf,
    ) -> Result<(), String> {
        let owner = SessionOwner {
            scope: scope.clone(),
            thread_id: thread_id.to_string(),
        };
        owner.validate()?;
        let session_id = uuid::Uuid::new_v4().hyphenated().to_string();
        let (control_tx, _control_rx) = mpsc::sync_channel(1);
        let (terminate_tx, terminate_rx) = mpsc::channel();
        {
            let mut inner = lock_manager(&self.inner)?;
            if inner.sessions.values().any(|entry| entry.owner == owner) {
                return Err("Synthetic terminal owner already exists".to_string());
            }
            inner.sessions.insert(
                session_id.clone(),
                SessionEntry {
                    owner,
                    control_tx,
                    terminate_tx,
                    closing: false,
                },
            );
        }

        let manager = Arc::clone(&self.inner);
        let actor_session_id = session_id.clone();
        thread::Builder::new()
            .name(format!("code-terminal-test-{session_id}"))
            .spawn(move || {
                if let Ok(control) = terminate_rx.recv() {
                    let result = std::fs::write(&drained_marker, b"drained")
                        .map_err(|error| error.to_string());
                    if let Ok(mut inner) = manager.lock() {
                        inner.sessions.remove(&actor_session_id);
                    }
                    if let Some(reply) = control.reply {
                        let _ = reply.send(result);
                    }
                }
            })
            .map_err(|error| {
                if let Ok(mut inner) = self.inner.lock() {
                    inner.sessions.remove(&session_id);
                }
                format!("failed to start synthetic terminal actor: {error}")
            })?;
        Ok(())
    }
}
