//! Audited Codex 0.145/0.149 thread lifecycle and authoritative graph contracts.
//!
//! Lifecycle commands deliberately use a global, cwd-free thread inventory.
//! The inventory is stricter than the user-facing thread projection because it
//! is an authorization proof: an unknown source, incomplete ancestry, duplicate
//! id, or malformed page must fail before `thread/archive` can be sent.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::bindings::CodeThreadBindingScope;
use super::paths::canonical_workspace_root;
use super::protocol::{self, CodeThreadSummary};

pub(crate) const CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT: u32 = 100;
pub(crate) const MAX_AUTHORITATIVE_THREADS: usize = 4_096;
pub(crate) const MAX_AUTHORITATIVE_PAGES: usize = 64;

/// Exact audited Codex 0.145/0.149 `ThreadSourceKind` filter, including both
/// the sub-agent umbrella and every specialized sub-agent classification.
pub(crate) const SUPPORTED_CODEX_THREAD_SOURCE_KINDS: [&str; 10] = [
    "cli",
    "vscode",
    "exec",
    "appServer",
    "subAgent",
    "subAgentReview",
    "subAgentCompact",
    "subAgentThreadSpawn",
    "subAgentOther",
    "unknown",
];

/// Public lifecycle command coordinate. The webview never supplies cwd,
/// archive paths, operation ids, or filesystem authority.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadLifecycleInput {
    /// Exact community/project/repository scope that owns the binding.
    pub scope: CodeThreadBindingScope,
    /// Opaque Codex thread id within that persisted scope.
    pub thread_id: String,
}

impl CodeThreadLifecycleInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.scope.validate()?;
        protocol::validate_id("thread", &self.thread_id)
    }

    pub(crate) fn rpc_params(&self) -> Result<Value, String> {
        self.validate()?;
        Ok(json!({ "threadId": self.thread_id }))
    }
}

/// Which mutually-exclusive app-server inventory currently owns a thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CodeThreadMembership {
    Active,
    Archived,
}

/// Closed pinned representation of the runtime status carried by a Thread.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum CodePinnedThreadStatus {
    NotLoaded,
    Idle,
    SystemError,
    Active {
        #[serde(rename = "activeFlags")]
        _active_flags: Vec<CodePinnedThreadActiveFlag>,
    },
}

impl CodePinnedThreadStatus {
    pub(crate) fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    pub(crate) fn proves_quiescent(&self) -> bool {
        matches!(self, Self::NotLoaded | Self::Idle)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CodePinnedThreadActiveFlag {
    WaitingOnApproval,
    WaitingOnUserInput,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CodePinnedTurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
}

impl CodePinnedTurnStatus {
    pub(crate) fn parse(status: &str) -> Result<Self, String> {
        match status {
            "completed" => Ok(Self::Completed),
            "interrupted" => Ok(Self::Interrupted),
            "failed" => Ok(Self::Failed),
            "inProgress" => Ok(Self::InProgress),
            _ => Err(format!("Codex returned unsupported turn status `{status}`")),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::InProgress => "inProgress",
        }
    }

    pub(crate) fn is_terminal(self) -> bool {
        !matches!(self, Self::InProgress)
    }
}

/// Strict metadata retained for lifecycle membership and leaf proofs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodeAuthoritativeThread {
    pub(crate) id: String,
    pub(crate) membership: CodeThreadMembership,
    pub(crate) cwd: String,
    pub(crate) parent_thread_id: Option<String>,
    pub(crate) forked_from_id: Option<String>,
    pub(crate) status: CodePinnedThreadStatus,
}

impl CodeAuthoritativeThread {
    fn ancestry(&self) -> impl Iterator<Item = &str> {
        self.parent_thread_id
            .as_deref()
            .into_iter()
            .chain(self.forked_from_id.as_deref())
    }
}

/// One strictly parsed active or archived list page.
#[derive(Clone, Debug)]
pub(crate) struct CodeAuthoritativeThreadPage {
    pub(crate) data: Vec<CodeAuthoritativeThread>,
    pub(crate) next_cursor: Option<String>,
}

/// Exact journal authority for one response-lost fork child that may be
/// loaded in memory before Codex persists it into either list membership.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodePendingForkExpectation {
    pub(crate) preparation_id: String,
    pub(crate) source_thread_id: String,
    pub(crate) execution_root: String,
    pub(crate) recovery_thread_baseline: Vec<String>,
}

/// Complete active+archived snapshot used for membership reconciliation and
/// archive leaf authorization.
#[derive(Clone, Debug)]
pub(crate) struct CodeAuthoritativeThreadGraph {
    threads: HashMap<String, CodeAuthoritativeThread>,
    children: HashMap<String, Vec<String>>,
}

impl CodeAuthoritativeThreadGraph {
    pub(crate) fn from_threads(
        threads: impl IntoIterator<Item = CodeAuthoritativeThread>,
    ) -> Result<Self, String> {
        let mut by_id = HashMap::new();
        for thread in threads {
            if by_id.len() >= MAX_AUTHORITATIVE_THREADS {
                return Err(format!(
                    "Codex authoritative graph exceeds the {MAX_AUTHORITATIVE_THREADS}-thread safety limit"
                ));
            }
            let id = thread.id.clone();
            if by_id.insert(id.clone(), thread.clone()).is_some() {
                return Err(format!(
                    "Codex authoritative graph contained duplicate thread id {id}"
                ));
            }
        }

        let mut children = HashMap::<String, Vec<String>>::new();
        let mut edge_count = 0_usize;
        for thread in by_id.values() {
            for ancestor in thread.ancestry() {
                if ancestor == thread.id {
                    return Err(format!(
                        "Codex thread {} contains a self-referential ancestry edge",
                        thread.id
                    ));
                }
                if !by_id.contains_key(ancestor) {
                    return Err(format!(
                        "Codex thread {} references missing ancestry target {ancestor}",
                        thread.id
                    ));
                }
                children
                    .entry(ancestor.to_string())
                    .or_default()
                    .push(thread.id.clone());
                edge_count = edge_count.saturating_add(1);
            }
        }
        for descendants in children.values_mut() {
            descendants.sort();
            descendants.dedup();
        }
        validate_acyclic_ancestry(&by_id, edge_count)?;

        Ok(Self {
            threads: by_id,
            children,
        })
    }

    pub(crate) fn thread(&self, thread_id: &str) -> Option<&CodeAuthoritativeThread> {
        self.threads.get(thread_id)
    }

    pub(crate) fn membership(&self, thread_id: &str) -> Option<CodeThreadMembership> {
        self.thread(thread_id).map(|thread| thread.membership)
    }

    /// Require the target to exist in exactly one membership and have no direct
    /// or transitive descendant through either pinned ancestry field.
    pub(crate) fn ensure_leaf(
        &self,
        target_thread_id: &str,
    ) -> Result<CodeThreadMembership, String> {
        protocol::validate_id("archive target thread", target_thread_id)?;
        let membership = self.membership(target_thread_id).ok_or_else(|| {
            "Codex authoritative graph did not contain the archive target".to_string()
        })?;

        let mut pending = VecDeque::from([target_thread_id.to_string()]);
        let mut visited = HashSet::new();
        visited.insert(target_thread_id.to_string());
        while let Some(parent) = pending.pop_front() {
            let Some(children) = self.children.get(&parent) else {
                continue;
            };
            for child in children {
                if child != target_thread_id {
                    return Err(format!(
                        "Codex thread {target_thread_id} has descendant {child}; archive cascade was refused"
                    ));
                }
                if visited.insert(child.clone()) {
                    pending.push_back(child.clone());
                }
            }
        }
        Ok(membership)
    }
}

fn validate_acyclic_ancestry(
    threads: &HashMap<String, CodeAuthoritativeThread>,
    expected_edges: usize,
) -> Result<(), String> {
    let mut indegree = threads
        .keys()
        .map(|thread_id| (thread_id.clone(), 0_usize))
        .collect::<HashMap<_, _>>();
    let mut outgoing = HashMap::<String, Vec<String>>::new();
    let mut actual_edges = 0_usize;
    for thread in threads.values() {
        for ancestor in thread.ancestry() {
            let degree = indegree
                .get_mut(ancestor)
                .ok_or_else(|| "Codex ancestry validation lost a referenced thread".to_string())?;
            *degree = degree.saturating_add(1);
            outgoing
                .entry(thread.id.clone())
                .or_default()
                .push(ancestor.to_string());
            actual_edges = actual_edges.saturating_add(1);
        }
    }
    if actual_edges != expected_edges {
        return Err("Codex ancestry edge accounting was inconsistent".to_string());
    }

    let mut queue = indegree
        .iter()
        .filter_map(|(thread_id, degree)| (*degree == 0).then_some(thread_id.clone()))
        .collect::<VecDeque<_>>();
    let mut processed = 0_usize;
    while let Some(thread_id) = queue.pop_front() {
        processed = processed.saturating_add(1);
        for ancestor in outgoing.get(&thread_id).into_iter().flatten() {
            let degree = indegree
                .get_mut(ancestor)
                .ok_or_else(|| "Codex ancestry validation lost an outgoing target".to_string())?;
            *degree = degree.saturating_sub(1);
            if *degree == 0 {
                queue.push_back(ancestor.clone());
            }
        }
    }
    if processed != threads.len() {
        return Err("Codex authoritative graph contains an ancestry cycle".to_string());
    }
    Ok(())
}

pub(crate) fn authoritative_thread_list_params(
    membership: CodeThreadMembership,
    cursor: Option<&str>,
) -> Result<Value, String> {
    if let Some(cursor) = cursor {
        protocol::validate_cursor(cursor)?;
    }
    let mut params = Map::from_iter([
        (
            "sourceKinds".to_string(),
            json!(SUPPORTED_CODEX_THREAD_SOURCE_KINDS),
        ),
        (
            "archived".to_string(),
            json!(membership == CodeThreadMembership::Archived),
        ),
        (
            "limit".to_string(),
            json!(CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT),
        ),
        ("useStateDbOnly".to_string(), json!(false)),
        ("sortDirection".to_string(), json!("desc")),
        ("sortKey".to_string(), json!("created_at")),
    ]);
    if let Some(cursor) = cursor {
        params.insert("cursor".to_string(), json!(cursor));
    }
    Ok(Value::Object(params))
}

pub(crate) fn parse_authoritative_thread_list(
    value: Value,
    membership: CodeThreadMembership,
) -> Result<CodeAuthoritativeThreadPage, String> {
    let page: WireThreadListResult = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex authoritative thread list: {error}"))?;
    if page.data.len() > CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT as usize {
        return Err(format!(
            "Codex authoritative thread list exceeded the {CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT}-thread page limit"
        ));
    }
    if let Some(cursor) = page.next_cursor.as_deref() {
        protocol::validate_cursor(cursor)?;
    }
    if let Some(cursor) = page.backwards_cursor.as_deref() {
        protocol::validate_cursor(cursor)?;
    }
    let data = page
        .data
        .into_iter()
        .map(|thread| normalize_authoritative_thread(thread, membership))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CodeAuthoritativeThreadPage {
        data,
        next_cursor: page.next_cursor,
    })
}

pub(crate) fn authoritative_thread_read_params(thread_id: &str) -> Result<Value, String> {
    protocol::validate_id("thread", thread_id)?;
    Ok(json!({ "threadId": thread_id, "includeTurns": true }))
}

/// Strict result of the pinned read used to prove that an archive target has no
/// active or in-progress turn. This is deliberately separate from the lenient
/// display projection used by the timeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodeAuthoritativeThreadActivity {
    pub(crate) id: String,
    pub(crate) cwd: String,
    pub(crate) status: CodePinnedThreadStatus,
    pub(crate) turns: Vec<(String, CodePinnedTurnStatus)>,
}

impl CodeAuthoritativeThreadActivity {
    pub(crate) fn ensure_quiescent(&self) -> Result<(), String> {
        if !self.status.proves_quiescent() {
            return Err(match &self.status {
                CodePinnedThreadStatus::Active { .. } => {
                    "Codex thread still has an active turn".to_string()
                }
                CodePinnedThreadStatus::SystemError => {
                    "Codex thread status cannot prove an idle archive boundary".to_string()
                }
                CodePinnedThreadStatus::NotLoaded | CodePinnedThreadStatus::Idle => {
                    "Codex thread did not prove an idle archive boundary".to_string()
                }
            });
        }
        if let Some((turn_id, _)) = self
            .turns
            .iter()
            .find(|(_, status)| *status == CodePinnedTurnStatus::InProgress)
        {
            return Err(format!(
                "Codex thread has in-progress turn {turn_id}; archive was refused"
            ));
        }
        Ok(())
    }
}

pub(crate) fn parse_authoritative_thread_read(
    value: Value,
) -> Result<CodeAuthoritativeThreadActivity, String> {
    let result: WireAuthoritativeThreadReadResult = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex authoritative thread read: {error}"))?;
    let WireAuthoritativeThreadWithTurns {
        id,
        cwd,
        source,
        status,
        parent_thread_id,
        forked_from_id,
        turns: wire_turns,
    } = result.thread;
    let metadata = normalize_authoritative_thread(
        WireAuthoritativeThread {
            id,
            cwd,
            source,
            status,
            parent_thread_id,
            forked_from_id,
        },
        CodeThreadMembership::Active,
    )?;
    let mut turns = Vec::with_capacity(wire_turns.len());
    for turn in wire_turns {
        protocol::validate_id("turn", &turn.id)?;
        turns.push((turn.id, turn.status));
    }
    Ok(CodeAuthoritativeThreadActivity {
        id: metadata.id,
        cwd: metadata.cwd,
        status: metadata.status,
        turns,
    })
}

/// Strictly normalize an exact durable binding that Codex reports as loaded
/// but has not yet persisted into `thread/list`. This permits either a root or
/// one fork edge without admitting arbitrary unbound loaded ids.
pub(crate) fn parse_authoritative_deferred_bound_thread_read(
    value: Value,
) -> Result<CodeAuthoritativeThread, String> {
    let thread = parse_deferred_schoolx_thread(value)?;
    if thread.parent_thread_id.is_some()
        || thread.session_id != thread.id
        || thread.ephemeral
        || !is_schoolx_app_server_source(&thread.source)
        || thread
            .thread_source
            .as_deref()
            .is_none_or(|source| protocol::validate_code_thread_source_marker(source).is_err())
        || !thread.status.proves_quiescent()
        || !thread.turns.is_empty()
    {
        return Err(
            "Codex list-absent bound thread was not a quiescent SchoolX root or fork".to_string(),
        );
    }
    normalize_authoritative_thread(
        WireAuthoritativeThread {
            id: thread.id,
            cwd: thread.cwd,
            source: thread.source,
            status: thread.status,
            parent_thread_id: thread.parent_thread_id,
            forked_from_id: thread.forked_from_id,
        },
        CodeThreadMembership::Active,
    )
}

/// Match one list-absent loaded child to one exact Starting fork journal.
pub(crate) fn parse_authoritative_pending_fork_thread_read(
    value: Value,
    expectation: &CodePendingForkExpectation,
) -> Result<CodeAuthoritativeThread, String> {
    protocol::validate_id("fork preparation", &expectation.preparation_id)?;
    protocol::validate_id("fork source thread", &expectation.source_thread_id)?;
    let expected_source = protocol::code_thread_source(&expectation.preparation_id)?;
    let thread = parse_deferred_schoolx_thread(value)?;
    if thread.id == expectation.source_thread_id
        || thread.session_id != thread.id
        || thread.ephemeral
        || thread.parent_thread_id.is_some()
        || thread.forked_from_id.as_deref() != Some(expectation.source_thread_id.as_str())
        || thread.thread_source.as_deref() != Some(expected_source.as_str())
        || expectation
            .recovery_thread_baseline
            .binary_search(&thread.id)
            .is_ok()
        || !is_schoolx_app_server_source(&thread.source)
        || !thread.status.proves_quiescent()
        || !thread.turns.is_empty()
        || canonical_workspace_root(&thread.cwd)? != expectation.execution_root
    {
        return Err(
            "Codex list-absent loaded thread did not match the exact pending fork journal"
                .to_string(),
        );
    }
    normalize_authoritative_thread(
        WireAuthoritativeThread {
            id: thread.id,
            cwd: thread.cwd,
            source: thread.source,
            status: thread.status,
            parent_thread_id: thread.parent_thread_id,
            forked_from_id: thread.forked_from_id,
        },
        CodeThreadMembership::Active,
    )
}

/// Codex 0.149 currently labels threads created through its app-server as
/// `vscode`; retain the 0.145/schema spelling while rejecting every unrelated
/// source. Exact SchoolX source markers and roots are checked independently.
fn is_schoolx_app_server_source(source: &WireSessionSource) -> bool {
    matches!(
        source,
        WireSessionSource::Simple(
            WireSimpleSessionSource::AppServer | WireSimpleSessionSource::Vscode
        )
    )
}

fn parse_deferred_schoolx_thread(value: Value) -> Result<WireDeferredSchoolXThread, String> {
    let result: WireDeferredSchoolXThreadReadResult = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex deferred SchoolX thread read: {error}"))?;
    protocol::validate_id("deferred thread", &result.thread.id)?;
    protocol::validate_id("deferred session", &result.thread.session_id)?;
    if let Some(thread_source) = result.thread.thread_source.as_deref() {
        protocol::validate_id("thread source", thread_source)?;
    }
    if let Some(forked_from_id) = result.thread.forked_from_id.as_deref() {
        protocol::validate_id("fork source thread", forked_from_id)?;
    }
    for turn in &result.thread.turns {
        protocol::validate_id("turn", &turn.id)?;
    }
    Ok(result.thread)
}

pub(crate) fn parse_thread_archive(value: Value) -> Result<(), String> {
    match value.as_object() {
        Some(result) if result.is_empty() => Ok(()),
        _ => Err("invalid Codex thread archive response: expected an empty object".to_string()),
    }
}

pub(crate) fn parse_thread_unarchive(value: Value) -> Result<CodeThreadSummary, String> {
    protocol::parse_thread_read(value)
        .map_err(|error| format!("invalid Codex thread unarchive response: {error}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireThreadListResult {
    data: Vec<WireAuthoritativeThread>,
    #[serde(default)]
    next_cursor: Option<String>,
    #[serde(default)]
    backwards_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAuthoritativeThread {
    id: String,
    cwd: String,
    source: WireSessionSource,
    status: CodePinnedThreadStatus,
    #[serde(default)]
    parent_thread_id: Option<String>,
    #[serde(default)]
    forked_from_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireAuthoritativeThreadReadResult {
    thread: WireAuthoritativeThreadWithTurns,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireDeferredSchoolXThreadReadResult {
    thread: WireDeferredSchoolXThread,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireDeferredSchoolXThread {
    id: String,
    session_id: String,
    cwd: String,
    source: WireSessionSource,
    status: CodePinnedThreadStatus,
    ephemeral: bool,
    #[serde(default)]
    parent_thread_id: Option<String>,
    #[serde(default)]
    forked_from_id: Option<String>,
    #[serde(default)]
    thread_source: Option<String>,
    turns: Vec<WireAuthoritativeTurn>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAuthoritativeThreadWithTurns {
    id: String,
    cwd: String,
    source: WireSessionSource,
    status: CodePinnedThreadStatus,
    #[serde(default)]
    parent_thread_id: Option<String>,
    #[serde(default)]
    forked_from_id: Option<String>,
    turns: Vec<WireAuthoritativeTurn>,
}

#[derive(Deserialize)]
struct WireAuthoritativeTurn {
    id: String,
    status: CodePinnedTurnStatus,
    #[serde(rename = "items")]
    _items: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
enum WireSessionSource {
    Simple(WireSimpleSessionSource),
    Custom(WireCustomSessionSource),
    SubAgent(WireSubAgentSessionSource),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
enum WireSimpleSessionSource {
    #[serde(rename = "cli")]
    Cli,
    #[serde(rename = "vscode")]
    Vscode,
    #[serde(rename = "exec")]
    Exec,
    #[serde(rename = "appServer")]
    AppServer,
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct WireCustomSessionSource {
    custom: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireSubAgentSessionSource {
    sub_agent: WireSubAgentSource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
enum WireSubAgentSource {
    Simple(WireSimpleSubAgentSource),
    ThreadSpawn(WireThreadSpawnSubAgentSource),
    Other(WireOtherSubAgentSource),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
enum WireSimpleSubAgentSource {
    #[serde(rename = "review")]
    Review,
    #[serde(rename = "compact")]
    Compact,
    #[serde(rename = "memory_consolidation")]
    MemoryConsolidation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct WireThreadSpawnSubAgentSource {
    thread_spawn: WireThreadSpawnSource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct WireThreadSpawnSource {
    #[serde(rename = "depth")]
    _depth: i32,
    parent_thread_id: String,
    #[serde(default)]
    #[serde(rename = "agent_nickname")]
    _agent_nickname: Option<String>,
    #[serde(default)]
    #[serde(rename = "agent_path")]
    _agent_path: Option<String>,
    #[serde(default)]
    #[serde(rename = "agent_role")]
    _agent_role: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct WireOtherSubAgentSource {
    other: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CodePinnedSessionSource {
    Cli,
    Vscode,
    Exec,
    AppServer,
    SubAgent { thread_spawn_parent: Option<String> },
}

fn normalize_source(source: WireSessionSource) -> Result<CodePinnedSessionSource, String> {
    match source {
        WireSessionSource::Simple(WireSimpleSessionSource::Cli) => Ok(CodePinnedSessionSource::Cli),
        WireSessionSource::Simple(WireSimpleSessionSource::Vscode) => {
            Ok(CodePinnedSessionSource::Vscode)
        }
        WireSessionSource::Simple(WireSimpleSessionSource::Exec) => {
            Ok(CodePinnedSessionSource::Exec)
        }
        WireSessionSource::Simple(WireSimpleSessionSource::AppServer) => {
            Ok(CodePinnedSessionSource::AppServer)
        }
        WireSessionSource::Simple(WireSimpleSessionSource::Unknown) => {
            Err("Codex authoritative graph returned an unknown thread source".to_string())
        }
        WireSessionSource::Custom(_) => {
            Err("Codex authoritative graph returned a custom thread source".to_string())
        }
        WireSessionSource::SubAgent(source) => {
            let thread_spawn_parent = match source.sub_agent {
                WireSubAgentSource::ThreadSpawn(source) => {
                    protocol::validate_id(
                        "sub-agent source parent thread",
                        &source.thread_spawn.parent_thread_id,
                    )?;
                    Some(source.thread_spawn.parent_thread_id)
                }
                WireSubAgentSource::Simple(_simple) => None,
                WireSubAgentSource::Other(_other) => None,
            };
            Ok(CodePinnedSessionSource::SubAgent {
                thread_spawn_parent,
            })
        }
    }
}

fn normalize_authoritative_thread(
    thread: WireAuthoritativeThread,
    membership: CodeThreadMembership,
) -> Result<CodeAuthoritativeThread, String> {
    protocol::validate_id("authoritative thread", &thread.id)?;
    if thread.cwd.is_empty() {
        return Err(format!(
            "Codex authoritative thread {} returned an empty cwd",
            thread.id
        ));
    }
    for ancestry in [
        thread.parent_thread_id.as_deref(),
        thread.forked_from_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        protocol::validate_id("thread ancestry", ancestry)?;
    }
    if let (Some(parent), Some(forked_from)) = (
        thread.parent_thread_id.as_deref(),
        thread.forked_from_id.as_deref(),
    ) {
        if parent != forked_from {
            return Err(format!(
                "Codex thread {} reported conflicting parentThreadId and forkedFromId ancestry",
                thread.id
            ));
        }
    }

    let source = normalize_source(thread.source)?;
    if let CodePinnedSessionSource::SubAgent {
        thread_spawn_parent,
    } = &source
    {
        let parent = thread.parent_thread_id.as_deref().ok_or_else(|| {
            format!(
                "Codex sub-agent thread {} did not report parentThreadId",
                thread.id
            )
        })?;
        if thread_spawn_parent
            .as_deref()
            .is_some_and(|source_parent| source_parent != parent)
        {
            return Err(format!(
                "Codex thread {} reported conflicting source and parentThreadId ancestry",
                thread.id
            ));
        }
    }

    Ok(CodeAuthoritativeThread {
        id: thread.id,
        membership,
        cwd: thread.cwd,
        parent_thread_id: thread.parent_thread_id,
        forked_from_id: thread.forked_from_id,
        status: thread.status,
    })
}

#[cfg(test)]
mod tests;
