use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::approvals::{CodeApprovalResponseInput, PendingApprovalStore};
use super::discovery::{ensure_supported_codex_version, probe_codex, CodeRuntimeProbe};
use super::jsonrpc::{self, IncomingMessage};
use super::paths::canonical_workspace_root;
use super::protocol::{
    self, CodeRecoveryThread, CodeRuntimeEvent, CodeRuntimeEventBacklog, CodeThreadResumeInput,
    CodeThreadRpcOpenResult, CodeThreadStartInput, CodeThreadSummary, CodeTurnInterruptInput,
    CodeTurnStartInput, CodeTurnSteerInput, CodeTurnSummary, CodeWorkspaceEventDraft,
};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_NOTIFICATION_BACKLOG: usize = 512;
const STDERR_TAIL_BYTES: usize = 64 * 1024;
const MAX_RECOVERY_THREADS: usize = 4_096;
const MAX_RECOVERY_PAGES: usize = 64;

/// Build the stable, non-experimental Codex app-server handshake payload.
pub(crate) fn initialize_params() -> Value {
    json!({
        "clientInfo": {
            "name": "schoolx-code",
            "title": "SchoolX Code",
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {
            "experimentalApi": false,
            "requestAttestation": false
        }
    })
}

/// Delivery classification for the one RPC whose durable journal must decide
/// whether a retry is safe.
#[derive(Debug)]
pub(crate) enum CodeThreadStartRpcError {
    /// Validation or runtime state failed before any request bytes were written.
    NotSent(String),
    /// A write was attempted or completed, so Codex may have created a thread.
    Uncertain(String),
}

impl CodeThreadStartRpcError {
    pub(crate) fn definitely_not_sent(&self) -> bool {
        matches!(self, Self::NotSent(_))
    }

    pub(crate) fn into_message(self) -> String {
        match self {
            Self::NotSent(message) | Self::Uncertain(message) => message,
        }
    }
}

/// Event name carrying scoped SchoolX Code workspace events at the Tauri boundary.
pub const CODE_WORKSPACE_EVENT: &str = "schoolx-code-workspace-event";

pub(crate) type CodeEventEmitter = Arc<dyn Fn(CodeRuntimeEvent) + Send + Sync>;

/// Lifecycle state of the desktop-wide Codex app-server process.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeRuntimePhase {
    NotInstalled,
    #[default]
    Stopped,
    Starting,
    Initializing,
    Ready,
    Stopping,
    Failed,
}

/// Current process and handshake status exposed to the frontend.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRuntimeStatus {
    pub phase: CodeRuntimePhase,
    pub generation: u64,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub pid: Option<u32>,
    pub user_agent: Option<String>,
    pub codex_home: Option<String>,
    pub platform_family: Option<String>,
    pub platform_os: Option<String>,
    pub queued_notifications: usize,
    pub last_error: Option<String>,
}

/// Desktop-wide owner of the Codex process, event bridge, and approval gate.
#[derive(Clone)]
pub struct CodeRuntime {
    inner: Arc<Mutex<RuntimeInner>>,
    events: Arc<EventBridge>,
    approvals: Arc<PendingApprovalStore>,
    explicit_executable: Option<PathBuf>,
}

struct RuntimeInner {
    phase: CodeRuntimePhase,
    generation: u64,
    probe: Option<CodeRuntimeProbe>,
    initialized: Option<InitializeResult>,
    process: Option<RuntimeProcess>,
    last_error: Option<String>,
}

struct EventBridge {
    inner: Mutex<EventBridgeInner>,
}

struct EventBridgeInner {
    generation: u64,
    next_sequence: u64,
    backlog: VecDeque<CodeRuntimeEvent>,
    emitter: CodeEventEmitter,
}

impl EventBridge {
    fn new() -> Self {
        Self {
            inner: Mutex::new(EventBridgeInner {
                generation: 0,
                next_sequence: 1,
                backlog: VecDeque::new(),
                emitter: Arc::new(|_| {}),
            }),
        }
    }

    fn reset(&self, generation: u64, emitter: CodeEventEmitter) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        inner.generation = generation;
        inner.next_sequence = 1;
        inner.backlog.clear();
        inner.emitter = emitter;
        Ok(())
    }

    fn replace_emitter(&self, emitter: CodeEventEmitter) -> Result<(), String> {
        self.inner
            .lock()
            .map_err(|error| error.to_string())?
            .emitter = emitter;
        Ok(())
    }

    fn publish(&self, generation: u64, draft: CodeWorkspaceEventDraft) {
        let publication = self.inner.lock().ok().and_then(|mut inner| {
            if inner.generation != generation {
                return None;
            }
            let sequence = inner.next_sequence;
            inner.next_sequence = inner.next_sequence.saturating_add(1);
            let event = CodeRuntimeEvent {
                runtime_generation: generation,
                sequence,
                thread_id: draft.thread_id,
                turn_id: draft.turn_id,
                item_id: draft.item_id,
                kind: draft.kind,
                payload: draft.payload,
            };
            if inner.backlog.len() == MAX_NOTIFICATION_BACKLOG {
                inner.backlog.pop_front();
            }
            inner.backlog.push_back(event.clone());
            Some((Arc::clone(&inner.emitter), event))
        });
        if let Some((emitter, event)) = publication {
            emitter(event);
        }
    }

    fn snapshot(
        &self,
        requested_generation: Option<u64>,
        after_sequence: Option<u64>,
    ) -> Result<CodeRuntimeEventBacklog, String> {
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        let generation_changed =
            requested_generation.is_some_and(|value| value != inner.generation);
        let after_sequence = if generation_changed {
            0
        } else {
            after_sequence.unwrap_or_default()
        };
        let oldest_sequence = inner.backlog.front().map(|event| event.sequence);
        let truncated = generation_changed
            || oldest_sequence.is_some_and(|oldest| after_sequence.saturating_add(1) < oldest);
        let events = inner
            .backlog
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .cloned()
            .collect();
        Ok(CodeRuntimeEventBacklog {
            runtime_generation: inner.generation,
            latest_sequence: inner.next_sequence.saturating_sub(1),
            truncated,
            events,
        })
    }

    fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|inner| inner.backlog.len())
            .unwrap_or_default()
    }
}

impl Default for CodeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeInner {
                phase: CodeRuntimePhase::Stopped,
                generation: 0,
                probe: None,
                initialized: None,
                process: None,
                last_error: None,
            })),
            events: Arc::new(EventBridge::new()),
            approvals: Arc::new(PendingApprovalStore::default()),
            explicit_executable: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_executable(path: PathBuf) -> Self {
        let mut runtime = Self::new();
        runtime.explicit_executable = Some(path);
        runtime
    }

    pub fn probe(&self) -> CodeRuntimeProbe {
        let probe = probe_codex(self.explicit_executable.as_deref());
        if let Ok(mut inner) = self.inner.lock() {
            inner.probe = Some(probe.clone());
            if inner.process.is_none() {
                inner.phase = if probe.available {
                    CodeRuntimePhase::Stopped
                } else {
                    CodeRuntimePhase::NotInstalled
                };
                inner.last_error = probe.error.clone();
            }
        }
        probe
    }

    pub fn status(&self) -> Result<CodeRuntimeStatus, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals);
        Ok(status_from_inner(&inner, self.events.len()))
    }

    pub(crate) fn start(&self, emitter: CodeEventEmitter) -> Result<CodeRuntimeStatus, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals);
        if inner.phase == CodeRuntimePhase::Ready {
            self.events.replace_emitter(emitter)?;
            return Ok(status_from_inner(&inner, self.events.len()));
        }
        if let Some(mut process) = inner.process.take() {
            let _ = process.stop();
        }

        inner.generation = inner
            .generation
            .checked_add(1)
            .ok_or_else(|| "Codex runtime generation was exhausted".to_string())?;
        let generation = inner.generation;
        self.events.reset(generation, emitter)?;
        self.approvals.reset(generation);
        inner.phase = CodeRuntimePhase::Starting;
        inner.initialized = None;
        inner.last_error = None;

        let probe = probe_codex(self.explicit_executable.as_deref());
        inner.probe = Some(probe.clone());
        if !probe.available {
            let error = probe
                .error
                .clone()
                .unwrap_or_else(|| "Codex CLI is not available".to_string());
            inner.phase = CodeRuntimePhase::NotInstalled;
            inner.last_error = Some(error.clone());
            return Err(error);
        }
        if let Err(error) = ensure_supported_codex_version(&probe) {
            inner.phase = CodeRuntimePhase::Failed;
            inner.last_error = Some(error.clone());
            return Err(error);
        }
        let executable = probe
            .executable
            .as_deref()
            .map(Path::new)
            .ok_or_else(|| "Codex probe returned no executable path".to_string())?;

        let mut process = match RuntimeProcess::spawn(
            executable,
            generation,
            Arc::clone(&self.events),
            Arc::clone(&self.approvals),
        ) {
            Ok(process) => process,
            Err(error) => {
                inner.phase = CodeRuntimePhase::Failed;
                inner.last_error = Some(error.clone());
                return Err(error);
            }
        };
        inner.phase = CodeRuntimePhase::Initializing;
        match process.initialize() {
            Ok(initialized) => {
                inner.initialized = Some(initialized);
                inner.phase = CodeRuntimePhase::Ready;
                inner.process = Some(process);
                Ok(status_from_inner(&inner, self.events.len()))
            }
            Err(error) => {
                let stderr = process.stderr_tail();
                let _ = process.stop();
                self.approvals.clear_generation(generation);
                let detail = if stderr.trim().is_empty() {
                    error
                } else {
                    format!("{error} ({})", first_line(&stderr))
                };
                inner.phase = CodeRuntimePhase::Failed;
                inner.last_error = Some(detail.clone());
                Err(detail)
            }
        }
    }

    pub fn stop(&self) -> Result<CodeRuntimeStatus, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        inner.phase = CodeRuntimePhase::Stopping;
        let generation = inner.generation;
        let result = match inner.process.take() {
            Some(mut process) => process.stop(),
            None => Ok(()),
        };
        self.approvals.clear_generation(generation);
        inner.phase = CodeRuntimePhase::Stopped;
        inner.initialized = None;
        if let Err(error) = result {
            inner.last_error = Some(error.clone());
            return Err(error);
        }
        inner.last_error = None;
        Ok(status_from_inner(&inner, self.events.len()))
    }

    pub(crate) fn events(
        &self,
        runtime_generation: Option<u64>,
        after_sequence: Option<u64>,
    ) -> Result<CodeRuntimeEventBacklog, String> {
        self.events.snapshot(runtime_generation, after_sequence)
    }

    pub(crate) fn thread_start_at(
        &self,
        input: CodeThreadStartInput,
        workspace_root: &str,
    ) -> Result<CodeThreadRpcOpenResult, CodeThreadStartRpcError> {
        let workspace_root =
            canonical_workspace_root(workspace_root).map_err(CodeThreadStartRpcError::NotSent)?;
        let params = input
            .rpc_params(&workspace_root)
            .map_err(CodeThreadStartRpcError::NotSent)?;
        let result = self.request_ready_for_thread_start(params)?;
        protocol::parse_thread_open(result).map_err(CodeThreadStartRpcError::Uncertain)
    }

    pub(crate) fn ensure_ready(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        Ok(())
    }

    pub(crate) fn thread_resume_at(
        &self,
        input: CodeThreadResumeInput,
        workspace_root: &str,
    ) -> Result<CodeThreadRpcOpenResult, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let result = self.request_ready("thread/resume", params)?;
        protocol::parse_thread_open(result)
    }

    pub(crate) fn thread_read(&self, thread_id: &str) -> Result<CodeThreadSummary, String> {
        let params = protocol::thread_read_params(thread_id)?;
        let result = self.request_ready("thread/read", params)?;
        protocol::parse_thread_read(result)
    }

    /// Discover every persisted or currently-loaded Codex thread at one exact
    /// native execution root. This is intentionally not a general frontend
    /// listing API: it exists only to reconcile an ambiguous `thread/start`.
    pub(crate) fn recovery_threads_at(
        &self,
        workspace_root: &str,
    ) -> Result<Vec<CodeRecoveryThread>, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let mut candidates = HashMap::<String, CodeRecoveryThread>::new();

        let mut cursor = None;
        let mut seen_cursors = HashSet::new();
        for _ in 0..MAX_RECOVERY_PAGES {
            let params = protocol::recovery_thread_list_params(&workspace_root, cursor.as_deref())?;
            let page =
                protocol::parse_recovery_thread_list(self.request_ready("thread/list", params)?)?;
            for candidate in page.data {
                validate_recovery_candidate_root(&candidate, &workspace_root)?;
                if candidates
                    .insert(candidate.thread.id.clone(), candidate)
                    .is_some()
                {
                    return Err(
                        "Codex recovery thread list contained a duplicate thread id".to_string()
                    );
                }
                if candidates.len() > MAX_RECOVERY_THREADS {
                    return Err(format!(
                        "Codex recovery exceeded the {MAX_RECOVERY_THREADS}-thread safety limit"
                    ));
                }
            }
            match page.next_cursor {
                Some(next_cursor) => {
                    if !seen_cursors.insert(next_cursor.clone()) {
                        return Err("Codex recovery pagination repeated a cursor".to_string());
                    }
                    cursor = Some(next_cursor);
                }
                None => {
                    cursor = None;
                    break;
                }
            }
        }
        if cursor.is_some() {
            return Err(format!(
                "Codex recovery exceeded the {MAX_RECOVERY_PAGES}-page safety limit"
            ));
        }

        // A newly-created empty 0.145 thread can remain deferred in memory and
        // therefore be absent from `thread/list`. Include loaded IDs and read
        // their metadata before deciding whether the start produced a thread.
        let mut loaded_ids = Vec::new();
        let mut loaded_cursor = None;
        let mut seen_loaded_cursors = HashSet::new();
        for _ in 0..MAX_RECOVERY_PAGES {
            let params = protocol::loaded_thread_list_params(loaded_cursor.as_deref())?;
            let page = protocol::parse_loaded_thread_list(
                self.request_ready("thread/loaded/list", params)?,
            )?;
            loaded_ids.extend(page.data);
            if loaded_ids.len() > MAX_RECOVERY_THREADS {
                return Err(format!(
                    "Codex loaded-thread recovery exceeded the {MAX_RECOVERY_THREADS}-thread safety limit"
                ));
            }
            match page.next_cursor {
                Some(next_cursor) => {
                    if !seen_loaded_cursors.insert(next_cursor.clone()) {
                        return Err(
                            "Codex loaded-thread recovery pagination repeated a cursor".to_string()
                        );
                    }
                    loaded_cursor = Some(next_cursor);
                }
                None => {
                    loaded_cursor = None;
                    break;
                }
            }
        }
        if loaded_cursor.is_some() {
            return Err(format!(
                "Codex loaded-thread recovery exceeded the {MAX_RECOVERY_PAGES}-page safety limit"
            ));
        }

        let mut seen_loaded_ids = HashSet::new();
        for thread_id in loaded_ids {
            if !seen_loaded_ids.insert(thread_id.clone()) || candidates.contains_key(&thread_id) {
                continue;
            }
            let params = protocol::recovery_thread_read_params(&thread_id)?;
            let candidate =
                protocol::parse_recovery_thread_read(self.request_ready("thread/read", params)?)?;
            if candidate.thread.id != thread_id {
                return Err(
                    "Codex returned a different thread during loaded-thread recovery".to_string(),
                );
            }
            if recovery_candidate_matches_root(&candidate, &workspace_root)? {
                candidates.insert(thread_id, candidate);
            }
        }

        let mut candidates = candidates.into_values().collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.thread.id.cmp(&right.thread.id));
        Ok(candidates)
    }

    pub(crate) fn recovery_thread_read(
        &self,
        thread_id: &str,
    ) -> Result<CodeRecoveryThread, String> {
        let params = protocol::recovery_thread_read_params(thread_id)?;
        let result = self.request_ready("thread/read", params)?;
        protocol::parse_recovery_thread_read(result)
    }

    pub(crate) fn turn_start_at(
        &self,
        input: CodeTurnStartInput,
        workspace_root: &str,
    ) -> Result<CodeTurnSummary, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let result = self.request_ready("turn/start", params)?;
        protocol::parse_turn_start(result)
    }

    pub fn turn_steer(&self, input: CodeTurnSteerInput) -> Result<CodeTurnSummary, String> {
        let params = input.rpc_params()?;
        let result = self.request_ready("turn/steer", params)?;
        protocol::parse_turn_steer(result)
    }

    pub fn turn_interrupt(&self, input: CodeTurnInterruptInput) -> Result<(), String> {
        let params = input.rpc_params()?;
        self.request_ready("turn/interrupt", params)?;
        let generation = self
            .inner
            .lock()
            .map_err(|error| error.to_string())?
            .generation;
        self.approvals
            .clear_turn(generation, &input.thread_id, &input.turn_id);
        Ok(())
    }

    pub fn approval_respond(&self, input: CodeApprovalResponseInput) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals);
        if inner.phase != CodeRuntimePhase::Ready || inner.generation != input.runtime_generation {
            return Err("Codex approval belongs to an inactive runtime generation".to_string());
        }
        let process = inner
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        respond_to_pending_approval(&self.approvals, &input, |request_id, result| {
            process.respond(request_id, result)
        })
    }

    fn request_ready(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        inner
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?
            .request(method, params, REQUEST_TIMEOUT)
    }

    fn request_ready_for_thread_start(
        &self,
        params: Value,
    ) -> Result<Value, CodeThreadStartRpcError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|error| CodeThreadStartRpcError::NotSent(error.to_string()))?;
        refresh_process_health(&mut inner, &self.approvals);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err(CodeThreadStartRpcError::NotSent(
                "Codex app-server is not ready".to_string(),
            ));
        }
        inner
            .process
            .as_ref()
            .ok_or_else(|| {
                CodeThreadStartRpcError::NotSent("Codex app-server is not running".to_string())
            })?
            .request_with_delivery("thread/start", params, REQUEST_TIMEOUT)
    }
}

fn validate_recovery_candidate_root(
    candidate: &CodeRecoveryThread,
    expected_root: &str,
) -> Result<(), String> {
    if recovery_candidate_matches_root(candidate, expected_root)? {
        Ok(())
    } else {
        Err("Codex exact-root thread list returned a thread outside the requested root".to_string())
    }
}

fn recovery_candidate_matches_root(
    candidate: &CodeRecoveryThread,
    expected_root: &str,
) -> Result<bool, String> {
    let Some(reported_root) = candidate.thread.cwd.as_deref() else {
        return Err("Codex recovery thread did not report an execution root".to_string());
    };
    match canonical_workspace_root(reported_root) {
        Ok(reported_root) => Ok(reported_root == expected_root),
        // Loaded threads are global to the app-server and can legitimately
        // point at a checkout that was removed. Such a thread cannot match the
        // live canonical SchoolX root and is ignored.
        Err(_) if reported_root != expected_root => Ok(false),
        Err(error) => Err(error),
    }
}

fn respond_to_pending_approval(
    approvals: &PendingApprovalStore,
    input: &CodeApprovalResponseInput,
    respond: impl FnOnce(Value, Value) -> Result<(), String>,
) -> Result<(), String> {
    let reservation = approvals.reserve_response(input)?;
    let (request_id, result) = reservation.wire_response();
    match respond(request_id, result) {
        Ok(()) => approvals.commit_response(&reservation),
        Err(response_error) => match approvals.restore_response(&reservation) {
            Ok(()) => Err(response_error),
            Err(restore_error) => Err(format!(
                "{response_error}; failed to restore pending Codex approval: {restore_error}"
            )),
        },
    }
}

fn refresh_process_health(inner: &mut RuntimeInner, approvals: &PendingApprovalStore) {
    let failure = inner
        .process
        .as_mut()
        .and_then(RuntimeProcess::health_error);
    if let Some(error) = failure {
        if let Some(mut process) = inner.process.take() {
            let _ = process.stop();
        }
        approvals.clear_generation(inner.generation);
        inner.phase = CodeRuntimePhase::Failed;
        inner.initialized = None;
        inner.last_error = Some(error);
    }
}

fn status_from_inner(inner: &RuntimeInner, queued_notifications: usize) -> CodeRuntimeStatus {
    let initialized = inner.initialized.as_ref();
    CodeRuntimeStatus {
        phase: inner.phase,
        generation: inner.generation,
        executable: inner
            .probe
            .as_ref()
            .and_then(|probe| probe.executable.clone()),
        version: inner.probe.as_ref().and_then(|probe| probe.version.clone()),
        pid: inner.process.as_ref().map(|process| process.child.id()),
        user_agent: initialized.map(|result| result.user_agent.clone()),
        codex_home: initialized.map(|result| result.codex_home.clone()),
        platform_family: initialized.map(|result| result.platform_family.clone()),
        platform_os: initialized.map(|result| result.platform_os.clone()),
        queued_notifications,
        last_error: inner.last_error.clone(),
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeResult {
    user_agent: String,
    codex_home: String,
    platform_family: String,
    platform_os: String,
}

type RpcReply = Result<Value, String>;
type PendingRequests = Arc<Mutex<HashMap<u64, mpsc::SyncSender<RpcReply>>>>;

struct RuntimeProcess {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: PendingRequests,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    alive: Arc<AtomicBool>,
    stream_error: Arc<Mutex<Option<String>>>,
    next_request_id: AtomicU64,
    stopped: bool,
}

impl RuntimeProcess {
    fn spawn(
        executable: &Path,
        generation: u64,
        events: Arc<EventBridge>,
        approvals: Arc<PendingApprovalStore>,
    ) -> Result<Self, String> {
        let mut command = Command::new(executable);
        command
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(workdir) = crate::managed_agents::default_agent_workdir() {
            command.current_dir(workdir);
        }
        if let Some(path) = crate::managed_agents::login_shell_path() {
            command.env("PATH", path);
        }
        crate::util::configure_no_window(&mut command);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start Codex app-server: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Codex app-server stdin was not captured".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Codex app-server stdout was not captured".to_string())?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| "Codex app-server stderr was not captured".to_string())?;

        let stdin = Arc::new(Mutex::new(stdin));
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let stream_error = Arc::new(Mutex::new(None));

        spawn_stdout_dispatcher(
            stdout,
            generation,
            Arc::clone(&stdin),
            Arc::clone(&pending),
            events,
            approvals,
            Arc::clone(&alive),
            Arc::clone(&stream_error),
        );
        spawn_stderr_drain(stderr_pipe, Arc::clone(&stderr));

        Ok(Self {
            child,
            stdin,
            pending,
            stderr,
            alive,
            stream_error,
            next_request_id: AtomicU64::new(1),
            stopped: false,
        })
    }

    fn initialize(&mut self) -> Result<InitializeResult, String> {
        let result = self.request("initialize", initialize_params(), INITIALIZE_TIMEOUT)?;
        let initialized = serde_json::from_value(result)
            .map_err(|error| format!("invalid Codex initialize response: {error}"))?;
        let notification = jsonrpc::notification("initialized");
        let mut writer = self.stdin.lock().map_err(|error| error.to_string())?;
        jsonrpc::write_value(&mut *writer, &notification)?;
        Ok(initialized)
    }

    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        self.request_with_delivery(method, params, timeout)
            .map_err(CodeThreadStartRpcError::into_message)
    }

    fn request_with_delivery(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CodeThreadStartRpcError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(CodeThreadStartRpcError::NotSent(
                self.stream_error()
                    .unwrap_or_else(|| "Codex app-server stream is closed".to_string()),
            ));
        }
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let message = jsonrpc::request(id, method, params);
        jsonrpc::validate_value_size(&message).map_err(CodeThreadStartRpcError::NotSent)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending
            .lock()
            .map_err(|error| CodeThreadStartRpcError::NotSent(error.to_string()))?
            .insert(id, sender);

        let write_result = match self.stdin.lock() {
            Ok(mut writer) => jsonrpc::write_value(&mut *writer, &message),
            Err(error) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                return Err(CodeThreadStartRpcError::NotSent(error.to_string()));
            }
        };
        if let Err(error) = write_result {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            return Err(CodeThreadStartRpcError::Uncertain(error));
        }

        match receiver.recv_timeout(timeout) {
            Ok(reply) => reply.map_err(CodeThreadStartRpcError::Uncertain),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                Err(CodeThreadStartRpcError::Uncertain(format!(
                    "Codex `{method}` request timed out"
                )))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(CodeThreadStartRpcError::Uncertain(
                self.stream_error()
                    .unwrap_or_else(|| "Codex app-server response channel closed".to_string()),
            )),
        }
    }

    fn respond(&self, request_id: Value, result: Value) -> Result<(), String> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(self
                .stream_error()
                .unwrap_or_else(|| "Codex app-server stream is closed".to_string()));
        }
        let response = jsonrpc::response(request_id, result);
        self.stdin
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|mut writer| jsonrpc::write_value(&mut *writer, &response))
    }

    fn health_error(&mut self) -> Option<String> {
        if !self.alive.load(Ordering::Acquire) {
            return Some(
                self.stream_error()
                    .unwrap_or_else(|| "Codex app-server stream closed".to_string()),
            );
        }
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!("Codex app-server exited with {status}")),
            Ok(None) => None,
            Err(error) => Some(format!("failed to inspect Codex app-server: {error}")),
        }
    }

    fn stream_error(&self) -> Option<String> {
        self.stream_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
    }

    fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .map(|mut bytes| String::from_utf8_lossy(bytes.make_contiguous()).into_owned())
            .unwrap_or_default()
    }

    fn stop(&mut self) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        self.alive.store(false, Ordering::Release);
        terminate_child(&mut self.child)
    }
}

impl Drop for RuntimeProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_stdout_dispatcher(
    stdout: impl Read + Send + 'static,
    generation: u64,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: PendingRequests,
    events: Arc<EventBridge>,
    approvals: Arc<PendingApprovalStore>,
    alive: Arc<AtomicBool>,
    stream_error: Arc<Mutex<Option<String>>>,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            let value = match jsonrpc::read_json_line(&mut reader, &mut line) {
                Ok(Some(value)) => value,
                Ok(None) => {
                    set_stream_error(&stream_error, "Codex app-server stdout closed".to_string());
                    break;
                }
                Err(error) => {
                    set_stream_error(&stream_error, error.to_string());
                    break;
                }
            };
            match jsonrpc::classify(value) {
                Ok(IncomingMessage::Response { id, result, error }) => {
                    let Some(id) = id.as_u64() else {
                        continue;
                    };
                    let sender = pending.lock().ok().and_then(|mut map| map.remove(&id));
                    if let Some(sender) = sender {
                        let reply = match error {
                            Some(error) => Err(format!(
                                "Codex request failed ({}): {}",
                                error.code,
                                protocol::redact_protocol_text(&error.message)
                            )),
                            None => Ok(result.unwrap_or(Value::Null)),
                        };
                        let _ = sender.send(reply);
                    }
                }
                Ok(IncomingMessage::Request { id, method, params }) => {
                    match approvals.insert_request(generation, id.clone(), &method, params) {
                        Ok(Some(event)) => events.publish(generation, event),
                        Ok(None) => {
                            let response = jsonrpc::method_not_found(id, &method);
                            if let Err(error) = write_dispatcher_response(&stdin, &response) {
                                set_stream_error(&stream_error, error);
                                break;
                            }
                        }
                        Err(error) => {
                            let response = jsonrpc::error_response(id, -32602, error);
                            if let Err(error) = write_dispatcher_response(&stdin, &response) {
                                set_stream_error(&stream_error, error);
                                break;
                            }
                        }
                    }
                }
                Ok(IncomingMessage::Notification { method, params }) => {
                    let raw_params = params.clone();
                    match protocol::normalize_notification(&method, params) {
                        Ok(Some(event)) => {
                            if method == "serverRequest/resolved" {
                                if let Some(params) = raw_params.as_ref() {
                                    approvals.resolve_notification(generation, params);
                                }
                            }
                            if method == "turn/completed" {
                                if let (Some(thread_id), Some(turn_id)) =
                                    (event.thread_id.as_deref(), event.turn_id.as_deref())
                                {
                                    approvals.clear_turn(generation, thread_id, turn_id);
                                }
                            }
                            events.publish(generation, event);
                        }
                        Ok(None) => {
                            eprintln!(
                                "buzz-desktop: ignored unsupported Codex notification `{method}`"
                            );
                        }
                        Err(error) => {
                            set_stream_error(&stream_error, error);
                            break;
                        }
                    }
                }
                Err(error) => {
                    set_stream_error(&stream_error, error);
                    break;
                }
            }
        }
        alive.store(false, Ordering::Release);
        approvals.clear_generation(generation);
        fail_pending(&pending, &stream_error);
    });
}

fn write_dispatcher_response(stdin: &Mutex<ChildStdin>, response: &Value) -> Result<(), String> {
    stdin
        .lock()
        .map_err(|error| error.to_string())
        .and_then(|mut writer| jsonrpc::write_value(&mut *writer, response))
}

fn spawn_stderr_drain(stderr: impl Read + Send + 'static, tail: Arc<Mutex<VecDeque<u8>>>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buffer = [0u8; 4096];
        loop {
            let read = match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            if let Ok(mut bytes) = tail.lock() {
                for byte in &buffer[..read] {
                    if bytes.len() == STDERR_TAIL_BYTES {
                        bytes.pop_front();
                    }
                    bytes.push_back(*byte);
                }
            }
        }
    });
}

fn set_stream_error(target: &Mutex<Option<String>>, error: String) {
    if let Ok(mut current) = target.lock() {
        if current.is_none() {
            *current = Some(error);
        }
    }
}

fn fail_pending(pending: &PendingRequests, stream_error: &Mutex<Option<String>>) {
    let error = stream_error
        .lock()
        .ok()
        .and_then(|error| error.clone())
        .unwrap_or_else(|| "Codex app-server stream closed".to_string());
    let senders = pending
        .lock()
        .map(|mut map| map.drain().map(|(_, sender)| sender).collect::<Vec<_>>())
        .unwrap_or_default();
    for sender in senders {
        let _ = sender.send(Err(error.clone()));
    }
}

fn terminate_child(child: &mut Child) -> Result<(), String> {
    if child
        .try_wait()
        .map_err(|error| format!("failed to inspect Codex app-server: {error}"))?
        .is_some()
    {
        return Ok(());
    }

    crate::managed_agents::terminate_process(child.id())?;
    child
        .wait()
        .map(|_| ())
        .map_err(|error| format!("failed to reap Codex app-server: {error}"))
}

fn first_line(value: &str) -> String {
    let line = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(value);
    crate::managed_agents::redact_secrets_with(line, &[])
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use super::super::approvals::{
        CodeApprovalDecision, CodeApprovalResponse, CodeApprovalResponseInput,
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

    fn wait_for_event(runtime: &CodeRuntime, kind: &str) -> Result<CodeRuntimeEvent, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
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
        assert!(error.contains("requires codex-cli 0.145.<numeric patch>"));
        let status = runtime.status()?;
        assert_eq!(status.phase, CodeRuntimePhase::Failed);
        assert_eq!(status.version.as_deref(), Some("codex-cli 0.146.0"));
        assert!(status.pid.is_none());
        assert!(!spawn_marker.exists());
        Ok(())
    }

    #[test]
    fn event_backlog_is_bounded_and_reports_a_replay_gap() -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        for index in 0..=MAX_NOTIFICATION_BACKLOG {
            bridge.publish(
                7,
                CodeWorkspaceEventDraft {
                    thread_id: Some("thread-1".to_string()),
                    turn_id: None,
                    item_id: None,
                    kind: "warning".to_string(),
                    payload: json!({ "index": index }),
                },
            );
        }

        let snapshot = bridge.snapshot(Some(7), Some(0))?;
        assert_eq!(snapshot.events.len(), MAX_NOTIFICATION_BACKLOG);
        assert_eq!(snapshot.events.first().map(|event| event.sequence), Some(2));
        assert_eq!(snapshot.latest_sequence, 513);
        assert!(snapshot.truncated);
        Ok(())
    }

    #[test]
    fn failed_approval_write_restores_pending_request_for_retry() -> Result<(), String> {
        let approvals = PendingApprovalStore::default();
        approvals.reset(1);
        approvals.insert_request(
            1,
            json!("approval-1"),
            "item/fileChange/requestApproval",
            Some(json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1"
            })),
        )?;
        let input = CodeApprovalResponseInput {
            runtime_generation: 1,
            request_id: CodeRequestId::String("approval-1".to_string()),
            scope: binding_scope(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            response: CodeApprovalResponse::Decision {
                decision: CodeApprovalDecision::Accept,
            },
        };

        let error = respond_to_pending_approval(&approvals, &input, |_, _| {
            Err("simulated app-server write failure".to_string())
        })
        .expect_err("first response must fail");
        assert!(error.contains("simulated app-server write failure"));
        assert_eq!(approvals.len(), 1);

        respond_to_pending_approval(&approvals, &input, |request_id, result| {
            assert_eq!(request_id, json!("approval-1"));
            assert_eq!(result, json!({ "decision": "accept" }));
            Ok(())
        })?;
        assert_eq!(approvals.len(), 0);
        assert!(respond_to_pending_approval(&approvals, &input, |_, _| Ok(())).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn bridges_delta_approval_interrupt_and_reconnect_contract() -> Result<(), String> {
        let (_directory, executable) = fake_codex(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"thread/start"'*)
      case "$line" in
        *'"approvalPolicy":"on-request"'*'"sandbox":"workspace-write"'*) ;;
        *) printf '%s\n' "{\"id\":$request_id,\"error\":{\"code\":-32602,\"message\":\"unsafe thread defaults\"}}"; continue ;;
      esac
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"thread\":{\"id\":\"thread-1\"},\"instructionSources\":[]}}"
      printf '%s\n' '{"method":"thread/started","params":{"thread":{"id":"thread-1"}}}'
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"thread\":{\"id\":\"thread-1\",\"turns\":[{\"id\":\"past-turn\",\"status\":\"completed\",\"items\":[{\"type\":\"agentMessage\",\"text\":\"restored\"}],\"error\":null}]},\"instructionSources\":[]}}"
      ;;
    *'"method":"thread/read"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"thread\":{\"id\":\"thread-1\",\"cwd\":\"/native/stored-root\"}}}"
      ;;
    *'"method":"turn/start"'*)
      case "$line" in
        *'"approvalPolicy":"on-request"'*'"networkAccess":false'*'"type":"workspaceWrite"'*) ;;
        *) printf '%s\n' "{\"id\":$request_id,\"error\":{\"code\":-32602,\"message\":\"unsafe turn defaults\"}}"; continue ;;
      esac
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"turn\":{\"id\":\"turn-1\",\"status\":\"inProgress\"}}}"
      printf '%s\n' '{"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}}'
      printf '%s\n' '{"method":"item/agentMessage/delta","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"item-1","delta":"working"}}'
      printf '%s\n' '{"id":"approval-1","method":"item/commandExecution/requestApproval","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"item-2","startedAtMs":1,"command":"cargo test","cwd":"/tmp"}}'
      ;;
    *'"method":"turn/steer"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"turnId\":\"turn-1\"}}"
      ;;
    *'"id":"approval-1"'*)
      printf '%s\n' '{"method":"serverRequest/resolved","params":{"threadId":"thread-1","requestId":"approval-1"}}'
      ;;
    *'"method":"turn/interrupt"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{}}"
      printf '%s\n' '{"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"interrupted"}}}'
      ;;
    *)
      printf '%s\n' "{\"id\":$request_id,\"error\":{\"code\":-32601,\"message\":\"unexpected method\"}}"
      ;;
  esac
done
"#,
        )?;
        let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
        let uncanonical_workspace_root = format!("{}/.", workspace.path().to_string_lossy());
        let workspace_root = workspace
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let request_log_executable = executable.clone();
        let runtime = CodeRuntime::with_executable(executable);
        let scope = binding_scope();

        let first = runtime.start(noop_emitter())?;
        let opened = runtime
            .thread_start_at(
                CodeThreadStartInput {
                    scope: scope.clone(),
                    preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
                    model: None,
                },
                &uncanonical_workspace_root,
            )
            .map_err(CodeThreadStartRpcError::into_message)?;
        assert_eq!(opened.thread.id, "thread-1");
        let resumed = runtime.thread_resume_at(
            CodeThreadResumeInput {
                scope: scope.clone(),
                thread_id: "thread-1".to_string(),
                model: None,
            },
            &uncanonical_workspace_root,
        )?;
        assert_eq!(resumed.thread.id, "thread-1");
        assert_eq!(resumed.thread.turns.len(), 1);
        assert_eq!(resumed.thread.turns[0].id, "past-turn");
        let read = runtime.thread_read("thread-1")?;
        assert_eq!(read.id, "thread-1");
        assert_eq!(read.cwd.as_deref(), Some("/native/stored-root"));

        let turn = runtime.turn_start_at(
            CodeTurnStartInput {
                scope: scope.clone(),
                thread_id: "thread-1".to_string(),
                prompt: "Run the tests".to_string(),
                model: None,
                effort: None,
            },
            &uncanonical_workspace_root,
        )?;
        assert_eq!(turn.id, "turn-1");
        let steered = runtime.turn_steer(CodeTurnSteerInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
            expected_turn_id: "turn-1".to_string(),
            prompt: "Start with unit tests".to_string(),
        })?;
        assert_eq!(steered.id, "turn-1");

        let delta = wait_for_event(&runtime, "item/agentMessage/delta")?;
        assert_eq!(delta.runtime_generation, first.generation);
        assert_eq!(delta.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(delta.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(delta.item_id.as_deref(), Some("item-1"));
        let approval = wait_for_event(&runtime, "item/commandExecution/requestApproval")?;
        assert!(approval.sequence > delta.sequence);

        runtime.approval_respond(CodeApprovalResponseInput {
            runtime_generation: first.generation,
            request_id: CodeRequestId::String("approval-1".to_string()),
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            response: CodeApprovalResponse::Decision {
                decision: CodeApprovalDecision::Accept,
            },
        })?;
        let resolved = wait_for_event(&runtime, "serverRequest/resolved")?;
        assert!(resolved.sequence > approval.sequence);

        runtime.turn_interrupt(CodeTurnInterruptInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        })?;
        let completed = wait_for_event(&runtime, "turn/completed")?;
        assert!(completed.sequence > resolved.sequence);
        let first_snapshot = runtime.events(Some(first.generation), Some(delta.sequence))?;
        assert!(first_snapshot
            .events
            .iter()
            .all(|event| event.sequence > delta.sequence));
        assert!(first_snapshot
            .events
            .windows(2)
            .all(|events| events[0].sequence < events[1].sequence));

        runtime.stop()?;
        let second = runtime.start(noop_emitter())?;
        assert!(second.generation > first.generation);
        let reconnect = runtime.events(Some(first.generation), Some(completed.sequence))?;
        assert_eq!(reconnect.runtime_generation, second.generation);
        assert!(reconnect.truncated);
        assert!(reconnect.events.is_empty());
        let resumed_after_reconnect = runtime.thread_resume_at(
            CodeThreadResumeInput {
                scope,
                thread_id: "thread-1".to_string(),
                model: None,
            },
            &uncanonical_workspace_root,
        )?;
        assert_eq!(resumed_after_reconnect.thread.id, "thread-1");
        runtime.stop()?;

        let requests = recorded_requests(&request_log_executable)?;
        for request in &requests {
            assert!(request["params"].get("runtimeWorkspaceRoots").is_none());
            assert!(request["params"].get("scope").is_none());
            assert!(request["params"].get("communityId").is_none());
            assert!(request["params"].get("projectDtag").is_none());
            assert!(request["params"].get("repositoryIdentity").is_none());
            assert!(request["params"].get("descriptor").is_none());
        }
        let thread_starts = requests_for_method(&requests, "thread/start");
        let thread_resumes = requests_for_method(&requests, "thread/resume");
        assert_eq!(thread_starts.len(), 1);
        assert_eq!(thread_resumes.len(), 2);
        for request in thread_starts.into_iter().chain(thread_resumes) {
            assert_eq!(request["params"]["cwd"], workspace_root);
        }
        let thread_reads = requests_for_method(&requests, "thread/read");
        assert_eq!(thread_reads.len(), 1);
        assert_eq!(thread_reads[0]["params"]["threadId"], "thread-1");
        assert_eq!(thread_reads[0]["params"]["includeTurns"], false);
        let turn_requests = requests_for_method(&requests, "turn/start");
        assert_eq!(turn_requests.len(), 1);
        assert_eq!(turn_requests[0]["params"]["cwd"], workspace_root);
        assert_eq!(
            turn_requests[0]["params"]["sandboxPolicy"]["writableRoots"],
            json!([workspace_root])
        );
        Ok(())
    }
}
