use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::approvals::{
    CodeApprovalResponseInput, PendingApprovalAdmissionGuard, PendingApprovalStore,
};
use super::discovery::{ensure_supported_codex_version, probe_codex, CodeRuntimeProbe};
use super::jsonrpc::{self, IncomingMessage};
use super::model_catalog::{collect_model_catalog, turn_selection, CodeModelCatalogSnapshot};
use super::paths::canonical_workspace_root;
use super::protocol::{
    self, CodeRecoveryThread, CodeRuntimeActiveTurnCheckpoint, CodeRuntimeApprovalCheckpoint,
    CodeRuntimeEvent, CodeRuntimeEventBacklog, CodeRuntimeEventCheckpoint, CodeThreadForkInput,
    CodeThreadRenameInput, CodeThreadResumeInput, CodeThreadRpcOpenResult, CodeThreadStartInput,
    CodeThreadSummary, CodeTurnInterruptInput, CodeTurnStartInput, CodeTurnSteerInput,
    CodeTurnSummary, CodeWorkspaceEventDraft,
};
use super::thread_lifecycle::{
    authoritative_thread_list_params, authoritative_thread_read_params,
    parse_authoritative_deferred_bound_thread_read, parse_authoritative_pending_fork_thread_read,
    parse_authoritative_thread_list, parse_authoritative_thread_read, parse_thread_archive,
    parse_thread_unarchive, CodeAuthoritativeThreadActivity, CodeAuthoritativeThreadGraph,
    CodePendingForkExpectation, CodePinnedThreadStatus, CodePinnedTurnStatus,
    CodeThreadLifecycleInput, CodeThreadMembership, CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT,
    MAX_AUTHORITATIVE_PAGES, MAX_AUTHORITATIVE_THREADS,
};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_NOTIFICATION_BACKLOG: usize = 512;
const MAX_TURN_TOMBSTONES: usize = 4_096;
const MAX_INFLIGHT_TERMINAL_TURNS: usize = 4_096;
const MAX_LIFECYCLE_DIRTY_THREADS: usize = 4_096;
const MAX_TOPOLOGY_CHANGES: usize = 4_096;
const STDERR_TAIL_BYTES: usize = 64 * 1024;
const MAX_RECOVERY_THREADS: usize = 4_096;
const MAX_RECOVERY_PAGES: usize = 64;
#[cfg(unix)]
const PROCESS_GROUP_TERM_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(unix)]
const PROCESS_GROUP_POLL_INTERVAL: Duration = Duration::from_millis(50);

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

/// Delivery classification for a mutating RPC whose caller must decide whether
/// a retry is safe.
#[derive(Debug)]
pub(crate) enum CodeRpcDeliveryError {
    /// Validation or runtime state failed before any request bytes were written.
    NotSent(String),
    /// A write was attempted or completed, so Codex may have created a thread.
    Uncertain(String),
}

impl CodeRpcDeliveryError {
    pub(crate) fn definitely_not_sent(&self) -> bool {
        matches!(self, Self::NotSent(_))
    }

    pub(crate) fn into_message(self) -> String {
        match self {
            Self::NotSent(message) | Self::Uncertain(message) => message,
        }
    }
}

/// Backward-compatible name retained for the durable thread-start journal.
pub(crate) type CodeThreadStartRpcError = CodeRpcDeliveryError;

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
    #[cfg(test)]
    fail_next_fork_before_write: Arc<AtomicBool>,
}

/// Locks runtime generation, thread activity, and approval admission while a
/// private operation relies on a previously completed authoritative idle read.
pub(crate) struct CodeThreadIdleAdmissionGuard<'a> {
    _runtime: MutexGuard<'a, RuntimeInner>,
    _events: MutexGuard<'a, EventBridgeInner>,
    _approvals: PendingApprovalAdmissionGuard<'a>,
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
    active_turns: HashMap<(String, String), CodeRuntimeActiveTurnCheckpoint>,
    inflight_turn_starts: HashMap<String, InflightTurnStart>,
    uncertain_turn_threads: HashSet<String>,
    turn_tombstones: HashSet<(String, String)>,
    turn_tombstone_order: VecDeque<(String, String)>,
    thread_activity_revisions: HashMap<String, u64>,
    next_activity_revision: u64,
    next_turn_start_token: u64,
    lifecycle_generation_dirty: bool,
    lifecycle_clean_threads: HashSet<String>,
    lifecycle_dirty_revisions: HashMap<String, LifecycleDirtyRevision>,
    lifecycle_boundary_revision: u64,
    next_lifecycle_revision: u64,
    topology_changes: VecDeque<TopologyChange>,
    topology_boundary_revision: u64,
    next_topology_revision: u64,
    emitter: CodeEventEmitter,
}

#[derive(Clone, Copy, Debug)]
struct ThreadActivitySnapshot {
    revision: u64,
    active_or_starting: bool,
    uncertain: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodeThreadLifecycleSignal {
    Archived,
    Unarchived,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TopologyChangeKind {
    Lifecycle(CodeThreadLifecycleSignal),
    Started,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TopologyChange {
    revision: u64,
    thread_id: String,
    kind: TopologyChangeKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LifecycleDirtyRevision {
    revision: u64,
    signal: CodeThreadLifecycleSignal,
}

struct InflightTurnStart {
    token: u64,
    terminal_turn_ids: HashSet<String>,
    terminal_overflow: bool,
    thread_closed: bool,
}

enum CodeModelCatalogRequirement<'a> {
    Model(&'a str),
}

impl CodeModelCatalogRequirement<'_> {
    fn validate(&self, catalog: &CodeModelCatalogSnapshot) -> Result<(), String> {
        match self {
            Self::Model(model) => catalog.require_model(model),
        }
    }
}

/// Generation- and revision-bound dirty-state proof used to clear an exact
/// thread only after durable lifecycle reconciliation.
#[derive(Clone, Debug)]
pub(crate) struct CodeThreadLifecycleDirtyCheckpoint {
    generation: u64,
    thread_id: String,
    boundary_revision: u64,
    graph_revision: u64,
    thread_dirty: Option<LifecycleDirtyRevision>,
    dirty: bool,
}

/// Source-side proof retained after `thread/fork` byte admission and consumed
/// only while atomically committing the new child binding.
pub(crate) struct CodeThreadForkCompletion {
    generation: u64,
    source_thread_id: String,
    lifecycle_checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    activity_revision: u64,
}

/// Parsed fork response paired with the exact source proof from its write.
pub(crate) struct CodeThreadForkGuardedResult {
    pub(crate) opened: CodeThreadRpcOpenResult,
    pub(crate) completion: CodeThreadForkCompletion,
}

/// Exhaustive-thread-graph proof tied to one exact lifecycle target.
///
/// The graph and both native notification checkpoints are captured across the
/// same bounded list scan. A guarded mutation accepts this proof only while no
/// topology or exact lifecycle state has changed.
pub(crate) struct CodeThreadLifecycleGraphProof {
    generation: u64,
    thread_id: String,
    graph: CodeAuthoritativeThreadGraph,
    topology_boundary_revision: u64,
    topology_revision: u64,
}

impl CodeThreadLifecycleGraphProof {
    pub(crate) fn graph(&self) -> &CodeAuthoritativeThreadGraph {
        &self.graph
    }
}

/// Response-time proof returned by one guarded archive or unarchive write.
///
/// The caller must durably commit the corresponding binding transition and
/// then consume this proof with the matching completion method.
pub(crate) struct CodeThreadLifecycleMutationCompletion {
    generation: u64,
    thread_id: String,
    expected: CodeThreadLifecycleSignal,
    lifecycle_boundary_revision: u64,
    topology_boundary_revision: u64,
    topology_revision: u64,
    thread_dirty: Option<LifecycleDirtyRevision>,
    expected_signal_seen: bool,
}

/// Normalized unarchive response and the native completion proof that covers
/// its guarded JSON-RPC write.
pub(crate) struct CodeThreadUnarchiveGuardedResult {
    pub(crate) thread: CodeThreadSummary,
    pub(crate) completion: CodeThreadLifecycleMutationCompletion,
}

#[derive(Clone, Debug)]
struct LifecycleWriteReceipt {
    generation: u64,
    thread_id: String,
    expected: CodeThreadLifecycleSignal,
    lifecycle_boundary_revision: u64,
    topology_boundary_revision: u64,
    topology_revision: u64,
}

impl CodeThreadLifecycleDirtyCheckpoint {
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether a successful archive response can reconcile this checkpoint.
    #[cfg(test)]
    pub(crate) fn accepts_archive_completion(&self) -> bool {
        self.accepts_completion(CodeThreadLifecycleSignal::Archived)
    }

    /// Whether a successful unarchive response can reconcile this checkpoint.
    #[cfg(test)]
    pub(crate) fn accepts_unarchive_completion(&self) -> bool {
        self.accepts_completion(CodeThreadLifecycleSignal::Unarchived)
    }

    #[cfg(test)]
    fn accepts_completion(&self, expected: CodeThreadLifecycleSignal) -> bool {
        if !self.dirty {
            return true;
        }
        self.thread_dirty
            .is_some_and(|dirty| dirty.signal == expected)
    }
}

mod actions;
mod event_bridge;
mod event_state;
mod graph;
mod lifecycle;
mod process;

use event_state::*;
use graph::*;
use process::*;

#[cfg(test)]
mod tests;
