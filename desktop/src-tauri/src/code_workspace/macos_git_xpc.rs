//! Signed macOS XPC launch authority for typed SchoolX Code Git operations.
//!
//! The Rust surface intentionally contains no raw XPC or spawn FFI. Those
//! calls live in the pinned Swift bridge TCB and expose only owned strings,
//! integers, and already-open file descriptors through safe generated calls.

#![cfg_attr(test, allow(dead_code))]
#![allow(
    clippy::too_many_arguments,
    reason = "the generated Swift bridge preserves typed descriptor observations as ABI-stable scalar fields"
)]

use std::cell::RefCell;
use std::fs::File;
use std::io::{Seek as _, SeekFrom, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::process::ExitStatusExt as _;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::ThreadId;

use serde::{Deserialize, Serialize};

#[path = "macos_git_xpc_session.rs"]
mod session_lifecycle;

const ROOT_TRUSTED_MACOS_GIT: &str = "/usr/bin/git";
const MAX_PROCESS_ARGUMENTS: usize = 256;
const MAX_PROCESS_ENVIRONMENT: usize = 128;
const MAX_PROCESS_SPEC_BYTES: usize = 512 * 1024;
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_SESSION_ID: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// A weak reference deliberately avoids making thread exit an authority
    /// release event. The process-global gate is cleared only by a proved
    /// session end, never by TLS destruction.
    static THREAD_SESSION: RefCell<Weak<SessionScope>> = const { RefCell::new(Weak::new()) };
}

#[swift_bridge::bridge]
mod ffi {
    extern "Rust" {
        fn schoolx_git_xpc_prepare(
            family: u8,
            payload: String,
            cwd_device: u64,
            cwd_inode: u64,
            cwd_mode: u32,
            stdin_device: u64,
            stdin_inode: u64,
            stdin_mode: u32,
            stdin_size: u64,
        ) -> String;
        fn schoolx_git_xpc_session_cleanup_proven(session_id: u64) -> bool;
    }

    extern "Swift" {
        fn schoolx_git_xpc_is_service() -> bool;
        fn schoolx_git_xpc_service_main() -> i32;
        fn schoolx_git_xpc_session_begin(session_id: u64) -> String;
        fn schoolx_git_xpc_session_end(session_id: u64) -> String;
        fn schoolx_git_xpc_launch(
            session_id: u64,
            request_id: u64,
            family: u8,
            cwd_fd: i32,
            stdin_fd: i32,
            stdout_fd: i32,
            stderr_fd: i32,
            payload: String,
        ) -> String;
        fn schoolx_git_xpc_poll(request_id: u64) -> String;
        fn schoolx_git_xpc_cancel(request_id: u64) -> String;
    }
}

/// Closed request families understood by the signed service.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub(crate) enum MacGitFamily {
    GitWrite = 1,
    Pinned = 2,
    Removal = 3,
}

/// Kernel metadata observed by the Swift service on an XPC-transferred FD.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DescriptorObservation {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) mode: u32,
    pub(crate) size: u64,
}

/// A process specification derived inside the signed helper from a closed
/// typed request. The executable is deliberately absent: Swift always invokes
/// the fixed root-trusted `/usr/bin/git` after Rust revalidates its identity.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MacGitProcessSpec {
    args: Vec<String>,
    environment: Vec<MacGitEnvironmentEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MacGitEnvironmentEntry {
    key: String,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrepareResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    spec: Option<MacGitProcessSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionResponse {
    ok: bool,
    session_id: u64,
    session_cleanup_proven: bool,
    session_authority_retained: bool,
    #[serde(default)]
    error: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaunchResponse {
    ok: bool,
    #[serde(default)]
    pid: u32,
    /// This disposition concerns only the attempted child. The persistent
    /// session reservation remains live until `schoolx_git_xpc_session_end`.
    #[serde(default)]
    child_cleanup_proven: bool,
    #[serde(default = "retained_by_default")]
    child_authority_retained: bool,
    #[serde(default)]
    error: String,
}

#[derive(Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum PollResponse {
    Pending,
    Finished {
        raw_status: i32,
    },
    Failed {
        error: String,
        #[serde(default)]
        child_cleanup_proven: bool,
        #[serde(default = "retained_by_default")]
        child_authority_retained: bool,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CancelResponse {
    ok: bool,
    #[serde(default, rename = "pid")]
    _pid: u32,
    /// A successful transport reply is not cleanup proof. Both fields must
    /// explicitly describe a finished child authority before Rust releases
    /// its permit.
    #[serde(default)]
    child_cleanup_proven: bool,
    #[serde(default = "retained_by_default")]
    child_authority_retained: bool,
    #[serde(default)]
    error: String,
}

const fn retained_by_default() -> bool {
    true
}

/// Exact stdin authority passed independently from the descriptor cwd.
pub(crate) enum MacGitInput {
    Null,
    File(File),
    Bytes(Vec<u8>),
}

/// A Git child owned by the persistent signed XPC service.
pub(crate) struct MacGitChild {
    request_id: u64,
    stdout: Option<std::io::PipeReader>,
    stderr: Option<std::io::PipeReader>,
    finished: Option<ExitStatus>,
    terminal_error: Option<String>,
    permit: Option<ChildPermit>,
}

/// A high-level mutation authority. Clones share one persistent XPC session,
/// allowing sequential Git children without opening a TOCTOU gap between the
/// caller's durable steps.
pub(crate) struct MacGitAuthoritySession {
    scope: Arc<SessionScope>,
    explicit_end: bool,
}

impl Clone for MacGitAuthoritySession {
    fn clone(&self) -> Self {
        Self {
            scope: Arc::clone(&self.scope),
            explicit_end: false,
        }
    }
}

struct SessionScope {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    session_id: u64,
    owner_thread: ThreadId,
    lifecycle: Mutex<SessionLifecycle>,
}

#[derive(Default)]
struct SessionLifecycle {
    active_child: Option<Arc<ChildAuthority>>,
    close_requested: bool,
    close_in_progress: bool,
    end_started: bool,
    end_complete: bool,
    poison_reason: Option<String>,
}

struct ChildAuthority {
    request_id: u64,
    cleanup_proven: AtomicBool,
}

struct ChildPermit {
    session: Arc<SessionInner>,
    authority: Arc<ChildAuthority>,
}

impl MacGitAuthoritySession {
    /// Admit a durable operation before its first mutation. A nested call on
    /// the same synchronous thread reuses the ambient session; every other
    /// thread is rejected by the process-global gate. Fresh admissions perform
    /// their signed capability validation inside the Swift session-begin TCB,
    /// immediately before reserving the fixed system Git executable.
    pub(crate) fn begin() -> Result<Self, String> {
        let owner_thread = std::thread::current().id();
        let ambient = THREAD_SESSION.with(|slot| slot.borrow().upgrade());
        if let Some(scope) = ambient {
            match scope.inner.can_reuse_from(&owner_thread) {
                Ok(true) => {
                    return Ok(Self {
                        scope,
                        explicit_end: false,
                    });
                }
                Ok(false) => {
                    THREAD_SESSION.with(|slot| *slot.borrow_mut() = Weak::new());
                }
                Err(error) => return Err(error),
            }
        }

        let session_id = next_identifier(&NEXT_SESSION_ID, "session")?;
        ACTIVE_SESSION_ID
            .compare_exchange(0, session_id, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|active| {
                format!(
                    "another macOS SchoolX Code Git operation is already active (session {active})"
                )
            })?;

        let encoded = ffi::schoolx_git_xpc_session_begin(session_id);
        let response: SessionResponse = match serde_json::from_str(&encoded) {
            Ok(response) => response,
            Err(error) => {
                return Err(format!(
                    "invalid macOS XPC Git session-begin response: {error}; session {session_id} is fail-closed"
                ));
            }
        };
        if response.session_id != session_id {
            return Err(format!(
                "macOS XPC Git session-begin response changed authority identity; session {session_id} is fail-closed"
            ));
        }
        let begin_admitted =
            response.ok && response.session_authority_retained && !response.session_cleanup_proven;
        if !begin_admitted {
            if session_cleanup_is_proven(&response) {
                release_global_session(session_id)?;
                return Err(response_diagnostic(
                    "macOS XPC Git session admission was rejected",
                    &response.error,
                ));
            }
            return Err(format!(
                "{}; session {session_id} is fail-closed",
                response_diagnostic(
                    "macOS XPC Git session admission had an ambiguous disposition",
                    &response.error,
                )
            ));
        }

        let scope = Arc::new(SessionScope {
            inner: Arc::new(SessionInner {
                session_id,
                owner_thread,
                lifecycle: Mutex::new(SessionLifecycle::default()),
            }),
        });
        THREAD_SESSION.with(|slot| *slot.borrow_mut() = Arc::downgrade(&scope));
        Ok(Self {
            scope,
            explicit_end: true,
        })
    }

    /// End the outermost durable operation. Reused and cloned handles are
    /// intentionally no-ops; the fresh root handle owns the end transition.
    pub(crate) fn end(mut self) -> Result<(), String> {
        self.scope.inner.ensure_owner_or_poison("session end")?;
        if !self.explicit_end {
            return Ok(());
        }
        self.explicit_end = false;
        if Arc::strong_count(&self.scope) != 1 {
            let error = format!(
                "macOS XPC Git session {} still has live authority handles; session retained",
                self.scope.inner.session_id
            );
            self.scope.inner.poison(error.clone());
            return Err(error);
        }
        self.scope.inner.request_close()
    }

    /// Launch one typed Git child under this persistent authority.
    pub(crate) fn spawn(
        &self,
        family: MacGitFamily,
        payload: String,
        cwd: &File,
        input: MacGitInput,
    ) -> Result<MacGitChild, String> {
        spawn(self, family, payload, cwd, input)
    }
}

impl Drop for MacGitAuthoritySession {
    fn drop(&mut self) {
        if !self.scope.inner.on_owner_thread() {
            self.scope.inner.poison(format!(
                "macOS XPC Git session {} handle was dropped outside its admitted thread; authority retained",
                self.scope.inner.session_id
            ));
        }
    }
}

impl Drop for SessionScope {
    fn drop(&mut self) {
        if self
            .inner
            .ensure_owner_or_poison("last session handle drop")
            .is_ok()
        {
            let _ = self.inner.request_close();
        }
    }
}

impl MacGitChild {
    /// Take the bounded-capture stdout pipe exactly once.
    pub(crate) fn take_stdout(&mut self) -> Result<std::io::PipeReader, String> {
        self.stdout
            .take()
            .ok_or_else(|| "macOS XPC Git stdout was unavailable".to_string())
    }

    /// Take the bounded-capture stderr pipe exactly once.
    pub(crate) fn take_stderr(&mut self) -> Result<std::io::PipeReader, String> {
        self.stderr
            .take()
            .ok_or_else(|| "macOS XPC Git stderr was unavailable".to_string())
    }

    /// Poll the signed helper without relinquishing ownership while pending.
    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        if let Some(status) = self.finished {
            return Ok(Some(status));
        }
        let permit = self
            .permit
            .as_ref()
            .ok_or_else(|| "macOS XPC Git child no longer has authority".to_string())?;
        permit.ensure_owner("child poll")?;
        let encoded = ffi::schoolx_git_xpc_poll(self.request_id);
        let response: PollResponse = match serde_json::from_str(&encoded) {
            Ok(response) => response,
            Err(error) => {
                let diagnostic = format!(
                    "invalid macOS XPC Git poll response: {error}; child authority retained"
                );
                permit.poison(diagnostic.clone());
                self.terminal_error = Some(diagnostic.clone());
                return Err(diagnostic);
            }
        };
        match response {
            PollResponse::Pending => Ok(None),
            PollResponse::Finished { raw_status } => {
                let status = ExitStatus::from_raw(raw_status);
                let result = self
                    .permit
                    .as_mut()
                    .ok_or_else(|| "macOS XPC Git child lost its permit".to_string())?
                    .prove_cleanup();
                match result {
                    Ok(()) => {
                        self.permit.take();
                        self.finished = Some(status);
                        Ok(Some(status))
                    }
                    Err(error) => {
                        self.terminal_error = Some(error.clone());
                        Err(error)
                    }
                }
            }
            PollResponse::Failed {
                error,
                child_cleanup_proven,
                child_authority_retained,
            } => {
                let mut diagnostic = response_diagnostic("macOS XPC Git failed", &error);
                if child_cleanup_is_proven(child_cleanup_proven, child_authority_retained) {
                    let cleanup = self
                        .permit
                        .as_mut()
                        .ok_or_else(|| "macOS XPC Git child lost its permit".to_string())?
                        .prove_cleanup();
                    match cleanup {
                        Ok(()) => {
                            self.permit.take();
                        }
                        Err(cleanup_error) => {
                            diagnostic = format!("{diagnostic}; {cleanup_error}");
                        }
                    }
                } else if let Some(permit) = self.permit.as_ref() {
                    diagnostic = format!(
                        "{diagnostic}; child cleanup disposition was ambiguous and authority is retained"
                    );
                    permit.poison(diagnostic.clone());
                }
                self.terminal_error = Some(diagnostic.clone());
                Err(diagnostic)
            }
        }
    }

    /// Kill and reap the dedicated Git process group through the signed helper.
    pub(crate) fn terminate(&mut self) -> Result<(), String> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        if self.finished.is_some() {
            return Ok(());
        }
        let Some(permit) = self.permit.as_ref() else {
            return Ok(());
        };
        permit.ensure_owner("child termination")?;
        let encoded = ffi::schoolx_git_xpc_cancel(self.request_id);
        let response: CancelResponse = match serde_json::from_str(&encoded) {
            Ok(response) => response,
            Err(error) => {
                let diagnostic = format!(
                    "invalid macOS XPC Git cancel response: {error}; child authority retained"
                );
                permit.poison(diagnostic.clone());
                self.terminal_error = Some(diagnostic.clone());
                return Err(diagnostic);
            }
        };
        if !child_cleanup_is_proven(
            response.child_cleanup_proven,
            response.child_authority_retained,
        ) {
            let diagnostic = format!(
                "{}; child cleanup disposition was ambiguous and authority is retained",
                response_diagnostic("failed to cancel macOS XPC Git", &response.error)
            );
            permit.poison(diagnostic.clone());
            self.terminal_error = Some(diagnostic.clone());
            return Err(diagnostic);
        }
        let cleanup = self
            .permit
            .as_mut()
            .ok_or_else(|| "macOS XPC Git child lost its permit".to_string())?
            .prove_cleanup();
        if let Err(error) = cleanup {
            self.terminal_error = Some(error.clone());
            return Err(error);
        }
        self.permit.take();
        if response.ok {
            Ok(())
        } else {
            let diagnostic = response_diagnostic(
                "failed to cancel macOS XPC Git after cleanup",
                &response.error,
            );
            self.terminal_error = Some(diagnostic.clone());
            Err(diagnostic)
        }
    }
}

impl Drop for MacGitChild {
    fn drop(&mut self) {
        let Some(mut permit) = self.permit.take() else {
            return;
        };
        if permit.cleanup_proven() {
            return;
        }
        if permit.ensure_owner("child drop").is_err() {
            return;
        }
        let encoded = ffi::schoolx_git_xpc_cancel(self.request_id);
        let response: CancelResponse = match serde_json::from_str(&encoded) {
            Ok(response) => response,
            Err(error) => {
                permit.poison(format!(
                    "invalid macOS XPC Git cancel response during child drop: {error}; authority retained"
                ));
                return;
            }
        };
        if child_cleanup_is_proven(
            response.child_cleanup_proven,
            response.child_authority_retained,
        ) {
            let _ = permit.prove_cleanup();
        } else {
            permit.poison(response_diagnostic(
                "macOS XPC Git child drop did not prove cleanup; authority retained",
                &response.error,
            ));
        }
    }
}

/// Enter the embedded XPC service before Tauri initialization when launchd
/// starts the nested signed helper bundle.
pub(crate) fn run_service_if_requested() -> Result<bool, String> {
    if !ffi::schoolx_git_xpc_is_service() {
        return Ok(false);
    }
    let status = ffi::schoolx_git_xpc_service_main();
    Err(format!(
        "macOS SchoolX Code Git XPC service returned unexpectedly with status {status}"
    ))
}

/// Run a durable operation inside one same-thread ambient authority session.
/// The operation error and any independent session-end error are both kept.
pub(crate) fn with_authority_session<T>(
    operation: impl FnOnce(&MacGitAuthoritySession) -> Result<T, String>,
) -> Result<T, String> {
    let session = MacGitAuthoritySession::begin()?;
    let operation_result = operation(&session);
    let end_result = session.end();
    match (operation_result, end_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(end_error)) => Err(end_error),
        (Err(error), Err(end_error)) => Err(format!(
            "{error}; additionally failed to end macOS XPC Git authority session: {end_error}"
        )),
    }
}

/// Launch one typed Git request with an opened directory as its exact cwd.
pub(crate) fn spawn(
    session: &MacGitAuthoritySession,
    family: MacGitFamily,
    payload: String,
    cwd: &File,
    input: MacGitInput,
) -> Result<MacGitChild, String> {
    session
        .scope
        .inner
        .ensure_owner_or_poison("child preparation")?;
    let request_id = next_identifier(&NEXT_REQUEST_ID, "request")?;
    let stdin = materialize_input(input)?;
    let (stdout_reader, stdout_writer) = std::io::pipe()
        .map_err(|error| format!("failed to create macOS XPC Git stdout pipe: {error}"))?;
    let (stderr_reader, stderr_writer) = std::io::pipe()
        .map_err(|error| format!("failed to create macOS XPC Git stderr pipe: {error}"))?;
    let mut permit = session.scope.inner.acquire_child(request_id)?;
    let encoded = ffi::schoolx_git_xpc_launch(
        session.scope.inner.session_id,
        request_id,
        family as u8,
        cwd.as_raw_fd(),
        stdin.as_raw_fd(),
        stdout_writer.as_raw_fd(),
        stderr_writer.as_raw_fd(),
        payload,
    );
    drop(stdout_writer);
    drop(stderr_writer);
    drop(stdin);

    let response: LaunchResponse = match serde_json::from_str(&encoded) {
        Ok(response) => response,
        Err(error) => {
            let diagnostic =
                format!("invalid macOS XPC Git launch response: {error}; child authority retained");
            permit.poison(diagnostic.clone());
            return Err(diagnostic);
        }
    };
    let launched = response.ok
        && response.pid != 0
        && response.child_authority_retained
        && !response.child_cleanup_proven;
    if !launched {
        let diagnostic = response_diagnostic(
            "failed to launch macOS XPC Git",
            if response.error.is_empty() {
                "signed helper did not return a live Git child authority"
            } else {
                &response.error
            },
        );
        if child_cleanup_is_proven(
            response.child_cleanup_proven,
            response.child_authority_retained,
        ) {
            permit.prove_cleanup()?;
            return Err(diagnostic);
        }
        let poisoned = format!("{diagnostic}; child authority disposition was ambiguous");
        permit.poison(poisoned.clone());
        return Err(poisoned);
    }
    Ok(MacGitChild {
        request_id,
        stdout: Some(stdout_reader),
        stderr: Some(stderr_reader),
        finished: None,
        terminal_error: None,
        permit: Some(permit),
    })
}

fn next_identifier(counter: &AtomicU64, label: &str) -> Result<u64, String> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1).filter(|next| *next != 0)
        })
        .map_err(|_| format!("macOS XPC Git {label} identifier exhausted"))
}

fn release_global_session(session_id: u64) -> Result<(), String> {
    if session_id == 0 {
        return Err("macOS XPC Git cannot release the empty session identity".to_string());
    }
    ACTIVE_SESSION_ID
        .compare_exchange(session_id, 0, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|active| {
            format!(
                "macOS XPC Git global session authority changed from {session_id} to {active}; process is fail-closed"
            )
        })
}

/// Release only the exact Rust-side gate whose signed helper died after an
/// ambiguous admission/end response. Swift calls this only after its prearmed
/// process-exit source proves helper death, no child authority remains, and
/// the shared Git reservation FD has been unlocked and closed locally.
fn schoolx_git_xpc_session_cleanup_proven(session_id: u64) -> bool {
    release_global_session(session_id).is_ok()
}

fn session_cleanup_is_proven(response: &SessionResponse) -> bool {
    response.session_cleanup_proven && !response.session_authority_retained
}

fn child_cleanup_is_proven(cleanup_proven: bool, authority_retained: bool) -> bool {
    cleanup_proven && !authority_retained
}

fn response_diagnostic(prefix: &str, detail: &str) -> String {
    if detail.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {detail}")
    }
}

fn materialize_input(input: MacGitInput) -> Result<File, String> {
    match input {
        MacGitInput::Null => File::open("/dev/null")
            .map_err(|error| format!("failed to open null Git input: {error}")),
        MacGitInput::File(file) => Ok(file),
        MacGitInput::Bytes(bytes) => {
            let mut file = tempfile::tempfile()
                .map_err(|error| format!("failed to create bounded Git input file: {error}"))?;
            file.write_all(&bytes)
                .map_err(|error| format!("failed to write bounded Git input: {error}"))?;
            file.seek(SeekFrom::Start(0))
                .map_err(|error| format!("failed to rewind bounded Git input: {error}"))?;
            Ok(file)
        }
    }
}

/// Convert an already validated typed `Command` into the narrow Swift spawn
/// format. Environment inheritance and executable replacement are rejected.
pub(crate) fn process_spec_from_command(command: &Command) -> Result<MacGitProcessSpec, String> {
    if command.get_program() != Path::new(ROOT_TRUSTED_MACOS_GIT).as_os_str() {
        return Err("macOS XPC helper rejected a non-system Git executable".to_string());
    }
    let args = command
        .get_args()
        .map(|argument| {
            argument
                .to_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| "macOS XPC Git argument was not UTF-8".to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    if args.len() > MAX_PROCESS_ARGUMENTS {
        return Err("macOS XPC Git argument count exceeded its limit".to_string());
    }
    let environment = command
        .get_envs()
        .map(|(key, value)| {
            let key = key
                .to_str()
                .ok_or_else(|| "macOS XPC Git environment key was not UTF-8".to_string())?;
            let value = value
                .and_then(std::ffi::OsStr::to_str)
                .ok_or_else(|| "macOS XPC Git environment attempted an unset".to_string())?;
            Ok(MacGitEnvironmentEntry {
                key: key.to_string(),
                value: value.to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if environment.len() > MAX_PROCESS_ENVIRONMENT {
        return Err("macOS XPC Git environment count exceeded its limit".to_string());
    }
    Ok(MacGitProcessSpec { args, environment })
}

#[allow(
    clippy::too_many_arguments,
    reason = "swift-bridge exposes the two typed fstat observations as ABI-stable scalar fields"
)]
fn schoolx_git_xpc_prepare(
    family: u8,
    payload: String,
    cwd_device: u64,
    cwd_inode: u64,
    cwd_mode: u32,
    stdin_device: u64,
    stdin_inode: u64,
    stdin_mode: u32,
    stdin_size: u64,
) -> String {
    let cwd = DescriptorObservation {
        device: cwd_device,
        inode: cwd_inode,
        mode: cwd_mode,
        size: 0,
    };
    let stdin = DescriptorObservation {
        device: stdin_device,
        inode: stdin_inode,
        mode: stdin_mode,
        size: stdin_size,
    };
    let result = prepare_request(family, &payload, cwd, stdin);
    let response = match result {
        Ok(spec) => PrepareResponse {
            ok: true,
            spec: Some(spec),
            error: None,
        },
        Err(error) => PrepareResponse {
            ok: false,
            spec: None,
            error: Some(error),
        },
    };
    match serde_json::to_string(&response) {
        Ok(encoded) if encoded.len() <= MAX_PROCESS_SPEC_BYTES => encoded,
        Ok(_) => "{\"ok\":false,\"error\":\"typed Git process specification exceeded its limit\"}"
            .to_string(),
        Err(_) => "{\"ok\":false,\"error\":\"failed to encode typed Git process specification\"}"
            .to_string(),
    }
}

fn prepare_request(
    family: u8,
    payload: &str,
    cwd: DescriptorObservation,
    stdin: DescriptorObservation,
) -> Result<MacGitProcessSpec, String> {
    match family {
        value if value == MacGitFamily::GitWrite as u8 => {
            super::git_write::prepare_macos_git_write(payload, cwd, stdin)
        }
        value if value == MacGitFamily::Pinned as u8 => {
            super::worktrees::prepare_macos_pinned_git(payload, cwd, stdin)
        }
        value if value == MacGitFamily::Removal as u8 => {
            super::bindings::removal::prepare_macos_removal_git(payload, cwd, stdin)
        }
        _ => Err("macOS XPC helper rejected an unknown typed Git family".to_string()),
    }
}

/// Match a transferred cwd descriptor against the typed envelope evidence.
pub(crate) fn validate_directory_observation(
    observed: DescriptorObservation,
    device: u64,
    inode: u64,
    mode: Option<u32>,
    label: &str,
) -> Result<(), String> {
    if observed.mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFDIR)
        || observed.device != device
        || observed.inode != inode
        || mode.is_some_and(|expected| observed.mode != expected)
    {
        return Err(format!(
            "macOS XPC {label} descriptor identity did not match its request"
        ));
    }
    Ok(())
}

/// Match a transferred regular-file stdin descriptor against exact evidence.
pub(crate) fn validate_regular_observation(
    observed: DescriptorObservation,
    device: u64,
    inode: u64,
    mode: u32,
    size: u64,
    label: &str,
) -> Result<(), String> {
    if observed.mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFREG)
        || observed.device != device
        || observed.inode != inode
        || observed.mode != mode
        || observed.size != size
    {
        return Err(format!(
            "macOS XPC {label} descriptor identity did not match its request"
        ));
    }
    Ok(())
}

/// Require a transferred input to be a bounded regular file when its exact
/// identity is held by the mutually authenticated client rather than encoded.
pub(crate) fn validate_bounded_regular_observation(
    observed: DescriptorObservation,
    max_size: u64,
    label: &str,
) -> Result<(), String> {
    if observed.mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFREG)
        || observed.size > max_size
    {
        return Err(format!("macOS XPC {label} was not a bounded regular file"));
    }
    Ok(())
}

/// Require stdin to be the platform's exact `/dev/null` device.
pub(crate) fn validate_null_observation(
    observed: DescriptorObservation,
    label: &str,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;

    let expected = std::fs::metadata("/dev/null")
        .map_err(|error| format!("failed to inspect macOS null input: {error}"))?;
    if observed.mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFCHR)
        || observed.device != expected.dev()
        || observed.inode != expected.ino()
        || observed.mode != expected.mode()
    {
        return Err(format!("macOS XPC {label} was not the null device"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "macos_git_xpc_tests.rs"]
mod tests;
