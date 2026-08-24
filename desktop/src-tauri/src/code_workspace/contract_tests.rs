use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::Read as _;
use std::path::Path;
use std::process::Command;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use flate2::read::GzDecoder;
use regex::Regex;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tar::Archive;

use super::approvals::{
    CodeApprovalDecision, CodeApprovalResponse, CodePermissionIntent, CodePermissionScope,
    PendingApprovalStore,
};
use super::bindings::{
    CodeExecutionMode, CodeThreadBinding, CodeThreadBindingScope, CodeThreadBindingStore,
    CodeThreadLifecycleStatus, CodeThreadPreparationOperation, CodeThreadPreparationState,
    CODE_THREAD_BINDING_SCHEMA_VERSION,
};
use super::jsonrpc;
use super::protocol::{
    loaded_thread_list_params, normalize_notification, parse_loaded_thread_list,
    parse_recovery_thread_list, parse_recovery_thread_read, parse_thread_name_set,
    parse_thread_open, parse_thread_read, parse_turn_start, parse_turn_steer,
    recovery_thread_list_params, thread_read_params, CodeBoundThreadOpenResult,
    CodeBoundThreadSummary, CodeEventBacklog, CodeEventCheckpoint, CodePreparedWorktree,
    CodeThreadBindingRecoverInput, CodeThreadChangeStatus, CodeThreadChangedFile,
    CodeThreadChanges, CodeThreadChangesInput, CodeThreadForkInput,
    CodeThreadLifecycleMutationResult, CodeThreadListInput, CodeThreadRenameInput,
    CodeThreadResumeInput, CodeThreadStartError, CodeThreadStartInput, CodeThreadsPage,
    CodeTurnInterruptInput, CodeTurnStartInput, CodeTurnSteerInput, CodeTurnSummary,
    CodeWorkspaceEvent, CodeWorktreePrepareCommandInput,
};
use super::runtime::{initialize_params, CodeRuntimePhase, CodeRuntimeStatus};
use super::terminal::{
    CodeTerminalEvent, CodeTerminalOpenInput, CodeTerminalResizeInput, CodeTerminalSession,
    CodeTerminalStdinInput, CodeTerminalTerminateInput,
};
use super::thread_lifecycle::{parse_thread_archive, parse_thread_unarchive};
use super::worktree_inventory::{
    CodeWorktreeInspection, CodeWorktreeInventoryAuthority, CodeWorktreeInventoryBlocker,
};
use super::worktrees::{
    CodeRepositoryDescriptor, CodeRepositoryInspectInput, CodeWorktreeDescriptor,
    CodeWorktreePrepareResult, CodeWorktreeStatus,
};
use super::{
    CodeApprovalResponseInput, CodeModelOption, CodeModelSelection, CodeModelsListResult,
    CodeReasoningEffortOption, CodeRuntimeProbe, CodeThreadLifecycleInput,
    CodeThreadPreparationListInput, CodeWorktreeInventoryRow, CodeWorktreeRemovalReceipt,
    CodeWorktreeRemoveInput, CodeWorktreesListInput, CODE_WORKSPACE_EVENT,
};

const TAURI_CONTRACT: &str = include_str!("fixtures/tauri-contract-v1.json");
const WORKTREE_REMOVAL_GATE_CONTRACT: &str =
    include_str!("fixtures/worktree-removal-gates-v1.json");
const WORKTREE_REMOVAL_GATE_DESIGN: &str =
    include_str!("../../../../docs/schoolx-2/SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md");
const STORE_FIXTURE: &str = include_str!("fixtures/thread-bindings-v1.json");
const SCHEMA_MANIFEST: &str = include_str!("fixtures/codex-0.145.0-schema-manifest.json");
const WIRE_FIXTURE: &str = include_str!("fixtures/codex-0.145.0-wire.json");
const SELECTED_SCHEMA_ARCHIVE: &str =
    include_str!("fixtures/codex-0.145.0-selected-schemas.tar.gz.base64");
const SCHEMA_MANIFEST_0_149: &str = include_str!("fixtures/codex-0.149.0-schema-manifest.json");
const WIRE_FIXTURE_0_149: &str = include_str!("fixtures/codex-0.149.0-wire.json");
const SELECTED_SCHEMA_ARCHIVE_0_149: &str =
    include_str!("fixtures/codex-0.149.0-selected-schemas.tar.gz.base64");
const LIB_SOURCE: &str = include_str!("../lib.rs");
const COMMAND_SOURCE: &str = include_str!("../commands/code_workspace.rs");
const TERMINAL_COMMAND_SOURCE: &str = include_str!("../commands/code_terminal.rs");
const THREAD_MANAGEMENT_COMMAND_SOURCE: &str =
    include_str!("../commands/code_thread_management.rs");
const THREAD_LIFECYCLE_COMMAND_SOURCE: &str = include_str!("../commands/code_thread_lifecycle.rs");
const THREAD_FORK_COMMAND_SOURCE: &str = include_str!("../commands/code_thread_fork.rs");
const WORKTREE_INVENTORY_COMMAND_SOURCE: &str =
    include_str!("../commands/code_worktree_inventory.rs");
const GIT_HANDOFF_COMMAND_SOURCE: &str = include_str!("../commands/code_git_handoff.rs");

fn fixture(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|error| error.to_string())
}

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

fn encode_values<T: Serialize>(values: impl IntoIterator<Item = T>) -> Result<Value, String> {
    values
        .into_iter()
        .map(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn keys(value: &Value) -> Result<BTreeSet<String>, String> {
    value
        .as_object()
        .ok_or_else(|| "fixture value must be an object".to_string())
        .map(|object| object.keys().cloned().collect())
}

fn string_set(value: &Value, label: &str) -> Result<BTreeSet<String>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{label} must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{label} entries must be strings"))
        })
        .collect()
}

fn assert_required_fields(value: &Value, required: &Value, label: &str) -> Result<(), String> {
    let actual = keys(value)?;
    let required = string_set(required, label)?;
    if !required.is_subset(&actual) {
        let missing = required.difference(&actual).cloned().collect::<Vec<_>>();
        return Err(format!("{label} is missing required fields: {missing:?}"));
    }
    Ok(())
}

fn assert_schema_shape(manifest: &Value, schema_path: &str, value: &Value) -> Result<(), String> {
    let shape = manifest["schemaShapes"]
        .get(schema_path)
        .ok_or_else(|| format!("missing curated shape for {schema_path}"))?;
    assert_required_fields(value, &shape["required"], schema_path)?;
    let actual = keys(value)?;
    let properties = string_set(&shape["properties"], schema_path)?;
    if !actual.is_subset(&properties) {
        let unknown = actual.difference(&properties).cloned().collect::<Vec<_>>();
        return Err(format!(
            "{schema_path} fixture contains fields outside its curated schema: {unknown:?}"
        ));
    }
    Ok(())
}

fn assert_wire_schema(
    manifest: &Value,
    schemas: &BTreeMap<String, (Value, Vec<u8>)>,
    schema_path: &str,
    value: &Value,
    instance_path: &str,
) -> Result<(), String> {
    assert_schema_shape(manifest, schema_path, value)?;
    let schema = schemas
        .get(schema_path)
        .map(|(schema, _)| schema)
        .ok_or_else(|| format!("selected schema artifact is missing {schema_path}"))?;
    validate_schema_instance(schema, schema, value, instance_path)
        .map_err(|error| format!("{schema_path}: {error}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical_schema_bytes(value: Value) -> Result<Vec<u8>, String> {
    fn sort(value: Value) -> Value {
        match value {
            Value::Array(values) => Value::Array(values.into_iter().map(sort).collect()),
            Value::Object(values) => {
                let mut entries = values.into_iter().collect::<Vec<_>>();
                entries.sort_by(|left, right| left.0.cmp(&right.0));
                Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key, sort(value)))
                        .collect(),
                )
            }
            other => other,
        }
    }

    let mut bytes = serde_json::to_vec(&sort(value)).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn collect_json_paths(
    root: &Path,
    directory: &Path,
    paths: &mut Vec<String>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() {
            collect_json_paths(root, &path, paths)?;
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            paths.push(relative);
        }
    }
    Ok(())
}

fn manifest_aggregate(schemas: &Value) -> Result<String, String> {
    let schemas = schemas
        .as_array()
        .ok_or_else(|| "manifest schemas must be an array".to_string())?;
    let mut input = String::new();
    for schema in schemas {
        let tuple = schema
            .as_array()
            .filter(|tuple| tuple.len() == 2)
            .ok_or_else(|| "manifest schema entry must be a hash/path pair".to_string())?;
        let hash = tuple[0]
            .as_str()
            .ok_or_else(|| "manifest schema hash must be a string".to_string())?;
        let path = tuple[1]
            .as_str()
            .ok_or_else(|| "manifest schema path must be a string".to_string())?;
        input.push_str(hash);
        input.push_str("  ");
        input.push_str(path);
        input.push('\n');
    }
    Ok(sha256_hex(input.as_bytes()))
}

fn selected_schema_artifact(
    selected_schema_archive: &str,
) -> Result<BTreeMap<String, (Value, Vec<u8>)>, String> {
    let encoded = selected_schema_archive
        .lines()
        .map(str::trim)
        .collect::<String>();
    let compressed = BASE64_STANDARD
        .decode(encoded)
        .map_err(|error| format!("invalid selected Codex schema artifact base64: {error}"))?;
    let mut archive = Archive::new(GzDecoder::new(compressed.as_slice()));
    let mut schemas = BTreeMap::new();
    for entry in archive
        .entries()
        .map_err(|error| format!("invalid selected Codex schema archive: {error}"))?
    {
        let mut entry = entry.map_err(|error| format!("invalid schema archive entry: {error}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|error| format!("invalid schema artifact path: {error}"))?
            .to_string_lossy()
            .trim_start_matches("./")
            .to_string();
        let mut raw = Vec::new();
        entry
            .read_to_end(&mut raw)
            .map_err(|error| format!("failed to read archived schema {path}: {error}"))?;
        let schema = serde_json::from_slice(&raw)
            .map_err(|error| format!("invalid archived schema {path}: {error}"))?;
        if schemas.insert(path.clone(), (schema, raw)).is_some() {
            return Err(format!("duplicate archived schema path: {path}"));
        }
    }
    Ok(schemas)
}

fn schema_type_matches(expected: &str, instance: &Value) -> bool {
    match expected {
        "null" => instance.is_null(),
        "boolean" => instance.is_boolean(),
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "number" => instance.is_number(),
        "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
        "string" => instance.is_string(),
        _ => false,
    }
}

fn validate_schema_instance(
    root: &Value,
    schema: &Value,
    instance: &Value,
    instance_path: &str,
) -> Result<(), String> {
    if schema == &Value::Bool(true) {
        return Ok(());
    }
    if schema == &Value::Bool(false) {
        return Err(format!("{instance_path} matched a false schema"));
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let pointer = reference
            .strip_prefix('#')
            .ok_or_else(|| format!("external schema reference is not frozen: {reference}"))?;
        let target = root
            .pointer(pointer)
            .ok_or_else(|| format!("unresolved schema reference: {reference}"))?;
        validate_schema_instance(root, target, instance, instance_path)?;
    }
    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
        for branch in all_of {
            validate_schema_instance(root, branch, instance, instance_path)?;
        }
    }
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
        if !any_of
            .iter()
            .any(|branch| validate_schema_instance(root, branch, instance, instance_path).is_ok())
        {
            return Err(format!("{instance_path} did not match anyOf"));
        }
    }
    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        let matches = one_of
            .iter()
            .filter(|branch| {
                validate_schema_instance(root, branch, instance, instance_path).is_ok()
            })
            .count();
        if matches != 1 {
            return Err(format!(
                "{instance_path} matched {matches} oneOf branches instead of exactly one"
            ));
        }
    }
    if let Some(not_schema) = schema.get("not") {
        if validate_schema_instance(root, not_schema, instance, instance_path).is_ok() {
            return Err(format!("{instance_path} matched a forbidden schema"));
        }
    }
    if let Some(condition) = schema.get("if") {
        let branch = if validate_schema_instance(root, condition, instance, instance_path).is_ok() {
            schema.get("then")
        } else {
            schema.get("else")
        };
        if let Some(branch) = branch {
            validate_schema_instance(root, branch, instance, instance_path)?;
        }
    }
    if let Some(constant) = schema.get("const") {
        if constant != instance {
            return Err(format!("{instance_path} did not match its schema constant"));
        }
    }
    if let Some(variants) = schema.get("enum").and_then(Value::as_array) {
        if !variants.iter().any(|variant| variant == instance) {
            return Err(format!("{instance_path} was outside its schema enum"));
        }
    }
    if let Some(expected_type) = schema.get("type") {
        let matches = match expected_type {
            Value::String(expected) => schema_type_matches(expected, instance),
            Value::Array(expected) => expected.iter().any(|expected| {
                expected
                    .as_str()
                    .is_some_and(|expected| schema_type_matches(expected, instance))
            }),
            _ => false,
        };
        if !matches {
            return Err(format!("{instance_path} had the wrong JSON type"));
        }
    }

    if let Some(object) = instance.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(format!("{instance_path} is missing required field `{key}`"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, value) in object {
            if let Some(property_schema) = properties.and_then(|properties| properties.get(key)) {
                validate_schema_instance(
                    root,
                    property_schema,
                    value,
                    &format!("{instance_path}.{key}"),
                )?;
            } else if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                return Err(format!("{instance_path} has forbidden field `{key}`"));
            } else if let Some(additional) = schema.get("additionalProperties") {
                validate_schema_instance(
                    root,
                    additional,
                    value,
                    &format!("{instance_path}.{key}"),
                )?;
            }
        }
    }

    if let Some(array) = instance.as_array() {
        if let Some(min_items) = schema.get("minItems").and_then(Value::as_u64) {
            if array.len() < min_items as usize {
                return Err(format!("{instance_path} has fewer than {min_items} items"));
            }
        }
        if let Some(max_items) = schema.get("maxItems").and_then(Value::as_u64) {
            if array.len() > max_items as usize {
                return Err(format!("{instance_path} has more than {max_items} items"));
            }
        }
        if schema.get("uniqueItems") == Some(&Value::Bool(true)) {
            let has_duplicate = array
                .iter()
                .enumerate()
                .any(|(index, value)| array[index + 1..].contains(value));
            if has_duplicate {
                return Err(format!("{instance_path} contains duplicate items"));
            }
        }
        if let Some(items) = schema.get("items") {
            for (index, value) in array.iter().enumerate() {
                validate_schema_instance(root, items, value, &format!("{instance_path}[{index}]"))?;
            }
        }
    }

    if let Some(text) = instance.as_str() {
        let length = text.chars().count();
        if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64) {
            if length < minimum as usize {
                return Err(format!("{instance_path} is shorter than {minimum}"));
            }
        }
        if let Some(maximum) = schema.get("maxLength").and_then(Value::as_u64) {
            if length > maximum as usize {
                return Err(format!("{instance_path} is longer than {maximum}"));
            }
        }
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            let pattern = Regex::new(pattern)
                .map_err(|error| format!("invalid frozen schema regex `{pattern}`: {error}"))?;
            if !pattern.is_match(text) {
                return Err(format!("{instance_path} did not match its schema pattern"));
            }
        }
    }
    if let Some(number) = instance.as_f64() {
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
            if number < minimum {
                return Err(format!("{instance_path} is below schema minimum {minimum}"));
            }
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
            if number > maximum {
                return Err(format!("{instance_path} is above schema maximum {maximum}"));
            }
        }
        if let Some(minimum) = schema.get("exclusiveMinimum").and_then(Value::as_f64) {
            if number <= minimum {
                return Err(format!(
                    "{instance_path} is not above exclusive minimum {minimum}"
                ));
            }
        }
        if let Some(maximum) = schema.get("exclusiveMaximum").and_then(Value::as_f64) {
            if number >= maximum {
                return Err(format!(
                    "{instance_path} is not below exclusive maximum {maximum}"
                ));
            }
        }
    }
    Ok(())
}

fn reject_unknown<T: DeserializeOwned>(value: &Value) -> Result<(), String> {
    serde_json::from_value::<T>(value.clone())
        .map_err(|error| format!("strict input fixture is not a valid native DTO: {error}"))?;
    let mut invalid = value.clone();
    invalid
        .as_object_mut()
        .ok_or_else(|| "strict input fixture must be an object".to_string())?
        .insert("unexpected".to_string(), json!(true));
    if serde_json::from_value::<T>(invalid).is_ok() {
        return Err("strict native input accepted an unknown field".to_string());
    }
    Ok(())
}

fn camel_case_identifier(identifier: &str) -> String {
    let mut parts = identifier.split('_');
    let mut camel = parts.next().unwrap_or_default().to_string();
    for part in parts {
        let mut characters = part.chars();
        if let Some(first) = characters.next() {
            camel.extend(first.to_uppercase());
            camel.extend(characters);
        }
    }
    camel
}

fn signature_arguments(signature: &str) -> Vec<&str> {
    let mut arguments = Vec::new();
    let mut start = 0;
    let mut angle_depth = 0_u32;
    for (index, character) in signature.char_indices() {
        match character {
            '<' => angle_depth = angle_depth.saturating_add(1),
            '>' => angle_depth = angle_depth.saturating_sub(1),
            ',' if angle_depth == 0 => {
                arguments.push(signature[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    arguments.push(signature[start..].trim());
    arguments
}

fn command_arguments(command: &str) -> Result<Vec<String>, String> {
    let async_marker = format!("pub async fn {command}(");
    let sync_marker = format!("pub fn {command}(");
    let signature = COMMAND_SOURCE
        .split_once(&async_marker)
        .or_else(|| COMMAND_SOURCE.split_once(&sync_marker))
        .or_else(|| TERMINAL_COMMAND_SOURCE.split_once(&async_marker))
        .or_else(|| TERMINAL_COMMAND_SOURCE.split_once(&sync_marker))
        .or_else(|| THREAD_MANAGEMENT_COMMAND_SOURCE.split_once(&async_marker))
        .or_else(|| THREAD_MANAGEMENT_COMMAND_SOURCE.split_once(&sync_marker))
        .or_else(|| THREAD_LIFECYCLE_COMMAND_SOURCE.split_once(&async_marker))
        .or_else(|| THREAD_LIFECYCLE_COMMAND_SOURCE.split_once(&sync_marker))
        .or_else(|| THREAD_FORK_COMMAND_SOURCE.split_once(&async_marker))
        .or_else(|| THREAD_FORK_COMMAND_SOURCE.split_once(&sync_marker))
        .or_else(|| WORKTREE_INVENTORY_COMMAND_SOURCE.split_once(&async_marker))
        .or_else(|| WORKTREE_INVENTORY_COMMAND_SOURCE.split_once(&sync_marker))
        .or_else(|| GIT_HANDOFF_COMMAND_SOURCE.split_once(&async_marker))
        .or_else(|| GIT_HANDOFF_COMMAND_SOURCE.split_once(&sync_marker))
        .map(|(_, remainder)| remainder)
        .ok_or_else(|| format!("missing native command signature for {command}"))?;
    let signature = signature
        .split_once(") ->")
        .map(|(signature, _)| signature)
        .ok_or_else(|| format!("unterminated native command signature for {command}"))?;
    let mut arguments = signature_arguments(signature)
        .into_iter()
        .filter(|argument| !argument.is_empty())
        .map(|argument| {
            argument
                .split_once(':')
                .map(|(name, _)| name.trim())
                .ok_or_else(|| format!("invalid native command argument in {command}"))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|argument| !matches!(*argument, "app" | "state"))
        .map(camel_case_identifier)
        .collect::<Vec<_>>();
    arguments.sort();
    Ok(arguments)
}

fn registered_code_commands() -> Result<Vec<String>, String> {
    let handler = LIB_SOURCE
        .split_once(".invoke_handler(tauri::generate_handler![")
        .map(|(_, remainder)| remainder)
        .ok_or_else(|| "missing Tauri command registration".to_string())?;
    let handler = handler
        .split_once("])")
        .map(|(handler, _)| handler)
        .ok_or_else(|| "unterminated Tauri command registration".to_string())?;
    Ok(handler
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_suffix(','))
        .filter(|command| command.starts_with("code_"))
        .map(str::to_string)
        .collect())
}

mod binding_store_contract;
mod schema_contracts;
mod tauri_contract;
mod wire_contracts;
mod worktree_removal_contract;
