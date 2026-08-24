use std::fs;
use std::time::{Duration, Instant};

#[cfg(unix)]
use axum::{
    extract::State, http::StatusCode, response::IntoResponse as _, routing::post, Json, Router,
};

use super::super::approvals::{
    CodeApprovalDecision, CodeApprovalResponse, CodeApprovalResponseInput, CodePermissionScope,
};
use super::super::bindings::CodeThreadBindingScope;
use super::super::protocol::CodeRequestId;
use super::*;

#[cfg(unix)]
fn fake_codex(script_body: &str) -> Result<(tempfile::TempDir, PathBuf), String> {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let path = directory.path().join("codex");
    fs::write(&path, script_body).map_err(|error| error.to_string())?;
    let mut permissions = fs::metadata(&path)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).map_err(|error| error.to_string())?;
    Ok((directory, path))
}

#[cfg(unix)]
#[derive(Clone)]
struct MockResponsesState {
    responses: Arc<Mutex<VecDeque<String>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

#[cfg(unix)]
struct MockResponsesServer {
    base_url: String,
    responses: Arc<Mutex<VecDeque<String>>>,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

#[cfg(unix)]
impl MockResponsesServer {
    async fn start(responses: Vec<String>) -> Result<Self, String> {
        let state = MockResponsesState {
            responses: Arc::new(Mutex::new(responses.into())),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new()
            .route("/v1/responses", post(serve_mock_response))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                eprintln!("SchoolX Code mock Responses server failed: {error}");
            }
        });
        Ok(Self {
            base_url: format!("http://{address}/v1"),
            responses: state.responses,
            requests: state.requests,
            task,
        })
    }

    fn requests(&self) -> Result<Vec<Value>, String> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|error| error.to_string())
    }

    fn remaining_responses(&self) -> Result<usize, String> {
        self.responses
            .lock()
            .map(|responses| responses.len())
            .map_err(|error| error.to_string())
    }
}

#[cfg(unix)]
impl Drop for MockResponsesServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(unix)]
async fn serve_mock_response(
    State(state): State<MockResponsesState>,
    Json(request): Json<Value>,
) -> axum::response::Response {
    let Ok(mut requests) = state.requests.lock() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "mock request log is unavailable",
        )
            .into_response();
    };
    requests.push(request);
    drop(requests);

    let Ok(mut responses) = state.responses.lock() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "mock response queue is unavailable",
        )
            .into_response();
    };
    let Some(response) = responses.pop_front() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "mock response queue was exhausted",
        )
            .into_response();
    };
    (
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        response,
    )
        .into_response()
}

#[cfg(unix)]
fn sse_response(events: &[Value]) -> Result<String, String> {
    let mut response = String::new();
    for event in events {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "mock SSE event has no type".to_string())?;
        response.push_str("event: ");
        response.push_str(kind);
        response.push('\n');
        response.push_str("data: ");
        response.push_str(&serde_json::to_string(event).map_err(|error| error.to_string())?);
        response.push_str("\n\n");
    }
    Ok(response)
}

#[cfg(unix)]
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
struct ProcessGroupCleanupGuard {
    pid: Option<rustix::process::Pid>,
}

#[cfg(unix)]
impl ProcessGroupCleanupGuard {
    fn new(pid: rustix::process::Pid) -> Self {
        Self { pid: Some(pid) }
    }

    fn disarm(&mut self) {
        self.pid = None;
    }
}

#[cfg(unix)]
impl Drop for ProcessGroupCleanupGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
    }
}

#[cfg(unix)]
fn wait_for_process_exit(pid: rustix::process::Pid, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match rustix::process::test_kill_process(pid) {
            Err(error) if error == rustix::io::Errno::SRCH => return Ok(()),
            Ok(()) => {}
            Err(error) => {
                return Err(format!("failed to inspect descendant process: {error}"));
            }
        }
        if Instant::now() >= deadline {
            return Err("Codex app-server descendant survived teardown".to_string());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn wait_for_process_group_exit(pid: rustix::process::Pid, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match rustix::process::test_kill_process_group(pid) {
            Err(error) if error == rustix::io::Errno::SRCH => return Ok(()),
            Ok(()) => {}
            Err(error) => {
                return Err(format!("failed to inspect Codex process group: {error}"));
            }
        }
        if Instant::now() >= deadline {
            return Err("Codex app-server process group survived teardown".to_string());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn real_codex_wrapper(
    executable: &Path,
    codex_home: &Path,
) -> Result<(tempfile::TempDir, PathBuf), String> {
    let managed_config = codex_home.join("managed-config.toml");
    fs::write(&managed_config, "").map_err(|error| error.to_string())?;
    fake_codex(&format!(
            "#!/bin/sh\nexport CODEX_HOME={}\nexport CODEX_APP_SERVER_MANAGED_CONFIG_PATH={}\nexport CODEX_APP_SERVER_DISABLE_MANAGED_CONFIG=1\nexport CODEX_PERMISSION_PROBE_TOKEN=local-only-dummy\nexec {} \"$@\"\n",
            shell_quote(codex_home),
            shell_quote(&managed_config),
            shell_quote(executable)
        ))
}

fn noop_emitter() -> CodeEventEmitter {
    Arc::new(|_| {})
}

fn binding_scope() -> CodeThreadBindingScope {
    CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: "a".repeat(64),
    }
}

#[cfg(unix)]
fn recorded_requests(executable: &Path) -> Result<Vec<Value>, String> {
    let path = executable.with_file_name("codex.requests");
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    contents
        .lines()
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
        .collect()
}

#[cfg(unix)]
fn requests_for_method<'a>(requests: &'a [Value], method: &str) -> Vec<&'a Value> {
    requests
        .iter()
        .filter(|request| request["method"] == method)
        .collect()
}

fn wait_for_event_with_timeout(
    runtime: &CodeRuntime,
    kind: &str,
    timeout: Duration,
) -> Result<CodeRuntimeEvent, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(event) = runtime
            .events(None, None)?
            .events
            .into_iter()
            .find(|event| event.kind == kind)
        {
            return Ok(event);
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for `{kind}`"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_event(runtime: &CodeRuntime, kind: &str) -> Result<CodeRuntimeEvent, String> {
    wait_for_event_with_timeout(runtime, kind, Duration::from_secs(2))
}

mod event_tests;
mod graph_tests;
mod lifecycle_tests;
mod manual_tests;
mod process_tests;
mod turn_tests;
