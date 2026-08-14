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
    CodeApprovalDecision, CodeApprovalResponse, CodePermissionScope, PendingApprovalStore,
};
use super::bindings::{
    CodeExecutionMode, CodeThreadBinding, CodeThreadBindingScope, CodeThreadBindingStore,
    CodeThreadPreparationState,
};
use super::jsonrpc;
use super::protocol::{
    loaded_thread_list_params, normalize_notification, parse_loaded_thread_list,
    parse_recovery_thread_list, parse_recovery_thread_read, parse_thread_open, parse_thread_read,
    parse_turn_start, parse_turn_steer, recovery_thread_list_params, thread_read_params,
    CodeBoundThreadOpenResult, CodeBoundThreadSummary, CodeEventBacklog, CodePreparedWorktree,
    CodeThreadBindingRecoverInput, CodeThreadListInput, CodeThreadResumeInput,
    CodeThreadStartError, CodeThreadStartInput, CodeThreadsPage, CodeTurnInterruptInput,
    CodeTurnStartInput, CodeTurnSteerInput, CodeTurnSummary, CodeWorkspaceEvent,
    CodeWorktreePrepareCommandInput,
};
use super::runtime::{initialize_params, CodeRuntimePhase, CodeRuntimeStatus};
use super::worktrees::{
    CodeRepositoryDescriptor, CodeRepositoryInspectInput, CodeWorktreeDescriptor,
    CodeWorktreePrepareResult, CodeWorktreeStatus,
};
use super::{
    CodeApprovalResponseInput, CodeRuntimeProbe, CodeThreadPreparationListInput,
    CODE_WORKSPACE_EVENT,
};

const TAURI_CONTRACT: &str = include_str!("fixtures/tauri-contract-v1.json");
const STORE_FIXTURE: &str = include_str!("fixtures/thread-bindings-v1.json");
const SCHEMA_MANIFEST: &str = include_str!("fixtures/codex-0.145.0-schema-manifest.json");
const WIRE_FIXTURE: &str = include_str!("fixtures/codex-0.145.0-wire.json");
const SELECTED_SCHEMA_ARCHIVE: &str =
    include_str!("fixtures/codex-0.145.0-selected-schemas.tar.gz.base64");
const LIB_SOURCE: &str = include_str!("../lib.rs");
const COMMAND_SOURCE: &str = include_str!("../commands/code_workspace.rs");

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

fn selected_schema_artifact() -> Result<BTreeMap<String, (Value, Vec<u8>)>, String> {
    let encoded = SELECTED_SCHEMA_ARCHIVE
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

#[test]
fn schema_manifest_freezes_the_audited_codex_0_145_0_contract() -> Result<(), String> {
    let manifest = fixture(SCHEMA_MANIFEST)?;
    assert_eq!(manifest["snapshotSchemaVersion"], 1);
    assert_eq!(manifest["source"]["cliVersion"], "codex-cli 0.145.0");
    assert_eq!(manifest["source"]["experimental"], false);
    assert_eq!(manifest["source"]["generatedFileCount"], 273);
    assert_eq!(
        manifest["runtimeVersionRequirement"],
        "codex-cli 0.145.<numeric patch>"
    );
    assert_eq!(manifest["provenSnapshotVersion"], "codex-cli 0.145.0");
    assert_eq!(manifest["clientNotifications"]["initialized"], Value::Null);

    let schemas = manifest["schemas"]
        .as_array()
        .ok_or_else(|| "missing schemas".to_string())?;
    assert_eq!(schemas.len(), 54);
    assert_eq!(manifest["source"]["selectedSchemaCount"], 54);
    let paths = schemas
        .iter()
        .map(|schema| schema[1].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(paths.iter().copied().collect::<HashSet<_>>().len(), 54);
    assert!(schemas.iter().all(|schema| {
        schema[0]
            .as_str()
            .is_some_and(|hash| hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit()))
    }));
    assert_eq!(
        manifest_aggregate(&manifest["schemas"])?,
        manifest["source"]["selectedSchemasSha256"]
    );
    assert_eq!(
        manifest["source"]["selectedSchemasSha256"],
        "df00b0eff4563354d1a6ab799f1d1f446dcf439745bfb1e04742ae566ffedcd5"
    );
    assert_eq!(
        manifest["source"]["fullGeneratedSetSha256"],
        "757aa191b6d452c6e6d05f6c1f1cb093b9f673da2d185a29ee8d5d96feae67a8"
    );

    let methods = manifest["methods"]
        .as_object()
        .ok_or_else(|| "missing method map".to_string())?;
    assert_eq!(methods.len(), 9);
    assert_eq!(
        manifest["notifications"].as_object().map(|map| map.len()),
        Some(21)
    );
    assert_eq!(
        manifest["serverRequests"].as_object().map(|map| map.len()),
        Some(3)
    );
    assert_eq!(
        manifest["dispatchSchemas"].as_array().map(Vec::len),
        Some(4)
    );
    Ok(())
}

#[test]
fn selected_schema_artifact_recomputes_every_manifest_hash() -> Result<(), String> {
    let manifest = fixture(SCHEMA_MANIFEST)?;
    let archived = selected_schema_artifact()?;
    let entries = manifest["schemas"]
        .as_array()
        .ok_or_else(|| "manifest schemas must be an array".to_string())?;
    assert_eq!(archived.len(), entries.len());
    let expected_paths = entries
        .iter()
        .filter_map(|entry| entry.get(1).and_then(Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        archived.keys().cloned().collect::<BTreeSet<_>>(),
        expected_paths
    );

    let mut aggregate = String::new();
    for entry in entries {
        let expected_hash = entry[0]
            .as_str()
            .ok_or_else(|| "manifest schema hash must be a string".to_string())?;
        let path = entry[1]
            .as_str()
            .ok_or_else(|| "manifest schema path must be a string".to_string())?;
        let (schema, raw) = archived
            .get(path)
            .ok_or_else(|| format!("selected schema artifact is missing {path}"))?;
        if raw.last() != Some(&b'\n') || raw[..raw.len().saturating_sub(1)].contains(&b'\n') {
            return Err(format!(
                "archived schema is not canonical one-line JSON: {path}"
            ));
        }
        assert_eq!(
            &canonical_schema_bytes(schema.clone())?,
            raw,
            "archived schema key order drifted for {path}"
        );
        let actual_hash = sha256_hex(raw);
        assert_eq!(actual_hash, expected_hash, "schema hash drifted for {path}");
        aggregate.push_str(&actual_hash);
        aggregate.push_str("  ");
        aggregate.push_str(path);
        aggregate.push('\n');
    }
    assert_eq!(
        sha256_hex(aggregate.as_bytes()),
        manifest["source"]["selectedSchemasSha256"]
    );
    Ok(())
}

#[test]
fn wire_fixture_conforms_to_the_curated_schema_shapes() -> Result<(), String> {
    let manifest = fixture(SCHEMA_MANIFEST)?;
    let wire = fixture(WIRE_FIXTURE)?;
    let selected_schemas = selected_schema_artifact()?;
    let mut referenced_schemas = BTreeSet::new();

    assert_eq!(wire["initialize"]["request"]["method"], "initialize");
    let initialize_schemas = manifest["methods"]["initialize"]
        .as_array()
        .filter(|schemas| schemas.len() == 2)
        .ok_or_else(|| "initialize schema map must contain params and response".to_string())?;
    for (schema, value) in [
        (
            initialize_schemas[0]
                .as_str()
                .ok_or_else(|| "initialize params schema must be a string".to_string())?,
            &wire["initialize"]["request"]["params"],
        ),
        (
            initialize_schemas[1]
                .as_str()
                .ok_or_else(|| "initialize response schema must be a string".to_string())?,
            &wire["initialize"]["response"],
        ),
    ] {
        assert_wire_schema(&manifest, &selected_schemas, schema, value, "initialize")?;
        referenced_schemas.insert(schema.to_string());
    }
    assert_eq!(
        wire["initialize"]["initializedNotification"]["method"],
        "initialized"
    );

    let method_fixtures = [
        ("thread/start", "threadStart"),
        ("thread/list", "threadList"),
        ("thread/loaded/list", "threadLoadedList"),
        ("thread/read", "threadRead"),
        ("thread/resume", "threadResume"),
        ("turn/start", "turnStart"),
        ("turn/steer", "turnSteer"),
        ("turn/interrupt", "turnInterrupt"),
    ];
    for (method, fixture_key) in method_fixtures {
        assert_eq!(wire[fixture_key]["method"], method);
        let schemas = manifest["methods"][method]
            .as_array()
            .filter(|schemas| schemas.len() == 2)
            .ok_or_else(|| format!("{method} schema map must contain params and response"))?;
        for (schema, value) in [
            (
                schemas[0]
                    .as_str()
                    .ok_or_else(|| format!("{method} params schema must be a string"))?,
                &wire[fixture_key]["params"],
            ),
            (
                schemas[1]
                    .as_str()
                    .ok_or_else(|| format!("{method} response schema must be a string"))?,
                &wire[fixture_key]["result"],
            ),
        ] {
            assert_wire_schema(&manifest, &selected_schemas, schema, value, method)?;
            referenced_schemas.insert(schema.to_string());
        }
    }

    let notification_map = manifest["notifications"]
        .as_object()
        .ok_or_else(|| "missing notification schema map".to_string())?;
    let mut notification_methods = BTreeSet::new();
    for notification in wire["notifications"]
        .as_array()
        .ok_or_else(|| "wire notifications must be an array".to_string())?
    {
        let method = notification["method"]
            .as_str()
            .ok_or_else(|| "wire notification method must be a string".to_string())?;
        let schema = notification_map
            .get(method)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing notification schema for {method}"))?;
        assert_wire_schema(
            &manifest,
            &selected_schemas,
            schema,
            &notification["params"],
            method,
        )?;
        notification_methods.insert(method.to_string());
        referenced_schemas.insert(schema.to_string());
    }
    assert_eq!(
        notification_methods,
        notification_map.keys().cloned().collect()
    );

    let server_request_map = manifest["serverRequests"]
        .as_object()
        .ok_or_else(|| "missing server request schema map".to_string())?;
    let mut server_request_methods = BTreeSet::new();
    for approval in wire["approvals"]
        .as_array()
        .ok_or_else(|| "wire approvals must be an array".to_string())?
    {
        let method = approval["request"]["method"]
            .as_str()
            .ok_or_else(|| "approval method must be a string".to_string())?;
        let schemas = server_request_map
            .get(method)
            .and_then(Value::as_array)
            .filter(|schemas| schemas.len() == 2)
            .ok_or_else(|| format!("missing approval schema pair for {method}"))?;
        for (schema, value) in [
            (
                schemas[0]
                    .as_str()
                    .ok_or_else(|| format!("{method} params schema must be a string"))?,
                &approval["request"]["params"],
            ),
            (
                schemas[1]
                    .as_str()
                    .ok_or_else(|| format!("{method} response schema must be a string"))?,
                &approval["wireResult"],
            ),
        ] {
            assert_wire_schema(&manifest, &selected_schemas, schema, value, method)?;
            referenced_schemas.insert(schema.to_string());
        }
        server_request_methods.insert(method.to_string());
    }
    assert_eq!(
        server_request_methods,
        server_request_map.keys().cloned().collect()
    );
    assert_eq!(
        referenced_schemas,
        manifest["schemaShapes"]
            .as_object()
            .ok_or_else(|| "missing curated schema shapes".to_string())?
            .keys()
            .cloned()
            .collect()
    );

    let facts = &manifest["structuralFacts"];
    for thread in [
        &wire["threadStart"]["result"]["thread"],
        &wire["threadRead"]["result"]["thread"],
        &wire["threadResume"]["result"]["thread"],
    ] {
        assert_required_fields(thread, &facts["threadRequired"], "Codex Thread")?;
        if facts["threadSourceRequiredBySchoolX"] == true && thread.get("threadSource").is_none() {
            return Err("SchoolX wire thread is missing threadSource".to_string());
        }
    }
    for response in [
        &wire["threadStart"]["result"],
        &wire["threadResume"]["result"],
    ] {
        assert_required_fields(
            response,
            &facts["threadOpenResponseRequired"],
            "Codex thread open response",
        )?;
    }
    for turn in [
        &wire["turnStart"]["result"]["turn"],
        &wire["threadResume"]["result"]["thread"]["turns"][0],
    ] {
        assert_required_fields(turn, &facts["turnRequired"], "Codex Turn")?;
        let status = turn["status"]
            .as_str()
            .ok_or_else(|| "Codex Turn status must be a string".to_string())?;
        if !string_set(&facts["turnStatuses"], "turn statuses")?.contains(status) {
            return Err(format!(
                "unsupported Codex Turn status in fixture: {status}"
            ));
        }
    }
    assert_required_fields(
        &wire["threadResume"]["result"]["thread"]["turns"][0]["items"][0],
        &facts["agentMessageItemRequired"],
        "Codex agentMessage item",
    )?;
    Ok(())
}

#[test]
fn tauri_command_input_enum_and_event_contract_is_exact() -> Result<(), String> {
    let contract = fixture(TAURI_CONTRACT)?;
    let expected_commands = [
        ("code_runtime_probe", &[][..]),
        ("code_runtime_start", &[][..]),
        ("code_runtime_stop", &[][..]),
        ("code_runtime_status", &[][..]),
        (
            "code_runtime_events",
            &["afterSequence", "runtimeGeneration", "scope"][..],
        ),
        ("code_repository_inspect", &["input"][..]),
        ("code_worktree_prepare", &["input"][..]),
        ("code_worktree_status", &["descriptor"][..]),
        ("code_thread_preparations_list", &["input"][..]),
        ("code_threads_list", &["input"][..]),
        ("code_thread_start", &["input"][..]),
        ("code_thread_binding_recover", &["input"][..]),
        ("code_thread_resume", &["input"][..]),
        ("code_turn_start", &["input"][..]),
        ("code_turn_steer", &["input"][..]),
        ("code_turn_interrupt", &["input"][..]),
        ("code_approval_respond", &["input"][..]),
    ];
    let actual = contract["commands"]
        .as_array()
        .ok_or_else(|| "missing commands".to_string())?;
    assert_eq!(actual.len(), expected_commands.len());
    for (actual, (name, args)) in actual.iter().zip(expected_commands.iter()) {
        assert_eq!(actual["name"], *name);
        assert_eq!(actual["topLevelArgs"], json!(args));
        assert_eq!(
            command_arguments(name)?,
            args.iter()
                .map(|argument| argument.to_string())
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        registered_code_commands()?,
        expected_commands
            .iter()
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(contract["eventName"], CODE_WORKSPACE_EVENT);
    assert_eq!(
        contract["enums"]["executionMode"],
        encode_values([CodeExecutionMode::Worktree, CodeExecutionMode::Local])?
    );
    assert_eq!(
        contract["enums"]["preparationState"],
        encode_values([
            CodeThreadPreparationState::Prepared,
            CodeThreadPreparationState::Starting,
        ])?
    );
    assert_eq!(
        contract["enums"]["runtimePhase"],
        encode_values([
            CodeRuntimePhase::NotInstalled,
            CodeRuntimePhase::Stopped,
            CodeRuntimePhase::Starting,
            CodeRuntimePhase::Initializing,
            CodeRuntimePhase::Ready,
            CodeRuntimePhase::Stopping,
            CodeRuntimePhase::Failed,
        ])?
    );
    assert_eq!(
        contract["enums"]["approvalDecision"],
        encode_values([
            CodeApprovalDecision::Accept,
            CodeApprovalDecision::AcceptForSession,
            CodeApprovalDecision::Decline,
            CodeApprovalDecision::Cancel,
        ])?
    );
    assert_eq!(
        contract["enums"]["permissionScope"],
        encode_values([CodePermissionScope::Turn, CodePermissionScope::Session])?
    );

    let inputs = &contract["strictInputs"];
    reject_unknown::<CodeRepositoryInspectInput>(&inputs["repositoryInspect"])?;
    reject_unknown::<CodeWorktreePrepareCommandInput>(&inputs["worktreePrepare"])?;
    reject_unknown::<CodeThreadPreparationListInput>(&inputs["threadPreparationList"])?;
    reject_unknown::<CodeThreadStartInput>(&inputs["threadStart"])?;
    reject_unknown::<CodeThreadBindingRecoverInput>(&inputs["threadBindingRecover"])?;
    reject_unknown::<CodeThreadListInput>(&inputs["threadList"])?;
    reject_unknown::<CodeThreadResumeInput>(&inputs["threadResume"])?;
    reject_unknown::<CodeTurnStartInput>(&inputs["turnStart"])?;
    reject_unknown::<CodeTurnSteerInput>(&inputs["turnSteer"])?;
    reject_unknown::<CodeTurnInterruptInput>(&inputs["turnInterrupt"])?;
    reject_unknown::<CodeApprovalResponseInput>(&inputs["approvalDecision"])?;
    reject_unknown::<CodeApprovalResponseInput>(&inputs["approvalPermissions"])?;
    let response_types = ["approvalDecision", "approvalPermissions"]
        .into_iter()
        .map(|fixture_key| {
            decode::<CodeApprovalResponseInput>(&inputs[fixture_key]).map(|input| {
                match input.response {
                    CodeApprovalResponse::Decision { .. } => "decision",
                    CodeApprovalResponse::Permissions { .. } => "permissions",
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        contract["enums"]["approvalResponseType"],
        json!(response_types)
    );
    assert_eq!(
        keys(&contract["invocations"]["runtimeEvents"])?,
        ["afterSequence", "runtimeGeneration", "scope"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    let _: CodeWorktreeDescriptor =
        decode(&contract["invocations"]["worktreeStatus"]["descriptor"])?;

    let binding: CodeThreadBinding = decode(&contract["outputs"]["binding"])?;
    assert_eq!(
        serde_json::to_value(&binding).map_err(|error| error.to_string())?,
        contract["outputs"]["binding"]
    );
    assert_eq!(keys(&contract["outputs"]["binding"])?.len(), 8);
    let preparation_list: Vec<super::bindings::CodeThreadPreparation> =
        decode(&contract["outputs"]["preparationList"])?;
    assert_eq!(
        serde_json::to_value(preparation_list).map_err(|error| error.to_string())?,
        contract["outputs"]["preparationList"]
    );

    let probe = CodeRuntimeProbe {
        available: true,
        executable: Some("/usr/local/bin/codex".to_string()),
        version: Some("codex-cli 0.145.0".to_string()),
        error: None,
    };
    assert_eq!(
        serde_json::to_value(probe).map_err(|error| error.to_string())?,
        contract["outputs"]["runtimeProbe"]
    );
    let status = CodeRuntimeStatus {
        phase: super::runtime::CodeRuntimePhase::Ready,
        generation: 7,
        executable: Some("/usr/local/bin/codex".to_string()),
        version: Some("codex-cli 0.145.0".to_string()),
        pid: Some(1234),
        user_agent: Some("codex-test".to_string()),
        codex_home: Some("/native/codex-home".to_string()),
        platform_family: Some("unix".to_string()),
        platform_os: Some("macos".to_string()),
        queued_notifications: 2,
        last_error: None,
    };
    assert_eq!(
        serde_json::to_value(status).map_err(|error| error.to_string())?,
        contract["outputs"]["runtimeStatus"]
    );

    let repository_descriptor = CodeRepositoryDescriptor {
        repository_root: "/native/repository".to_string(),
        git_common_dir: "/native/repository/.git".to_string(),
        repository_identity: "a".repeat(64),
    };
    assert_eq!(
        serde_json::to_value(&repository_descriptor).map_err(|error| error.to_string())?,
        contract["outputs"]["repositoryDescriptor"]
    );
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: super::bindings::CodeExecutionMode::Local,
        repository_identity: "a".repeat(64),
        execution_root: "/native/stored-root".to_string(),
        base_ref: "b".repeat(40),
        worktree_id: None,
    };
    let worktree = CodeWorktreePrepareResult {
        repository: repository_descriptor,
        descriptor: descriptor.clone(),
        head_commit: "b".repeat(40),
        branch: Some("main".to_string()),
        dirty: false,
    };
    let prepared = CodePreparedWorktree {
        preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
        scope: decode(&contract["strictInputs"]["threadList"]["scope"])?,
        worktree: worktree.clone(),
    };
    assert_eq!(
        serde_json::to_value(prepared).map_err(|error| error.to_string())?,
        contract["outputs"]["preparedWorktree"]
    );
    let worktree_status = CodeWorktreeStatus {
        descriptor,
        head_commit: "b".repeat(40),
        branch: Some("main".to_string()),
        dirty: false,
    };
    assert_eq!(
        serde_json::to_value(worktree_status).map_err(|error| error.to_string())?,
        contract["outputs"]["worktreeStatus"]
    );

    let wire = fixture(WIRE_FIXTURE)?;
    let mut thread = parse_thread_open(wire["threadResume"]["result"].clone())?.thread;
    thread.name = Some("Native contract".to_string());
    assert_eq!(
        serde_json::to_value(&thread).map_err(|error| error.to_string())?,
        contract["outputs"]["threadSummary"]
    );
    let threads_page = CodeThreadsPage {
        data: vec![CodeBoundThreadSummary {
            binding: binding.clone(),
            thread: None,
            unavailable: Some("Codex app-server is not ready".to_string()),
        }],
        next_cursor: None,
        backwards_cursor: None,
    };
    assert_eq!(
        serde_json::to_value(threads_page).map_err(|error| error.to_string())?,
        contract["outputs"]["threadsPage"]
    );
    let open = CodeBoundThreadOpenResult {
        binding: binding.clone(),
        thread,
        instruction_sources: vec!["AGENTS.md".to_string()],
    };
    assert_eq!(
        serde_json::to_value(open).map_err(|error| error.to_string())?,
        contract["outputs"]["boundThreadOpen"]
    );

    let event = CodeWorkspaceEvent {
        scope: decode(&contract["outputs"]["event"]["scope"])?,
        runtime_generation: 7,
        sequence: 11,
        thread_id: Some("thread-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        item_id: Some("item-1".to_string()),
        kind: "item/agentMessage/delta".to_string(),
        payload: contract["outputs"]["event"]["payload"].clone(),
    };
    assert_eq!(
        serde_json::to_value(&event).map_err(|error| error.to_string())?,
        contract["outputs"]["event"]
    );
    assert_eq!(keys(&contract["outputs"]["event"])?.len(), 8);
    let event_without_ids = CodeWorkspaceEvent {
        scope: decode(&contract["outputs"]["eventWithoutIds"]["scope"])?,
        runtime_generation: 7,
        sequence: 12,
        thread_id: None,
        turn_id: None,
        item_id: None,
        kind: "configWarning".to_string(),
        payload: contract["outputs"]["eventWithoutIds"]["payload"].clone(),
    };
    assert_eq!(
        serde_json::to_value(event_without_ids).map_err(|error| error.to_string())?,
        contract["outputs"]["eventWithoutIds"]
    );
    let backlog = CodeEventBacklog {
        runtime_generation: 7,
        latest_sequence: 11,
        truncated: false,
        events: vec![event],
    };
    assert_eq!(
        serde_json::to_value(backlog).map_err(|error| error.to_string())?,
        contract["outputs"]["eventBacklog"]
    );
    let turn = CodeTurnSummary {
        id: "turn-1".to_string(),
        status: "inProgress".to_string(),
    };
    assert_eq!(
        serde_json::to_value(turn).map_err(|error| error.to_string())?,
        contract["outputs"]["turnSummary"]
    );
    let start_error = CodeThreadStartError::recovery(
        "threadStartUncertain",
        "Codex response was interrupted".to_string(),
        "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
        None,
        Some("/native/stored-root".to_string()),
    );
    assert_eq!(
        serde_json::to_value(start_error).map_err(|error| error.to_string())?,
        contract["outputs"]["threadStartError"]
    );
    assert_eq!(
        serde_json::to_value(()).map_err(|error| error.to_string())?,
        contract["outputs"]["unitResponse"]
    );
    Ok(())
}

#[test]
fn binding_store_fixture_reloads_and_public_list_scrubs_recovery_baseline() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = directory.path().join("execution-root");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(directory.path())?;
    let payload = STORE_FIXTURE.replace("{{EXECUTION_ROOT}}", &root.to_string_lossy());
    fs::write(store.store_path(), payload).map_err(|error| error.to_string())?;

    let loaded = store.load()?;
    assert_eq!(loaded.version, 1);
    assert_eq!(loaded.bindings.len(), 1);
    assert_eq!(loaded.preparations.len(), 1);
    let scope = CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: "a".repeat(64),
    };
    let public = store.list_preparations(&scope)?;
    assert_eq!(public.len(), 1);
    let public_value = serde_json::to_value(&public[0]).map_err(|error| error.to_string())?;
    assert!(public_value.get("recoveryThreadBaseline").is_none());
    let mut expected = fixture(TAURI_CONTRACT)?["outputs"]["preparationPublicBaseline"].clone();
    expected["executionRoot"] = json!(root.to_string_lossy());
    assert_eq!(public_value, expected);

    let reopened = CodeThreadBindingStore::for_app_data(directory.path())?;
    assert_eq!(reopened.load()?, loaded);
    Ok(())
}

#[test]
fn native_codex_builders_and_parsers_match_the_wire_fixture() -> Result<(), String> {
    let wire = fixture(WIRE_FIXTURE)?;
    let contract = fixture(TAURI_CONTRACT)?;
    let inputs = &contract["strictInputs"];

    let start: CodeThreadStartInput = decode(&inputs["threadStart"])?;
    assert_eq!(
        start.rpc_params("/native/stored-root")?,
        wire["threadStart"]["params"]
    );
    let opened = parse_thread_open(wire["threadStart"]["result"].clone())?;
    assert_eq!(opened.thread.id, "thread-1");
    assert_eq!(
        opened.thread_source.as_deref(),
        Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb")
    );

    assert_eq!(
        recovery_thread_list_params("/native/stored-root", Some("list-next"))?,
        wire["threadList"]["params"]
    );
    let listed = parse_recovery_thread_list(wire["threadList"]["result"].clone())?;
    assert_eq!(listed.data[0].thread.id, "thread-1");
    assert_eq!(listed.next_cursor.as_deref(), Some("list-page-2"));

    assert_eq!(
        loaded_thread_list_params(Some("loaded-next"))?,
        wire["threadLoadedList"]["params"]
    );
    let loaded = parse_loaded_thread_list(wire["threadLoadedList"]["result"].clone())?;
    assert_eq!(loaded.data, vec!["thread-1", "thread-2"]);

    assert_eq!(
        thread_read_params("thread-1")?,
        wire["threadRead"]["params"]
    );
    assert_eq!(
        parse_thread_read(wire["threadRead"]["result"].clone())?.id,
        "thread-1"
    );
    let recovered = parse_recovery_thread_read(wire["threadRead"]["result"].clone())?;
    assert_eq!(
        recovered.thread_source.as_deref(),
        Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb")
    );

    let resume: CodeThreadResumeInput = decode(&inputs["threadResume"])?;
    assert_eq!(
        resume.rpc_params("/native/stored-root")?,
        wire["threadResume"]["params"]
    );
    assert_eq!(
        parse_thread_open(wire["threadResume"]["result"].clone())?
            .thread
            .turns[0]
            .id,
        "past-turn"
    );

    let turn: CodeTurnStartInput = decode(&inputs["turnStart"])?;
    assert_eq!(
        turn.rpc_params("/native/stored-root")?,
        wire["turnStart"]["params"]
    );
    assert_eq!(
        parse_turn_start(wire["turnStart"]["result"].clone())?.id,
        "turn-1"
    );

    let steer: CodeTurnSteerInput = decode(&inputs["turnSteer"])?;
    assert_eq!(steer.rpc_params()?, wire["turnSteer"]["params"]);
    assert_eq!(
        parse_turn_steer(wire["turnSteer"]["result"].clone())?.id,
        "turn-1"
    );

    let interrupt: CodeTurnInterruptInput = decode(&inputs["turnInterrupt"])?;
    assert_eq!(interrupt.rpc_params()?, wire["turnInterrupt"]["params"]);
    for params in [
        &wire["threadStart"]["params"],
        &wire["threadResume"]["params"],
        &wire["turnStart"]["params"],
        &wire["turnSteer"]["params"],
        &wire["turnInterrupt"]["params"],
    ] {
        for forbidden in [
            "scope",
            "communityId",
            "projectDtag",
            "repositoryIdentity",
            "workspaceRoot",
            "descriptor",
            "runtimeWorkspaceRoots",
        ] {
            assert!(params.get(forbidden).is_none(), "wire leaked {forbidden}");
        }
    }

    assert_eq!(initialize_params(), wire["initialize"]["request"]["params"]);
    let initialize = jsonrpc::request(1, "initialize", initialize_params());
    assert!(initialize.get("jsonrpc").is_none());
    assert_eq!(
        jsonrpc::notification("initialized"),
        wire["initialize"]["initializedNotification"]
    );
    assert!(wire["initialize"]["initializedNotification"]
        .get("jsonrpc")
        .is_none());
    Ok(())
}

#[test]
fn all_supported_notifications_and_approvals_match_wire_fixtures() -> Result<(), String> {
    let wire = fixture(WIRE_FIXTURE)?;
    let notifications = wire["notifications"]
        .as_array()
        .ok_or_else(|| "missing notifications".to_string())?;
    assert_eq!(notifications.len(), 21);
    let mut methods = HashSet::new();
    for notification in notifications {
        let method = notification["method"]
            .as_str()
            .ok_or_else(|| "notification has no method".to_string())?;
        assert!(methods.insert(method));
        let normalized = normalize_notification(method, Some(notification["params"].clone()))?
            .ok_or_else(|| format!("supported notification `{method}` was ignored"))?;
        assert_eq!(normalized.kind, method);
        assert_eq!(normalized.payload, notification["params"]);
        if let Some(thread_id) = notification["params"]
            .get("threadId")
            .and_then(Value::as_str)
        {
            assert_eq!(normalized.thread_id.as_deref(), Some(thread_id));
        }
    }
    assert!(normalize_notification("future/unknown", Some(json!({})))?.is_none());

    let approvals = PendingApprovalStore::default();
    approvals.reset(7);
    for approval in wire["approvals"]
        .as_array()
        .ok_or_else(|| "missing approvals".to_string())?
    {
        let request = &approval["request"];
        let event = approvals
            .insert_request(
                7,
                request["id"].clone(),
                request["method"].as_str().unwrap_or_default(),
                Some(request["params"].clone()),
            )?
            .ok_or_else(|| "approval request was not recognized".to_string())?;
        assert_eq!(event.kind, request["method"]);
        let input: CodeApprovalResponseInput = decode(&approval["responseInput"])?;
        let reservation = approvals.reserve_response(&input)?;
        let (request_id, result) = reservation.wire_response();
        assert_eq!(request_id, request["id"]);
        assert_eq!(result, approval["wireResult"]);
        approvals.commit_response(&reservation)?;
    }
    assert_eq!(approvals.len(), 0);
    Ok(())
}

#[test]
#[ignore = "manual audit: requires the pinned Codex 0.145.0 CLI"]
fn refresh_schema_snapshot_is_manual_only() -> Result<(), String> {
    let executable = crate::managed_agents::resolve_command("codex")
        .ok_or_else(|| "Codex CLI is not installed".to_string())?;
    let version = Command::new(&executable)
        .arg("--version")
        .output()
        .map_err(|error| error.to_string())?;
    if !version.status.success()
        || String::from_utf8_lossy(&version.stdout).trim() != "codex-cli 0.145.0"
    {
        return Err("schema refresh requires exact codex-cli 0.145.0".to_string());
    }

    let generated = tempfile::tempdir().map_err(|error| error.to_string())?;
    let output = Command::new(executable)
        .args(["app-server", "generate-json-schema", "--out"])
        .arg(generated.path())
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Codex schema generation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let mut paths = Vec::new();
    collect_json_paths(generated.path(), generated.path(), &mut paths)?;
    paths.sort();
    let manifest = fixture(SCHEMA_MANIFEST)?;
    assert_eq!(
        paths.len(),
        manifest["source"]["generatedFileCount"]
            .as_u64()
            .ok_or_else(|| "generatedFileCount must be numeric".to_string())? as usize
    );
    let selected = manifest["schemas"]
        .as_array()
        .ok_or_else(|| "manifest schemas must be an array".to_string())?
        .iter()
        .map(|entry| {
            let hash = entry[0]
                .as_str()
                .ok_or_else(|| "manifest schema hash must be a string".to_string())?;
            let path = entry[1]
                .as_str()
                .ok_or_else(|| "manifest schema path must be a string".to_string())?;
            Ok((path.to_string(), hash.to_string()))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let mut full_aggregate = String::new();
    let mut selected_aggregate = String::new();
    for path in paths {
        let schema = serde_json::from_slice(
            &fs::read(generated.path().join(&path)).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("invalid generated schema {path}: {error}"))?;
        let hash = sha256_hex(&canonical_schema_bytes(schema)?);
        full_aggregate.push_str(&hash);
        full_aggregate.push_str("  ");
        full_aggregate.push_str(&path);
        full_aggregate.push('\n');
        if let Some(expected) = selected.get(&path) {
            assert_eq!(&hash, expected, "generated schema drifted for {path}");
            selected_aggregate.push_str(&hash);
            selected_aggregate.push_str("  ");
            selected_aggregate.push_str(&path);
            selected_aggregate.push('\n');
        }
    }
    assert_eq!(
        sha256_hex(full_aggregate.as_bytes()),
        manifest["source"]["fullGeneratedSetSha256"]
    );
    assert_eq!(
        sha256_hex(selected_aggregate.as_bytes()),
        manifest["source"]["selectedSchemasSha256"]
    );
    Ok(())
}
