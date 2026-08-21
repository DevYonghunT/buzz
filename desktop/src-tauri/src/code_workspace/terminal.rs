//! Exact bound-thread PTY ownership for the SchoolX Code terminal drawer.
//!
//! User shell sessions are deliberately independent from the Codex app-server
//! runtime. Each session actor owns one native PTY and child process tree, and
//! accepts only controls carrying the complete persisted binding owner.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    Arc, Mutex, Weak,
};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::ExitStatus;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;

use super::bindings::CodeThreadBindingScope;

mod control;
mod process;

use process::{spawn_pty, SessionProcess};

const MAX_ACTIVE_SESSIONS: usize = 8;
const MAX_STDIN_BYTES: usize = 64 * 1024;
const MAX_TERMINAL_DIMENSION: u16 = 1_000;
const CONTROL_QUEUE_CAPACITY: usize = 64;
const OUTPUT_QUEUE_CAPACITY: usize = 32;
const WRITER_QUEUE_CAPACITY: usize = 32;
const OUTPUT_CHUNK_BYTES: usize = 16 * 1024;
const ACTOR_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CONTROL_REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(4);

/// Request to start a user shell at one exact bound Codex thread root.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeTerminalOpenInput {
    /// Complete community/project/repository binding scope.
    pub scope: CodeThreadBindingScope,
    /// Opaque Codex thread identifier within the exact scope.
    pub thread_id: String,
    /// Initial terminal column count.
    pub cols: u16,
    /// Initial terminal row count.
    pub rows: u16,
}

/// Native-owned terminal session returned after the PTY child is registered.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeTerminalSession {
    /// Exact scope that owns the terminal.
    pub scope: CodeThreadBindingScope,
    /// Exact bound Codex thread that owns the terminal.
    pub thread_id: String,
    /// Opaque native-issued session UUID.
    pub session_id: String,
    /// Registered terminal column count.
    pub cols: u16,
    /// Registered terminal row count.
    pub rows: u16,
}

/// Resize request for one exact native terminal owner.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeTerminalResizeInput {
    /// Complete community/project/repository binding scope.
    pub scope: CodeThreadBindingScope,
    /// Exact bound Codex thread identifier.
    pub thread_id: String,
    /// Native-issued terminal session UUID.
    pub session_id: String,
    /// New terminal column count.
    pub cols: u16,
    /// New terminal row count.
    pub rows: u16,
}

/// Raw stdin request for one exact native terminal owner.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeTerminalStdinInput {
    /// Complete community/project/repository binding scope.
    pub scope: CodeThreadBindingScope,
    /// Exact bound Codex thread identifier.
    pub thread_id: String,
    /// Native-issued terminal session UUID.
    pub session_id: String,
    /// Raw bytes written to the PTY master.
    pub data: Vec<u8>,
}

/// Termination request for one exact native terminal owner.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeTerminalTerminateInput {
    /// Complete community/project/repository binding scope.
    pub scope: CodeThreadBindingScope,
    /// Exact bound Codex thread identifier.
    pub thread_id: String,
    /// Native-issued terminal session UUID.
    pub session_id: String,
}

/// Raw PTY event payloads delivered only to the opening channel.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CodeTerminalEvent {
    /// Raw PTY output, including ANSI escape sequences and partial UTF-8.
    Output {
        /// Exact scope that owns the terminal.
        scope: CodeThreadBindingScope,
        /// Exact bound Codex thread identifier.
        #[serde(rename = "threadId")]
        thread_id: String,
        /// Native-issued terminal session UUID.
        #[serde(rename = "sessionId")]
        session_id: String,
        /// Monotonic sequence within this session.
        sequence: u64,
        /// Byte-preserving PTY output for this emitted chunk.
        data: Vec<u8>,
    },
    /// Terminal leader exit after native descendant cleanup.
    Exit {
        /// Exact scope that owned the terminal.
        scope: CodeThreadBindingScope,
        /// Exact bound Codex thread identifier.
        #[serde(rename = "threadId")]
        thread_id: String,
        /// Native-issued terminal session UUID.
        #[serde(rename = "sessionId")]
        session_id: String,
        /// Monotonic sequence within this session.
        sequence: u64,
        /// Portable process exit code.
        #[serde(rename = "exitCode")]
        exit_code: u32,
        /// Platform signal description when the process was signaled.
        signal: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SessionOwner {
    scope: CodeThreadBindingScope,
    thread_id: String,
}

impl SessionOwner {
    fn validate(&self) -> Result<(), String> {
        self.scope.validate()?;
        super::protocol::validate_id("terminal thread", &self.thread_id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagerLifecycle {
    Accepting,
    Draining,
    Shutdown,
}

struct SessionEntry {
    owner: SessionOwner,
    control_tx: SyncSender<SessionControl>,
    terminate_tx: mpsc::Sender<TerminateControl>,
    closing: bool,
}

struct ManagerInner {
    lifecycle: ManagerLifecycle,
    sessions: HashMap<String, SessionEntry>,
    opening_owners: HashSet<SessionOwner>,
}

impl Default for ManagerInner {
    fn default() -> Self {
        Self {
            lifecycle: ManagerLifecycle::Accepting,
            sessions: HashMap::new(),
            opening_owners: HashSet::new(),
        }
    }
}

/// Independent owner of all user-shell PTYs opened by SchoolX Code.
#[derive(Clone, Default)]
pub struct CodeTerminalManager {
    inner: Arc<Mutex<ManagerInner>>,
}

impl CodeTerminalManager {
    /// Construct an empty terminal manager accepting new sessions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn and register one default OS shell at a native-revalidated root.
    ///
    /// `execution_root` must come from the persisted binding; callers must not
    /// pass a webview-provided path through this boundary.
    pub(crate) fn open(
        &self,
        input: CodeTerminalOpenInput,
        execution_root: &Path,
        on_event: Channel<CodeTerminalEvent>,
    ) -> Result<CodeTerminalSession, String> {
        let owner = SessionOwner {
            scope: input.scope.clone(),
            thread_id: input.thread_id.clone(),
        };
        owner.validate()?;
        validate_dimensions(input.cols, input.rows)?;
        if !execution_root.is_absolute() || !execution_root.is_dir() {
            return Err(
                "SchoolX Code terminal execution root is no longer an absolute directory"
                    .to_string(),
            );
        }

        // Serialize opens per exact owner. A reloaded webview can replace and
        // reap its stale session, but two concurrent opens can never replace
        // each other's newly-created PTYs.
        let mut reservation = OpenReservation::acquire(&self.inner, owner.clone())?;
        self.replace_existing_owner(&owner)?;

        // Holding the lifecycle lock through spawn and registration closes the
        // open-vs-shutdown race: a drain can never snapshot before this child
        // becomes visible in the session map.
        let mut inner = lock_manager(&self.inner)?;
        if inner.lifecycle != ManagerLifecycle::Accepting {
            return Err("SchoolX Code terminal manager is not accepting sessions".to_string());
        }
        if inner.sessions.len() >= MAX_ACTIVE_SESSIONS {
            return Err(format!(
                "SchoolX Code terminal reached the {MAX_ACTIVE_SESSIONS}-session limit"
            ));
        }
        if inner.sessions.values().any(|entry| entry.owner == owner) {
            return Err("The stale SchoolX Code terminal session did not close".to_string());
        }

        // Check immediately before portable-pty resolves its cwd. This narrows
        // its documented missing-cwd fallback to the unavoidable pathname
        // replacement interval after the native binding revalidation.
        if !execution_root.is_dir() {
            return Err(
                "SchoolX Code terminal execution root disappeared before spawn".to_string(),
            );
        }

        let session_id = uuid::Uuid::new_v4().hyphenated().to_string();
        let (process, reader, writer) = spawn_pty(execution_root, input.cols, input.rows)?;
        let discard_output = process.output_discard_flag();
        let (output_tx, output_rx) = mpsc::sync_channel(OUTPUT_QUEUE_CAPACITY);
        spawn_reader(session_id.clone(), reader, output_tx, discard_output)?;
        let (writer_tx, writer_rx) = mpsc::sync_channel(WRITER_QUEUE_CAPACITY);
        spawn_writer(session_id.clone(), writer, writer_rx)?;

        let (control_tx, control_rx) = mpsc::sync_channel(CONTROL_QUEUE_CAPACITY);
        let (terminate_tx, terminate_rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::sync_channel(0);
        let actor_owner = owner.clone();
        let actor_session_id = session_id.clone();
        let manager = Arc::downgrade(&self.inner);
        thread::Builder::new()
            .name(format!("code-terminal-{session_id}"))
            .spawn(move || {
                if start_rx.recv().is_ok() {
                    run_session_actor(SessionActor {
                        manager,
                        actor_owner,
                        actor_session_id,
                        process,
                        control_rx,
                        terminate_rx,
                        output_rx,
                        writer_tx,
                        on_event,
                    });
                }
            })
            .map_err(|error| format!("failed to start SchoolX Code terminal actor: {error}"))?;

        inner.sessions.insert(
            session_id.clone(),
            SessionEntry {
                owner: owner.clone(),
                control_tx,
                terminate_tx,
                closing: false,
            },
        );
        if start_tx.send(()).is_err() {
            inner.sessions.remove(&session_id);
            return Err("SchoolX Code terminal actor stopped before registration".to_string());
        }
        reservation.commit(&mut inner);

        Ok(CodeTerminalSession {
            scope: owner.scope,
            thread_id: owner.thread_id,
            session_id,
            cols: input.cols,
            rows: input.rows,
        })
    }

    /// Terminate and fully drain the session owned by one exact scope/thread.
    ///
    /// No session id is accepted at this lifecycle boundary. An absent owner is
    /// already drained and succeeds idempotently; an opening or duplicated
    /// owner fails closed.
    pub(crate) fn terminate_owner(
        &self,
        scope: &CodeThreadBindingScope,
        thread_id: &str,
    ) -> Result<(), String> {
        let owner = SessionOwner {
            scope: scope.clone(),
            thread_id: thread_id.to_string(),
        };
        owner.validate()?;
        let reply_rx = {
            let mut inner = lock_manager(&self.inner)?;
            if inner.opening_owners.contains(&owner) {
                return Err(
                    "SchoolX Code terminal owner is still opening and cannot be drained"
                        .to_string(),
                );
            }
            let matching = inner
                .sessions
                .iter()
                .filter_map(|(session_id, entry)| {
                    (entry.owner == owner).then_some(session_id.clone())
                })
                .collect::<Vec<_>>();
            if matching.len() > 1 {
                return Err(
                    "SchoolX Code terminal manager contained duplicate exact owners".to_string(),
                );
            }
            let Some(session_id) = matching.into_iter().next() else {
                return Ok(());
            };
            let entry = inner
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| "SchoolX Code terminal owner disappeared".to_string())?;
            entry.closing = true;
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            entry
                .terminate_tx
                .send(TerminateControl {
                    reply: Some(reply_tx),
                })
                .map_err(|_| "SchoolX Code terminal actor is unavailable".to_string())?;
            reply_rx
        };
        wait_for_reply(reply_rx, "owner terminate")
    }

    /// Prove that no open or opening PTY belongs to one exact thread owner.
    /// This is deliberately non-mutating: removal admission must refuse a
    /// live terminal instead of silently terminating user work.
    #[allow(dead_code)]
    pub(crate) fn ensure_owner_absent(
        &self,
        scope: &CodeThreadBindingScope,
        thread_id: &str,
    ) -> Result<(), String> {
        let owner = SessionOwner {
            scope: scope.clone(),
            thread_id: thread_id.to_string(),
        };
        owner.validate()?;
        let inner = lock_manager(&self.inner)?;
        if inner.opening_owners.contains(&owner)
            || inner.sessions.values().any(|entry| entry.owner == owner)
        {
            return Err("SchoolX Code worktree removal requires no open terminal".to_string());
        }
        Ok(())
    }

    /// Terminate every current session while allowing a later window reopen.
    pub fn terminate_all(&self) -> Result<(), String> {
        self.drain(false)
    }

    /// Permanently reject new sessions and drain every current PTY.
    pub fn shutdown(&self) -> Result<(), String> {
        self.drain(true)
    }

    fn drain(&self, permanent: bool) -> Result<(), String> {
        {
            let mut inner = lock_manager(&self.inner)?;
            if permanent {
                inner.lifecycle = ManagerLifecycle::Shutdown;
            } else if inner.lifecycle == ManagerLifecycle::Accepting {
                inner.lifecycle = ManagerLifecycle::Draining;
            }

            let mut disconnected = Vec::new();
            for (session_id, entry) in &mut inner.sessions {
                if entry.closing {
                    continue;
                }
                entry.closing = true;
                if entry
                    .terminate_tx
                    .send(TerminateControl { reply: None })
                    .is_err()
                {
                    disconnected.push(session_id.clone());
                }
            }
            for session_id in disconnected {
                inner.sessions.remove(&session_id);
            }
        }

        let deadline = Instant::now() + DRAIN_TIMEOUT;
        loop {
            let is_empty = {
                let inner = lock_manager(&self.inner)?;
                inner.sessions.is_empty() && inner.opening_owners.is_empty()
            };
            if is_empty {
                break;
            }
            if Instant::now() >= deadline {
                return Err("SchoolX Code terminal shutdown timed out".to_string());
            }
            thread::sleep(ACTOR_POLL_INTERVAL);
        }

        if !permanent {
            let mut inner = lock_manager(&self.inner)?;
            if inner.lifecycle == ManagerLifecycle::Draining {
                inner.lifecycle = ManagerLifecycle::Accepting;
            }
        }
        Ok(())
    }

    fn replace_existing_owner(&self, owner: &SessionOwner) -> Result<(), String> {
        let existing_session = {
            let mut inner = lock_manager(&self.inner)?;
            let existing = inner.sessions.iter().find_map(|(session_id, entry)| {
                (entry.owner == *owner).then(|| session_id.clone())
            });
            let Some(session_id) = existing else {
                return Ok(());
            };
            let entry = inner
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| "SchoolX Code terminal session disappeared".to_string())?;
            if !entry.closing {
                entry.closing = true;
                if entry
                    .terminate_tx
                    .send(TerminateControl { reply: None })
                    .is_err()
                {
                    inner.sessions.remove(&session_id);
                    return Ok(());
                }
            }
            session_id
        };

        let deadline = Instant::now() + DRAIN_TIMEOUT;
        loop {
            let exists = lock_manager(&self.inner)?
                .sessions
                .get(&existing_session)
                .is_some_and(|entry| entry.owner == *owner);
            if !exists {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("Stale SchoolX Code terminal replacement timed out".to_string());
            }
            thread::sleep(ACTOR_POLL_INTERVAL);
        }
    }
}

struct OpenReservation {
    manager: Weak<Mutex<ManagerInner>>,
    owner: SessionOwner,
    active: bool,
}

impl OpenReservation {
    fn acquire(manager: &Arc<Mutex<ManagerInner>>, owner: SessionOwner) -> Result<Self, String> {
        let mut inner = lock_manager(manager)?;
        if inner.lifecycle != ManagerLifecycle::Accepting {
            return Err("SchoolX Code terminal manager is not accepting sessions".to_string());
        }
        if !inner.opening_owners.insert(owner.clone()) {
            return Err("The requested SchoolX Code terminal is already opening".to_string());
        }
        Ok(Self {
            manager: Arc::downgrade(manager),
            owner,
            active: true,
        })
    }

    fn commit(&mut self, inner: &mut ManagerInner) {
        inner.opening_owners.remove(&self.owner);
        self.active = false;
    }
}

impl Drop for OpenReservation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        if let Ok(mut inner) = manager.lock() {
            inner.opening_owners.remove(&self.owner);
        };
    }
}

type ControlReply = SyncSender<Result<(), String>>;

enum SessionControl {
    Resize {
        cols: u16,
        rows: u16,
        reply: ControlReply,
    },
    Stdin {
        data: Vec<u8>,
        reply: ControlReply,
    },
}

struct TerminateControl {
    reply: Option<ControlReply>,
}

enum ReaderEvent {
    Output(Vec<u8>),
    Finished,
    Failed,
}

struct WriterRequest {
    data: Vec<u8>,
    reply: ControlReply,
}

struct SessionActor {
    manager: Weak<Mutex<ManagerInner>>,
    actor_owner: SessionOwner,
    actor_session_id: String,
    process: SessionProcess,
    control_rx: Receiver<SessionControl>,
    terminate_rx: Receiver<TerminateControl>,
    output_rx: Receiver<ReaderEvent>,
    writer_tx: SyncSender<WriterRequest>,
    on_event: Channel<CodeTerminalEvent>,
}

fn run_session_actor(actor: SessionActor) {
    let SessionActor {
        manager,
        actor_owner: owner,
        actor_session_id: session_id,
        mut process,
        control_rx,
        terminate_rx,
        output_rx,
        writer_tx,
        on_event,
    } = actor;
    let mut registration = SessionRegistration::new(manager, owner.clone(), session_id.clone());
    let mut sequence = 0_u64;
    let mut reader_finished = false;
    let mut channel_alive = true;
    let mut terminate_replies = Vec::new();

    let exit = loop {
        match terminate_rx.try_recv() {
            Ok(control) => {
                if let Some(reply) = control.reply {
                    terminate_replies.push(reply);
                }
                break process.terminate();
            }
            Err(TryRecvError::Disconnected) => break process.terminate(),
            Err(TryRecvError::Empty) => {}
        }

        match control_rx.try_recv() {
            Ok(SessionControl::Resize { cols, rows, reply }) => {
                let result = process.resize(cols, rows);
                let _ = reply.send(result);
            }
            Ok(SessionControl::Stdin { data, reply }) => {
                let request = WriterRequest { data, reply };
                match writer_tx.try_send(request) {
                    Ok(()) => {}
                    Err(TrySendError::Full(request)) => {
                        let _ = request
                            .reply
                            .send(Err("SchoolX Code terminal stdin queue is full".to_string()));
                    }
                    Err(TrySendError::Disconnected(request)) => {
                        let _ = request.reply.send(Err(
                            "SchoolX Code terminal stdin writer is unavailable".to_string(),
                        ));
                    }
                }
            }
            Err(TryRecvError::Disconnected) => break process.terminate(),
            Err(TryRecvError::Empty) => {}
        }

        if !reader_finished {
            match output_rx.recv_timeout(ACTOR_POLL_INTERVAL) {
                Ok(ReaderEvent::Output(data)) => {
                    sequence = sequence.saturating_add(1);
                    if on_event
                        .send(CodeTerminalEvent::Output {
                            scope: owner.scope.clone(),
                            thread_id: owner.thread_id.clone(),
                            session_id: session_id.clone(),
                            sequence,
                            data,
                        })
                        .is_err()
                    {
                        channel_alive = false;
                        break process.terminate();
                    }
                }
                Ok(ReaderEvent::Finished) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    reader_finished = true;
                }
                Ok(ReaderEvent::Failed) => break process.terminate(),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        } else {
            thread::sleep(ACTOR_POLL_INTERVAL);
        }

        match process.poll_exit() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(error) => break Err(error),
        }
    };

    // Stop accepting stdin before process teardown is published. The writer
    // worker owns the only write clone and will unblock once the slave tree is
    // gone.
    drop(writer_tx);
    // If the primary teardown reported an error, SessionProcess::drop performs
    // its final kill/reap fallback before the native owner leaves the map.
    let discard_output = process.output_discard_flag();
    drop(process);
    let (mut status, mut termination_result) = match exit {
        Ok(status) => (status, Ok(())),
        Err(error) => (
            ExitStatus::with_signal("terminal teardown failed"),
            Err(error),
        ),
    };

    // A natural leader exit must preserve bytes that were still in the native
    // PTY pipe when it was observed. Continue consuming the bounded reader
    // queue until the reader reports EOF; destructive teardown flips the
    // shared discard flag and reaches disconnect without backpressure.
    let reader_result = if discard_output.load(Ordering::Acquire) {
        // Destructive teardown intentionally abandons output. In particular,
        // a Windows ClosePseudoConsole helper may remain blocked on a cursor
        // query even after the Job dies; actor completion must not wait for it.
        Ok(())
    } else {
        let reader_deadline = Instant::now() + DRAIN_TIMEOUT;
        loop {
            if discard_output.load(Ordering::Acquire) {
                break Ok(());
            }
            match output_rx.recv_timeout(ACTOR_POLL_INTERVAL) {
                Ok(ReaderEvent::Output(data)) if channel_alive => {
                    sequence = sequence.saturating_add(1);
                    if on_event
                        .send(CodeTerminalEvent::Output {
                            scope: owner.scope.clone(),
                            thread_id: owner.thread_id.clone(),
                            session_id: session_id.clone(),
                            sequence,
                            data,
                        })
                        .is_err()
                    {
                        channel_alive = false;
                        discard_output.store(true, Ordering::Release);
                    }
                }
                Ok(ReaderEvent::Output(_)) => {}
                Ok(ReaderEvent::Finished | ReaderEvent::Failed)
                | Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= reader_deadline => {
                    discard_output.store(true, Ordering::Release);
                    break Err("SchoolX Code terminal output drain timed out".to_string());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    };
    if let Err(reader_error) = reader_result {
        status = ExitStatus::with_signal("terminal teardown failed");
        termination_result = Err(match termination_result {
            Ok(()) => reader_error,
            Err(error) => format!("{error}; {reader_error}"),
        });
    }
    if channel_alive {
        sequence = sequence.saturating_add(1);
        let _ = on_event.send(CodeTerminalEvent::Exit {
            scope: owner.scope,
            thread_id: owner.thread_id,
            session_id,
            sequence,
            exit_code: status.exit_code(),
            signal: status.signal().map(ToOwned::to_owned),
        });
    }

    // Every descendant is reaped and the PTY output queue is drained before
    // the owner disappears. `terminate_owner` can therefore treat absence as
    // an idempotent completed drain rather than a best-effort signal.
    registration.unregister();

    // Termination sends while holding the manager lock. Unregistering forms
    // the completion barrier: every sender that found this exact session has
    // queued its waiter, while later callers observe an already-drained owner.
    while let Ok(control) = terminate_rx.try_recv() {
        if let Some(reply) = control.reply {
            terminate_replies.push(reply);
        }
    }
    for reply in terminate_replies {
        let _ = reply.send(termination_result.clone());
    }
}

fn spawn_reader(
    session_id: String,
    mut reader: Box<dyn Read + Send>,
    output_tx: SyncSender<ReaderEvent>,
    discard_output: Arc<AtomicBool>,
) -> Result<(), String> {
    thread::Builder::new()
        .name(format!("code-terminal-reader-{session_id}"))
        .spawn(move || {
            let mut buffer = vec![0_u8; OUTPUT_CHUNK_BYTES];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ =
                            queue_reader_event(&output_tx, &discard_output, ReaderEvent::Finished);
                        break;
                    }
                    Ok(read) => {
                        if !queue_reader_event(
                            &output_tx,
                            &discard_output,
                            ReaderEvent::Output(buffer[..read].to_vec()),
                        ) {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        let _ =
                            queue_reader_event(&output_tx, &discard_output, ReaderEvent::Failed);
                        break;
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("failed to start SchoolX Code terminal reader: {error}"))
}

fn queue_reader_event(
    output_tx: &SyncSender<ReaderEvent>,
    discard_output: &AtomicBool,
    mut event: ReaderEvent,
) -> bool {
    loop {
        if discard_output.load(Ordering::Acquire) {
            return true;
        }
        match output_tx.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned)) => {
                event = returned;
                thread::sleep(Duration::from_millis(1));
            }
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }
}

fn spawn_writer(
    session_id: String,
    mut writer: Box<dyn Write + Send>,
    writer_rx: Receiver<WriterRequest>,
) -> Result<(), String> {
    thread::Builder::new()
        .name(format!("code-terminal-writer-{session_id}"))
        .spawn(move || {
            while let Ok(request) = writer_rx.recv() {
                let result = writer
                    .write_all(&request.data)
                    .and_then(|_| writer.flush())
                    .map_err(|error| format!("failed to write terminal stdin: {error}"));
                let failed = result.is_err();
                let _ = request.reply.send(result);
                if failed {
                    break;
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("failed to start SchoolX Code terminal writer: {error}"))
}

fn send_control<F>(
    manager: &Arc<Mutex<ManagerInner>>,
    owner: &SessionOwner,
    session_id: &str,
    build: F,
) -> Result<Receiver<Result<(), String>>, String>
where
    F: FnOnce(ControlReply) -> SessionControl,
{
    let inner = lock_manager(manager)?;
    let entry = inner
        .sessions
        .get(session_id)
        .ok_or_else(|| "SchoolX Code terminal session was not found".to_string())?;
    ensure_exact_owner(&entry.owner, owner)?;
    if entry.closing {
        return Err("SchoolX Code terminal session is closing".to_string());
    }
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    entry
        .control_tx
        .try_send(build(reply_tx))
        .map_err(|error| match error {
            TrySendError::Full(_) => "SchoolX Code terminal control queue is full".to_string(),
            TrySendError::Disconnected(_) => {
                "SchoolX Code terminal actor is unavailable".to_string()
            }
        })?;
    Ok(reply_rx)
}

fn wait_for_reply(reply: Receiver<Result<(), String>>, action: &str) -> Result<(), String> {
    reply
        .recv_timeout(CONTROL_REPLY_TIMEOUT)
        .map_err(|_| format!("SchoolX Code terminal {action} timed out"))?
}

fn owner_for_control(
    scope: &CodeThreadBindingScope,
    thread_id: &str,
    session_id: &str,
) -> Result<SessionOwner, String> {
    let owner = SessionOwner {
        scope: scope.clone(),
        thread_id: thread_id.to_string(),
    };
    owner.validate()?;
    validate_session_id(session_id)?;
    Ok(owner)
}

fn ensure_exact_owner(actual: &SessionOwner, requested: &SessionOwner) -> Result<(), String> {
    if actual != requested {
        return Err(
            "Terminal session is not owned by the requested SchoolX Code binding".to_string(),
        );
    }
    Ok(())
}

fn validate_session_id(session_id: &str) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(session_id)
        .map_err(|error| format!("SchoolX Code terminal session id is not a UUID: {error}"))?;
    if parsed.hyphenated().to_string() != session_id {
        return Err(
            "SchoolX Code terminal session id must be a canonical lowercase UUID".to_string(),
        );
    }
    Ok(())
}

fn validate_dimensions(cols: u16, rows: u16) -> Result<(), String> {
    if cols == 0 || rows == 0 || cols > MAX_TERMINAL_DIMENSION || rows > MAX_TERMINAL_DIMENSION {
        return Err(format!(
            "SchoolX Code terminal dimensions must be between 1 and {MAX_TERMINAL_DIMENSION}"
        ));
    }
    Ok(())
}

fn lock_manager(
    manager: &Arc<Mutex<ManagerInner>>,
) -> Result<std::sync::MutexGuard<'_, ManagerInner>, String> {
    manager
        .lock()
        .map_err(|_| "SchoolX Code terminal manager lock is unavailable".to_string())
}

struct SessionRegistration {
    manager: Weak<Mutex<ManagerInner>>,
    owner: SessionOwner,
    session_id: String,
    active: bool,
}

impl SessionRegistration {
    fn new(manager: Weak<Mutex<ManagerInner>>, owner: SessionOwner, session_id: String) -> Self {
        Self {
            manager,
            owner,
            session_id,
            active: true,
        }
    }

    fn unregister(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let Ok(mut inner) = manager.lock() else {
            return;
        };
        if inner
            .sessions
            .get(&self.session_id)
            .is_some_and(|entry| entry.owner == self.owner)
        {
            inner.sessions.remove(&self.session_id);
        }
    }
}

impl Drop for SessionRegistration {
    fn drop(&mut self) {
        self.unregister();
    }
}

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod tests;
