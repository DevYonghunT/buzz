use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::bindings::{CodeExecutionMode, CodeThreadBinding, CodeThreadBindingScope};
use super::worktrees::{CodeWorktreePrepareInput, CodeWorktreePrepareResult};

const MAX_PROMPT_BYTES: usize = 1024 * 1024;
const MAX_ID_BYTES: usize = 512;
const MAX_CURSOR_BYTES: usize = 16 * 1024;
pub(crate) const CODE_RECOVERY_THREAD_PAGE_LIMIT: u32 = 100;
const CODE_THREAD_SOURCE_PREFIX: &str = "schoolx-code/";

static SENSITIVE_ENV_VALUES: OnceLock<Vec<String>> = OnceLock::new();

/// JSON-RPC request ids used by app-server can be either numbers or strings.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CodeRequestId {
    Number(u64),
    String(String),
}

impl CodeRequestId {
    pub(crate) fn from_value(value: Value) -> Result<Self, String> {
        if let Some(number) = value.as_u64() {
            return Ok(Self::Number(number));
        }
        if let Some(string) = value.as_str() {
            validate_id("request", string)?;
            return Ok(Self::String(string.to_string()));
        }
        Err("Codex request id must be an unsigned number or string".to_string())
    }

    pub(crate) fn to_value(&self) -> Value {
        match self {
            Self::Number(value) => json!(value),
            Self::String(value) => json!(value),
        }
    }
}

/// Stable event envelope exposed to the frontend instead of raw JSON-RPC.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeWorkspaceEvent {
    /// Exact persisted SchoolX scope that owns this Codex thread.
    pub scope: CodeThreadBindingScope,
    pub runtime_generation: u64,
    pub sequence: u64,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub kind: String,
    pub payload: Value,
}

/// Normalized runtime event retained before a durable binding scope is known.
///
/// This type must not cross the Tauri boundary directly. The command facade
/// resolves its thread id through the native binding index and converts it to
/// [`CodeWorkspaceEvent`] only after finding an exact binding.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeRuntimeEvent {
    pub(crate) runtime_generation: u64,
    pub(crate) sequence: u64,
    pub(crate) thread_id: Option<String>,
    pub(crate) turn_id: Option<String>,
    pub(crate) item_id: Option<String>,
    pub(crate) kind: String,
    pub(crate) payload: Value,
}

impl CodeRuntimeEvent {
    pub(crate) fn into_scoped(self, scope: CodeThreadBindingScope) -> Option<CodeWorkspaceEvent> {
        self.thread_id.as_ref()?;
        Some(CodeWorkspaceEvent {
            scope,
            runtime_generation: self.runtime_generation,
            sequence: self.sequence,
            thread_id: self.thread_id,
            turn_id: self.turn_id,
            item_id: self.item_id,
            kind: self.kind,
            payload: self.payload,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CodeWorkspaceEventDraft {
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub kind: String,
    pub payload: Value,
}

/// Replay snapshot for a frontend listener that was detached temporarily.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeEventBacklog {
    pub runtime_generation: u64,
    pub latest_sequence: u64,
    pub truncated: bool,
    pub events: Vec<CodeWorkspaceEvent>,
}

/// Runtime replay snapshot before binding-scope filtering and enrichment.
#[derive(Clone, Debug)]
pub(crate) struct CodeRuntimeEventBacklog {
    pub(crate) runtime_generation: u64,
    pub(crate) latest_sequence: u64,
    pub(crate) truncated: bool,
    pub(crate) events: Vec<CodeRuntimeEvent>,
}

/// Parameters for opening a new Codex thread in one native-prepared execution root.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadStartInput {
    pub scope: CodeThreadBindingScope,
    pub preparation_id: String,
    #[serde(default)]
    pub model: Option<String>,
}

impl CodeThreadStartInput {
    pub(crate) fn rpc_params(&self, workspace_root: &str) -> Result<Value, String> {
        self.scope.validate()?;
        validate_id("preparation", &self.preparation_id)?;
        let thread_source = code_thread_source(&self.preparation_id)?;
        let mut params = Map::from_iter([
            ("cwd".to_string(), json!(workspace_root)),
            ("approvalPolicy".to_string(), json!("on-request")),
            ("sandbox".to_string(), json!("workspace-write")),
            ("serviceName".to_string(), json!("schoolx-code")),
            ("threadSource".to_string(), json!(thread_source)),
        ]);
        insert_optional_string(&mut params, "model", self.model.as_deref())?;
        Ok(Value::Object(params))
    }
}

/// Return the durable, per-preparation source marker used to correlate an
/// uncertain `thread/start` with exactly one Codex thread whenever app-server
/// still exposes that thread.
pub(crate) fn code_thread_source(preparation_id: &str) -> Result<String, String> {
    validate_id("preparation", preparation_id)?;
    let source = format!("{CODE_THREAD_SOURCE_PREFIX}{preparation_id}");
    validate_id("thread source", &source)?;
    Ok(source)
}

/// Scope plus Git selection used by the native worktree preparation command.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeWorktreePrepareCommandInput {
    /// Community/project/repository boundary that will own the preparation.
    pub scope: CodeThreadBindingScope,
    /// Existing path inside the selected Git worktree.
    pub repository_root: String,
    /// Git revision resolved to an immutable commit before preparation.
    pub base_ref: String,
    /// Managed-worktree default or explicit local-checkout mode.
    pub execution_mode: CodeExecutionMode,
}

impl CodeWorktreePrepareCommandInput {
    pub(crate) fn into_native(self) -> CodeWorktreePrepareInput {
        CodeWorktreePrepareInput {
            repository_root: self.repository_root,
            base_ref: self.base_ref,
            execution_mode: self.execution_mode,
        }
    }
}

/// Native-issued, durably journaled execution preparation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodePreparedWorktree {
    /// Opaque UUID required by `code_thread_start`.
    pub preparation_id: String,
    /// Exact scope persisted with the native descriptor.
    pub scope: CodeThreadBindingScope,
    /// Revalidated Git/worktree details shown to a future UI.
    pub worktree: CodeWorktreePrepareResult,
}

/// Exact scope whose unfinished native preparations should be listed.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadPreparationListInput {
    /// Community/project/repository scope used to filter the durable journal.
    pub scope: CodeThreadBindingScope,
}

/// Explicit reconciliation input for an ambiguous or failed binding commit.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadBindingRecoverInput {
    /// Exact scope that owns the durable `starting` preparation.
    pub scope: CodeThreadBindingScope,
    /// Opaque preparation UUID returned before `thread/start`.
    pub preparation_id: String,
    /// Optional model override forwarded while resuming the orphan thread.
    #[serde(default)]
    pub model: Option<String>,
}

/// Structured failure returned by `code_thread_start` so an orphaned Codex
/// thread can be reconciled without parsing a free-form message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeThreadStartError {
    /// Stable native error category.
    pub code: String,
    /// Redacted human-readable explanation.
    pub message: String,
    /// Durable preparation that remains reserved after an uncertain start.
    pub preparation_id: Option<String>,
    /// Codex thread id when app-server returned one before persistence failed.
    pub thread_id: Option<String>,
    /// Native execution root preserved for manual inspection and recovery.
    pub execution_root: Option<String>,
}

impl CodeThreadStartError {
    pub(crate) fn simple(code: &str, message: String) -> Self {
        Self {
            code: code.to_string(),
            message,
            preparation_id: None,
            thread_id: None,
            execution_root: None,
        }
    }

    pub(crate) fn recovery(
        code: &str,
        message: String,
        preparation_id: String,
        thread_id: Option<String>,
        execution_root: Option<String>,
    ) -> Self {
        Self {
            code: code.to_string(),
            message,
            preparation_id: Some(preparation_id),
            thread_id,
            execution_root,
        }
    }
}

/// Parameters for reconnecting a bound Codex thread in its persisted scope.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadResumeInput {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    #[serde(default)]
    pub model: Option<String>,
}

impl CodeThreadResumeInput {
    pub(crate) fn rpc_params(&self, workspace_root: &str) -> Result<Value, String> {
        self.scope.validate()?;
        validate_id("thread", &self.thread_id)?;
        let mut params = Map::from_iter([
            ("threadId".to_string(), json!(self.thread_id)),
            ("cwd".to_string(), json!(workspace_root)),
            ("approvalPolicy".to_string(), json!("on-request")),
            ("sandbox".to_string(), json!("workspace-write")),
        ]);
        insert_optional_string(&mut params, "model", self.model.as_deref())?;
        Ok(Value::Object(params))
    }
}

/// Exact persisted scope whose bound SchoolX Code threads should be listed.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadListInput {
    pub scope: CodeThreadBindingScope,
}

/// Narrow thread metadata exposed to the frontend.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeThreadSummary {
    pub id: String,
    pub session_id: Option<String>,
    pub forked_from_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub preview: Option<String>,
    pub ephemeral: bool,
    pub model_provider: Option<String>,
    pub created_at: Option<u64>,
    pub updated_at: Option<u64>,
    pub cwd: Option<String>,
    pub name: Option<String>,
    pub status: Option<Value>,
    pub turns: Vec<CodeTurnSnapshot>,
}

/// Persisted turn state returned while resuming a thread.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeTurnSnapshot {
    pub id: String,
    pub status: String,
    pub items: Vec<Value>,
    pub error: Option<Value>,
}

/// One app-server thread paired with its authoritative native binding.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeBoundThreadSummary {
    pub binding: CodeThreadBinding,
    /// App-server metadata when both the execution root and thread are available.
    pub thread: Option<CodeThreadSummary>,
    /// Per-binding recovery detail; one unavailable binding does not hide healthy peers.
    pub unavailable: Option<String>,
}

/// Result shared by bound thread start and resume commands.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeBoundThreadOpenResult {
    pub binding: CodeThreadBinding,
    pub thread: CodeThreadSummary,
    pub instruction_sources: Vec<String>,
}

/// One page of project-scoped app-server threads.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeThreadsPage {
    pub data: Vec<CodeBoundThreadSummary>,
    pub next_cursor: Option<String>,
    pub backwards_cursor: Option<String>,
}

/// Parameters for starting a prompt with fixed safe sandbox defaults.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeTurnStartInput {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl CodeTurnStartInput {
    pub(crate) fn rpc_params(&self, workspace_root: &str) -> Result<Value, String> {
        self.scope.validate()?;
        validate_id("thread", &self.thread_id)?;
        validate_prompt(&self.prompt)?;
        let mut params = Map::from_iter([
            ("threadId".to_string(), json!(self.thread_id)),
            (
                "input".to_string(),
                json!([{ "type": "text", "text": self.prompt, "text_elements": [] }]),
            ),
            ("cwd".to_string(), json!(workspace_root)),
            ("approvalPolicy".to_string(), json!("on-request")),
            (
                "sandboxPolicy".to_string(),
                json!({
                    "type": "workspaceWrite",
                    "writableRoots": [workspace_root],
                    "networkAccess": false,
                    "excludeTmpdirEnvVar": true,
                    "excludeSlashTmp": true
                }),
            ),
        ]);
        insert_optional_string(&mut params, "model", self.model.as_deref())?;
        insert_optional_string(&mut params, "effort", self.effort.as_deref())?;
        Ok(Value::Object(params))
    }
}

/// Parameters for adding guidance to the currently active turn.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeTurnSteerInput {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub expected_turn_id: String,
    pub prompt: String,
}

impl CodeTurnSteerInput {
    pub(crate) fn rpc_params(&self) -> Result<Value, String> {
        self.scope.validate()?;
        validate_id("thread", &self.thread_id)?;
        validate_id("turn", &self.expected_turn_id)?;
        validate_prompt(&self.prompt)?;
        Ok(json!({
            "threadId": self.thread_id,
            "input": [{ "type": "text", "text": self.prompt, "text_elements": [] }],
            "expectedTurnId": self.expected_turn_id
        }))
    }
}

/// Exact thread and turn identity required to request interruption.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeTurnInterruptInput {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub turn_id: String,
}

impl CodeTurnInterruptInput {
    pub(crate) fn rpc_params(&self) -> Result<Value, String> {
        self.scope.validate()?;
        validate_id("thread", &self.thread_id)?;
        validate_id("turn", &self.turn_id)?;
        Ok(json!({ "threadId": self.thread_id, "turnId": self.turn_id }))
    }
}

/// Narrow result returned when a turn starts or accepts steering input.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeTurnSummary {
    pub id: String,
    pub status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireThread {
    id: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    forked_from_id: Option<String>,
    #[serde(default)]
    parent_thread_id: Option<String>,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    ephemeral: bool,
    #[serde(default)]
    model_provider: Option<String>,
    #[serde(default)]
    created_at: Option<u64>,
    #[serde(default)]
    updated_at: Option<u64>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<Value>,
    #[serde(default)]
    thread_source: Option<String>,
    #[serde(default)]
    turns: Vec<WireTurnSnapshot>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireThreadListResult {
    data: Vec<WireThread>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct WireTurnSnapshot {
    id: String,
    status: String,
    #[serde(default)]
    items: Vec<Value>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireThreadOpenResult {
    thread: WireThread,
    #[serde(default)]
    instruction_sources: Vec<String>,
}

#[derive(Deserialize)]
struct WireThreadReadResult {
    thread: WireThread,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireThreadLoadedListResult {
    data: Vec<String>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct WireTurnEnvelope {
    turn: WireTurn,
}

#[derive(Deserialize)]
struct WireTurn {
    id: String,
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTurnSteerResult {
    turn_id: String,
}

/// Raw, unbound app-server response used only while the native runtime commits
/// the authoritative SchoolX binding.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeThreadRpcOpenResult {
    pub(crate) thread: CodeThreadSummary,
    pub(crate) instruction_sources: Vec<String>,
    pub(crate) thread_source: Option<String>,
}

/// One app-server thread returned while reconciling a native start attempt.
///
/// The opaque per-preparation source marker is consumed only by the native
/// binding logic. Together with the persisted pre-start baseline it excludes
/// both foreign app-server threads and concurrent SchoolX starts without
/// encoding scope or filesystem coordinates in the marker.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeRecoveryThread {
    pub(crate) thread: CodeThreadSummary,
    pub(crate) thread_source: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeRecoveryThreadPage {
    pub(crate) data: Vec<CodeRecoveryThread>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeLoadedThreadPage {
    pub(crate) data: Vec<String>,
    pub(crate) next_cursor: Option<String>,
}

pub(crate) fn parse_thread_open(value: Value) -> Result<CodeThreadRpcOpenResult, String> {
    let result: WireThreadOpenResult = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex thread response: {error}"))?;
    let thread_source = result.thread.thread_source.clone();
    if let Some(thread_source) = thread_source.as_deref() {
        validate_id("thread source", thread_source)?;
    }
    Ok(CodeThreadRpcOpenResult {
        thread: normalize_thread(result.thread)?,
        instruction_sources: result.instruction_sources,
        thread_source,
    })
}

/// Build the stable Codex 0.145 `thread/read` request used to hydrate one
/// thread selected from the native binding index.
pub(crate) fn thread_read_params(thread_id: &str) -> Result<Value, String> {
    validate_id("thread", thread_id)?;
    Ok(json!({ "threadId": thread_id, "includeTurns": false }))
}

/// Build the stable Codex 0.145 exact-root list request used only for native
/// start recovery. `appServer` is the spelling in the generated 0.145 schema.
pub(crate) fn recovery_thread_list_params(
    workspace_root: &str,
    cursor: Option<&str>,
) -> Result<Value, String> {
    if workspace_root.is_empty() {
        return Err("SchoolX Code recovery root cannot be empty".to_string());
    }
    if let Some(cursor) = cursor {
        validate_cursor(cursor)?;
    }
    let mut params = Map::from_iter([
        ("sourceKinds".to_string(), json!(["appServer"])),
        ("archived".to_string(), json!(false)),
        ("cwd".to_string(), json!(workspace_root)),
        ("limit".to_string(), json!(CODE_RECOVERY_THREAD_PAGE_LIMIT)),
        ("useStateDbOnly".to_string(), json!(false)),
        ("sortDirection".to_string(), json!("desc")),
        ("sortKey".to_string(), json!("created_at")),
    ]);
    if let Some(cursor) = cursor {
        params.insert("cursor".to_string(), json!(cursor));
    }
    Ok(Value::Object(params))
}

pub(crate) fn parse_recovery_thread_list(value: Value) -> Result<CodeRecoveryThreadPage, String> {
    let result: WireThreadListResult = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex thread list response: {error}"))?;
    let mut data = Vec::with_capacity(result.data.len());
    for thread in result.data {
        let thread_source = thread.thread_source.clone();
        if let Some(thread_source) = thread_source.as_deref() {
            validate_id("thread source", thread_source)?;
        }
        data.push(CodeRecoveryThread {
            thread: normalize_thread(thread)?,
            thread_source,
        });
    }
    if let Some(cursor) = result.next_cursor.as_deref() {
        validate_cursor(cursor)?;
    }
    Ok(CodeRecoveryThreadPage {
        data,
        next_cursor: result.next_cursor,
    })
}

pub(crate) fn recovery_thread_read_params(thread_id: &str) -> Result<Value, String> {
    thread_read_params(thread_id)
}

pub(crate) fn parse_recovery_thread_read(value: Value) -> Result<CodeRecoveryThread, String> {
    let result: WireThreadReadResult = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex recovery thread response: {error}"))?;
    let thread_source = result.thread.thread_source.clone();
    if let Some(thread_source) = thread_source.as_deref() {
        validate_id("thread source", thread_source)?;
    }
    Ok(CodeRecoveryThread {
        thread: normalize_thread(result.thread)?,
        thread_source,
    })
}

pub(crate) fn loaded_thread_list_params(cursor: Option<&str>) -> Result<Value, String> {
    if let Some(cursor) = cursor {
        validate_cursor(cursor)?;
    }
    let mut params =
        Map::from_iter([("limit".to_string(), json!(CODE_RECOVERY_THREAD_PAGE_LIMIT))]);
    if let Some(cursor) = cursor {
        params.insert("cursor".to_string(), json!(cursor));
    }
    Ok(Value::Object(params))
}

pub(crate) fn parse_loaded_thread_list(value: Value) -> Result<CodeLoadedThreadPage, String> {
    let result: WireThreadLoadedListResult = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex loaded thread list response: {error}"))?;
    for thread_id in &result.data {
        validate_id("thread", thread_id)?;
    }
    if let Some(cursor) = result.next_cursor.as_deref() {
        validate_cursor(cursor)?;
    }
    Ok(CodeLoadedThreadPage {
        data: result.data,
        next_cursor: result.next_cursor,
    })
}

pub(crate) fn parse_thread_read(value: Value) -> Result<CodeThreadSummary, String> {
    let result: WireThreadReadResult = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex thread read response: {error}"))?;
    normalize_thread(result.thread)
}

fn normalize_thread(thread: WireThread) -> Result<CodeThreadSummary, String> {
    validate_id("thread", &thread.id)?;
    let mut turns = Vec::with_capacity(thread.turns.len());
    for turn in thread.turns {
        validate_id("turn", &turn.id)?;
        turns.push(CodeTurnSnapshot {
            id: turn.id,
            status: turn.status,
            items: turn.items.into_iter().map(redact_protocol_value).collect(),
            error: turn.error.map(redact_protocol_value),
        });
    }
    Ok(CodeThreadSummary {
        id: thread.id,
        session_id: thread.session_id,
        forked_from_id: thread.forked_from_id,
        parent_thread_id: thread.parent_thread_id,
        preview: thread.preview.map(|preview| redact_protocol_text(&preview)),
        ephemeral: thread.ephemeral,
        model_provider: thread.model_provider,
        created_at: thread.created_at,
        updated_at: thread.updated_at,
        cwd: thread.cwd,
        name: thread.name,
        status: thread.status.map(redact_protocol_value),
        turns,
    })
}

pub(crate) fn parse_turn_start(value: Value) -> Result<CodeTurnSummary, String> {
    let result: WireTurnEnvelope = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex turn response: {error}"))?;
    validate_id("turn", &result.turn.id)?;
    Ok(CodeTurnSummary {
        id: result.turn.id,
        status: result.turn.status,
    })
}

pub(crate) fn parse_turn_steer(value: Value) -> Result<CodeTurnSummary, String> {
    let result: WireTurnSteerResult = serde_json::from_value(value)
        .map_err(|error| format!("invalid Codex steer response: {error}"))?;
    validate_id("turn", &result.turn_id)?;
    Ok(CodeTurnSummary {
        id: result.turn_id,
        status: "inProgress".to_string(),
    })
}

pub(crate) fn normalize_notification(
    method: &str,
    params: Option<Value>,
) -> Result<Option<CodeWorkspaceEventDraft>, String> {
    if !is_supported_notification(method) {
        return Ok(None);
    }
    let payload = params.unwrap_or(Value::Null);
    if !payload.is_object() && payload != Value::Null {
        return Err(format!(
            "Codex `{method}` notification params must be an object"
        ));
    }
    let thread_id = nested_id(&payload, "threadId", "thread")?;
    let turn_id = nested_id(&payload, "turnId", "turn")?;
    let item_id = nested_id(&payload, "itemId", "item")?;
    Ok(Some(CodeWorkspaceEventDraft {
        thread_id,
        turn_id,
        item_id,
        kind: method.to_string(),
        payload: redact_protocol_value(payload),
    }))
}

pub(crate) fn redact_protocol_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_protocol_text(&text)),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(redact_protocol_value).collect())
        }
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let value = if is_sensitive_payload_key(&key) {
                        Value::String("[REDACTED]".to_string())
                    } else {
                        redact_protocol_value(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        other => other,
    }
}

pub(crate) fn redact_protocol_text(text: &str) -> String {
    let secrets = SENSITIVE_ENV_VALUES.get_or_init(|| {
        std::env::vars()
            .filter(|(name, value)| is_sensitive_env_name(name) && value.len() >= 4)
            .map(|(_, value)| value)
            .collect()
    });
    let secret_refs = secrets.iter().map(String::as_str).collect::<Vec<_>>();
    let mut redacted = crate::managed_agents::redact_secrets_with(text, &secret_refs);
    for prefix in ["sk-"] {
        while let Some(position) = redacted.find(prefix) {
            let end = redacted[position..]
                .find(|character: char| {
                    character.is_whitespace() || character == '"' || character == '\''
                })
                .map(|offset| position + offset)
                .unwrap_or(redacted.len());
            redacted.replace_range(position..end, "[REDACTED]");
        }
    }
    redacted
}

fn is_sensitive_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "PRIVATE_KEY",
        "API_KEY",
        "APIKEY",
        "NSEC",
    ]
    .iter()
    .any(|marker| upper.contains(marker))
}

fn is_sensitive_payload_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    normalized.ends_with("token")
        || normalized.ends_with("secret")
        || normalized.ends_with("password")
        || normalized.ends_with("privatekey")
        || normalized.ends_with("apikey")
        || normalized == "authorization"
        || normalized == "nsec"
}

fn is_supported_notification(method: &str) -> bool {
    matches!(
        method,
        "error"
            | "warning"
            | "configWarning"
            | "thread/started"
            | "thread/status/changed"
            | "thread/closed"
            | "turn/started"
            | "turn/completed"
            | "turn/diff/updated"
            | "turn/plan/updated"
            | "item/started"
            | "item/completed"
            | "item/agentMessage/delta"
            | "item/plan/delta"
            | "item/reasoning/summaryTextDelta"
            | "item/reasoning/summaryPartAdded"
            | "item/reasoning/textDelta"
            | "item/commandExecution/outputDelta"
            | "item/commandExecution/terminalInteraction"
            | "item/fileChange/patchUpdated"
            | "serverRequest/resolved"
    )
}

fn nested_id(value: &Value, direct_key: &str, object_key: &str) -> Result<Option<String>, String> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if let Some(value) = object.get(direct_key) {
        return value
            .as_str()
            .ok_or_else(|| format!("Codex `{direct_key}` must be a string"))
            .and_then(|id| {
                validate_id(direct_key, id)?;
                Ok(Some(id.to_string()))
            });
    }
    let Some(value) = object.get(object_key).and_then(Value::as_object) else {
        return Ok(None);
    };
    let Some(id) = value.get("id") else {
        return Ok(None);
    };
    id.as_str()
        .ok_or_else(|| format!("Codex `{object_key}.id` must be a string"))
        .and_then(|id| {
            validate_id(object_key, id)?;
            Ok(Some(id.to_string()))
        })
}

pub(crate) fn validate_id(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(format!(
            "SchoolX Code {label} id must be between 1 and {MAX_ID_BYTES} bytes"
        ));
    }
    Ok(())
}

fn validate_prompt(prompt: &str) -> Result<(), String> {
    if prompt.trim().is_empty() {
        return Err("SchoolX Code prompt cannot be empty".to_string());
    }
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err(format!(
            "SchoolX Code prompt exceeded {MAX_PROMPT_BYTES} bytes"
        ));
    }
    Ok(())
}

fn insert_optional_string(
    target: &mut Map<String, Value>,
    key: &str,
    value: Option<&str>,
) -> Result<(), String> {
    if let Some(value) = value {
        if value.trim().is_empty() || value.len() > MAX_ID_BYTES {
            return Err(format!(
                "SchoolX Code {key} must be between 1 and {MAX_ID_BYTES} bytes"
            ));
        }
        target.insert(key.to_string(), json!(value));
    }
    Ok(())
}

fn validate_cursor(cursor: &str) -> Result<(), String> {
    if cursor.is_empty()
        || cursor.len() > MAX_CURSOR_BYTES
        || cursor.trim() != cursor
        || cursor.chars().any(char::is_control)
    {
        return Err(format!(
            "SchoolX Code thread list cursor must be a trimmed, non-control string between 1 and {MAX_CURSOR_BYTES} bytes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: "community-1".to_string(),
            project_dtag: "project-1".to_string(),
            repository_identity: "a".repeat(64),
        }
    }

    #[test]
    fn thread_start_uses_only_the_native_workspace_root_and_stable_fields() -> Result<(), String> {
        let input = CodeThreadStartInput {
            scope: scope(),
            preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
            model: Some("test-model".to_string()),
        };

        let params = input.rpc_params("/native/stored-root")?;

        assert_eq!(params["cwd"], "/native/stored-root");
        assert_eq!(params["approvalPolicy"], "on-request");
        assert_eq!(params["sandbox"], "workspace-write");
        assert_eq!(params["model"], "test-model");
        assert_eq!(
            params["threadSource"],
            "schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb"
        );
        assert!(params.get("scope").is_none());
        assert!(params.get("preparationId").is_none());
        assert!(params.get("descriptor").is_none());
        assert!(params.get("workspaceRoot").is_none());
        assert!(params.get("runtimeWorkspaceRoots").is_none());
        assert_ne!(
            code_thread_source("67f11a1d-0274-4d40-9b0c-e406e51c64fb")?,
            code_thread_source("77f11a1d-0274-4d40-9b0c-e406e51c64fb")?
        );
        Ok(())
    }

    #[test]
    fn thread_resume_uses_only_the_native_workspace_root_and_stable_fields() -> Result<(), String> {
        let input = CodeThreadResumeInput {
            scope: scope(),
            thread_id: "thread-1".to_string(),
            model: None,
        };

        let params = input.rpc_params("/native/stored-root")?;

        assert_eq!(params["threadId"], "thread-1");
        assert_eq!(params["cwd"], "/native/stored-root");
        assert_eq!(params["approvalPolicy"], "on-request");
        assert_eq!(params["sandbox"], "workspace-write");
        assert!(params.get("scope").is_none());
        assert!(params.get("workspaceRoot").is_none());
        assert!(params.get("runtimeWorkspaceRoots").is_none());
        Ok(())
    }

    #[test]
    fn turn_start_confines_cwd_and_writable_roots_to_the_native_root() -> Result<(), String> {
        let input = CodeTurnStartInput {
            scope: scope(),
            thread_id: "thread-1".to_string(),
            prompt: "Run the tests".to_string(),
            model: None,
            effort: None,
        };

        let params = input.rpc_params("/native/stored-root")?;

        assert_eq!(params["cwd"], "/native/stored-root");
        assert_eq!(
            params["sandboxPolicy"]["writableRoots"],
            json!(["/native/stored-root"])
        );
        assert_eq!(params["sandboxPolicy"]["networkAccess"], false);
        assert!(params.get("scope").is_none());
        assert!(params.get("workspaceRoot").is_none());
        assert!(params.get("runtimeWorkspaceRoots").is_none());
        Ok(())
    }

    #[test]
    fn scoped_inputs_reject_the_removed_workspace_root_field() {
        let decoded = serde_json::from_value::<CodeThreadResumeInput>(json!({
            "scope": scope(),
            "threadId": "thread-1",
            "workspaceRoot": "/caller/root"
        }));

        assert!(decoded.is_err());
    }

    #[test]
    fn recovery_input_rejects_a_caller_supplied_thread_id() {
        let decoded = serde_json::from_value::<CodeThreadBindingRecoverInput>(json!({
            "scope": scope(),
            "preparationId": "67f11a1d-0274-4d40-9b0c-e406e51c64fb",
            "threadId": "caller-selected-thread"
        }));

        assert!(decoded.is_err());
    }

    #[test]
    fn builds_and_parses_exact_root_recovery_list_contract() -> Result<(), String> {
        assert_eq!(
            recovery_thread_list_params("/native/stored-root", Some("next-page"))?,
            json!({
                "sourceKinds": ["appServer"],
                "archived": false,
                "cursor": "next-page",
                "cwd": "/native/stored-root",
                "limit": 100,
                "useStateDbOnly": false,
                "sortDirection": "desc",
                "sortKey": "created_at"
            })
        );

        let page = parse_recovery_thread_list(json!({
            "data": [{
                "id": "thread-1",
                "cwd": "/native/stored-root",
                "threadSource": "schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb"
            }],
            "nextCursor": "page-2",
            "backwardsCursor": "ignored"
        }))?;
        assert_eq!(page.data.len(), 1);
        assert_eq!(page.data[0].thread.id, "thread-1");
        assert_eq!(
            page.data[0].thread.cwd.as_deref(),
            Some("/native/stored-root")
        );
        assert_eq!(
            page.data[0].thread_source.as_deref(),
            Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb")
        );
        assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
        Ok(())
    }

    #[test]
    fn parses_loaded_and_read_recovery_contracts() -> Result<(), String> {
        assert_eq!(
            loaded_thread_list_params(Some("loaded-next"))?,
            json!({ "limit": 100, "cursor": "loaded-next" })
        );
        let loaded = parse_loaded_thread_list(json!({
            "data": ["thread-1", "thread-2"],
            "nextCursor": null
        }))?;
        assert_eq!(loaded.data, vec!["thread-1", "thread-2"]);
        assert_eq!(loaded.next_cursor, None);

        let recovered = parse_recovery_thread_read(json!({
            "thread": {
                "id": "thread-1",
                "cwd": "/native/stored-root",
                "threadSource": "schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb"
            }
        }))?;
        assert_eq!(recovered.thread.id, "thread-1");
        assert_eq!(
            recovered.thread_source.as_deref(),
            Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb")
        );
        Ok(())
    }

    #[test]
    fn builds_and_parses_thread_read_contract() -> Result<(), String> {
        assert_eq!(
            thread_read_params("thread-1")?,
            json!({ "threadId": "thread-1", "includeTurns": false })
        );

        let thread = parse_thread_read(json!({
            "thread": {
                "id": "thread-1",
                "cwd": "/native/stored-root",
                "preview": "Continue the native work"
            }
        }))?;
        assert_eq!(thread.id, "thread-1");
        assert_eq!(thread.cwd.as_deref(), Some("/native/stored-root"));
        assert_eq!(thread.preview.as_deref(), Some("Continue the native work"));
        assert!(thread.turns.is_empty());
        Ok(())
    }

    #[test]
    fn tauri_event_serializes_with_its_exact_binding_scope() -> Result<(), String> {
        let event = CodeRuntimeEvent {
            runtime_generation: 7,
            sequence: 11,
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            item_id: None,
            kind: "turn/started".to_string(),
            payload: json!({ "threadId": "thread-1" }),
        }
        .into_scoped(scope())
        .ok_or_else(|| "expected a thread-bound event".to_string())?;

        let value = serde_json::to_value(event).map_err(|error| error.to_string())?;
        assert_eq!(value["scope"]["communityId"], "community-1");
        assert_eq!(value["scope"]["projectDtag"], "project-1");
        assert_eq!(value["scope"]["repositoryIdentity"], "a".repeat(64));
        assert_eq!(value["runtimeGeneration"], 7);
        assert_eq!(value["sequence"], 11);
        assert_eq!(value["threadId"], "thread-1");
        Ok(())
    }

    #[test]
    fn redacts_secret_shaped_text_in_rpc_diagnostics() {
        let redacted = redact_protocol_text("request failed with sk-project-secret");
        assert!(!redacted.contains("sk-project-secret"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn normalizes_delta_ids_and_redacts_known_secret_shapes() -> Result<(), String> {
        let event = normalize_notification(
            "item/agentMessage/delta",
            Some(json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1",
                "delta": "tokens ghp_abc123456789 and sk-project-secret",
                "accessToken": "arbitrary-value"
            })),
        )?
        .ok_or_else(|| "expected a supported notification".to_string())?;

        assert_eq!(event.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(event.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(event.item_id.as_deref(), Some("item-1"));
        assert_eq!(event.payload["delta"], "tokens [REDACTED] and [REDACTED]");
        assert_eq!(event.payload["accessToken"], "[REDACTED]");
        Ok(())
    }

    #[test]
    fn ignores_unknown_notifications() -> Result<(), String> {
        assert!(normalize_notification("future/event", Some(json!({})))?.is_none());
        Ok(())
    }
}
