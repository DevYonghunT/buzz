use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::bindings::{
    CodeExecutionMode, CodeThreadBinding, CodeThreadBindingScope, CodeThreadLifecycleStatus,
};
use super::worktrees::{CodeWorktreePrepareInput, CodeWorktreePrepareResult};

const MAX_PROMPT_BYTES: usize = 1024 * 1024;
const MAX_ID_BYTES: usize = 512;
const MAX_CURSOR_BYTES: usize = 16 * 1024;
const MAX_THREAD_NAME_SCALARS: usize = 128;
const MAX_THREAD_NAME_BYTES: usize = 512;
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

/// One active turn captured at an exact runtime event watermark.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActiveTurnCheckpoint {
    pub thread_id: String,
    pub turn_id: String,
    pub status: String,
    pub started_sequence: u64,
}

/// One pending native approval captured at an exact runtime event watermark.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeApprovalCheckpoint {
    pub event: CodeWorkspaceEvent,
    /// False only while native has reserved the response for an in-flight write.
    pub respondable: bool,
}

/// Authoritative transient runtime state paired with one replay watermark.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeEventCheckpoint {
    pub runtime_generation: u64,
    pub sequence_watermark: u64,
    pub active_turns: Vec<CodeActiveTurnCheckpoint>,
    pub pending_approvals: Vec<CodeApprovalCheckpoint>,
}

/// Replay snapshot for a frontend listener that was detached temporarily.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeEventBacklog {
    pub runtime_generation: u64,
    pub latest_sequence: u64,
    pub truncated: bool,
    pub checkpoint: Option<CodeEventCheckpoint>,
    pub events: Vec<CodeWorkspaceEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodeRuntimeActiveTurnCheckpoint {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) status: String,
    pub(crate) started_sequence: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeRuntimeApprovalCheckpoint {
    pub(crate) event: CodeRuntimeEvent,
    pub(crate) respondable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeRuntimeEventCheckpoint {
    pub(crate) runtime_generation: u64,
    pub(crate) sequence_watermark: u64,
    pub(crate) active_turns: Vec<CodeRuntimeActiveTurnCheckpoint>,
    pub(crate) pending_approvals: Vec<CodeRuntimeApprovalCheckpoint>,
}

/// Runtime replay snapshot before binding-scope filtering and enrichment.
#[derive(Clone, Debug)]
pub(crate) struct CodeRuntimeEventBacklog {
    pub(crate) runtime_generation: u64,
    pub(crate) latest_sequence: u64,
    pub(crate) truncated: bool,
    pub(crate) checkpoint: Option<CodeRuntimeEventCheckpoint>,
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
    let parsed = uuid::Uuid::parse_str(preparation_id)
        .map_err(|_| "SchoolX Code preparation id must be a UUID".to_string())?;
    if parsed.hyphenated().to_string() != preparation_id {
        return Err(
            "SchoolX Code preparation id must use canonical lowercase UUID form".to_string(),
        );
    }
    let source = format!("{CODE_THREAD_SOURCE_PREFIX}{preparation_id}");
    validate_id("thread source", &source)?;
    Ok(source)
}

/// Verify the opaque source marker emitted only from a native preparation.
pub(crate) fn validate_code_thread_source_marker(source: &str) -> Result<(), String> {
    let preparation_id = source
        .strip_prefix(CODE_THREAD_SOURCE_PREFIX)
        .ok_or_else(|| "Codex thread source is not a SchoolX Code marker".to_string())?;
    if code_thread_source(preparation_id)? == source {
        Ok(())
    } else {
        Err("Codex thread source is not canonical".to_string())
    }
}

/// Exact bound source accepted for a whole-history `thread/fork`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadForkInput {
    /// Exact community/project/repository scope that owns the source binding.
    pub scope: CodeThreadBindingScope,
    /// Stable-active managed source whose complete history will be copied.
    pub thread_id: String,
}

impl CodeThreadForkInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.scope.validate()?;
        validate_id("fork source thread", &self.thread_id)?;
        Ok(())
    }

    pub(crate) fn rpc_params(
        &self,
        workspace_root: &str,
        preparation_id: &str,
    ) -> Result<Value, String> {
        self.validate()?;
        if workspace_root.is_empty() {
            return Err("SchoolX Code fork destination root cannot be empty".to_string());
        }
        let thread_source = code_thread_source(preparation_id)?;
        Ok(Value::Object(Map::from_iter([
            ("threadId".to_string(), json!(self.thread_id)),
            ("cwd".to_string(), json!(workspace_root)),
            ("approvalPolicy".to_string(), json!("on-request")),
            ("sandbox".to_string(), json!("workspace-write")),
            ("threadSource".to_string(), json!(thread_source)),
        ])))
    }
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

/// Explicit continuation or reconciliation input for an unfinished binding.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadBindingRecoverInput {
    /// Exact scope that owns the durable preparation.
    pub scope: CodeThreadBindingScope,
    /// Opaque preparation UUID returned before `thread/start` or `thread/fork`.
    pub preparation_id: String,
    /// Optional model override forwarded only for root-thread recovery.
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

    pub(crate) fn preserved_root(code: &str, message: String, execution_root: String) -> Self {
        Self {
            code: code.to_string(),
            message,
            preparation_id: None,
            thread_id: None,
            execution_root: Some(execution_root),
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

/// Exact bound thread and user-facing title accepted by the native rename gate.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadRenameInput {
    /// Community, project, and repository coordinate that must own the thread.
    pub scope: CodeThreadBindingScope,
    /// Opaque Codex thread identifier within the exact binding scope.
    pub thread_id: String,
    /// Trimmed user-facing title forwarded to Codex without path authority.
    pub name: String,
}

impl CodeThreadRenameInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.scope.validate()?;
        validate_id("thread", &self.thread_id)?;
        validate_thread_name(&self.name)
    }

    pub(crate) fn rpc_params(&self) -> Result<Value, String> {
        self.validate()?;
        Ok(json!({
            "threadId": self.thread_id,
            "name": self.name,
        }))
    }
}

/// Exact bound thread whose persisted execution root should be inspected for
/// current Git changes.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadChangesInput {
    /// Community, project, and native repository identity of the binding.
    pub scope: CodeThreadBindingScope,
    /// Opaque Codex thread identifier within the exact binding scope.
    pub thread_id: String,
}

/// Base-relative Git status for one SchoolX Code change.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeThreadChangeStatus {
    /// The path did not exist in the persisted base tree.
    Added,
    /// The path content or executable mode changed.
    Modified,
    /// The path was removed from the current execution root.
    Deleted,
    /// The Git object type changed, for example from a regular file to a symlink.
    TypeChanged,
    /// Git reports an unresolved index entry for the path.
    Unmerged,
    /// The path is not tracked and is not excluded by Git ignore rules.
    Untracked,
}

/// One bounded, read-only file diff in a SchoolX Code execution root.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeThreadChangedFile {
    /// Repository-relative path reported by Git.
    pub path: String,
    /// Base-relative Git status from the native bounded inventory.
    pub status: CodeThreadChangeStatus,
    /// Whether Git classified this path as binary rather than textual.
    pub binary: bool,
    /// Saturating count of added lines before patch truncation.
    pub additions: usize,
    /// Saturating count of deleted lines before patch truncation.
    pub deletions: usize,
    /// Bounded unified diff suitable for the existing read-only renderer.
    pub patch: String,
    /// Whether the native reader truncated this file's patch.
    pub truncated: bool,
}

/// Current execution-root changes relative to the binding's persisted base.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeThreadChanges {
    /// Bounded changed files in stable path order.
    pub files: Vec<CodeThreadChangedFile>,
    /// Total distinct changed paths before the response file limit is applied.
    pub total_files: usize,
    /// Whether `files` omits paths because the native file limit was reached.
    pub files_truncated: bool,
    /// Saturating additions total for the returned file subset.
    pub additions: usize,
    /// Saturating deletions total for the returned file subset.
    pub deletions: usize,
    /// Commit body slot retained for compatibility with the project diff UI.
    pub commit_body: Option<String>,
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
    /// Public five-state lifecycle projection stored beside the frozen binding.
    pub lifecycle: CodeThreadLifecycleStatus,
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
    /// App-server-authoritative model after start, resume, or fork.
    pub model: String,
    /// App-server-authoritative effort when Codex reports one.
    pub reasoning_effort: Option<String>,
}

/// Stable result of an archive or unarchive lifecycle mutation.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeThreadLifecycleMutationResult {
    /// Exact persisted native binding after lifecycle reconciliation.
    pub binding: CodeThreadBinding,
    /// Public five-state lifecycle projection; operation ids stay native-only.
    pub lifecycle: CodeThreadLifecycleStatus,
    /// Normalized app-server thread metadata when the resulting state is active.
    pub thread: Option<CodeThreadSummary>,
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
        super::model_catalog::turn_selection(self.model.as_deref(), self.effort.as_deref())?;
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

mod wire;

#[cfg(test)]
pub(crate) use wire::redact_protocol_text_with_sensitive_values;
pub(crate) use wire::{
    loaded_thread_list_params, normalize_notification, parse_loaded_thread_list,
    parse_recovery_thread_list, parse_recovery_thread_read, parse_thread_name_set,
    parse_thread_open, parse_thread_read, parse_turn_start, parse_turn_steer,
    recovery_thread_list_params, recovery_thread_read_params, redact_protocol_text,
    redact_protocol_value, thread_read_params, CodeRecoveryThread, CodeThreadRpcOpenResult,
};
pub(crate) fn validate_id(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(format!(
            "SchoolX Code {label} id must be between 1 and {MAX_ID_BYTES} bytes"
        ));
    }
    Ok(())
}

fn validate_thread_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.trim() != name {
        return Err("Codex thread name must be non-empty and trimmed".to_string());
    }
    if name.chars().count() > MAX_THREAD_NAME_SCALARS {
        return Err(format!(
            "Codex thread name exceeds the {MAX_THREAD_NAME_SCALARS}-character limit"
        ));
    }
    if name.len() > MAX_THREAD_NAME_BYTES {
        return Err(format!(
            "Codex thread name exceeds the {MAX_THREAD_NAME_BYTES}-byte limit"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("Codex thread name cannot contain control characters".to_string());
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

fn validate_model_value(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "Codex {label} must be a trimmed, non-control string between 1 and {MAX_ID_BYTES} bytes"
        ));
    }
    Ok(())
}

pub(crate) fn validate_cursor(cursor: &str) -> Result<(), String> {
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
mod tests;
