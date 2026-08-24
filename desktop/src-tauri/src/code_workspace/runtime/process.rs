use super::*;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitializeResult {
    pub(super) user_agent: String,
    pub(super) codex_home: String,
    pub(super) platform_family: String,
    pub(super) platform_os: String,
}

type RpcReply = Result<Value, String>;
type PendingRequests = Arc<Mutex<HashMap<u64, mpsc::SyncSender<RpcReply>>>>;

pub(super) struct PendingRuntimeRequest {
    id: u64,
    method: String,
    receiver: mpsc::Receiver<RpcReply>,
    pending: PendingRequests,
    stream_error: Arc<Mutex<Option<String>>>,
}

pub(super) fn collect_model_catalog_from_process(
    runtime_generation: u64,
    process: &RuntimeProcess,
) -> Result<CodeModelCatalogSnapshot, String> {
    collect_model_catalog(runtime_generation, |params| {
        process.request("model/list", params, REQUEST_TIMEOUT)
    })
}

impl PendingRuntimeRequest {
    pub(super) fn wait(self, timeout: Duration) -> Result<Value, CodeRpcDeliveryError> {
        match self.receiver.recv_timeout(timeout) {
            Ok(reply) => reply.map_err(CodeRpcDeliveryError::Uncertain),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&self.id);
                }
                Err(CodeRpcDeliveryError::Uncertain(format!(
                    "Codex `{}` request timed out",
                    self.method
                )))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(CodeRpcDeliveryError::Uncertain(
                self.stream_error
                    .lock()
                    .ok()
                    .and_then(|error| error.clone())
                    .unwrap_or_else(|| "Codex app-server response channel closed".to_string()),
            )),
        }
    }
}

pub(super) struct RuntimeProcess {
    pub(super) child: Child,
    #[cfg(windows)]
    job: Option<crate::managed_agents::JobHandle>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: PendingRequests,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    alive: Arc<AtomicBool>,
    stream_error: Arc<Mutex<Option<String>>>,
    next_request_id: AtomicU64,
    stopped: bool,
    #[cfg(test)]
    pub(super) stop_failures_remaining: usize,
}

impl RuntimeProcess {
    pub(super) fn spawn(
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
        #[cfg(windows)]
        let job = match crate::managed_agents::create_kill_on_close_job_for_child(child.id()) {
            Some(job) => job,
            None => {
                let pid = child.id();
                let tree_cleanup_error = crate::managed_agents::taskkill_tree(pid).err();
                if tree_cleanup_error.is_some() {
                    let _ = child.kill();
                }
                let reap_error = child.wait().err();
                let mut error =
                    format!("failed to secure Codex app-server process tree for pid {pid}");
                if let Some(cleanup_error) = tree_cleanup_error {
                    error.push_str(&format!("; tree cleanup also failed: {cleanup_error}"));
                }
                if let Some(reap_error) = reap_error {
                    error.push_str(&format!("; leader reap also failed: {reap_error}"));
                }
                return Err(error);
            }
        };
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
            #[cfg(windows)]
            job: Some(job),
            stdin,
            pending,
            stderr,
            alive,
            stream_error,
            next_request_id: AtomicU64::new(1),
            stopped: false,
            #[cfg(test)]
            stop_failures_remaining: 0,
        })
    }

    pub(super) fn initialize(&mut self) -> Result<InitializeResult, String> {
        let result = self.request("initialize", initialize_params(), INITIALIZE_TIMEOUT)?;
        let initialized = serde_json::from_value(protocol::redact_protocol_value(result))
            .map_err(|error| format!("invalid Codex initialize response: {error}"))?;
        let notification = jsonrpc::notification("initialized");
        let mut writer = self.stdin.lock().map_err(|error| error.to_string())?;
        jsonrpc::write_value(&mut *writer, &notification)?;
        Ok(initialized)
    }

    pub(super) fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.request_with_delivery(method, params, timeout)
            .map_err(CodeRpcDeliveryError::into_message)
    }

    pub(super) fn request_with_delivery(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CodeRpcDeliveryError> {
        self.begin_request_with_delivery(method, params)?
            .wait(timeout)
    }

    pub(super) fn begin_request_with_delivery(
        &self,
        method: &str,
        params: Value,
    ) -> Result<PendingRuntimeRequest, CodeRpcDeliveryError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(CodeRpcDeliveryError::NotSent(
                self.stream_error()
                    .unwrap_or_else(|| "Codex app-server stream is closed".to_string()),
            ));
        }
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let message = jsonrpc::request(id, method, params);
        jsonrpc::validate_value_size(&message).map_err(CodeRpcDeliveryError::NotSent)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?
            .insert(id, sender);

        let write_result = match self.stdin.lock() {
            Ok(mut writer) => jsonrpc::write_value(&mut *writer, &message),
            Err(error) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                return Err(CodeRpcDeliveryError::NotSent(error.to_string()));
            }
        };
        if let Err(error) = write_result {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            return Err(CodeRpcDeliveryError::Uncertain(error));
        }
        Ok(PendingRuntimeRequest {
            id,
            method: method.to_string(),
            receiver,
            pending: Arc::clone(&self.pending),
            stream_error: Arc::clone(&self.stream_error),
        })
    }

    pub(super) fn respond(&self, request_id: Value, result: Value) -> Result<(), String> {
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

    pub(super) fn health_error(&mut self) -> Option<String> {
        if !self.alive.load(Ordering::Acquire) {
            return Some(
                self.stream_error()
                    .unwrap_or_else(|| "Codex app-server stream closed".to_string()),
            );
        }
        match observe_child_exit(&mut self.child) {
            Ok(Some(status)) => Some(format!("Codex app-server exited with {status}")),
            Ok(None) => None,
            Err(error) => Some(error),
        }
    }

    fn stream_error(&self) -> Option<String> {
        self.stream_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
    }

    pub(super) fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .map(|mut bytes| String::from_utf8_lossy(bytes.make_contiguous()).into_owned())
            .unwrap_or_default()
    }

    pub(super) fn stop(&mut self) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        #[cfg(test)]
        if self.stop_failures_remaining > 0 {
            self.stop_failures_remaining = self.stop_failures_remaining.saturating_sub(1);
            return Err("injected Codex app-server stop failure".to_string());
        }
        self.alive.store(false, Ordering::Release);
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            drop(job);
        }
        let result = terminate_child(&mut self.child);
        if result.is_ok() {
            self.stopped = true;
        }
        result
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
                    match events.insert_approval_and_publish(
                        &approvals,
                        generation,
                        id.clone(),
                        &method,
                        params,
                    ) {
                        Ok(true) => {}
                        Ok(false) => {
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
                            if let Err(error) = events.publish_notification(
                                &approvals,
                                generation,
                                &method,
                                raw_params.as_ref(),
                                event,
                            ) {
                                set_stream_error(&stream_error, error);
                                break;
                            }
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn observe_child_exit(child: &mut Child) -> Result<Option<String>, String> {
    let pid = runtime_process_pid(child.id())?;
    rustix::process::waitid(
        rustix::process::WaitId::Pid(pid),
        rustix::process::WaitIdOptions::EXITED
            | rustix::process::WaitIdOptions::NOHANG
            | rustix::process::WaitIdOptions::NOWAIT,
    )
    .map(|status| status.map(|status| format!("{status:?}")))
    .map_err(|error| format!("failed to inspect Codex app-server: {error}"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn observe_child_exit(child: &mut Child) -> Result<Option<String>, String> {
    child
        .try_wait()
        .map(|status| status.map(|status| status.to_string()))
        .map_err(|error| format!("failed to inspect Codex app-server: {error}"))
}

#[cfg(unix)]
pub(super) fn runtime_process_pid(raw_pid: u32) -> Result<rustix::process::Pid, String> {
    let raw_pid = i32::try_from(raw_pid)
        .map_err(|_| "Codex app-server returned an invalid process ID".to_string())?;
    rustix::process::Pid::from_raw(raw_pid)
        .ok_or_else(|| "Codex app-server returned an invalid process ID".to_string())
}

#[cfg(unix)]
fn signal_runtime_process_group(
    pid: rustix::process::Pid,
    signal: rustix::process::Signal,
    action: &str,
) -> Result<bool, String> {
    match rustix::process::kill_process_group(pid, signal) {
        Ok(()) => Ok(true),
        Err(error) if error == rustix::io::Errno::SRCH => Ok(false),
        Err(error) => Err(format!(
            "failed to {action} Codex app-server process group: {error}"
        )),
    }
}

#[cfg(unix)]
fn terminate_child(child: &mut Child) -> Result<(), String> {
    let pid = runtime_process_pid(child.id())?;
    if !signal_runtime_process_group(pid, rustix::process::Signal::TERM, "terminate")? {
        if observe_child_exit(child)?.is_none() {
            child
                .kill()
                .map_err(|error| format!("failed to kill Codex app-server leader: {error}"))?;
        }
        return child
            .wait()
            .map(|_| ())
            .map_err(|error| format!("failed to reap Codex app-server: {error}"));
    }

    let deadline = Instant::now() + PROCESS_GROUP_TERM_TIMEOUT;
    loop {
        if observe_child_exit(child)?.is_some() {
            child
                .wait()
                .map_err(|error| format!("failed to reap Codex app-server: {error}"))?;
            signal_runtime_process_group(pid, rustix::process::Signal::KILL, "kill")?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            if let Err(error) =
                signal_runtime_process_group(pid, rustix::process::Signal::KILL, "kill")
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            break;
        }
        std::thread::sleep(PROCESS_GROUP_POLL_INTERVAL);
    }

    child
        .wait()
        .map(|_| ())
        .map_err(|error| format!("failed to reap Codex app-server: {error}"))
}

#[cfg(windows)]
fn terminate_child(child: &mut Child) -> Result<(), String> {
    child
        .wait()
        .map(|_| ())
        .map_err(|error| format!("failed to reap Codex app-server: {error}"))
}

#[cfg(not(any(unix, windows)))]
fn terminate_child(child: &mut Child) -> Result<(), String> {
    if child
        .try_wait()
        .map_err(|error| format!("failed to inspect Codex app-server: {error}"))?
        .is_none()
    {
        crate::managed_agents::terminate_process(child.id())?;
    }
    child
        .wait()
        .map(|_| ())
        .map_err(|error| format!("failed to reap Codex app-server: {error}"))
}

pub(super) fn first_line(value: &str) -> String {
    let line = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(value);
    protocol::redact_protocol_text(line)
}
