use super::*;

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
    ephemeral: Option<bool>,
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
    source: Option<Value>,
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
    model: String,
    #[serde(default)]
    reasoning_effort: Option<String>,
    #[serde(default)]
    instruction_sources: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
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
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) thread_source: Option<String>,
    pub(crate) session_source: Option<Value>,
    pub(crate) response_cwd: Option<String>,
    pub(crate) ephemeral_present: bool,
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
    pub(crate) session_source: Option<Value>,
    pub(crate) ephemeral_present: bool,
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
    validate_model_value("model", &result.model)?;
    if let Some(reasoning_effort) = result.reasoning_effort.as_deref() {
        validate_model_value("reasoning effort", reasoning_effort)?;
    }
    let thread_source = result.thread.thread_source.clone();
    let session_source = result.thread.source.clone();
    let ephemeral_present = result.thread.ephemeral.is_some();
    if let Some(thread_source) = thread_source.as_deref() {
        validate_id("thread source", thread_source)?;
    }
    Ok(CodeThreadRpcOpenResult {
        thread: normalize_thread(result.thread)?,
        instruction_sources: result.instruction_sources,
        model: result.model,
        reasoning_effort: result.reasoning_effort,
        thread_source,
        session_source,
        response_cwd: result.cwd,
        ephemeral_present,
    })
}

/// Build the audited Codex 0.145/0.149 `thread/read` request used to hydrate
/// one thread selected from the native binding index.
pub(crate) fn thread_read_params(thread_id: &str) -> Result<Value, String> {
    validate_id("thread", thread_id)?;
    Ok(json!({ "threadId": thread_id, "includeTurns": false }))
}

/// Build the audited Codex 0.145/0.149 exact-root list request used only for
/// native start recovery. Codex 0.149 reports SchoolX app-server sessions as
/// `vscode`, while 0.145 and the shared schema use `appServer`.
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
        ("sourceKinds".to_string(), json!(["appServer", "vscode"])),
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
        let session_source = thread.source.clone();
        let ephemeral_present = thread.ephemeral.is_some();
        if let Some(thread_source) = thread_source.as_deref() {
            validate_id("thread source", thread_source)?;
        }
        data.push(CodeRecoveryThread {
            thread: normalize_thread(thread)?,
            thread_source,
            session_source,
            ephemeral_present,
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
    let session_source = result.thread.source.clone();
    let ephemeral_present = result.thread.ephemeral.is_some();
    if let Some(thread_source) = thread_source.as_deref() {
        validate_id("thread source", thread_source)?;
    }
    Ok(CodeRecoveryThread {
        thread: normalize_thread(result.thread)?,
        thread_source,
        session_source,
        ephemeral_present,
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

/// Validate the exact empty result frozen for Codex 0.145/0.149 `thread/name/set`.
pub(crate) fn parse_thread_name_set(value: Value) -> Result<(), String> {
    match value.as_object() {
        Some(result) if result.is_empty() => Ok(()),
        _ => Err("invalid Codex thread name response: expected an empty object".to_string()),
    }
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
        ephemeral: thread.ephemeral.unwrap_or(false),
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
    if matches!(method, "thread/archived" | "thread/unarchived") {
        let object = payload
            .as_object()
            .ok_or_else(|| format!("Codex `{method}` notification params must be an object"))?;
        if object.len() != 1 || !object.contains_key("threadId") {
            return Err(format!(
                "Codex `{method}` notification must contain only `threadId`"
            ));
        }
        let thread_id = object
            .get("threadId")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Codex `{method}` notification has invalid `threadId`"))?;
        validate_id("threadId", thread_id)?;
        return Ok(Some(CodeWorkspaceEventDraft {
            thread_id: Some(thread_id.to_string()),
            turn_id: None,
            item_id: None,
            kind: method.to_string(),
            payload: redact_protocol_value(payload),
        }));
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
    redact_protocol_text_with_sensitive_values(text, &secret_refs)
}

/// Apply the protocol diagnostic scrubber with an explicit sensitive-value set.
pub(crate) fn redact_protocol_text_with_sensitive_values(text: &str, secrets: &[&str]) -> String {
    let mut redacted = crate::managed_agents::redact_secrets_with(text, secrets);
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
            | "thread/archived"
            | "thread/unarchived"
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
