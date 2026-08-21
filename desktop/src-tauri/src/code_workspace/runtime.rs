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

impl EventBridge {
    fn new() -> Self {
        Self {
            inner: Mutex::new(EventBridgeInner {
                generation: 0,
                next_sequence: 1,
                backlog: VecDeque::new(),
                active_turns: HashMap::new(),
                inflight_turn_starts: HashMap::new(),
                uncertain_turn_threads: HashSet::new(),
                turn_tombstones: HashSet::new(),
                turn_tombstone_order: VecDeque::new(),
                thread_activity_revisions: HashMap::new(),
                next_activity_revision: 0,
                next_turn_start_token: 0,
                lifecycle_generation_dirty: true,
                lifecycle_clean_threads: HashSet::new(),
                lifecycle_dirty_revisions: HashMap::new(),
                lifecycle_boundary_revision: 0,
                next_lifecycle_revision: 0,
                topology_changes: VecDeque::new(),
                topology_boundary_revision: 0,
                next_topology_revision: 0,
                emitter: Arc::new(|_| {}),
            }),
        }
    }

    fn reset(&self, generation: u64, emitter: CodeEventEmitter) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        inner.generation = generation;
        inner.next_sequence = 1;
        inner.backlog.clear();
        clear_event_activity(&mut inner);
        reset_lifecycle_dirty_boundary(&mut inner);
        inner.emitter = emitter;
        Ok(())
    }

    fn clear_activity(&self, generation: u64) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        if inner.generation == generation {
            clear_event_activity(&mut inner);
            advance_lifecycle_dirty_boundary(&mut inner);
        }
        Ok(())
    }

    fn replace_emitter(&self, emitter: CodeEventEmitter) -> Result<(), String> {
        self.inner
            .lock()
            .map_err(|error| error.to_string())?
            .emitter = emitter;
        Ok(())
    }

    #[cfg(test)]
    fn publish(&self, generation: u64, draft: CodeWorkspaceEventDraft) {
        let publication = self
            .inner
            .lock()
            .ok()
            .and_then(|mut inner| publish_locked(&mut inner, generation, draft));
        if let Some((emitter, event)) = publication {
            emitter(event);
        }
    }

    fn insert_approval_and_publish(
        &self,
        approvals: &PendingApprovalStore,
        generation: u64,
        request_id: Value,
        method: &str,
        params: Option<Value>,
    ) -> Result<bool, String> {
        let publication = {
            let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
            ensure_event_generation(&inner, generation)?;
            let Some(draft) = approvals.insert_request(generation, request_id, method, params)?
            else {
                return Ok(false);
            };
            publish_locked(&mut inner, generation, draft)
        };
        if let Some((emitter, event)) = publication {
            emitter(event);
        }
        Ok(true)
    }

    fn publish_notification(
        &self,
        approvals: &PendingApprovalStore,
        generation: u64,
        method: &str,
        raw_params: Option<&Value>,
        draft: CodeWorkspaceEventDraft,
    ) -> Result<(), String> {
        let publication = {
            let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
            ensure_event_generation(&inner, generation)?;
            if method == "serverRequest/resolved" {
                if let Some(params) = raw_params {
                    approvals.resolve_notification(generation, params);
                }
            }
            if method == "turn/completed" {
                if let (Some(thread_id), Some(turn_id)) =
                    (draft.thread_id.as_deref(), draft.turn_id.as_deref())
                {
                    approvals.clear_turn(generation, thread_id, turn_id);
                }
            }
            publish_locked(&mut inner, generation, draft)
        };
        if let Some((emitter, event)) = publication {
            emitter(event);
        }
        Ok(())
    }

    fn snapshot(
        &self,
        approvals: &PendingApprovalStore,
        requested_generation: Option<u64>,
        after_sequence: Option<u64>,
    ) -> Result<CodeRuntimeEventBacklog, String> {
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        let generation_changed =
            requested_generation.is_some_and(|value| value != inner.generation);
        let full_replay_requested =
            generation_changed || after_sequence.is_none() || after_sequence == Some(0);
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
        let latest_sequence = inner.next_sequence.saturating_sub(1);
        let checkpoint = if full_replay_requested || truncated {
            let mut active_turns = inner.active_turns.values().cloned().collect::<Vec<_>>();
            active_turns.sort_by(|left, right| {
                left.started_sequence
                    .cmp(&right.started_sequence)
                    .then_with(|| left.thread_id.cmp(&right.thread_id))
                    .then_with(|| left.turn_id.cmp(&right.turn_id))
            });
            let pending_approvals = approvals
                .checkpoint_events(inner.generation)?
                .into_iter()
                .map(|(draft, respondable)| CodeRuntimeApprovalCheckpoint {
                    event: CodeRuntimeEvent {
                        runtime_generation: inner.generation,
                        sequence: latest_sequence,
                        thread_id: draft.thread_id,
                        turn_id: draft.turn_id,
                        item_id: draft.item_id,
                        kind: draft.kind,
                        payload: draft.payload,
                    },
                    respondable,
                })
                .collect();
            Some(CodeRuntimeEventCheckpoint {
                runtime_generation: inner.generation,
                sequence_watermark: latest_sequence,
                active_turns,
                pending_approvals,
            })
        } else {
            None
        };
        Ok(CodeRuntimeEventBacklog {
            runtime_generation: inner.generation,
            latest_sequence,
            truncated,
            checkpoint,
            events,
        })
    }

    fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|inner| inner.backlog.len())
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn begin_turn_start(&self, generation: u64, thread_id: &str) -> Result<u64, String> {
        protocol::validate_id("turn-start thread", thread_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        begin_turn_start_locked(&mut inner, thread_id)
    }

    fn fail_turn_start(
        &self,
        generation: u64,
        thread_id: &str,
        token: u64,
        uncertain_delivery: bool,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        fail_turn_start_locked(&mut inner, thread_id, token, uncertain_delivery)
    }

    fn complete_turn_start(
        &self,
        generation: u64,
        thread_id: &str,
        token: u64,
        turn_id: &str,
        status: CodePinnedTurnStatus,
    ) -> Result<(), String> {
        protocol::validate_id("started turn", turn_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        let inflight = remove_matching_turn_start(&mut inner, thread_id, token)?;
        inner.uncertain_turn_threads.remove(thread_id);
        let key = (thread_id.to_string(), turn_id.to_string());
        if inflight.thread_closed {
            inner.active_turns.remove(&key);
            insert_turn_tombstone(&mut inner, key);
            bump_thread_activity(&mut inner, thread_id);
            return Err("Codex thread closed before the turn/start response completed".to_string());
        }
        if inflight.terminal_overflow {
            inner.active_turns.remove(&key);
            inner.uncertain_turn_threads.insert(thread_id.to_string());
            bump_thread_activity(&mut inner, thread_id);
            return Err("Codex turn/start terminal ordering exceeded its safety limit".to_string());
        }
        if status.is_terminal() || inflight.terminal_turn_ids.contains(turn_id) {
            inner.active_turns.remove(&key);
            insert_turn_tombstone(&mut inner, key);
        } else if !inner.turn_tombstones.contains(&key) {
            let started_sequence = inner.next_sequence.saturating_sub(1);
            inner
                .active_turns
                .entry(key)
                .or_insert_with(|| CodeRuntimeActiveTurnCheckpoint {
                    thread_id: thread_id.to_string(),
                    turn_id: turn_id.to_string(),
                    status: status.as_str().to_string(),
                    started_sequence,
                });
        }
        bump_thread_activity(&mut inner, thread_id);
        Ok(())
    }

    fn mark_thread_uncertain(&self, generation: u64, thread_id: &str) -> Result<(), String> {
        protocol::validate_id("uncertain turn thread", thread_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        inner.uncertain_turn_threads.insert(thread_id.to_string());
        bump_thread_activity(&mut inner, thread_id);
        Ok(())
    }

    fn reconcile_thread_summary(
        &self,
        generation: u64,
        thread: &CodeThreadSummary,
    ) -> Result<(), String> {
        protocol::validate_id("resumed thread", &thread.id)?;
        let status_value = thread
            .status
            .clone()
            .ok_or_else(|| "Codex resumed thread omitted its status".to_string())?;
        let status: CodePinnedThreadStatus = serde_json::from_value(status_value)
            .map_err(|error| format!("invalid Codex resumed thread status: {error}"))?;
        let mut turns = Vec::with_capacity(thread.turns.len());
        for turn in &thread.turns {
            protocol::validate_id("resumed turn", &turn.id)?;
            turns.push((turn.id.clone(), CodePinnedTurnStatus::parse(&turn.status)?));
        }

        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        if inner.inflight_turn_starts.contains_key(&thread.id) {
            inner.uncertain_turn_threads.insert(thread.id.clone());
            bump_thread_activity(&mut inner, &thread.id);
            return Err("Codex thread was resumed during an in-flight turn/start".to_string());
        }
        inner
            .active_turns
            .retain(|(candidate, _), _| candidate != &thread.id);
        inner.uncertain_turn_threads.remove(&thread.id);
        let mut in_progress = 0_usize;
        for (turn_id, turn_status) in turns {
            let key = (thread.id.clone(), turn_id.clone());
            if turn_status == CodePinnedTurnStatus::InProgress {
                in_progress = in_progress.saturating_add(1);
                if !inner.turn_tombstones.contains(&key) {
                    let started_sequence = inner.next_sequence.saturating_sub(1);
                    inner.active_turns.insert(
                        key,
                        CodeRuntimeActiveTurnCheckpoint {
                            thread_id: thread.id.clone(),
                            turn_id,
                            status: turn_status.as_str().to_string(),
                            started_sequence,
                        },
                    );
                }
            } else {
                insert_turn_tombstone(&mut inner, key);
            }
        }
        let contradictory_idle = status.proves_quiescent() && in_progress > 0;
        if status.is_active() && in_progress == 0
            || matches!(status, CodePinnedThreadStatus::SystemError)
            || contradictory_idle
        {
            inner.uncertain_turn_threads.insert(thread.id.clone());
        }
        bump_thread_activity(&mut inner, &thread.id);
        if contradictory_idle {
            return Err("Codex resumed thread reported idle with an in-progress turn".to_string());
        }
        Ok(())
    }

    fn activity_snapshot(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<ThreadActivitySnapshot, String> {
        protocol::validate_id("activity thread", thread_id)?;
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        Ok(ThreadActivitySnapshot {
            revision: inner
                .thread_activity_revisions
                .get(thread_id)
                .copied()
                .unwrap_or_default(),
            active_or_starting: inner
                .active_turns
                .keys()
                .any(|(candidate, _)| candidate == thread_id)
                || inner.inflight_turn_starts.contains_key(thread_id),
            uncertain: inner.uncertain_turn_threads.contains(thread_id),
        })
    }

    fn confirm_authoritative_idle(
        &self,
        generation: u64,
        thread_id: &str,
        expected_revision: u64,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        let revision = inner
            .thread_activity_revisions
            .get(thread_id)
            .copied()
            .unwrap_or_default();
        let blocking = inner
            .active_turns
            .keys()
            .any(|(candidate, _)| candidate == thread_id)
            || inner.inflight_turn_starts.contains_key(thread_id);
        if revision != expected_revision || blocking {
            return Err("Codex thread activity changed during the idle proof".to_string());
        }
        if inner.uncertain_turn_threads.remove(thread_id) {
            bump_thread_activity(&mut inner, thread_id);
        }
        Ok(())
    }

    fn lifecycle_dirty_checkpoint(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<CodeThreadLifecycleDirtyCheckpoint, String> {
        protocol::validate_id("lifecycle-dirty thread", thread_id)?;
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        Ok(lifecycle_checkpoint_locked(&inner, generation, thread_id))
    }

    fn topology_checkpoint(&self, generation: u64) -> Result<(u64, u64), String> {
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        Ok((
            inner.topology_boundary_revision,
            inner.next_topology_revision,
        ))
    }

    fn confirm_topology_checkpoint(
        &self,
        generation: u64,
        boundary_revision: u64,
        revision: u64,
    ) -> Result<(), String> {
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        if inner.topology_boundary_revision != boundary_revision
            || inner.next_topology_revision != revision
        {
            return Err(
                "Codex thread topology changed during the authoritative graph scan".to_string(),
            );
        }
        Ok(())
    }

    fn clear_lifecycle_dirty(
        &self,
        generation: u64,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<(), String> {
        protocol::validate_id("lifecycle-dirty thread", thread_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        if checkpoint.generation != generation
            || checkpoint.thread_id != thread_id
            || checkpoint.boundary_revision != inner.lifecycle_boundary_revision
            || checkpoint.graph_revision != inner.next_lifecycle_revision
            || checkpoint.thread_dirty != inner.lifecycle_dirty_revisions.get(thread_id).copied()
        {
            return Err("Codex thread lifecycle changed during durable reconciliation".to_string());
        }
        mark_lifecycle_clean_locked(&mut inner, thread_id)
    }

    #[cfg(test)]
    fn mark_new_thread_lifecycle_clean(
        &self,
        generation: u64,
        thread_id: &str,
    ) -> Result<(), String> {
        protocol::validate_id("new lifecycle thread", thread_id)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, generation)?;
        validate_new_thread_lifecycle_locked(&inner, thread_id)?;
        ensure_lifecycle_clean_capacity_locked(&inner, thread_id)?;
        inner.lifecycle_clean_threads.insert(thread_id.to_string());
        Ok(())
    }

    fn mutation_response_checkpoint(
        &self,
        receipt: LifecycleWriteReceipt,
    ) -> Result<CodeThreadLifecycleMutationCompletion, String> {
        let inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, receipt.generation)?;
        let expected_signals = validate_expected_topology_changes_locked(
            &inner,
            receipt.topology_boundary_revision,
            receipt.topology_revision,
            &receipt.thread_id,
            receipt.expected,
        )?;
        if expected_signals > 1 {
            return Err("Codex emitted duplicate lifecycle completion signals".to_string());
        }
        if inner.lifecycle_boundary_revision != receipt.lifecycle_boundary_revision {
            return Err(
                "Codex lifecycle boundary changed after the guarded mutation write".to_string(),
            );
        }
        let thread_dirty = inner
            .lifecycle_dirty_revisions
            .get(&receipt.thread_id)
            .copied();
        Ok(CodeThreadLifecycleMutationCompletion {
            generation: receipt.generation,
            thread_id: receipt.thread_id,
            expected: receipt.expected,
            lifecycle_boundary_revision: inner.lifecycle_boundary_revision,
            topology_boundary_revision: inner.topology_boundary_revision,
            topology_revision: inner.next_topology_revision,
            thread_dirty,
            expected_signal_seen: expected_signals == 1,
        })
    }

    fn complete_lifecycle_mutation<T>(
        &self,
        thread_id: &str,
        completion: CodeThreadLifecycleMutationCompletion,
        expected: CodeThreadLifecycleSignal,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        protocol::validate_id("lifecycle completion thread", thread_id)?;
        if completion.thread_id != thread_id || completion.expected != expected {
            return Err(
                "Codex lifecycle completion proof does not match the requested mutation"
                    .to_string(),
            );
        }
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        ensure_event_generation(&inner, completion.generation)?;
        if inner.lifecycle_boundary_revision != completion.lifecycle_boundary_revision {
            return Err(
                "Codex lifecycle boundary changed during durable mutation commit".to_string(),
            );
        }
        let expected_signals = validate_expected_topology_changes_locked(
            &inner,
            completion.topology_boundary_revision,
            completion.topology_revision,
            thread_id,
            expected,
        )?;
        if usize::from(completion.expected_signal_seen).saturating_add(expected_signals) > 1 {
            return Err("Codex emitted duplicate lifecycle completion signals".to_string());
        }
        let current_dirty = inner.lifecycle_dirty_revisions.get(thread_id).copied();
        if current_dirty != completion.thread_dirty
            && !current_dirty.is_some_and(|dirty| dirty.signal == expected)
        {
            return Err(
                "Codex lifecycle signal conflicted with the durable mutation commit".to_string(),
            );
        }
        ensure_lifecycle_clean_capacity_locked(&inner, thread_id)?;
        let committed = commit()?;
        inner.lifecycle_dirty_revisions.remove(thread_id);
        inner.lifecycle_clean_threads.insert(thread_id.to_string());
        Ok(committed)
    }
}

fn update_active_turn_checkpoint(inner: &mut EventBridgeInner, event: &CodeRuntimeEvent) {
    match event.kind.as_str() {
        "turn/started" => {
            let (Some(thread_id), Some(turn_id)) =
                (event.thread_id.as_ref(), event.turn_id.as_ref())
            else {
                return;
            };
            let status = event
                .payload
                .get("turn")
                .and_then(|turn| turn.get("status"))
                .and_then(Value::as_str)
                .filter(|status| !status.is_empty())
                .unwrap_or("inProgress");
            let key = (thread_id.clone(), turn_id.clone());
            match CodePinnedTurnStatus::parse(status) {
                Ok(CodePinnedTurnStatus::InProgress) if !inner.turn_tombstones.contains(&key) => {
                    inner.uncertain_turn_threads.remove(thread_id);
                    inner.active_turns.insert(
                        key,
                        CodeRuntimeActiveTurnCheckpoint {
                            thread_id: thread_id.clone(),
                            turn_id: turn_id.clone(),
                            status: status.to_string(),
                            started_sequence: event.sequence,
                        },
                    );
                }
                Ok(terminal) => {
                    inner.active_turns.remove(&key);
                    if terminal.is_terminal() {
                        insert_turn_tombstone(inner, key);
                    }
                }
                Err(_) => {
                    inner.uncertain_turn_threads.insert(thread_id.clone());
                }
            }
            bump_thread_activity(inner, thread_id);
        }
        "turn/completed" => {
            if let (Some(thread_id), Some(turn_id)) =
                (event.thread_id.as_ref(), event.turn_id.as_ref())
            {
                if let Some(inflight) = inner.inflight_turn_starts.get_mut(thread_id) {
                    if !inflight.terminal_overflow
                        && !inflight.terminal_turn_ids.contains(turn_id)
                        && inflight.terminal_turn_ids.len() >= MAX_INFLIGHT_TERMINAL_TURNS
                    {
                        inflight.terminal_turn_ids.clear();
                        inflight.terminal_overflow = true;
                    }
                    if !inflight.terminal_overflow {
                        inflight.terminal_turn_ids.insert(turn_id.clone());
                    }
                }
                let key = (thread_id.clone(), turn_id.clone());
                inner.active_turns.remove(&key);
                inner.uncertain_turn_threads.remove(thread_id);
                insert_turn_tombstone(inner, key);
                bump_thread_activity(inner, thread_id);
            }
        }
        "thread/closed" => {
            if let Some(thread_id) = event.thread_id.as_ref() {
                record_topology_change(inner, thread_id, TopologyChangeKind::Closed);
                if let Some(inflight) = inner.inflight_turn_starts.get_mut(thread_id) {
                    inflight.thread_closed = true;
                }
                let completed = inner
                    .active_turns
                    .keys()
                    .filter(|(candidate, _)| candidate == thread_id)
                    .cloned()
                    .collect::<Vec<_>>();
                inner
                    .active_turns
                    .retain(|(candidate, _), _| candidate != thread_id);
                inner.uncertain_turn_threads.remove(thread_id);
                for key in completed {
                    insert_turn_tombstone(inner, key);
                }
                bump_thread_activity(inner, thread_id);
            }
        }
        "thread/archived" | "thread/unarchived" => {
            if let Some(thread_id) = event.thread_id.as_ref() {
                let signal = if event.kind == "thread/archived" {
                    CodeThreadLifecycleSignal::Archived
                } else {
                    CodeThreadLifecycleSignal::Unarchived
                };
                mark_lifecycle_dirty(inner, thread_id, signal);
            }
        }
        "thread/started" => {
            if let Some(thread_id) = event.thread_id.as_ref() {
                record_topology_change(inner, thread_id, TopologyChangeKind::Started);
            }
        }
        _ => {}
    }
}

fn ensure_event_generation(inner: &EventBridgeInner, generation: u64) -> Result<(), String> {
    if inner.generation == generation {
        Ok(())
    } else {
        Err("Codex thread activity belongs to a stale runtime generation".to_string())
    }
}

fn lifecycle_checkpoint_locked(
    inner: &EventBridgeInner,
    generation: u64,
    thread_id: &str,
) -> CodeThreadLifecycleDirtyCheckpoint {
    let thread_dirty = inner.lifecycle_dirty_revisions.get(thread_id).copied();
    let dirty = thread_dirty.is_some()
        || inner.lifecycle_generation_dirty && !inner.lifecycle_clean_threads.contains(thread_id);
    CodeThreadLifecycleDirtyCheckpoint {
        generation,
        thread_id: thread_id.to_string(),
        boundary_revision: inner.lifecycle_boundary_revision,
        graph_revision: inner.next_lifecycle_revision,
        thread_dirty,
        dirty,
    }
}

fn validate_exact_lifecycle_checkpoint_locked(
    inner: &EventBridgeInner,
    generation: u64,
    thread_id: &str,
    checkpoint: &CodeThreadLifecycleDirtyCheckpoint,
) -> Result<(), String> {
    if checkpoint.generation != generation
        || checkpoint.thread_id != thread_id
        || checkpoint.boundary_revision != inner.lifecycle_boundary_revision
        || checkpoint.thread_dirty != inner.lifecycle_dirty_revisions.get(thread_id).copied()
        || checkpoint.dirty != lifecycle_checkpoint_locked(inner, generation, thread_id).dirty
    {
        return Err("Codex exact thread lifecycle changed before native admission".to_string());
    }
    Ok(())
}

fn validate_expected_topology_changes_locked(
    inner: &EventBridgeInner,
    checkpoint_boundary_revision: u64,
    checkpoint_revision: u64,
    thread_id: &str,
    expected: CodeThreadLifecycleSignal,
) -> Result<usize, String> {
    if checkpoint_boundary_revision != inner.topology_boundary_revision
        || checkpoint_revision < inner.topology_boundary_revision
    {
        return Err("Codex topology history crossed its bounded safety boundary".to_string());
    }
    let mut expected_signals = 0_usize;
    for change in inner
        .topology_changes
        .iter()
        .filter(|change| change.revision > checkpoint_revision)
    {
        if change.thread_id != thread_id || change.kind != TopologyChangeKind::Lifecycle(expected) {
            return Err(
                "Codex topology changed incompatibly with the lifecycle mutation".to_string(),
            );
        }
        expected_signals = expected_signals.saturating_add(1);
    }
    Ok(expected_signals)
}

fn mark_lifecycle_clean_locked(
    inner: &mut EventBridgeInner,
    thread_id: &str,
) -> Result<(), String> {
    ensure_lifecycle_clean_capacity_locked(inner, thread_id)?;
    inner.lifecycle_dirty_revisions.remove(thread_id);
    inner.lifecycle_clean_threads.insert(thread_id.to_string());
    Ok(())
}

fn ensure_lifecycle_clean_capacity_locked(
    inner: &EventBridgeInner,
    thread_id: &str,
) -> Result<(), String> {
    if !inner.lifecycle_clean_threads.contains(thread_id)
        && inner.lifecycle_clean_threads.len() >= MAX_LIFECYCLE_DIRTY_THREADS
    {
        return Err(format!(
            "Codex lifecycle clean-thread limit of {MAX_LIFECYCLE_DIRTY_THREADS} was reached"
        ));
    }
    Ok(())
}

fn validate_new_thread_lifecycle_locked(
    inner: &EventBridgeInner,
    thread_id: &str,
) -> Result<(), String> {
    if !inner.lifecycle_generation_dirty
        || inner.lifecycle_boundary_revision != 0
        || inner.lifecycle_dirty_revisions.contains_key(thread_id)
        || inner.lifecycle_clean_threads.contains(thread_id)
    {
        return Err(
            "Codex new thread crossed an unverified lifecycle boundary or notification".to_string(),
        );
    }
    Ok(())
}

fn clear_event_activity(inner: &mut EventBridgeInner) {
    inner.active_turns.clear();
    inner.inflight_turn_starts.clear();
    inner.uncertain_turn_threads.clear();
    inner.turn_tombstones.clear();
    inner.turn_tombstone_order.clear();
    inner.thread_activity_revisions.clear();
    inner.next_activity_revision = 0;
    inner.next_turn_start_token = 0;
}

fn reset_lifecycle_dirty_boundary(inner: &mut EventBridgeInner) {
    inner.lifecycle_generation_dirty = true;
    inner.lifecycle_clean_threads.clear();
    inner.lifecycle_dirty_revisions.clear();
    inner.lifecycle_boundary_revision = 0;
    inner.next_lifecycle_revision = 0;
    inner.topology_changes.clear();
    inner.topology_boundary_revision = 0;
    inner.next_topology_revision = 0;
}

fn advance_lifecycle_dirty_boundary(inner: &mut EventBridgeInner) {
    inner.next_lifecycle_revision = inner.next_lifecycle_revision.saturating_add(1);
    inner.lifecycle_boundary_revision = inner.next_lifecycle_revision;
    inner.lifecycle_generation_dirty = true;
    inner.lifecycle_clean_threads.clear();
    inner.lifecycle_dirty_revisions.clear();
    advance_topology_boundary(inner);
}

fn advance_topology_boundary(inner: &mut EventBridgeInner) {
    inner.next_topology_revision = inner.next_topology_revision.saturating_add(1);
    inner.topology_boundary_revision = inner.next_topology_revision;
    inner.topology_changes.clear();
}

fn mark_lifecycle_dirty(
    inner: &mut EventBridgeInner,
    thread_id: &str,
    signal: CodeThreadLifecycleSignal,
) {
    if !inner.lifecycle_dirty_revisions.contains_key(thread_id)
        && inner.lifecycle_dirty_revisions.len() >= MAX_LIFECYCLE_DIRTY_THREADS
    {
        advance_lifecycle_dirty_boundary(inner);
    }
    inner.next_lifecycle_revision = inner.next_lifecycle_revision.saturating_add(1);
    record_topology_change(inner, thread_id, TopologyChangeKind::Lifecycle(signal));
    inner.lifecycle_dirty_revisions.insert(
        thread_id.to_string(),
        LifecycleDirtyRevision {
            revision: inner.next_lifecycle_revision,
            signal,
        },
    );
    inner.lifecycle_clean_threads.remove(thread_id);
}

fn record_topology_change(inner: &mut EventBridgeInner, thread_id: &str, kind: TopologyChangeKind) {
    inner.next_topology_revision = inner.next_topology_revision.saturating_add(1);
    if inner.topology_changes.len() == MAX_TOPOLOGY_CHANGES {
        if let Some(evicted) = inner.topology_changes.pop_front() {
            inner.topology_boundary_revision = evicted.revision;
        }
    }
    inner.topology_changes.push_back(TopologyChange {
        revision: inner.next_topology_revision,
        thread_id: thread_id.to_string(),
        kind,
    });
}

fn publish_locked(
    inner: &mut EventBridgeInner,
    generation: u64,
    draft: CodeWorkspaceEventDraft,
) -> Option<(CodeEventEmitter, CodeRuntimeEvent)> {
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
    update_active_turn_checkpoint(inner, &event);
    if inner.backlog.len() == MAX_NOTIFICATION_BACKLOG {
        inner.backlog.pop_front();
    }
    inner.backlog.push_back(event.clone());
    Some((Arc::clone(&inner.emitter), event))
}

fn remove_matching_turn_start(
    inner: &mut EventBridgeInner,
    thread_id: &str,
    token: u64,
) -> Result<InflightTurnStart, String> {
    if inner
        .inflight_turn_starts
        .get(thread_id)
        .map(|inflight| inflight.token)
        != Some(token)
    {
        return Err("Codex turn/start ordering token is no longer current".to_string());
    }
    inner
        .inflight_turn_starts
        .remove(thread_id)
        .ok_or_else(|| "Codex turn/start ordering state disappeared".to_string())
}

fn begin_turn_start_locked(inner: &mut EventBridgeInner, thread_id: &str) -> Result<u64, String> {
    if inner.inflight_turn_starts.contains_key(thread_id)
        || inner
            .active_turns
            .keys()
            .any(|(candidate, _)| candidate == thread_id)
        || inner.uncertain_turn_threads.contains(thread_id)
    {
        return Err("Codex thread already has active or uncertain turn state".to_string());
    }
    let token = inner
        .next_turn_start_token
        .checked_add(1)
        .ok_or_else(|| "Codex turn/start ordering token was exhausted".to_string())?;
    inner.next_turn_start_token = token;
    inner.inflight_turn_starts.insert(
        thread_id.to_string(),
        InflightTurnStart {
            token,
            terminal_turn_ids: HashSet::new(),
            terminal_overflow: false,
            thread_closed: false,
        },
    );
    bump_thread_activity(inner, thread_id);
    Ok(token)
}

fn fail_turn_start_locked(
    inner: &mut EventBridgeInner,
    thread_id: &str,
    token: u64,
    uncertain_delivery: bool,
) -> Result<(), String> {
    let _inflight = remove_matching_turn_start(inner, thread_id, token)?;
    if uncertain_delivery {
        inner.uncertain_turn_threads.insert(thread_id.to_string());
    }
    bump_thread_activity(inner, thread_id);
    Ok(())
}

fn bump_thread_activity(inner: &mut EventBridgeInner, thread_id: &str) {
    inner.next_activity_revision = inner.next_activity_revision.saturating_add(1);
    inner
        .thread_activity_revisions
        .insert(thread_id.to_string(), inner.next_activity_revision);
}

fn insert_turn_tombstone(inner: &mut EventBridgeInner, key: (String, String)) {
    if !inner.turn_tombstones.insert(key.clone()) {
        return;
    }
    if inner.turn_tombstone_order.len() == MAX_TURN_TOMBSTONES {
        if let Some(expired) = inner.turn_tombstone_order.pop_front() {
            inner.turn_tombstones.remove(&expired);
        }
    }
    inner.turn_tombstone_order.push_back(key);
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
            #[cfg(test)]
            fail_next_fork_before_write: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_executable(path: PathBuf) -> Self {
        let mut runtime = Self::new();
        runtime.explicit_executable = Some(path);
        runtime
    }

    #[cfg(test)]
    pub(crate) fn fail_next_fork_before_write_for_test(&self) {
        self.fail_next_fork_before_write
            .store(true, Ordering::Release);
    }

    pub fn probe(&self) -> CodeRuntimeProbe {
        let probe = probe_codex(self.explicit_executable.as_deref());
        let egress_probe = probe.redacted_for_egress();
        if let Ok(mut inner) = self.inner.lock() {
            inner.probe = Some(probe.clone());
            if inner.process.is_none() {
                inner.phase = if probe.available {
                    CodeRuntimePhase::Stopped
                } else {
                    CodeRuntimePhase::NotInstalled
                };
                inner.last_error = egress_probe.error.clone();
            }
        }
        egress_probe
    }

    pub fn status(&self) -> Result<CodeRuntimeStatus, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        Ok(status_from_inner(&inner, self.events.len()))
    }

    pub(crate) fn replace_emitter_if_ready(
        &self,
        emitter: CodeEventEmitter,
    ) -> Result<Option<CodeRuntimeStatus>, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Ok(None);
        }
        self.events.replace_emitter(emitter)?;
        Ok(Some(status_from_inner(&inner, self.events.len())))
    }

    pub(crate) fn start(&self, emitter: CodeEventEmitter) -> Result<CodeRuntimeStatus, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase == CodeRuntimePhase::Ready {
            self.events.replace_emitter(emitter)?;
            return Ok(status_from_inner(&inner, self.events.len()));
        }
        if let Err(error) = stop_runtime_process(&mut inner) {
            let detail =
                format!("failed to verify shutdown of the previous Codex app-server: {error}");
            inner.phase = CodeRuntimePhase::Failed;
            inner.initialized = None;
            inner.last_error = Some(detail.clone());
            return Err(detail);
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
                .redacted_for_egress()
                .error
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
                inner.process = Some(process);
                let stop_error = stop_runtime_process(&mut inner).err();
                self.approvals.clear_generation(generation);
                let mut detail = if stderr.trim().is_empty() {
                    error
                } else {
                    format!("{error} ({})", first_line(&stderr))
                };
                if let Some(stop_error) = stop_error {
                    detail.push_str(&format!(
                        "; failed to verify app-server shutdown: {stop_error}"
                    ));
                }
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
        let result = stop_runtime_process(&mut inner);
        self.events.clear_activity(generation)?;
        self.approvals.clear_generation(generation);
        inner.initialized = None;
        if let Err(error) = result {
            inner.phase = CodeRuntimePhase::Failed;
            inner.last_error = Some(error.clone());
            return Err(error);
        }
        inner.phase = CodeRuntimePhase::Stopped;
        inner.last_error = None;
        Ok(status_from_inner(&inner, self.events.len()))
    }

    pub(crate) fn events(
        &self,
        runtime_generation: Option<u64>,
        after_sequence: Option<u64>,
    ) -> Result<CodeRuntimeEventBacklog, String> {
        self.events
            .snapshot(&self.approvals, runtime_generation, after_sequence)
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
        let requirement = input
            .model
            .as_deref()
            .map(CodeModelCatalogRequirement::Model);
        let (_, pending) = self.begin_ready_request("thread/start", params, requirement)?;
        let result = pending.wait(REQUEST_TIMEOUT)?;
        protocol::parse_thread_open(result).map_err(CodeThreadStartRpcError::Uncertain)
    }

    /// Return a bounded, strict visible model catalog for one ready generation.
    pub(crate) fn model_catalog(&self) -> Result<CodeModelCatalogSnapshot, String> {
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        let process = runtime
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        collect_model_catalog_from_process(runtime.generation, process)
    }

    /// Fork one exact stable-active source into a native-owned destination.
    /// Lifecycle, turn, and approval admission stay serialized through the
    /// JSON-RPC byte write; delivery after that point is deliberately sticky.
    pub(crate) fn thread_fork_guarded(
        &self,
        input: &CodeThreadForkInput,
        workspace_root: &str,
        preparation_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeThreadForkGuardedResult, CodeRpcDeliveryError> {
        let workspace_root =
            canonical_workspace_root(workspace_root).map_err(CodeRpcDeliveryError::NotSent)?;
        let params = input
            .rpc_params(&workspace_root, preparation_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let (pending, completion) =
            self.begin_fork_request(&input.thread_id, checkpoint, params)?;
        let result = pending.wait(REQUEST_TIMEOUT)?;
        let opened =
            protocol::parse_thread_open(result).map_err(CodeRpcDeliveryError::Uncertain)?;
        Ok(CodeThreadForkGuardedResult { opened, completion })
    }

    pub(crate) fn ensure_ready(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn thread_resume_at(
        &self,
        input: CodeThreadResumeInput,
        workspace_root: &str,
    ) -> Result<CodeThreadRpcOpenResult, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let requirement = input
            .model
            .as_deref()
            .map(CodeModelCatalogRequirement::Model);
        let (generation, pending) = self
            .begin_ready_request("thread/resume", params, requirement)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let resumed = match protocol::parse_thread_open(result) {
            Ok(resumed) => resumed,
            Err(error) => {
                let _ = self
                    .events
                    .mark_thread_uncertain(generation, &input.thread_id);
                return Err(error);
            }
        };
        if resumed.thread.id != input.thread_id {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err("Codex returned a different thread while resuming".to_string());
        }
        if let Err(error) = self
            .events
            .reconcile_thread_summary(generation, &resumed.thread)
        {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err(error);
        }
        Ok(resumed)
    }

    /// Resume one durable active thread with lifecycle validation held through
    /// JSON-RPC byte admission.
    pub(crate) fn thread_resume_at_guarded(
        &self,
        input: CodeThreadResumeInput,
        workspace_root: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeThreadRpcOpenResult, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let requirement = input
            .model
            .as_deref()
            .map(CodeModelCatalogRequirement::Model);
        let (generation, pending) = self
            .begin_active_request(
                &input.thread_id,
                checkpoint,
                "thread/resume",
                params,
                requirement,
            )
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let resumed = match protocol::parse_thread_open(result) {
            Ok(resumed) => resumed,
            Err(error) => {
                let _ = self
                    .events
                    .mark_thread_uncertain(generation, &input.thread_id);
                return Err(error);
            }
        };
        if resumed.thread.id != input.thread_id {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err("Codex returned a different thread while resuming".to_string());
        }
        if let Err(error) = self
            .events
            .reconcile_thread_summary(generation, &resumed.thread)
        {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err(error);
        }
        Ok(resumed)
    }

    /// Resume one unbound recovery candidate while the current-generation new
    /// thread seam remains free of an exact lifecycle signal through RPC write.
    pub(crate) fn thread_resume_recovery_at_guarded(
        &self,
        input: CodeThreadResumeInput,
        workspace_root: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeThreadRpcOpenResult, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let requirement = input
            .model
            .as_deref()
            .map(CodeModelCatalogRequirement::Model);
        let (generation, pending) = self
            .begin_recovery_resume_request(&input.thread_id, checkpoint, params, requirement)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let resumed = match protocol::parse_thread_open(result) {
            Ok(resumed) => resumed,
            Err(error) => {
                let _ = self
                    .events
                    .mark_thread_uncertain(generation, &input.thread_id);
                return Err(error);
            }
        };
        if resumed.thread.id != input.thread_id {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err("Codex returned a different thread while resuming".to_string());
        }
        if let Err(error) = self
            .events
            .reconcile_thread_summary(generation, &resumed.thread)
        {
            let _ = self
                .events
                .mark_thread_uncertain(generation, &input.thread_id);
            return Err(error);
        }
        Ok(resumed)
    }

    pub(crate) fn thread_read(&self, thread_id: &str) -> Result<CodeThreadSummary, String> {
        let params = protocol::thread_read_params(thread_id)?;
        let result = self.request_ready("thread/read", params)?;
        protocol::parse_thread_read(result)
    }

    /// Read normalized timeline metadata with the pinned `includeTurns:true`
    /// contract for rows that cannot be resumed to hydrate their turns.
    pub(crate) fn thread_read_with_turns(
        &self,
        thread_id: &str,
    ) -> Result<CodeThreadSummary, String> {
        let params = authoritative_thread_read_params(thread_id)?;
        let result = self.request_ready("thread/read", params)?;
        protocol::parse_thread_read(result)
    }

    /// Fetch a complete cwd-free active+archived snapshot from one runtime
    /// generation and validate its pinned ancestry graph.
    pub(crate) fn authoritative_thread_graph(
        &self,
        deferred_target_ids: &[String],
        pending_forks: &[CodePendingForkExpectation],
    ) -> Result<CodeAuthoritativeThreadGraph, String> {
        let deferred_targets = validate_deferred_targets(deferred_target_ids)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        let generation = inner.generation;
        let (topology_boundary_revision, topology_revision) =
            self.events.topology_checkpoint(generation)?;
        let process = inner
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        let graph = collect_authoritative_thread_graph_with_pending_forks(
            &deferred_targets,
            pending_forks,
            |method, params| process.request(method, params, REQUEST_TIMEOUT),
        )?;
        self.events.confirm_topology_checkpoint(
            generation,
            topology_boundary_revision,
            topology_revision,
        )?;
        Ok(graph)
    }

    /// Fetch one exhaustive graph after idle/terminal drain and retain the
    /// topology epoch that a guarded lifecycle write must consume.
    pub(crate) fn authoritative_thread_graph_for_lifecycle_admission(
        &self,
        deferred_target_ids: &[String],
        pending_forks: &[CodePendingForkExpectation],
        thread_id: &str,
    ) -> Result<CodeThreadLifecycleGraphProof, String> {
        protocol::validate_id("lifecycle graph target", thread_id)?;
        let deferred_targets = validate_deferred_targets(deferred_target_ids)?;
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        let generation = inner.generation;
        let (topology_boundary_revision, topology_revision) =
            self.events.topology_checkpoint(generation)?;
        let process = inner
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        let graph = collect_authoritative_thread_graph_with_pending_forks(
            &deferred_targets,
            pending_forks,
            |method, params| process.request(method, params, REQUEST_TIMEOUT),
        )?;
        self.events.confirm_topology_checkpoint(
            generation,
            topology_boundary_revision,
            topology_revision,
        )?;
        Ok(CodeThreadLifecycleGraphProof {
            generation,
            thread_id: thread_id.to_string(),
            graph,
            topology_boundary_revision,
            topology_revision,
        })
    }

    /// Atomically consume an exhaustive leaf proof, native idle state, and the
    /// no-approval gate through the `thread/archive` JSON-RPC byte write.
    pub(crate) fn thread_archive_guarded(
        &self,
        input: &CodeThreadLifecycleInput,
        proof: CodeThreadLifecycleGraphProof,
    ) -> Result<CodeThreadLifecycleMutationCompletion, CodeRpcDeliveryError> {
        let params = input.rpc_params().map_err(CodeRpcDeliveryError::NotSent)?;
        let (pending, receipt) = self.begin_lifecycle_mutation(
            input,
            proof,
            CodeThreadLifecycleSignal::Archived,
            params,
        )?;
        let result = pending.wait(REQUEST_TIMEOUT)?;
        parse_thread_archive(result).map_err(CodeRpcDeliveryError::Uncertain)?;
        self.events
            .mutation_response_checkpoint(receipt)
            .map_err(CodeRpcDeliveryError::Uncertain)
    }

    /// Atomically consume an exhaustive membership proof and native gates
    /// through the `thread/unarchive` JSON-RPC byte write.
    pub(crate) fn thread_unarchive_guarded(
        &self,
        input: &CodeThreadLifecycleInput,
        proof: CodeThreadLifecycleGraphProof,
    ) -> Result<CodeThreadUnarchiveGuardedResult, CodeRpcDeliveryError> {
        let params = input.rpc_params().map_err(CodeRpcDeliveryError::NotSent)?;
        let (pending, receipt) = self.begin_lifecycle_mutation(
            input,
            proof,
            CodeThreadLifecycleSignal::Unarchived,
            params,
        )?;
        let result = pending.wait(REQUEST_TIMEOUT)?;
        let thread = parse_thread_unarchive(result).map_err(CodeRpcDeliveryError::Uncertain)?;
        let completion = self
            .events
            .mutation_response_checkpoint(receipt)
            .map_err(CodeRpcDeliveryError::Uncertain)?;
        Ok(CodeThreadUnarchiveGuardedResult { thread, completion })
    }

    /// Consume a successful archive completion only after the matching durable
    /// binding transition is committed.
    pub(crate) fn complete_thread_archive_lifecycle<T>(
        &self,
        thread_id: &str,
        completion: CodeThreadLifecycleMutationCompletion,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != completion.generation {
            return Err(
                "Codex archive completion belongs to an inactive runtime generation".to_string(),
            );
        }
        self.events.complete_lifecycle_mutation(
            thread_id,
            completion,
            CodeThreadLifecycleSignal::Archived,
            commit,
        )
    }

    /// Consume a successful unarchive completion only after the matching
    /// durable binding transition is committed.
    pub(crate) fn complete_thread_unarchive_lifecycle<T>(
        &self,
        thread_id: &str,
        completion: CodeThreadLifecycleMutationCompletion,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != completion.generation {
            return Err(
                "Codex unarchive completion belongs to an inactive runtime generation".to_string(),
            );
        }
        self.events.complete_lifecycle_mutation(
            thread_id,
            completion,
            CodeThreadLifecycleSignal::Unarchived,
            commit,
        )
    }

    /// Whether a current-generation approval is pending or response-reserved
    /// for one exact thread.
    #[cfg(test)]
    pub fn has_pending_approval(&self, thread_id: &str) -> Result<bool, String> {
        let generation = self.ready_generation()?;
        self.approvals.has_for_thread(generation, thread_id)
    }

    #[cfg(test)]
    pub(crate) fn insert_pending_approval_for_test(
        &self,
        generation: u64,
        request_id: &str,
        thread_id: &str,
    ) -> Result<(), String> {
        let inserted = self.approvals.insert_request(
            generation,
            json!(request_id),
            "item/fileChange/requestApproval",
            Some(json!({
                "threadId": thread_id,
                "turnId": format!("turn-{request_id}"),
                "itemId": format!("item-{request_id}"),
                "availableDecisions": ["accept", "decline"]
            })),
        )?;
        if inserted.is_none() {
            return Err("test approval request was not normalized".to_string());
        }
        Ok(())
    }

    /// Whether a lifecycle notification or runtime boundary requires durable
    /// reconciliation before this thread may use an active-only command.
    #[cfg(test)]
    pub(crate) fn is_thread_lifecycle_dirty(&self, thread_id: &str) -> Result<bool, String> {
        Ok(self
            .thread_lifecycle_dirty_checkpoint(thread_id)?
            .is_dirty())
    }

    /// Capture the exact generation/revision that a durable lifecycle
    /// reconciliation must cover before clearing the native dirty gate.
    pub(crate) fn thread_lifecycle_dirty_checkpoint(
        &self,
        thread_id: &str,
    ) -> Result<CodeThreadLifecycleDirtyCheckpoint, String> {
        let generation = self.ready_generation()?;
        self.events
            .lifecycle_dirty_checkpoint(generation, thread_id)
    }

    /// Clear one exact dirty gate only if no lifecycle notification or runtime
    /// boundary occurred since the supplied reconciliation checkpoint.
    pub(crate) fn clear_thread_lifecycle_dirty(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<(), String> {
        let generation = self.ready_generation()?;
        self.events
            .clear_lifecycle_dirty(generation, thread_id, checkpoint)
    }

    /// Atomically validate the current-generation creation seam, commit one
    /// durable binding, and make that exact new thread lifecycle-clean.
    pub(crate) fn commit_new_thread_lifecycle<T>(
        &self,
        thread_id: &str,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        protocol::validate_id("new lifecycle thread", thread_id)?;
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready for a new thread commit".to_string());
        }
        let mut events = self
            .events
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        ensure_event_generation(&events, runtime.generation)?;
        validate_new_thread_lifecycle_locked(&events, thread_id)?;
        ensure_lifecycle_clean_capacity_locked(&events, thread_id)?;
        let committed = commit()?;
        events.lifecycle_clean_threads.insert(thread_id.to_string());
        Ok(committed)
    }

    /// Atomically prove the fork source stayed lifecycle/activity-clean after
    /// request admission, commit the exact destination binding, and mark only
    /// that child as a newly clean thread.
    pub(crate) fn commit_new_fork_lifecycle<T>(
        &self,
        source_thread_id: &str,
        child_thread_id: &str,
        completion: CodeThreadForkCompletion,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        protocol::validate_id("fork source thread", source_thread_id)?;
        protocol::validate_id("fork child thread", child_thread_id)?;
        if source_thread_id == child_thread_id {
            return Err("Codex fork child cannot reuse its source thread id".to_string());
        }
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready
            || runtime.generation != completion.generation
            || completion.source_thread_id != source_thread_id
        {
            return Err("Codex fork completion belongs to a stale source generation".to_string());
        }
        let mut events = self
            .events
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        ensure_event_generation(&events, completion.generation)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            completion.generation,
            source_thread_id,
            &completion.lifecycle_checkpoint,
        )?;
        if completion.lifecycle_checkpoint.dirty
            || events
                .thread_activity_revisions
                .get(source_thread_id)
                .copied()
                .unwrap_or_default()
                != completion.activity_revision
            || events
                .active_turns
                .keys()
                .any(|(thread_id, _)| thread_id == source_thread_id)
            || events.inflight_turn_starts.contains_key(source_thread_id)
            || events.uncertain_turn_threads.contains(source_thread_id)
        {
            return Err(
                "Codex fork source changed after request admission; destination commit was refused"
                    .to_string(),
            );
        }
        validate_new_thread_lifecycle_locked(&events, child_thread_id)?;
        ensure_lifecycle_clean_capacity_locked(&events, child_thread_id)?;
        let committed = commit()?;
        events
            .lifecycle_clean_threads
            .insert(child_thread_id.to_string());
        Ok(committed)
    }

    /// Hold the exact native lifecycle barrier from checkpoint validation
    /// through one non-RPC action such as PTY spawn/registration or stdin ack.
    pub(crate) fn with_thread_lifecycle_admission<T>(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        protocol::validate_id("active-only admission thread", thread_id)?;
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != checkpoint.generation {
            return Err(
                "Codex active-only admission belongs to an inactive runtime generation".to_string(),
            );
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        ensure_event_generation(&events, checkpoint.generation)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            checkpoint.generation,
            thread_id,
            &checkpoint,
        )?;
        if checkpoint.dirty {
            return Err(
                "Codex thread lifecycle is dirty and cannot admit an active-only operation"
                    .to_string(),
            );
        }
        action()
    }

    /// Prove the target quiescent with a strict `thread/read` while guarding
    /// against response/notification activity races and pending approvals.
    pub(crate) fn ensure_thread_idle(
        &self,
        thread_id: &str,
    ) -> Result<CodeAuthoritativeThreadActivity, String> {
        protocol::validate_id("archive target thread", thread_id)?;
        let generation = self.ready_generation()?;
        let before = self.events.activity_snapshot(generation, thread_id)?;
        if before.active_or_starting || before.uncertain {
            return Err("Codex thread has active, starting, or uncertain turn state".to_string());
        }
        if self.approvals.has_for_thread(generation, thread_id)? {
            return Err("Codex thread has a pending approval".to_string());
        }
        let params = authoritative_thread_read_params(thread_id)?;
        let result = self.request_ready_at_generation(generation, "thread/read", params)?;
        let activity = parse_authoritative_thread_read(result)?;
        if activity.id != thread_id {
            return Err("Codex authoritative read returned a different thread".to_string());
        }
        activity.ensure_quiescent()?;
        if self.approvals.has_for_thread(generation, thread_id)? {
            return Err("Codex thread gained a pending approval during the idle proof".to_string());
        }
        self.events
            .confirm_authoritative_idle(generation, thread_id, before.revision)?;
        Ok(activity)
    }

    /// Retain the exact runtime/activity/approval locks after an authoritative
    /// idle proof so no turn or approval can be admitted during a private
    /// physical-removal claim.
    pub(crate) fn lock_thread_idle_admission(
        &self,
        thread_id: &str,
    ) -> Result<CodeThreadIdleAdmissionGuard<'_>, String> {
        protocol::validate_id("removal admission thread", thread_id)?;
        let runtime = self.inner.lock().map_err(|error| error.to_string())?;
        if runtime.phase != CodeRuntimePhase::Ready {
            return Err("Codex runtime is not ready for removal admission".to_string());
        }
        let generation = runtime.generation;
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        ensure_event_generation(&events, generation)?;
        if events
            .active_turns
            .keys()
            .any(|(candidate, _)| candidate == thread_id)
            || events.inflight_turn_starts.contains_key(thread_id)
            || events.uncertain_turn_threads.contains(thread_id)
        {
            return Err("Codex thread gained activity before removal admission".to_string());
        }
        let approvals = self
            .approvals
            .lock_without_thread_approval(generation, thread_id)?;
        Ok(CodeThreadIdleAdmissionGuard {
            _runtime: runtime,
            _events: events,
            _approvals: approvals,
        })
    }

    /// Set one validated title and then read back authoritative thread metadata.
    #[cfg(test)]
    pub(crate) fn thread_rename(
        &self,
        input: &CodeThreadRenameInput,
    ) -> Result<CodeThreadSummary, String> {
        let params = input.rpc_params()?;
        let result = self.request_ready("thread/name/set", params)?;
        protocol::parse_thread_name_set(result)?;
        self.thread_read(&input.thread_id)
    }

    /// Rename one stable active or archived thread with exact lifecycle state
    /// held through the `thread/name/set` JSON-RPC byte write.
    pub(crate) fn thread_rename_guarded(
        &self,
        input: &CodeThreadRenameInput,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeThreadSummary, String> {
        let params = input.rpc_params()?;
        let (_, pending) = self
            .begin_active_request(
                &input.thread_id,
                checkpoint,
                "thread/name/set",
                params,
                None,
            )
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
        protocol::parse_thread_name_set(result)?;
        self.thread_read(&input.thread_id)
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

    #[cfg(test)]
    pub(crate) fn turn_start_at(
        &self,
        input: CodeTurnStartInput,
        workspace_root: &str,
    ) -> Result<CodeTurnSummary, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let selection = turn_selection(input.model.as_deref(), input.effort.as_deref())?;
        let (generation, token, pending) = {
            let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
            refresh_process_health(&mut runtime, &self.approvals, &self.events);
            if runtime.phase != CodeRuntimePhase::Ready {
                return Err("Codex app-server is not ready".to_string());
            }
            let generation = runtime.generation;
            let process = runtime
                .process
                .as_ref()
                .ok_or_else(|| "Codex app-server is not running".to_string())?;
            if let Some(selection) = selection.as_ref() {
                collect_model_catalog_from_process(generation, process)?
                    .require_selection(selection)?;
            }
            let mut events = self
                .events
                .inner
                .lock()
                .map_err(|error| error.to_string())?;
            ensure_event_generation(&events, generation)?;
            let token = begin_turn_start_locked(&mut events, &input.thread_id)?;
            let pending = match process.begin_request_with_delivery("turn/start", params) {
                Ok(pending) => pending,
                Err(error) => {
                    let uncertain_delivery = !error.definitely_not_sent();
                    let message = error.into_message();
                    if let Err(cleanup_error) = fail_turn_start_locked(
                        &mut events,
                        &input.thread_id,
                        token,
                        uncertain_delivery,
                    ) {
                        return Err(format!(
                            "{message}; turn/start state cleanup failed: {cleanup_error}"
                        ));
                    }
                    return Err(message);
                }
            };
            (generation, token, pending)
        };
        let result = match pending.wait(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(error) => {
                let uncertain_delivery = !error.definitely_not_sent();
                let message = error.into_message();
                if let Err(cleanup_error) = self.events.fail_turn_start(
                    generation,
                    &input.thread_id,
                    token,
                    uncertain_delivery,
                ) {
                    return Err(format!(
                        "{message}; turn/start state cleanup failed: {cleanup_error}"
                    ));
                }
                return Err(message);
            }
        };
        let turn = match protocol::parse_turn_start(result) {
            Ok(turn) => turn,
            Err(error) => {
                let _ = self
                    .events
                    .fail_turn_start(generation, &input.thread_id, token, true);
                return Err(error);
            }
        };
        let status = match CodePinnedTurnStatus::parse(&turn.status) {
            Ok(status) => status,
            Err(error) => {
                let _ = self
                    .events
                    .fail_turn_start(generation, &input.thread_id, token, true);
                return Err(error);
            }
        };
        self.events
            .complete_turn_start(generation, &input.thread_id, token, &turn.id, status)?;
        Ok(turn)
    }

    /// Start a turn with lifecycle validation, native active-turn reservation,
    /// and JSON-RPC byte admission serialized under one event barrier.
    pub(crate) fn turn_start_at_guarded(
        &self,
        input: CodeTurnStartInput,
        workspace_root: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeTurnSummary, String> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let params = input.rpc_params(&workspace_root)?;
        let selection = turn_selection(input.model.as_deref(), input.effort.as_deref())?;
        let (generation, token, pending) = {
            let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
            refresh_process_health(&mut runtime, &self.approvals, &self.events);
            if runtime.phase != CodeRuntimePhase::Ready
                || runtime.generation != checkpoint.generation
            {
                return Err(
                    "Codex turn/start belongs to an inactive runtime generation".to_string()
                );
            }
            let process = runtime
                .process
                .as_ref()
                .ok_or_else(|| "Codex app-server is not running".to_string())?;
            if let Some(selection) = selection.as_ref() {
                collect_model_catalog_from_process(checkpoint.generation, process)?
                    .require_selection(selection)?;
            }
            let mut events = self
                .events
                .inner
                .lock()
                .map_err(|error| error.to_string())?;
            ensure_event_generation(&events, checkpoint.generation)?;
            validate_exact_lifecycle_checkpoint_locked(
                &events,
                checkpoint.generation,
                &input.thread_id,
                &checkpoint,
            )?;
            if checkpoint.dirty {
                return Err(
                    "Codex thread lifecycle is dirty and cannot admit turn/start".to_string(),
                );
            }
            let token = begin_turn_start_locked(&mut events, &input.thread_id)?;
            let pending = match process.begin_request_with_delivery("turn/start", params) {
                Ok(pending) => pending,
                Err(error) => {
                    let uncertain_delivery = !error.definitely_not_sent();
                    let message = error.into_message();
                    if let Err(cleanup_error) = fail_turn_start_locked(
                        &mut events,
                        &input.thread_id,
                        token,
                        uncertain_delivery,
                    ) {
                        return Err(format!(
                            "{message}; turn/start state cleanup failed: {cleanup_error}"
                        ));
                    }
                    return Err(message);
                }
            };
            (checkpoint.generation, token, pending)
        };
        let result = match pending.wait(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(error) => {
                let message = error.into_message();
                if let Err(cleanup_error) =
                    self.events
                        .fail_turn_start(generation, &input.thread_id, token, true)
                {
                    return Err(format!(
                        "{message}; turn/start state cleanup failed: {cleanup_error}"
                    ));
                }
                return Err(message);
            }
        };
        let turn = match protocol::parse_turn_start(result) {
            Ok(turn) => turn,
            Err(error) => {
                let _ = self
                    .events
                    .fail_turn_start(generation, &input.thread_id, token, true);
                return Err(error);
            }
        };
        let status = match CodePinnedTurnStatus::parse(&turn.status) {
            Ok(status) => status,
            Err(error) => {
                let _ = self
                    .events
                    .fail_turn_start(generation, &input.thread_id, token, true);
                return Err(error);
            }
        };
        self.events
            .complete_turn_start(generation, &input.thread_id, token, &turn.id, status)?;
        Ok(turn)
    }

    #[cfg(test)]
    pub fn turn_steer(&self, input: CodeTurnSteerInput) -> Result<CodeTurnSummary, String> {
        let params = input.rpc_params()?;
        let result = self.request_ready("turn/steer", params)?;
        protocol::parse_turn_steer(result)
    }

    /// Steer an active turn only while the exact bound thread remains in the
    /// lifecycle state covered by `checkpoint` through JSON-RPC byte admission.
    pub(crate) fn turn_steer_guarded(
        &self,
        input: CodeTurnSteerInput,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<CodeTurnSummary, String> {
        let params = input.rpc_params()?;
        let (_, pending) = self
            .begin_active_request(&input.thread_id, checkpoint, "turn/steer", params, None)
            .map_err(CodeRpcDeliveryError::into_message)?;
        let result = pending
            .wait(REQUEST_TIMEOUT)
            .map_err(CodeRpcDeliveryError::into_message)?;
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

    #[cfg(test)]
    pub fn approval_respond(&self, input: CodeApprovalResponseInput) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
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

    /// Respond to one approval with exact lifecycle validation and response
    /// byte admission serialized against native lifecycle notifications.
    pub(crate) fn approval_respond_guarded(
        &self,
        input: CodeApprovalResponseInput,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
    ) -> Result<(), String> {
        let mut runtime = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready
            || runtime.generation != input.runtime_generation
            || runtime.generation != checkpoint.generation
        {
            return Err("Codex approval belongs to an inactive runtime generation".to_string());
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| error.to_string())?;
        ensure_event_generation(&events, checkpoint.generation)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            checkpoint.generation,
            &input.thread_id,
            &checkpoint,
        )?;
        if checkpoint.dirty {
            return Err(
                "Codex thread lifecycle is dirty and cannot admit an approval response".to_string(),
            );
        }
        let process = runtime
            .process
            .as_ref()
            .ok_or_else(|| "Codex app-server is not running".to_string())?;
        respond_to_pending_approval(&self.approvals, &input, |request_id, result| {
            process.respond(request_id, result)
        })
    }

    fn begin_lifecycle_mutation(
        &self,
        input: &CodeThreadLifecycleInput,
        proof: CodeThreadLifecycleGraphProof,
        expected: CodeThreadLifecycleSignal,
        params: Value,
    ) -> Result<(PendingRuntimeRequest, LifecycleWriteReceipt), CodeRpcDeliveryError> {
        if proof.thread_id != input.thread_id {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex lifecycle graph proof belongs to a different thread".to_string(),
            ));
        }
        let membership = proof
            .graph
            .ensure_leaf(&proof.thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let expected_membership = match expected {
            CodeThreadLifecycleSignal::Archived => CodeThreadMembership::Active,
            CodeThreadLifecycleSignal::Unarchived => CodeThreadMembership::Archived,
        };
        if membership != expected_membership {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex authoritative graph membership does not authorize the lifecycle mutation"
                    .to_string(),
            ));
        }
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != proof.generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex lifecycle graph proof belongs to an inactive runtime generation".to_string(),
            ));
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        ensure_event_generation(&events, proof.generation)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        if events.topology_boundary_revision != proof.topology_boundary_revision
            || events.next_topology_revision != proof.topology_revision
        {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread topology changed after the exhaustive leaf proof".to_string(),
            ));
        }
        let lifecycle = lifecycle_checkpoint_locked(&events, proof.generation, &proof.thread_id);
        if lifecycle.dirty {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread lifecycle is not durably reconciled for mutation admission"
                    .to_string(),
            ));
        }
        if events
            .active_turns
            .keys()
            .any(|(thread_id, _)| thread_id == &proof.thread_id)
            || events.inflight_turn_starts.contains_key(&proof.thread_id)
            || events.uncertain_turn_threads.contains(&proof.thread_id)
        {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread gained active or uncertain turn state before lifecycle mutation"
                    .to_string(),
            ));
        }
        let _approval_guard = self
            .approvals
            .lock_without_thread_approval(proof.generation, &proof.thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        let method = match expected {
            CodeThreadLifecycleSignal::Archived => "thread/archive",
            CodeThreadLifecycleSignal::Unarchived => "thread/unarchive",
        };
        let pending = process.begin_request_with_delivery(method, params)?;
        let receipt = LifecycleWriteReceipt {
            generation: proof.generation,
            thread_id: proof.thread_id,
            expected,
            lifecycle_boundary_revision: events.lifecycle_boundary_revision,
            topology_boundary_revision: events.topology_boundary_revision,
            topology_revision: events.next_topology_revision,
        };
        Ok((pending, receipt))
    }

    fn begin_active_request(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
        method: &str,
        params: Value,
        model_requirement: Option<CodeModelCatalogRequirement<'_>>,
    ) -> Result<(u64, PendingRuntimeRequest), CodeRpcDeliveryError> {
        protocol::validate_id("active-only request thread", thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != checkpoint.generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex active-only request belongs to an inactive runtime generation".to_string(),
            ));
        }
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        if let Some(requirement) = model_requirement {
            let catalog = collect_model_catalog_from_process(checkpoint.generation, process)
                .map_err(CodeRpcDeliveryError::NotSent)?;
            requirement
                .validate(&catalog)
                .map_err(CodeRpcDeliveryError::NotSent)?;
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        ensure_event_generation(&events, checkpoint.generation)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            checkpoint.generation,
            thread_id,
            &checkpoint,
        )
        .map_err(CodeRpcDeliveryError::NotSent)?;
        if checkpoint.dirty {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread lifecycle is dirty and cannot admit an active-only request"
                    .to_string(),
            ));
        }
        let pending = process.begin_request_with_delivery(method, params)?;
        Ok((checkpoint.generation, pending))
    }

    fn begin_fork_request(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
        params: Value,
    ) -> Result<(PendingRuntimeRequest, CodeThreadForkCompletion), CodeRpcDeliveryError> {
        protocol::validate_id("fork source thread", thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != checkpoint.generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex fork belongs to an inactive runtime generation".to_string(),
            ));
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        ensure_event_generation(&events, checkpoint.generation)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            checkpoint.generation,
            thread_id,
            &checkpoint,
        )
        .map_err(CodeRpcDeliveryError::NotSent)?;
        if checkpoint.dirty {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex thread lifecycle is dirty and cannot admit thread/fork".to_string(),
            ));
        }
        if events
            .active_turns
            .keys()
            .any(|(active_thread_id, _)| active_thread_id == thread_id)
            || events.inflight_turn_starts.contains_key(thread_id)
            || events.uncertain_turn_threads.contains(thread_id)
        {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex source thread gained active or uncertain turn state before fork".to_string(),
            ));
        }
        let _approval_guard = self
            .approvals
            .lock_without_thread_approval(checkpoint.generation, thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        let activity_revision = events
            .thread_activity_revisions
            .get(thread_id)
            .copied()
            .unwrap_or_default();
        #[cfg(test)]
        if self
            .fail_next_fork_before_write
            .swap(false, Ordering::AcqRel)
        {
            return Err(CodeRpcDeliveryError::NotSent(
                "injected Codex fork failure before byte admission".to_string(),
            ));
        }
        let pending = process.begin_request_with_delivery("thread/fork", params)?;
        Ok((
            pending,
            CodeThreadForkCompletion {
                generation: checkpoint.generation,
                source_thread_id: thread_id.to_string(),
                lifecycle_checkpoint: checkpoint,
                activity_revision,
            },
        ))
    }

    fn begin_recovery_resume_request(
        &self,
        thread_id: &str,
        checkpoint: CodeThreadLifecycleDirtyCheckpoint,
        params: Value,
        model_requirement: Option<CodeModelCatalogRequirement<'_>>,
    ) -> Result<(u64, PendingRuntimeRequest), CodeRpcDeliveryError> {
        protocol::validate_id("recovery resume thread", thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready || runtime.generation != checkpoint.generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex recovery resume belongs to an inactive runtime generation".to_string(),
            ));
        }
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        if let Some(requirement) = model_requirement {
            let catalog = collect_model_catalog_from_process(checkpoint.generation, process)
                .map_err(CodeRpcDeliveryError::NotSent)?;
            requirement
                .validate(&catalog)
                .map_err(CodeRpcDeliveryError::NotSent)?;
        }
        let events = self
            .events
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        ensure_event_generation(&events, checkpoint.generation)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        validate_exact_lifecycle_checkpoint_locked(
            &events,
            checkpoint.generation,
            thread_id,
            &checkpoint,
        )
        .map_err(CodeRpcDeliveryError::NotSent)?;
        validate_new_thread_lifecycle_locked(&events, thread_id)
            .map_err(CodeRpcDeliveryError::NotSent)?;
        let pending = process.begin_request_with_delivery("thread/resume", params)?;
        Ok((checkpoint.generation, pending))
    }

    fn begin_ready_request(
        &self,
        method: &str,
        params: Value,
        model_requirement: Option<CodeModelCatalogRequirement<'_>>,
    ) -> Result<(u64, PendingRuntimeRequest), CodeRpcDeliveryError> {
        let mut runtime = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut runtime, &self.approvals, &self.events);
        if runtime.phase != CodeRuntimePhase::Ready {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex app-server is not ready".to_string(),
            ));
        }
        let generation = runtime.generation;
        let process = runtime.process.as_ref().ok_or_else(|| {
            CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
        })?;
        if let Some(requirement) = model_requirement {
            let catalog = collect_model_catalog_from_process(generation, process)
                .map_err(CodeRpcDeliveryError::NotSent)?;
            requirement
                .validate(&catalog)
                .map_err(CodeRpcDeliveryError::NotSent)?;
        }
        let pending = process.begin_request_with_delivery(method, params)?;
        Ok((generation, pending))
    }

    fn ready_generation(&self) -> Result<u64, String> {
        let mut inner = self.inner.lock().map_err(|error| error.to_string())?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready {
            return Err("Codex app-server is not ready".to_string());
        }
        if inner.process.is_none() {
            return Err("Codex app-server is not running".to_string());
        }
        Ok(inner.generation)
    }

    fn request_ready(&self, method: &str, params: Value) -> Result<Value, String> {
        let generation = self.ready_generation()?;
        self.request_ready_at_generation(generation, method, params)
    }

    fn request_ready_at_generation(
        &self,
        generation: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        self.request_ready_with_delivery_at_generation(generation, method, params)
            .map_err(CodeRpcDeliveryError::into_message)
    }

    fn request_ready_with_delivery_at_generation(
        &self,
        generation: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, CodeRpcDeliveryError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|error| CodeRpcDeliveryError::NotSent(error.to_string()))?;
        refresh_process_health(&mut inner, &self.approvals, &self.events);
        if inner.phase != CodeRuntimePhase::Ready || inner.generation != generation {
            return Err(CodeRpcDeliveryError::NotSent(
                "Codex app-server runtime generation changed".to_string(),
            ));
        }
        inner
            .process
            .as_ref()
            .ok_or_else(|| {
                CodeRpcDeliveryError::NotSent("Codex app-server is not running".to_string())
            })?
            .request_with_delivery(method, params, REQUEST_TIMEOUT)
    }
}

#[cfg(test)]
fn collect_authoritative_thread_graph(
    deferred_targets: &HashSet<String>,
    request: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Result<CodeAuthoritativeThreadGraph, String> {
    collect_authoritative_thread_graph_with_pending_forks(deferred_targets, &[], request)
}

fn collect_authoritative_thread_graph_with_pending_forks(
    deferred_targets: &HashSet<String>,
    pending_forks: &[CodePendingForkExpectation],
    mut request: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Result<CodeAuthoritativeThreadGraph, String> {
    validate_pending_fork_expectations(pending_forks)?;
    let mut threads = Vec::new();
    for membership in [CodeThreadMembership::Active, CodeThreadMembership::Archived] {
        let mut cursor = None;
        let mut seen_cursors = HashSet::new();
        for _ in 0..MAX_AUTHORITATIVE_PAGES {
            let params = authoritative_thread_list_params(membership, cursor.as_deref())?;
            let page =
                parse_authoritative_thread_list(request("thread/list", params)?, membership)?;
            if threads.len().saturating_add(page.data.len()) > MAX_AUTHORITATIVE_THREADS {
                return Err(format!(
                    "Codex authoritative graph exceeds the {MAX_AUTHORITATIVE_THREADS}-thread safety limit"
                ));
            }
            threads.extend(page.data);
            match page.next_cursor {
                Some(next_cursor) => {
                    if !seen_cursors.insert(next_cursor.clone()) {
                        return Err(format!(
                            "Codex {membership:?} thread pagination repeated a cursor"
                        ));
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
                "Codex {membership:?} thread pagination exceeded the {MAX_AUTHORITATIVE_PAGES}-page safety limit"
            ));
        }
    }

    // Codex 0.145 can defer a just-created thread in memory, omitting it from
    // thread/list. Exhaustively merge loaded ids from the same generation;
    // exact bound ids and Starting fork journals use separate strict parsers.
    let mut listed_ids = threads
        .iter()
        .map(|thread| thread.id.clone())
        .collect::<HashSet<_>>();
    let mut loaded_ids = HashSet::new();
    let mut matched_pending_forks = HashSet::new();
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();
    for _ in 0..MAX_AUTHORITATIVE_PAGES {
        let params = protocol::loaded_thread_list_params(cursor.as_deref())?;
        let page = protocol::parse_loaded_thread_list(request("thread/loaded/list", params)?)?;
        if page.data.len() > CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT as usize {
            return Err(format!(
                "Codex loaded thread list exceeded the {CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT}-thread page limit"
            ));
        }
        for thread_id in page.data {
            if !loaded_ids.insert(thread_id.clone()) {
                return Err(format!(
                    "Codex loaded thread list contained duplicate thread id {thread_id}"
                ));
            }
            if loaded_ids.len() > MAX_AUTHORITATIVE_THREADS {
                return Err(format!(
                    "Codex loaded thread inventory exceeds the {MAX_AUTHORITATIVE_THREADS}-thread safety limit"
                ));
            }
            if listed_ids.contains(&thread_id) {
                continue;
            }
            if !deferred_targets.contains(&thread_id) && pending_forks.is_empty() {
                return Err(format!(
                    "Codex loaded thread {thread_id} was absent from both authoritative memberships"
                ));
            }
            let params = protocol::thread_read_params(&thread_id)?;
            let value = request("thread/read", params)?;
            let thread = if deferred_targets.contains(&thread_id) {
                parse_authoritative_deferred_bound_thread_read(value)?
            } else {
                let mut matches = Vec::new();
                for expectation in pending_forks {
                    if matched_pending_forks.contains(&expectation.preparation_id) {
                        continue;
                    }
                    if let Ok(candidate) =
                        parse_authoritative_pending_fork_thread_read(value.clone(), expectation)
                    {
                        matches.push((expectation.preparation_id.clone(), candidate));
                    }
                }
                match matches.len() {
                    1 => {
                        let (preparation_id, candidate) = matches.remove(0);
                        matched_pending_forks.insert(preparation_id);
                        candidate
                    }
                    0 => {
                        return Err(format!(
                            "Codex loaded thread {thread_id} was absent from both authoritative memberships and did not match a pending fork"
                        ));
                    }
                    count => {
                        return Err(format!(
                            "Codex loaded thread {thread_id} matched {count} pending fork journals"
                        ));
                    }
                }
            };
            if thread.id != thread_id {
                return Err("Codex loaded-thread read returned a different thread id".to_string());
            }
            if threads.len() >= MAX_AUTHORITATIVE_THREADS {
                return Err(format!(
                    "Codex authoritative graph exceeds the {MAX_AUTHORITATIVE_THREADS}-thread safety limit"
                ));
            }
            listed_ids.insert(thread_id);
            threads.push(thread);
        }
        match page.next_cursor {
            Some(next_cursor) => {
                if !seen_cursors.insert(next_cursor.clone()) {
                    return Err("Codex loaded thread pagination repeated a cursor".to_string());
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
            "Codex loaded thread pagination exceeded the {MAX_AUTHORITATIVE_PAGES}-page safety limit"
        ));
    }
    CodeAuthoritativeThreadGraph::from_threads(threads)
}

fn validate_pending_fork_expectations(
    pending_forks: &[CodePendingForkExpectation],
) -> Result<(), String> {
    let mut preparation_ids = HashSet::with_capacity(pending_forks.len());
    for expectation in pending_forks {
        protocol::validate_id("pending fork preparation", &expectation.preparation_id)?;
        protocol::validate_id("pending fork source", &expectation.source_thread_id)?;
        if canonical_workspace_root(&expectation.execution_root)? != expectation.execution_root {
            return Err("SchoolX pending fork root is not canonical".to_string());
        }
        if !preparation_ids.insert(expectation.preparation_id.as_str()) {
            return Err(
                "SchoolX pending fork expectations contain a duplicate journal".to_string(),
            );
        }
        let mut previous = None;
        for thread_id in &expectation.recovery_thread_baseline {
            protocol::validate_id("pending fork baseline thread", thread_id)?;
            if previous.is_some_and(|candidate: &String| candidate >= thread_id) {
                return Err(
                    "SchoolX pending fork recovery baseline is not strictly sorted".to_string(),
                );
            }
            previous = Some(thread_id);
        }
    }
    Ok(())
}

fn validate_deferred_targets(deferred_target_ids: &[String]) -> Result<HashSet<String>, String> {
    let mut deferred_targets = HashSet::with_capacity(deferred_target_ids.len());
    for thread_id in deferred_target_ids {
        protocol::validate_id("deferred authoritative target", thread_id)?;
        if !deferred_targets.insert(thread_id.clone()) {
            return Err("Codex deferred target list contained a duplicate id".to_string());
        }
    }
    Ok(deferred_targets)
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

fn refresh_process_health(
    inner: &mut RuntimeInner,
    approvals: &PendingApprovalStore,
    events: &EventBridge,
) {
    let failure = inner
        .process
        .as_mut()
        .and_then(RuntimeProcess::health_error);
    if let Some(error) = failure {
        let stop_error = stop_runtime_process(inner).err();
        let _ = events.clear_activity(inner.generation);
        approvals.clear_generation(inner.generation);
        inner.phase = CodeRuntimePhase::Failed;
        inner.initialized = None;
        inner.last_error = Some(match stop_error {
            Some(stop_error) => {
                format!("{error}; failed to verify app-server shutdown: {stop_error}")
            }
            None => error,
        });
    }
}

fn stop_runtime_process(inner: &mut RuntimeInner) -> Result<(), String> {
    let Some(mut process) = inner.process.take() else {
        return Ok(());
    };
    match process.stop() {
        Ok(()) => Ok(()),
        Err(error) => {
            inner.process = Some(process);
            Err(error)
        }
    }
}

fn status_from_inner(inner: &RuntimeInner, queued_notifications: usize) -> CodeRuntimeStatus {
    let initialized = inner.initialized.as_ref();
    let probe = inner
        .probe
        .as_ref()
        .map(CodeRuntimeProbe::redacted_for_egress);
    CodeRuntimeStatus {
        phase: inner.phase,
        generation: inner.generation,
        executable: probe.as_ref().and_then(|probe| probe.executable.clone()),
        version: probe.as_ref().and_then(|probe| probe.version.clone()),
        pid: inner.process.as_ref().map(|process| process.child.id()),
        user_agent: initialized.map(|result| protocol::redact_protocol_text(&result.user_agent)),
        codex_home: initialized.map(|result| protocol::redact_protocol_text(&result.codex_home)),
        platform_family: initialized
            .map(|result| protocol::redact_protocol_text(&result.platform_family)),
        platform_os: initialized.map(|result| protocol::redact_protocol_text(&result.platform_os)),
        queued_notifications,
        last_error: inner
            .last_error
            .as_deref()
            .map(protocol::redact_protocol_text),
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

struct PendingRuntimeRequest {
    id: u64,
    method: String,
    receiver: mpsc::Receiver<RpcReply>,
    pending: PendingRequests,
    stream_error: Arc<Mutex<Option<String>>>,
}

fn collect_model_catalog_from_process(
    runtime_generation: u64,
    process: &RuntimeProcess,
) -> Result<CodeModelCatalogSnapshot, String> {
    collect_model_catalog(runtime_generation, |params| {
        process.request("model/list", params, REQUEST_TIMEOUT)
    })
}

impl PendingRuntimeRequest {
    fn wait(self, timeout: Duration) -> Result<Value, CodeRpcDeliveryError> {
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

struct RuntimeProcess {
    child: Child,
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
    stop_failures_remaining: usize,
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

    fn initialize(&mut self) -> Result<InitializeResult, String> {
        let result = self.request("initialize", initialize_params(), INITIALIZE_TIMEOUT)?;
        let initialized = serde_json::from_value(protocol::redact_protocol_value(result))
            .map_err(|error| format!("invalid Codex initialize response: {error}"))?;
        let notification = jsonrpc::notification("initialized");
        let mut writer = self.stdin.lock().map_err(|error| error.to_string())?;
        jsonrpc::write_value(&mut *writer, &notification)?;
        Ok(initialized)
    }

    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        self.request_with_delivery(method, params, timeout)
            .map_err(CodeRpcDeliveryError::into_message)
    }

    fn request_with_delivery(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CodeRpcDeliveryError> {
        self.begin_request_with_delivery(method, params)?
            .wait(timeout)
    }

    fn begin_request_with_delivery(
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
fn runtime_process_pid(raw_pid: u32) -> Result<rustix::process::Pid, String> {
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

fn first_line(value: &str) -> String {
    let line = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(value);
    protocol::redact_protocol_text(line)
}

#[cfg(test)]
mod tests {
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
    fn wait_for_process_group_exit(
        pid: rustix::process::Pid,
        timeout: Duration,
    ) -> Result<(), String> {
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
    fn failed_stop_retains_process_and_blocks_restart_until_verified_teardown() -> Result<(), String>
    {
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
        let original_generation = ready.generation;
        let original_pid = ready
            .pid
            .ok_or_else(|| "ready runtime did not expose its process ID".to_string())?;
        {
            let mut inner = runtime.inner.lock().map_err(|error| error.to_string())?;
            let process = inner
                .process
                .as_mut()
                .ok_or_else(|| "ready runtime did not retain its process".to_string())?;
            process.stop_failures_remaining = 2;
        }

        assert!(runtime.stop().is_err());
        let failed_stop = runtime.status()?;
        assert_eq!(failed_stop.phase, CodeRuntimePhase::Failed);
        assert_eq!(failed_stop.generation, original_generation);
        assert_eq!(failed_stop.pid, Some(original_pid));

        assert!(runtime.start(noop_emitter()).is_err());
        let blocked_restart = runtime.status()?;
        assert_eq!(blocked_restart.phase, CodeRuntimePhase::Failed);
        assert_eq!(blocked_restart.generation, original_generation);
        assert_eq!(blocked_restart.pid, Some(original_pid));

        let restarted = runtime.start(noop_emitter())?;
        assert_eq!(restarted.phase, CodeRuntimePhase::Ready);
        assert_eq!(restarted.generation, original_generation.saturating_add(1));
        assert!(restarted.pid.is_some_and(|pid| pid != original_pid));
        runtime.stop()?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn leader_exit_kills_term_resistant_app_server_descendant() -> Result<(), String> {
        let pid_directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let descendant_pid_path = pid_directory.path().join("descendant.pid");
        let quoted_pid_path = shell_quote(&descendant_pid_path);
        let (_directory, executable) = fake_codex(&format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
/bin/sh -c 'trap "" TERM HUP; printf "%s\n" "$$" > "$1"; while :; do sleep 1; done' schoolx-descendant {quoted_pid_path} &
while [ ! -s {quoted_pid_path} ]; do :; done
IFS= read -r request
printf '%s\n' '{{"id":1,"result":{{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}}}'
IFS= read -r initialized
sleep 0.1
exit 23
"#
        ))?;
        let runtime = CodeRuntime::with_executable(executable);

        let ready = runtime.start(noop_emitter())?;
        let leader_pid = runtime_process_pid(
            ready
                .pid
                .ok_or_else(|| "ready runtime did not expose its process ID".to_string())?,
        )?;
        let mut cleanup_guard = ProcessGroupCleanupGuard::new(leader_pid);
        let descendant_pid = fs::read_to_string(&descendant_pid_path)
            .map_err(|error| error.to_string())?
            .trim()
            .parse::<u32>()
            .map_err(|error| format!("invalid descendant PID: {error}"))?;
        let descendant_pid = runtime_process_pid(descendant_pid)?;

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if runtime.status()?.phase == CodeRuntimePhase::Failed {
                break;
            }
            if Instant::now() >= deadline {
                return Err("runtime did not observe the app-server leader exit".to_string());
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        wait_for_process_exit(descendant_pid, Duration::from_secs(3))?;
        wait_for_process_group_exit(leader_pid, Duration::from_secs(3))?;
        cleanup_guard.disarm();
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
    fn failed_initialize_redacts_rpc_stderr_and_status_diagnostics() -> Result<(), String> {
        let rpc_canary = "sk-initialize-rpc-canary";
        let stderr_canary = "sk-initialize-stderr-canary";
        let (_directory, executable) = fake_codex(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
printf '%s\n' 'stderr sk-initialize-stderr-canary' >&2
sleep 0.1
IFS= read -r request
printf '%s\n' '{"id":1,"error":{"code":-32602,"message":"bad initialize sk-initialize-rpc-canary"}}'
while IFS= read -r line; do :; done
"#,
        )?;
        let runtime = CodeRuntime::with_executable(executable);

        let error = runtime
            .start(noop_emitter())
            .err()
            .ok_or_else(|| "initialize failure unexpectedly started the runtime".to_string())?;
        assert!(!error.contains(rpc_canary));
        assert!(!error.contains(stderr_canary));
        assert!(error.contains("[REDACTED]"));

        let status = runtime.status()?;
        let last_error = status
            .last_error
            .as_deref()
            .ok_or_else(|| "failed runtime status had no last error".to_string())?;
        assert_eq!(status.phase, CodeRuntimePhase::Failed);
        assert!(!last_error.contains(rpc_canary));
        assert!(!last_error.contains(stderr_canary));
        assert!(last_error.contains("[REDACTED]"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn successful_initialize_redacts_status_metadata() -> Result<(), String> {
        let canaries = [
            "sk-initialize-user-agent-canary",
            "sk-initialize-codex-home-canary",
            "sk-initialize-platform-family-canary",
            "sk-initialize-platform-os-canary",
        ];
        let (_directory, executable) = fake_codex(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r request
printf '%s\n' '{"id":1,"result":{"userAgent":"agent sk-initialize-user-agent-canary","codexHome":"/tmp/sk-initialize-codex-home-canary","platformFamily":"unix sk-initialize-platform-family-canary","platformOs":"macos sk-initialize-platform-os-canary"}}'
IFS= read -r initialized
while IFS= read -r line; do :; done
"#,
        )?;
        let runtime = CodeRuntime::with_executable(executable);

        let ready = runtime.start(noop_emitter())?;
        let status = runtime.status()?;
        for canary in canaries {
            for value in [
                ready.user_agent.as_deref(),
                ready.codex_home.as_deref(),
                ready.platform_family.as_deref(),
                ready.platform_os.as_deref(),
                status.user_agent.as_deref(),
                status.codex_home.as_deref(),
                status.platform_family.as_deref(),
                status.platform_os.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                assert!(!value.contains(canary));
            }
        }
        assert!(ready
            .user_agent
            .as_deref()
            .is_some_and(|value| value.contains("[REDACTED]")));
        assert!(status
            .codex_home
            .as_deref()
            .is_some_and(|value| value.contains("[REDACTED]")));
        runtime.stop()?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn public_probe_start_and_status_redact_child_version_diagnostics() -> Result<(), String> {
        let failure_canary = "sk-runtime-probe-failure-canary";
        let (_failure_directory, failure_executable) = fake_codex(
            r#"#!/bin/sh
printf '%s\n' 'version failed sk-runtime-probe-failure-canary' >&2
exit 1
"#,
        )?;
        let failed_runtime = CodeRuntime::with_executable(failure_executable);

        let failed_probe = failed_runtime.probe();
        let probe_error = failed_probe
            .error
            .as_deref()
            .ok_or_else(|| "failed public probe returned no diagnostic".to_string())?;
        assert!(!probe_error.contains(failure_canary));
        assert!(probe_error.contains("[REDACTED]"));

        let start_error = failed_runtime
            .start(noop_emitter())
            .err()
            .ok_or_else(|| "failed version probe unexpectedly started the runtime".to_string())?;
        assert!(!start_error.contains(failure_canary));
        assert!(start_error.contains("[REDACTED]"));
        let failed_status = failed_runtime.status()?;
        let last_error = failed_status
            .last_error
            .as_deref()
            .ok_or_else(|| "failed version status had no last error".to_string())?;
        assert!(!last_error.contains(failure_canary));
        assert!(last_error.contains("[REDACTED]"));

        let version_canary = "sk-runtime-probe-version-canary";
        let (_version_directory, version_executable) = fake_codex(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0 sk-runtime-probe-version-canary"
  exit 0
fi
exit 1
"#,
        )?;
        let version_runtime = CodeRuntime::with_executable(version_executable);
        let public_probe = version_runtime.probe();
        let public_version = public_probe
            .version
            .as_deref()
            .ok_or_else(|| "public probe returned no version".to_string())?;
        assert!(!public_version.contains(version_canary));
        assert!(public_version.contains("[REDACTED]"));

        let unsupported_error = version_runtime
            .start(noop_emitter())
            .err()
            .ok_or_else(|| "unsupported version unexpectedly started the runtime".to_string())?;
        assert!(!unsupported_error.contains(version_canary));
        let version_status = version_runtime.status()?;
        let status_version = version_status
            .version
            .as_deref()
            .ok_or_else(|| "unsupported version status had no version".to_string())?;
        assert!(!status_version.contains(version_canary));
        assert!(status_version.contains("[REDACTED]"));
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
    fn authoritative_graph_paginates_both_memberships_and_loaded_deferred_threads(
    ) -> Result<(), String> {
        let thread = |id: &str, parent: Option<&str>| {
            json!({
                "id": id,
                "cwd": "/tmp/schoolx-code",
                "source": "appServer",
                "status": { "type": "idle" },
                "parentThreadId": parent,
                "forkedFromId": null
            })
        };
        let mut list_calls = 0_usize;
        let mut loaded_calls = 0_usize;
        let deferred_targets = HashSet::from(["thread-deferred".to_string()]);
        let graph =
            collect_authoritative_thread_graph(&deferred_targets, |method, params| match method {
                "thread/list" => {
                    list_calls = list_calls.saturating_add(1);
                    assert!(params.get("cwd").is_none());
                    assert!(params.get("searchTerm").is_none());
                    assert_eq!(
                        params["sourceKinds"],
                        json!(super::super::thread_lifecycle::CODEX_0145_THREAD_SOURCE_KINDS)
                    );
                    let archived = params["archived"].as_bool().unwrap_or(false);
                    let cursor = params.get("cursor").and_then(Value::as_str);
                    Ok(match (archived, cursor) {
                        (false, None) => json!({
                            "data": [thread("thread-root", None)],
                            "nextCursor": "active-next"
                        }),
                        (false, Some("active-next")) => json!({
                            "data": [thread("thread-child", Some("thread-root"))],
                            "nextCursor": null
                        }),
                        (true, None) => json!({
                            "data": [thread("thread-archived-a", None)],
                            "nextCursor": "archived-next"
                        }),
                        (true, Some("archived-next")) => json!({
                            "data": [thread("thread-archived-b", None)],
                            "nextCursor": null
                        }),
                        _ => return Err("unexpected authoritative list page".to_string()),
                    })
                }
                "thread/loaded/list" => {
                    loaded_calls = loaded_calls.saturating_add(1);
                    let cursor = params.get("cursor").and_then(Value::as_str);
                    Ok(match cursor {
                        None => json!({ "data": ["thread-deferred"], "nextCursor": "loaded-next" }),
                        Some("loaded-next") => json!({ "data": [], "nextCursor": null }),
                        _ => return Err("unexpected loaded list page".to_string()),
                    })
                }
                "thread/read" if params["threadId"] == "thread-deferred" => Ok(json!({
                    "thread": {
                        "id": "thread-deferred",
                        "sessionId": "thread-deferred",
                        "cwd": "/tmp/schoolx-code",
                        "source": "appServer",
                        "status": { "type": "idle" },
                        "ephemeral": false,
                        "parentThreadId": null,
                        "forkedFromId": null,
                        "turns": []
                    }
                })),
                _ => Err(format!("unexpected authoritative method {method}")),
            })?;

        assert_eq!(list_calls, 4);
        assert_eq!(loaded_calls, 2);
        for active in ["thread-root", "thread-child", "thread-deferred"] {
            assert_eq!(graph.membership(active), Some(CodeThreadMembership::Active));
        }
        for archived in ["thread-archived-a", "thread-archived-b"] {
            assert_eq!(
                graph.membership(archived),
                Some(CodeThreadMembership::Archived)
            );
        }
        assert_eq!(
            graph.membership("thread-deferred"),
            Some(CodeThreadMembership::Active)
        );
        assert!(graph.ensure_leaf("thread-root").is_err());
        Ok(())
    }

    #[test]
    fn authoritative_graph_admits_only_the_exact_list_absent_pending_fork() -> Result<(), String> {
        let destination = tempfile::tempdir().map_err(|error| error.to_string())?;
        let destination_root = destination
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
        let pending = [CodePendingForkExpectation {
            preparation_id: preparation_id.to_string(),
            source_thread_id: "thread-source".to_string(),
            execution_root: destination_root.clone(),
            recovery_thread_baseline: vec!["thread-before".to_string()],
        }];
        let graph = collect_authoritative_thread_graph_with_pending_forks(
            &HashSet::from(["thread-source".to_string()]),
            &pending,
            |method, params| match method {
                "thread/list" if params["archived"] == false => Ok(json!({
                    "data": [{
                        "id": "thread-source",
                        "cwd": "/tmp/source",
                        "source": "appServer",
                        "status": { "type": "idle" }
                    }],
                    "nextCursor": null
                })),
                "thread/list" => Ok(json!({ "data": [], "nextCursor": null })),
                "thread/loaded/list" => Ok(json!({ "data": ["thread-child"], "nextCursor": null })),
                "thread/read" => {
                    assert_eq!(
                        params,
                        json!({ "threadId": "thread-child", "includeTurns": false })
                    );
                    Ok(json!({
                        "thread": {
                            "id": "thread-child",
                            "sessionId": "thread-child",
                            "cwd": destination_root,
                            "source": "appServer",
                            "threadSource": format!("schoolx-code/{preparation_id}"),
                            "status": { "type": "idle" },
                            "ephemeral": false,
                            "parentThreadId": null,
                            "forkedFromId": "thread-source",
                            "turns": []
                        }
                    }))
                }
                _ => Err(format!("unexpected authoritative method {method}")),
            },
        )?;

        assert_eq!(
            graph.membership("thread-child"),
            Some(CodeThreadMembership::Active)
        );
        assert!(graph.ensure_leaf("thread-source").is_err());

        let wrong_marker = collect_authoritative_thread_graph_with_pending_forks(
            &HashSet::from(["thread-source".to_string()]),
            &pending,
            |method, params| match method {
                "thread/list" if params["archived"] == false => Ok(json!({
                    "data": [{
                        "id": "thread-source",
                        "cwd": "/tmp/source",
                        "source": "appServer",
                        "status": { "type": "idle" }
                    }],
                    "nextCursor": null
                })),
                "thread/list" => Ok(json!({ "data": [], "nextCursor": null })),
                "thread/loaded/list" => Ok(json!({ "data": ["thread-child"], "nextCursor": null })),
                "thread/read" => Ok(json!({
                    "thread": {
                        "id": "thread-child",
                        "sessionId": "thread-child",
                        "cwd": destination_root,
                        "source": "appServer",
                        "threadSource": "schoolx-code/wrong-preparation",
                        "status": { "type": "idle" },
                        "ephemeral": false,
                        "parentThreadId": null,
                        "forkedFromId": "thread-source",
                        "turns": []
                    }
                })),
                _ => Err(format!("unexpected authoritative method {method}")),
            },
        );
        assert!(wrong_marker.is_err_and(|error| error.contains("did not match a pending fork")));
        Ok(())
    }

    #[test]
    fn authoritative_graph_rejects_membership_duplicates_and_cursor_cycles() {
        let duplicate =
            collect_authoritative_thread_graph(&HashSet::new(), |method, params| match method {
                "thread/list" => Ok(json!({
                    "data": [{
                        "id": "thread-duplicate",
                        "cwd": "/tmp/schoolx-code",
                        "source": "appServer",
                        "status": { "type": "idle" }
                    }],
                    "nextCursor": null
                })),
                "thread/loaded/list" => Ok(json!({ "data": [], "nextCursor": null })),
                _ => Err(format!("unexpected method {method}: {params}")),
            });
        assert!(duplicate.is_err_and(|error| error.contains("duplicate thread id")));

        let cycle =
            collect_authoritative_thread_graph(&HashSet::new(), |method, params| match method {
                "thread/list" => Ok(json!({
                    "data": [],
                    "nextCursor": "same-cursor"
                })),
                _ => Err(format!("unexpected method {method}: {params}")),
            });
        assert!(cycle.is_err_and(|error| error.contains("repeated a cursor")));
    }

    #[test]
    fn authoritative_graph_rejects_unbound_or_nonempty_list_absent_loaded_threads() {
        let foreign = collect_authoritative_thread_graph(&HashSet::new(), |method, _params| {
            Ok(match method {
                "thread/list" => json!({ "data": [], "nextCursor": null }),
                "thread/loaded/list" => {
                    json!({ "data": ["thread-foreign"], "nextCursor": null })
                }
                _ => return Err(format!("unexpected method {method}")),
            })
        });
        assert!(foreign.is_err_and(|error| error.contains("absent from both")));

        let allowed = HashSet::from(["thread-nonempty".to_string()]);
        let nonempty = collect_authoritative_thread_graph(&allowed, |method, params| {
            Ok(match method {
                "thread/list" => json!({ "data": [], "nextCursor": null }),
                "thread/loaded/list" => {
                    json!({ "data": ["thread-nonempty"], "nextCursor": null })
                }
                "thread/read" if params["threadId"] == "thread-nonempty" => json!({
                    "thread": {
                        "id": "thread-nonempty",
                        "sessionId": "thread-nonempty",
                        "cwd": "/tmp/schoolx-code",
                        "source": "appServer",
                        "status": { "type": "idle" },
                        "ephemeral": false,
                        "parentThreadId": null,
                        "forkedFromId": null,
                        "turns": [{ "id": "turn-1", "status": "completed", "items": [] }]
                    }
                }),
                _ => return Err(format!("unexpected method {method}")),
            })
        });
        assert!(nonempty.is_err_and(|error| error.contains("quiescent SchoolX root or fork")));
    }

    #[test]
    fn authoritative_graph_rejects_page_bound_exhaustion() {
        let mut page = 0_usize;
        let result =
            collect_authoritative_thread_graph(&HashSet::new(), |method, params| match method {
                "thread/list" => {
                    page = page.saturating_add(1);
                    Ok(json!({
                        "data": [],
                        "nextCursor": format!("cursor-{page}")
                    }))
                }
                _ => Err(format!("unexpected method {method}: {params}")),
            });
        assert_eq!(page, MAX_AUTHORITATIVE_PAGES);
        assert!(result.is_err_and(|error| error.contains("page safety limit")));
    }

    #[test]
    fn turn_start_response_marks_active_before_delayed_notification() -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        let token = bridge.begin_turn_start(7, "thread-1")?;
        bridge.complete_turn_start(
            7,
            "thread-1",
            token,
            "turn-1",
            CodePinnedTurnStatus::InProgress,
        )?;
        let before_notification = bridge.activity_snapshot(7, "thread-1")?;
        assert!(before_notification.active_or_starting);

        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_id: None,
                kind: "turn/started".to_string(),
                payload: json!({ "turn": { "status": "inProgress" } }),
            },
        );
        assert!(bridge.activity_snapshot(7, "thread-1")?.active_or_starting);
        Ok(())
    }

    #[test]
    fn completion_before_turn_start_response_cannot_resurrect_the_turn() -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        let token = bridge.begin_turn_start(7, "thread-1")?;
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_id: None,
                kind: "turn/completed".to_string(),
                payload: json!({ "turn": { "status": "completed" } }),
            },
        );
        bridge.complete_turn_start(
            7,
            "thread-1",
            token,
            "turn-1",
            CodePinnedTurnStatus::InProgress,
        )?;
        assert!(!bridge.activity_snapshot(7, "thread-1")?.active_or_starting);

        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_id: None,
                kind: "turn/started".to_string(),
                payload: json!({ "turn": { "status": "inProgress" } }),
            },
        );
        assert!(!bridge.activity_snapshot(7, "thread-1")?.active_or_starting);
        bridge.reset(8, noop_emitter())?;
        assert!(!bridge.activity_snapshot(8, "thread-1")?.active_or_starting);
        Ok(())
    }

    #[test]
    fn thread_close_before_turn_start_response_cannot_resurrect_the_turn() -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        let token = bridge.begin_turn_start(7, "thread-1")?;
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/closed".to_string(),
                payload: json!({ "threadId": "thread-1" }),
            },
        );
        assert!(bridge
            .complete_turn_start(
                7,
                "thread-1",
                token,
                "turn-1",
                CodePinnedTurnStatus::InProgress,
            )
            .is_err());
        assert!(!bridge.activity_snapshot(7, "thread-1")?.active_or_starting);
        Ok(())
    }

    #[test]
    fn inflight_terminal_proof_survives_global_tombstone_eviction() -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        let token = bridge.begin_turn_start(7, "thread-target")?;
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-target".to_string()),
                turn_id: Some("turn-target".to_string()),
                item_id: None,
                kind: "turn/completed".to_string(),
                payload: json!({ "turn": { "status": "completed" } }),
            },
        );
        for index in 0..MAX_TURN_TOMBSTONES {
            bridge.publish(
                7,
                CodeWorkspaceEventDraft {
                    thread_id: Some("thread-other".to_string()),
                    turn_id: Some(format!("turn-other-{index}")),
                    item_id: None,
                    kind: "turn/completed".to_string(),
                    payload: json!({ "turn": { "status": "completed" } }),
                },
            );
        }
        bridge.complete_turn_start(
            7,
            "thread-target",
            token,
            "turn-target",
            CodePinnedTurnStatus::InProgress,
        )?;
        assert!(
            !bridge
                .activity_snapshot(7, "thread-target")?
                .active_or_starting
        );
        Ok(())
    }

    #[test]
    fn lifecycle_notifications_and_runtime_boundaries_keep_dirty_gate_fail_closed(
    ) -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        let initial = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
        assert!(initial.is_dirty());
        assert!(!initial.accepts_archive_completion());
        assert!(!initial.accepts_unarchive_completion());
        bridge.clear_lifecycle_dirty(7, "thread-1", initial)?;
        assert!(!bridge.lifecycle_dirty_checkpoint(7, "thread-1")?.is_dirty());

        let clean = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
        assert!(clean.accepts_archive_completion());
        assert!(clean.accepts_unarchive_completion());
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/archived".to_string(),
                payload: json!({ "threadId": "thread-1" }),
            },
        );
        assert!(bridge.lifecycle_dirty_checkpoint(7, "thread-1")?.is_dirty());
        assert!(bridge.clear_lifecycle_dirty(7, "thread-1", clean).is_err());
        let archived = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
        assert!(archived.accepts_archive_completion());
        assert!(!archived.accepts_unarchive_completion());
        bridge.clear_lifecycle_dirty(7, "thread-1", archived.clone())?;
        assert!(!bridge.lifecycle_dirty_checkpoint(7, "thread-1")?.is_dirty());

        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/archived".to_string(),
                payload: json!({ "threadId": "thread-1" }),
            },
        );
        let archive_completion = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/unarchived".to_string(),
                payload: json!({ "threadId": "thread-1" }),
            },
        );
        assert!(archive_completion.accepts_archive_completion());
        assert!(bridge
            .clear_lifecycle_dirty(7, "thread-1", archive_completion)
            .is_err());
        let unarchived = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
        assert!(unarchived.is_dirty());
        assert!(!unarchived.accepts_archive_completion());
        assert!(unarchived.accepts_unarchive_completion());
        bridge.clear_lifecycle_dirty(7, "thread-1", unarchived)?;

        let before_foreign_change = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-descendant".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/archived".to_string(),
                payload: json!({ "threadId": "thread-descendant" }),
            },
        );
        assert!(bridge
            .clear_lifecycle_dirty(7, "thread-1", before_foreign_change)
            .is_err());
        let after_foreign_change = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
        bridge.clear_lifecycle_dirty(7, "thread-1", after_foreign_change)?;

        let before_stop = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
        bridge.clear_activity(7)?;
        assert!(bridge.lifecycle_dirty_checkpoint(7, "thread-1")?.is_dirty());
        assert!(bridge
            .clear_lifecycle_dirty(7, "thread-1", before_stop)
            .is_err());
        bridge.reset(8, noop_emitter())?;
        assert!(bridge.lifecycle_dirty_checkpoint(8, "thread-1")?.is_dirty());
        assert!(bridge
            .clear_lifecycle_dirty(8, "thread-1", archived)
            .is_err());
        Ok(())
    }

    #[test]
    fn new_thread_clean_seam_cannot_hide_prior_or_concurrent_lifecycle_notifications(
    ) -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-prior".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/archived".to_string(),
                payload: json!({ "threadId": "thread-prior" }),
            },
        );
        assert!(bridge
            .mark_new_thread_lifecycle_clean(7, "thread-prior")
            .is_err());
        assert!(bridge
            .lifecycle_dirty_checkpoint(7, "thread-prior")?
            .is_dirty());

        bridge.mark_new_thread_lifecycle_clean(7, "thread-later")?;
        assert!(!bridge
            .lifecycle_dirty_checkpoint(7, "thread-later")?
            .is_dirty());
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-later".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/unarchived".to_string(),
                payload: json!({ "threadId": "thread-later" }),
            },
        );
        assert!(bridge
            .lifecycle_dirty_checkpoint(7, "thread-later")?
            .is_dirty());
        assert!(bridge
            .mark_new_thread_lifecycle_clean(7, "thread-later")
            .is_err());

        bridge.clear_activity(7)?;
        assert!(bridge
            .mark_new_thread_lifecycle_clean(7, "thread-after-boundary")
            .is_err());
        Ok(())
    }

    #[test]
    fn thread_started_invalidates_graph_epoch_without_dirtying_new_thread_lifecycle(
    ) -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        let (_, before) = bridge.topology_checkpoint(7)?;

        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-new".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/started".to_string(),
                payload: json!({ "thread": { "id": "thread-new" } }),
            },
        );

        let (_, after) = bridge.topology_checkpoint(7)?;
        assert!(after > before);
        bridge.mark_new_thread_lifecycle_clean(7, "thread-new")?;
        assert!(!bridge
            .lifecycle_dirty_checkpoint(7, "thread-new")?
            .is_dirty());
        Ok(())
    }

    #[test]
    fn topology_epoch_rejects_descendant_started_between_membership_scans() -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        let (boundary, revision) = bridge.topology_checkpoint(7)?;
        let deferred = HashSet::new();
        let graph = collect_authoritative_thread_graph(&deferred, |method, params| match method {
            "thread/list" if params["archived"] == false => Ok(json!({
                "data": [{
                    "id": "thread-parent",
                    "cwd": "/tmp/schoolx-code",
                    "source": "appServer",
                    "status": { "type": "idle" },
                    "parentThreadId": null,
                    "forkedFromId": null
                }],
                "nextCursor": null
            })),
            "thread/list" if params["archived"] == true => {
                bridge.publish(
                    7,
                    CodeWorkspaceEventDraft {
                        thread_id: Some("thread-child".to_string()),
                        turn_id: None,
                        item_id: None,
                        kind: "thread/started".to_string(),
                        payload: json!({ "thread": { "id": "thread-child" } }),
                    },
                );
                Ok(json!({ "data": [], "nextCursor": null }))
            }
            "thread/loaded/list" => Ok(json!({ "data": [], "nextCursor": null })),
            _ => Err(format!("unexpected authoritative method {method}")),
        })?;
        assert_eq!(
            graph.membership("thread-parent"),
            Some(CodeThreadMembership::Active)
        );
        assert!(bridge
            .confirm_topology_checkpoint(7, boundary, revision)
            .is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn guarded_archive_rejects_post_graph_topology_before_writing_rpc_bytes() -> Result<(), String>
    {
        let (_directory, executable) = fake_codex(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
: > "$0.requests"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
done
"#,
        )?;
        let runtime = CodeRuntime::with_executable(executable.clone());
        let ready = runtime.start(noop_emitter())?;
        let request_log = executable.with_file_name("codex.requests");
        let deadline = Instant::now() + Duration::from_secs(1);
        while !request_log.exists() {
            if Instant::now() >= deadline {
                return Err("guarded archive test request log was not created".to_string());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let recovery_root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let recovery_checkpoint = runtime.thread_lifecycle_dirty_checkpoint("thread-recovery")?;
        runtime.events.publish(
            ready.generation,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-recovery".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/archived".to_string(),
                payload: json!({ "threadId": "thread-recovery" }),
            },
        );
        assert!(runtime
            .thread_resume_recovery_at_guarded(
                CodeThreadResumeInput {
                    scope: binding_scope(),
                    thread_id: "thread-recovery".to_string(),
                    model: None,
                },
                &recovery_root.path().to_string_lossy(),
                recovery_checkpoint,
            )
            .is_err());
        assert!(recorded_requests(&executable)?.is_empty());

        runtime.commit_new_thread_lifecycle("thread-target", || Ok(()))?;
        let (topology_boundary_revision, topology_revision) =
            runtime.events.topology_checkpoint(ready.generation)?;
        let graph = CodeAuthoritativeThreadGraph::from_threads([
            super::super::thread_lifecycle::CodeAuthoritativeThread {
                id: "thread-target".to_string(),
                membership: CodeThreadMembership::Active,
                cwd: "/tmp/schoolx-code".to_string(),
                parent_thread_id: None,
                forked_from_id: None,
                status: CodePinnedThreadStatus::Idle,
            },
        ])?;
        let proof = CodeThreadLifecycleGraphProof {
            generation: ready.generation,
            thread_id: "thread-target".to_string(),
            graph,
            topology_boundary_revision,
            topology_revision,
        };
        runtime.events.publish(
            ready.generation,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-child".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/started".to_string(),
                payload: json!({ "thread": { "id": "thread-child" } }),
            },
        );

        let error = runtime
            .thread_archive_guarded(
                &CodeThreadLifecycleInput {
                    scope: binding_scope(),
                    thread_id: "thread-target".to_string(),
                },
                proof,
            )
            .err()
            .ok_or_else(|| "stale graph proof unexpectedly wrote archive RPC".to_string())?;
        assert!(error.definitely_not_sent());
        assert!(recorded_requests(&executable)?.is_empty());

        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        let mut publisher = None;
        let events = Arc::clone(&runtime.events);
        runtime.commit_new_thread_lifecycle("thread-new", || {
            publisher = Some(std::thread::spawn(move || {
                let _ = started_tx.send(());
                events.publish(
                    ready.generation,
                    CodeWorkspaceEventDraft {
                        thread_id: Some("thread-new".to_string()),
                        turn_id: None,
                        item_id: None,
                        kind: "thread/archived".to_string(),
                        payload: json!({ "threadId": "thread-new" }),
                    },
                );
                let _ = done_tx.send(());
            }));
            started_rx.recv().map_err(|error| error.to_string())?;
            assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
            Ok(())
        })?;
        done_rx.recv().map_err(|error| error.to_string())?;
        publisher
            .ok_or_else(|| "new-thread publisher was not started".to_string())?
            .join()
            .map_err(|_| "new-thread publisher test thread panicked".to_string())?;
        assert!(runtime
            .thread_lifecycle_dirty_checkpoint("thread-new")?
            .is_dirty());
        runtime.stop()?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn fork_commit_revalidates_the_exact_source_after_response() -> Result<(), String> {
        let (_directory, executable) = fake_codex(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
while IFS= read -r line; do :; done
"#,
        )?;
        let runtime = CodeRuntime::with_executable(executable);
        let ready = runtime.start(noop_emitter())?;

        runtime.commit_new_thread_lifecycle("thread-source", || Ok(()))?;
        let checkpoint = runtime.thread_lifecycle_dirty_checkpoint("thread-source")?;
        let completion = CodeThreadForkCompletion {
            generation: ready.generation,
            source_thread_id: "thread-source".to_string(),
            lifecycle_checkpoint: checkpoint,
            activity_revision: 0,
        };
        runtime.events.publish(
            ready.generation,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-source".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/archived".to_string(),
                payload: json!({ "threadId": "thread-source" }),
            },
        );
        let committed = AtomicBool::new(false);
        assert!(runtime
            .commit_new_fork_lifecycle("thread-source", "thread-child", completion, || {
                committed.store(true, Ordering::Release);
                Ok(())
            },)
            .is_err());
        assert!(!committed.load(Ordering::Acquire));
        assert!(runtime
            .thread_lifecycle_dirty_checkpoint("thread-child")?
            .is_dirty());

        runtime.commit_new_thread_lifecycle("thread-source-clean", || Ok(()))?;
        let clean_checkpoint = runtime.thread_lifecycle_dirty_checkpoint("thread-source-clean")?;
        let clean_completion = CodeThreadForkCompletion {
            generation: ready.generation,
            source_thread_id: "thread-source-clean".to_string(),
            lifecycle_checkpoint: clean_checkpoint,
            activity_revision: 0,
        };
        runtime.commit_new_fork_lifecycle(
            "thread-source-clean",
            "thread-child-clean",
            clean_completion,
            || Ok(()),
        )?;
        assert!(!runtime
            .thread_lifecycle_dirty_checkpoint("thread-child-clean")?
            .is_dirty());
        runtime.stop()?;
        Ok(())
    }

    #[test]
    fn lifecycle_completion_atomically_commits_only_one_expected_signal() -> Result<(), String> {
        fn receipt(
            bridge: &EventBridge,
            expected: CodeThreadLifecycleSignal,
        ) -> Result<LifecycleWriteReceipt, String> {
            let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-target")?;
            let (topology_boundary_revision, topology_revision) = bridge.topology_checkpoint(7)?;
            Ok(LifecycleWriteReceipt {
                generation: 7,
                thread_id: "thread-target".to_string(),
                expected,
                lifecycle_boundary_revision: checkpoint.boundary_revision,
                topology_boundary_revision,
                topology_revision,
            })
        }

        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        bridge.mark_new_thread_lifecycle_clean(7, "thread-target")?;

        let completion = bridge
            .mutation_response_checkpoint(receipt(&bridge, CodeThreadLifecycleSignal::Archived)?)?;
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-target".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/archived".to_string(),
                payload: json!({ "threadId": "thread-target" }),
            },
        );
        let mut commit_calls = 0_usize;
        let committed = bridge.complete_lifecycle_mutation(
            "thread-target",
            completion,
            CodeThreadLifecycleSignal::Archived,
            || {
                commit_calls = commit_calls.saturating_add(1);
                Ok("saved")
            },
        )?;
        assert_eq!(committed, "saved");
        assert_eq!(commit_calls, 1);
        assert!(!bridge
            .lifecycle_dirty_checkpoint(7, "thread-target")?
            .is_dirty());

        let completion = bridge
            .mutation_response_checkpoint(receipt(&bridge, CodeThreadLifecycleSignal::Archived)?)?;
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-target".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/unarchived".to_string(),
                payload: json!({ "threadId": "thread-target" }),
            },
        );
        let mut conflicting_commit_calls = 0_usize;
        assert!(bridge
            .complete_lifecycle_mutation(
                "thread-target",
                completion,
                CodeThreadLifecycleSignal::Archived,
                || {
                    conflicting_commit_calls = conflicting_commit_calls.saturating_add(1);
                    Ok(())
                },
            )
            .is_err());
        assert_eq!(conflicting_commit_calls, 0);

        bridge.clear_lifecycle_dirty(
            7,
            "thread-target",
            bridge.lifecycle_dirty_checkpoint(7, "thread-target")?,
        )?;
        let completion = bridge
            .mutation_response_checkpoint(receipt(&bridge, CodeThreadLifecycleSignal::Archived)?)?;
        for _ in 0..2 {
            bridge.publish(
                7,
                CodeWorkspaceEventDraft {
                    thread_id: Some("thread-target".to_string()),
                    turn_id: None,
                    item_id: None,
                    kind: "thread/archived".to_string(),
                    payload: json!({ "threadId": "thread-target" }),
                },
            );
        }
        let mut duplicate_commit_calls = 0_usize;
        assert!(bridge
            .complete_lifecycle_mutation(
                "thread-target",
                completion,
                CodeThreadLifecycleSignal::Archived,
                || {
                    duplicate_commit_calls = duplicate_commit_calls.saturating_add(1);
                    Ok(())
                },
            )
            .is_err());
        assert_eq!(duplicate_commit_calls, 0);
        Ok(())
    }

    #[test]
    fn lifecycle_completion_rejects_foreign_topology_and_signal_reordering() -> Result<(), String> {
        fn clean_bridge() -> Result<EventBridge, String> {
            let bridge = EventBridge::new();
            bridge.reset(7, noop_emitter())?;
            bridge.mark_new_thread_lifecycle_clean(7, "thread-target")?;
            Ok(bridge)
        }

        let bridge = clean_bridge()?;
        let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-target")?;
        let (boundary, revision) = bridge.topology_checkpoint(7)?;
        let receipt = LifecycleWriteReceipt {
            generation: 7,
            thread_id: "thread-target".to_string(),
            expected: CodeThreadLifecycleSignal::Archived,
            lifecycle_boundary_revision: checkpoint.boundary_revision,
            topology_boundary_revision: boundary,
            topology_revision: revision,
        };
        for kind in ["thread/archived", "thread/unarchived", "thread/archived"] {
            bridge.publish(
                7,
                CodeWorkspaceEventDraft {
                    thread_id: Some("thread-target".to_string()),
                    turn_id: None,
                    item_id: None,
                    kind: kind.to_string(),
                    payload: json!({ "threadId": "thread-target" }),
                },
            );
        }
        assert!(bridge.mutation_response_checkpoint(receipt).is_err());

        let bridge = clean_bridge()?;
        let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-target")?;
        let (boundary, revision) = bridge.topology_checkpoint(7)?;
        let completion = bridge.mutation_response_checkpoint(LifecycleWriteReceipt {
            generation: 7,
            thread_id: "thread-target".to_string(),
            expected: CodeThreadLifecycleSignal::Archived,
            lifecycle_boundary_revision: checkpoint.boundary_revision,
            topology_boundary_revision: boundary,
            topology_revision: revision,
        })?;
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-foreign".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/started".to_string(),
                payload: json!({ "thread": { "id": "thread-foreign" } }),
            },
        );
        let mut commit_calls = 0_usize;
        assert!(bridge
            .complete_lifecycle_mutation(
                "thread-target",
                completion,
                CodeThreadLifecycleSignal::Archived,
                || {
                    commit_calls = commit_calls.saturating_add(1);
                    Ok(())
                },
            )
            .is_err());
        assert_eq!(commit_calls, 0);
        Ok(())
    }

    #[test]
    fn lifecycle_signal_waiting_on_durable_commit_barrier_remains_dirty_after_success(
    ) -> Result<(), String> {
        let bridge = Arc::new(EventBridge::new());
        bridge.reset(7, noop_emitter())?;
        bridge.mark_new_thread_lifecycle_clean(7, "thread-target")?;
        let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-target")?;
        let (topology_boundary_revision, topology_revision) = bridge.topology_checkpoint(7)?;
        let completion = bridge.mutation_response_checkpoint(LifecycleWriteReceipt {
            generation: 7,
            thread_id: "thread-target".to_string(),
            expected: CodeThreadLifecycleSignal::Archived,
            lifecycle_boundary_revision: checkpoint.boundary_revision,
            topology_boundary_revision,
            topology_revision,
        })?;

        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        let mut publisher = None;
        let publisher_bridge = Arc::clone(&bridge);
        bridge.complete_lifecycle_mutation(
            "thread-target",
            completion,
            CodeThreadLifecycleSignal::Archived,
            || {
                publisher = Some(std::thread::spawn(move || {
                    let _ = started_tx.send(());
                    publisher_bridge.publish(
                        7,
                        CodeWorkspaceEventDraft {
                            thread_id: Some("thread-target".to_string()),
                            turn_id: None,
                            item_id: None,
                            kind: "thread/archived".to_string(),
                            payload: json!({ "threadId": "thread-target" }),
                        },
                    );
                    let _ = done_tx.send(());
                }));
                started_rx.recv().map_err(|error| error.to_string())?;
                assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
                Ok(())
            },
        )?;
        done_rx.recv().map_err(|error| error.to_string())?;
        publisher
            .ok_or_else(|| "lifecycle publisher was not started".to_string())?
            .join()
            .map_err(|_| "lifecycle publisher test thread panicked".to_string())?;
        assert!(bridge
            .lifecycle_dirty_checkpoint(7, "thread-target")?
            .is_dirty());
        Ok(())
    }

    #[test]
    fn lifecycle_checkpoint_is_bound_to_one_exact_thread() -> Result<(), String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        bridge.mark_new_thread_lifecycle_clean(7, "thread-a")?;
        bridge.mark_new_thread_lifecycle_clean(7, "thread-b")?;
        let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-a")?;
        let inner = bridge.inner.lock().map_err(|error| error.to_string())?;
        assert!(
            validate_exact_lifecycle_checkpoint_locked(&inner, 7, "thread-b", &checkpoint,)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn event_backlog_is_bounded_and_reports_a_replay_gap() -> Result<(), String> {
        let bridge = EventBridge::new();
        let approvals = PendingApprovalStore::default();
        bridge.reset(7, noop_emitter())?;
        approvals.reset(7);
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

        let snapshot = bridge.snapshot(&approvals, Some(7), Some(0))?;
        assert_eq!(snapshot.events.len(), MAX_NOTIFICATION_BACKLOG);
        assert_eq!(snapshot.events.first().map(|event| event.sequence), Some(2));
        assert_eq!(snapshot.latest_sequence, 513);
        assert!(snapshot.truncated);
        let checkpoint = snapshot
            .checkpoint
            .ok_or_else(|| "truncated replay had no authoritative checkpoint".to_string())?;
        assert_eq!(checkpoint.runtime_generation, 7);
        assert_eq!(checkpoint.sequence_watermark, 513);
        assert!(checkpoint.active_turns.is_empty());
        assert!(checkpoint.pending_approvals.is_empty());
        Ok(())
    }

    #[test]
    fn checkpoint_preserves_active_turn_and_pending_approval_after_eviction() -> Result<(), String>
    {
        let bridge = EventBridge::new();
        let approvals = PendingApprovalStore::default();
        bridge.reset(7, noop_emitter())?;
        approvals.reset(7);
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_id: None,
                kind: "turn/started".to_string(),
                payload: json!({ "turn": { "status": "inProgress" } }),
            },
        );
        let approval = approvals
            .insert_request(
                7,
                json!("approval-1"),
                "item/fileChange/requestApproval",
                Some(json!({
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "item-1",
                    "availableDecisions": ["accept", "decline"]
                })),
            )?
            .ok_or_else(|| "approval request was not normalized".to_string())?;
        bridge.publish(7, approval);
        for index in 0..MAX_NOTIFICATION_BACKLOG {
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

        let snapshot = bridge.snapshot(&approvals, Some(7), Some(0))?;
        assert!(snapshot.truncated);
        assert_eq!(snapshot.events.first().map(|event| event.sequence), Some(3));
        let checkpoint = snapshot
            .checkpoint
            .ok_or_else(|| "evicted replay had no authoritative checkpoint".to_string())?;
        assert_eq!(checkpoint.sequence_watermark, 514);
        assert_eq!(checkpoint.active_turns.len(), 1);
        assert_eq!(checkpoint.active_turns[0].thread_id, "thread-1");
        assert_eq!(checkpoint.active_turns[0].turn_id, "turn-1");
        assert_eq!(checkpoint.active_turns[0].started_sequence, 1);
        assert_eq!(checkpoint.pending_approvals.len(), 1);
        assert!(checkpoint.pending_approvals[0].respondable);
        assert_eq!(checkpoint.pending_approvals[0].event.sequence, 514);
        assert_eq!(
            checkpoint.pending_approvals[0].event.payload["requestId"],
            "approval-1"
        );
        Ok(())
    }

    #[test]
    fn approval_insert_resolve_and_turn_clear_share_the_event_admission_barrier(
    ) -> Result<(), String> {
        let bridge = Arc::new(EventBridge::new());
        let approvals = Arc::new(PendingApprovalStore::default());
        bridge.reset(7, noop_emitter())?;
        approvals.reset(7);

        let event_guard = bridge.inner.lock().map_err(|error| error.to_string())?;
        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        let insertion_bridge = Arc::clone(&bridge);
        let insertion_approvals = Arc::clone(&approvals);
        let insertion = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let result = insertion_bridge.insert_approval_and_publish(
                &insertion_approvals,
                7,
                json!("approval-1"),
                "item/fileChange/requestApproval",
                Some(json!({
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "item-1"
                })),
            );
            let _ = done_tx.send(result);
        });
        started_rx.recv().map_err(|error| error.to_string())?;
        assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
        assert_eq!(approvals.len(), 0);
        drop(event_guard);
        assert!(done_rx.recv().map_err(|error| error.to_string())??);
        insertion
            .join()
            .map_err(|_| "approval insertion test thread panicked".to_string())?;
        assert_eq!(approvals.len(), 1);

        let event_guard = bridge.inner.lock().map_err(|error| error.to_string())?;
        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        let resolve_bridge = Arc::clone(&bridge);
        let resolve_approvals = Arc::clone(&approvals);
        let resolve = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let raw = json!({
                "requestId": "approval-1",
                "threadId": "thread-1"
            });
            let result = resolve_bridge.publish_notification(
                &resolve_approvals,
                7,
                "serverRequest/resolved",
                Some(&raw),
                CodeWorkspaceEventDraft {
                    thread_id: Some("thread-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    item_id: Some("item-1".to_string()),
                    kind: "serverRequest/resolved".to_string(),
                    payload: raw.clone(),
                },
            );
            let _ = done_tx.send(result);
        });
        started_rx.recv().map_err(|error| error.to_string())?;
        assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
        assert_eq!(approvals.len(), 1);
        drop(event_guard);
        done_rx.recv().map_err(|error| error.to_string())??;
        resolve
            .join()
            .map_err(|_| "approval resolve test thread panicked".to_string())?;
        assert_eq!(approvals.len(), 0);

        approvals.insert_request(
            7,
            json!("approval-2"),
            "item/fileChange/requestApproval",
            Some(json!({
                "threadId": "thread-1",
                "turnId": "turn-2",
                "itemId": "item-2"
            })),
        )?;
        let event_guard = bridge.inner.lock().map_err(|error| error.to_string())?;
        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        let completion_bridge = Arc::clone(&bridge);
        let completion_approvals = Arc::clone(&approvals);
        let completion = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let result = completion_bridge.publish_notification(
                &completion_approvals,
                7,
                "turn/completed",
                None,
                CodeWorkspaceEventDraft {
                    thread_id: Some("thread-1".to_string()),
                    turn_id: Some("turn-2".to_string()),
                    item_id: None,
                    kind: "turn/completed".to_string(),
                    payload: json!({ "turn": { "status": "completed" } }),
                },
            );
            let _ = done_tx.send(result);
        });
        started_rx.recv().map_err(|error| error.to_string())?;
        assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
        assert_eq!(approvals.len(), 1);
        drop(event_guard);
        done_rx.recv().map_err(|error| error.to_string())??;
        completion
            .join()
            .map_err(|_| "turn completion test thread panicked".to_string())?;
        assert_eq!(approvals.len(), 0);
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
    fn turn_selection_lists_before_write_and_rejects_unadvertised_pair_without_turn_bytes(
    ) -> Result<(), String> {
        let (_directory, executable) = fake_codex(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
: > "$0.requests"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"model/list"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"data\":[{\"id\":\"gpt-visible-id\",\"model\":\"gpt-visible\",\"displayName\":\"GPT Visible\",\"description\":\"Visible model\",\"hidden\":false,\"supportedReasoningEfforts\":[{\"reasoningEffort\":\"xhigh\",\"description\":\"Deep\"}],\"defaultReasoningEffort\":\"xhigh\",\"isDefault\":true}],\"nextCursor\":null}}"
      ;;
    *'"method":"turn/start"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"turn\":{\"id\":\"turn-model\",\"status\":\"inProgress\"}}}"
      ;;
    *)
      printf '%s\n' "{\"id\":$request_id,\"error\":{\"code\":-32601,\"message\":\"unexpected method\"}}"
      ;;
  esac
done
"#,
        )?;
        let request_log_executable = executable.clone();
        let runtime = CodeRuntime::with_executable(executable);
        let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
        runtime.start(noop_emitter())?;

        let accepted = runtime.turn_start_at(
            CodeTurnStartInput {
                scope: binding_scope(),
                thread_id: "thread-model".to_string(),
                prompt: "Use the selected model".to_string(),
                model: Some("gpt-visible".to_string()),
                effort: Some("xhigh".to_string()),
            },
            &workspace.path().to_string_lossy(),
        )?;
        assert_eq!(accepted.id, "turn-model");

        let error = runtime
            .turn_start_at(
                CodeTurnStartInput {
                    scope: binding_scope(),
                    thread_id: "thread-other".to_string(),
                    prompt: "Reject this effort".to_string(),
                    model: Some("gpt-visible".to_string()),
                    effort: Some("medium".to_string()),
                },
                &workspace.path().to_string_lossy(),
            )
            .expect_err("unadvertised effort must fail before turn/start");
        assert!(error.contains("not supported"));
        runtime.stop()?;

        let requests = recorded_requests(&request_log_executable)?;
        assert_eq!(
            requests
                .iter()
                .map(|request| request["method"].as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["model/list", "turn/start", "model/list"]
        );
        assert_eq!(requests[0]["params"]["includeHidden"], false);
        assert_eq!(requests[0]["params"]["limit"], 100);
        assert_eq!(requests_for_method(&requests, "turn/start").len(), 1);
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
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"thread\":{\"id\":\"thread-1\"},\"model\":\"gpt-test\",\"reasoningEffort\":null,\"instructionSources\":[]}}"
      printf '%s\n' '{"method":"thread/started","params":{"thread":{"id":"thread-1"}}}'
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"thread\":{\"id\":\"thread-1\",\"status\":{\"type\":\"idle\"},\"turns\":[{\"id\":\"past-turn\",\"status\":\"completed\",\"items\":[{\"type\":\"agentMessage\",\"text\":\"restored\"}],\"error\":null}]},\"model\":\"gpt-test\",\"reasoningEffort\":\"high\",\"instructionSources\":[]}}"
      ;;
    *'"method":"thread/name/set"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{}}"
      ;;
    *'"method":"thread/read"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"thread\":{\"id\":\"thread-1\",\"cwd\":\"/native/stored-root\",\"name\":\"Renamed native contract\"}}}"
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
        let renamed = runtime.thread_rename(&CodeThreadRenameInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
            name: "Renamed native contract".to_string(),
        })?;
        assert_eq!(renamed.id, "thread-1");
        assert_eq!(renamed.cwd.as_deref(), Some("/native/stored-root"));
        assert_eq!(renamed.name.as_deref(), Some("Renamed native contract"));

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
        assert_eq!(thread_reads.len(), 2);
        for thread_read in thread_reads {
            assert_eq!(
                thread_read["params"],
                json!({ "threadId": "thread-1", "includeTurns": false })
            );
        }
        let thread_name_sets = requests_for_method(&requests, "thread/name/set");
        assert_eq!(thread_name_sets.len(), 1);
        assert_eq!(
            thread_name_sets[0]["params"],
            json!({
                "threadId": "thread-1",
                "name": "Renamed native contract",
            })
        );
        let rename_index = requests
            .iter()
            .position(|request| request["method"] == "thread/name/set")
            .ok_or_else(|| "missing thread/name/set request".to_string())?;
        assert_eq!(
            requests
                .get(rename_index + 1)
                .map(|request| &request["method"]),
            Some(&json!("thread/read"))
        );
        let turn_requests = requests_for_method(&requests, "turn/start");
        assert_eq!(turn_requests.len(), 1);
        assert_eq!(turn_requests[0]["params"]["cwd"], workspace_root);
        assert_eq!(
            turn_requests[0]["params"]["sandboxPolicy"]["writableRoots"],
            json!([workspace_root])
        );
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "manual audit: requires the pinned Codex 0.145.0 CLI"]
    async fn pinned_codex_0_145_session_permission_round_trip_is_manual_only() -> Result<(), String>
    {
        let executable = crate::managed_agents::resolve_command("codex")
            .ok_or_else(|| "Codex CLI is not installed".to_string())?;
        let version = Command::new(&executable)
            .arg("--version")
            .output()
            .map_err(|error| error.to_string())?;
        if !version.status.success()
            || String::from_utf8_lossy(&version.stdout).trim() != "codex-cli 0.145.0"
        {
            return Err("permission round-trip requires exact codex-cli 0.145.0".to_string());
        }

        let request_permissions_arguments = serde_json::to_string(&json!({
            "reason": "Allow loopback permission probe",
            "permissions": {
                "network": {
                    "enabled": true
                }
            }
        }))
        .map_err(|error| error.to_string())?;
        let zero_usage = json!({
            "input_tokens": 0,
            "input_tokens_details": null,
            "output_tokens": 0,
            "output_tokens_details": null,
            "total_tokens": 0
        });
        let permission_response = sse_response(&[
            json!({
                "type": "response.created",
                "response": { "id": "response-permission" }
            }),
            json!({
                "type": "response.output_item.done",
                "item": {
                    "type": "function_call",
                    "call_id": "call1",
                    "name": "request_permissions",
                    "arguments": request_permissions_arguments
                }
            }),
            json!({
                "type": "response.completed",
                "response": {
                    "id": "response-permission",
                    "usage": zero_usage.clone()
                }
            }),
        ])?;
        let completion_response = sse_response(&[
            json!({
                "type": "response.created",
                "response": { "id": "response-complete" }
            }),
            json!({
                "type": "response.output_item.done",
                "item": {
                    "type": "message",
                    "role": "assistant",
                    "id": "message-complete",
                    "content": [{ "type": "output_text", "text": "done" }]
                }
            }),
            json!({
                "type": "response.completed",
                "response": {
                    "id": "response-complete",
                    "usage": zero_usage
                }
            }),
        ])?;
        let server =
            MockResponsesServer::start(vec![permission_response, completion_response]).await?;

        let codex_home = tempfile::tempdir().map_err(|error| error.to_string())?;
        fs::write(
            codex_home.path().join("config.toml"),
            format!(
                r#"
model = "mock-model"
model_provider = "mock_provider"
check_for_update_on_startup = false

[model_providers.mock_provider]
name = "SchoolX Code loopback permission test"
base_url = "{}"
env_key = "CODEX_PERMISSION_PROBE_TOKEN"
wire_api = "responses"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0

[features]
request_permissions_tool = true
"#,
                server.base_url
            ),
        )
        .map_err(|error| error.to_string())?;
        let (_wrapper_directory, wrapper) = real_codex_wrapper(&executable, codex_home.path())?;
        let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
        let workspace_root = workspace.path().to_string_lossy().into_owned();
        let expected_codex_home = fs::canonicalize(codex_home.path())
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let runtime = CodeRuntime::with_executable(wrapper);
        let scope = binding_scope();

        let ready = runtime.start(noop_emitter())?;
        assert_eq!(ready.version.as_deref(), Some("codex-cli 0.145.0"));
        assert_eq!(
            ready.codex_home.as_deref(),
            Some(expected_codex_home.as_str())
        );

        let opened = runtime
            .thread_start_at(
                CodeThreadStartInput {
                    scope: scope.clone(),
                    preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
                    model: None,
                },
                &workspace_root,
            )
            .map_err(CodeThreadStartRpcError::into_message)?;
        let turn = runtime.turn_start_at(
            CodeTurnStartInput {
                scope: scope.clone(),
                thread_id: opened.thread.id.clone(),
                prompt: "pick a directory".to_string(),
                model: None,
                effort: None,
            },
            &workspace_root,
        )?;

        let approval = wait_for_event_with_timeout(
            &runtime,
            "item/permissions/requestApproval",
            Duration::from_secs(10),
        )?;
        assert_eq!(approval.runtime_generation, ready.generation);
        assert_eq!(
            approval.thread_id.as_deref(),
            Some(opened.thread.id.as_str())
        );
        assert_eq!(approval.turn_id.as_deref(), Some(turn.id.as_str()));
        assert_eq!(approval.item_id.as_deref(), Some("call1"));
        assert_eq!(approval.payload["approvalKind"], "permissions");
        assert_eq!(
            approval.payload["request"]["reason"],
            "Allow loopback permission probe"
        );
        let request_id = CodeRequestId::from_value(approval.payload["requestId"].clone())?;
        let resolved_request_id = request_id.to_value();
        let requested_network = approval
            .payload
            .pointer("/request/permissionDisplay/network/enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| "permission request has no network grant".to_string())?;
        assert!(requested_network);

        runtime.approval_respond(CodeApprovalResponseInput {
            runtime_generation: ready.generation,
            request_id,
            scope,
            thread_id: opened.thread.id.clone(),
            turn_id: turn.id.clone(),
            response: CodeApprovalResponse::Permissions {
                intent: super::super::approvals::CodePermissionIntent::Grant,
                scope: CodePermissionScope::Session,
            },
        })?;
        let resolved = wait_for_event_with_timeout(
            &runtime,
            "serverRequest/resolved",
            Duration::from_secs(10),
        )?;
        assert_eq!(
            resolved.thread_id.as_deref(),
            Some(opened.thread.id.as_str())
        );
        assert_eq!(resolved.payload["requestId"], resolved_request_id);
        let completed =
            wait_for_event_with_timeout(&runtime, "turn/completed", Duration::from_secs(10))?;
        assert!(approval.sequence < resolved.sequence);
        assert!(resolved.sequence < completed.sequence);
        runtime.stop()?;

        let requests = server.requests()?;
        assert_eq!(requests.len(), 2);
        assert!(requests[0]["tools"].as_array().is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool["name"] == "request_permissions")
        }));
        let permission_output = requests[1]["input"]
            .as_array()
            .and_then(|input| {
                input.iter().find(|item| {
                    item["type"] == "function_call_output" && item["call_id"] == "call1"
                })
            })
            .and_then(|item| item["output"].as_str())
            .ok_or_else(|| "second model request has no permission tool output".to_string())?;
        let permission_output: Value =
            serde_json::from_str(permission_output).map_err(|error| error.to_string())?;
        assert_eq!(permission_output["permissions"]["network"]["enabled"], true);
        assert_eq!(permission_output["scope"], "session");
        assert_eq!(server.remaining_responses()?, 0);
        Ok(())
    }
}
