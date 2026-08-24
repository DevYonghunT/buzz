use serde_json::{Map, Value};

use super::super::protocol::redact_protocol_text;
use super::{
    PermissionAccessDisplay, PermissionDisplay, PermissionFileSystemDisplay,
    PermissionFileSystemEntryDisplay, PermissionNetworkDisplay, PermissionPathDisplay,
    PermissionSpecialPathDisplay, MAX_SAFE_JSON_INTEGER,
};

struct PermissionDisplayValidation {
    accurate: bool,
    non_empty: bool,
}
impl PermissionDisplayValidation {
    fn new() -> Self {
        Self {
            accurate: true,
            non_empty: false,
        }
    }

    fn invalidate(&mut self) {
        self.accurate = false;
    }
}

pub(super) fn permission_display_from_raw(raw: Option<&Value>) -> PermissionDisplay {
    let mut validation = PermissionDisplayValidation::new();
    let Some(permissions) = raw.and_then(Value::as_object) else {
        validation.invalidate();
        return PermissionDisplay {
            grantable: false,
            network: None,
            file_system: None,
        };
    };
    if !has_only_keys(permissions, &["network", "fileSystem"]) {
        validation.invalidate();
    }
    let network = permissions
        .get("network")
        .and_then(|value| parse_network_display(value, &mut validation));
    let file_system = permissions
        .get("fileSystem")
        .and_then(|value| parse_file_system_display(value, &mut validation));
    PermissionDisplay {
        grantable: validation.accurate && validation.non_empty,
        network,
        file_system,
    }
}

fn parse_network_display(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionNetworkDisplay> {
    if value.is_null() {
        return None;
    }
    let Some(network) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    if !has_only_keys(network, &["enabled"]) {
        validation.invalidate();
    }
    let enabled = match network.get("enabled") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(enabled)) => {
            validation.non_empty |= *enabled;
            Some(*enabled)
        }
        Some(_) => {
            validation.invalidate();
            None
        }
    };
    Some(PermissionNetworkDisplay { enabled })
}

fn parse_file_system_display(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionFileSystemDisplay> {
    if value.is_null() {
        return None;
    }
    let Some(file_system) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    if !has_only_keys(
        file_system,
        &["entries", "globScanMaxDepth", "read", "write"],
    ) {
        validation.invalidate();
    }
    let entries = file_system
        .get("entries")
        .and_then(|value| parse_file_system_entries(value, validation));
    let glob_scan_max_depth = match file_system.get("globScanMaxDepth") {
        None | Some(Value::Null) => None,
        Some(value) => match value
            .as_u64()
            .filter(|depth| *depth > 0 && *depth <= MAX_SAFE_JSON_INTEGER)
        {
            Some(depth) => Some(depth),
            None => {
                validation.invalidate();
                None
            }
        },
    };
    let read = file_system
        .get("read")
        .and_then(|value| parse_permission_paths(value, validation));
    let write = file_system
        .get("write")
        .and_then(|value| parse_permission_paths(value, validation));
    validation.non_empty |= entries.as_ref().is_some_and(|entries| !entries.is_empty())
        || read.as_ref().is_some_and(|paths| !paths.is_empty())
        || write.as_ref().is_some_and(|paths| !paths.is_empty());
    Some(PermissionFileSystemDisplay {
        entries,
        glob_scan_max_depth,
        read,
        write,
    })
}

fn parse_permission_paths(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<Vec<String>> {
    if value.is_null() {
        return None;
    }
    let Some(paths) = value.as_array() else {
        validation.invalidate();
        return None;
    };
    Some(
        paths
            .iter()
            .filter_map(|path| permission_text(path, validation))
            .collect(),
    )
}

fn parse_file_system_entries(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<Vec<PermissionFileSystemEntryDisplay>> {
    if value.is_null() {
        return None;
    }
    let Some(entries) = value.as_array() else {
        validation.invalidate();
        return None;
    };
    Some(
        entries
            .iter()
            .filter_map(|entry| parse_file_system_entry(entry, validation))
            .collect(),
    )
}

fn parse_file_system_entry(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionFileSystemEntryDisplay> {
    let Some(entry) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    if !has_only_keys(entry, &["access", "path"]) {
        validation.invalidate();
    }
    let access = match entry.get("access").and_then(Value::as_str) {
        Some("read") => Some(PermissionAccessDisplay::Read),
        Some("write") => Some(PermissionAccessDisplay::Write),
        Some("deny") => Some(PermissionAccessDisplay::Deny),
        _ => {
            validation.invalidate();
            None
        }
    };
    let path = match entry.get("path") {
        Some(path) => parse_permission_path(path, validation),
        None => {
            validation.invalidate();
            None
        }
    };
    match (access, path) {
        (Some(access), Some(path)) => Some(PermissionFileSystemEntryDisplay { access, path }),
        _ => None,
    }
}

fn parse_permission_path(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionPathDisplay> {
    let Some(path) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    match path.get("type").and_then(Value::as_str) {
        Some("path") => {
            if !has_only_keys(path, &["type", "path"]) {
                validation.invalidate();
            }
            match path.get("path") {
                Some(path) => permission_text(path, validation)
                    .map(|path| PermissionPathDisplay::Path { path }),
                None => {
                    validation.invalidate();
                    None
                }
            }
        }
        Some("glob_pattern") => {
            if !has_only_keys(path, &["type", "pattern"]) {
                validation.invalidate();
            }
            match path.get("pattern") {
                Some(pattern) => permission_text(pattern, validation)
                    .map(|pattern| PermissionPathDisplay::GlobPattern { pattern }),
                None => {
                    validation.invalidate();
                    None
                }
            }
        }
        Some("special") => {
            if !has_only_keys(path, &["type", "value"]) {
                validation.invalidate();
            }
            match path.get("value") {
                Some(value) => parse_special_path(value, validation)
                    .map(|value| PermissionPathDisplay::Special { value }),
                None => {
                    validation.invalidate();
                    None
                }
            }
        }
        _ => {
            validation.invalidate();
            None
        }
    }
}

fn parse_special_path(
    value: &Value,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionSpecialPathDisplay> {
    let Some(special) = value.as_object() else {
        validation.invalidate();
        return None;
    };
    match special.get("kind").and_then(Value::as_str) {
        Some("root") => exact_special(
            special,
            &["kind"],
            PermissionSpecialPathDisplay::Root,
            validation,
        ),
        Some("minimal") => exact_special(
            special,
            &["kind"],
            PermissionSpecialPathDisplay::Minimal,
            validation,
        ),
        Some("tmpdir") => exact_special(
            special,
            &["kind"],
            PermissionSpecialPathDisplay::Tmpdir,
            validation,
        ),
        Some("slash_tmp") => exact_special(
            special,
            &["kind"],
            PermissionSpecialPathDisplay::SlashTmp,
            validation,
        ),
        Some("project_roots") => {
            if !has_only_keys(special, &["kind", "subpath"]) {
                validation.invalidate();
            }
            optional_permission_text(special.get("subpath"), validation)
                .map(|subpath| PermissionSpecialPathDisplay::ProjectRoots { subpath })
        }
        Some("unknown") => {
            if !has_only_keys(special, &["kind", "path", "subpath"]) {
                validation.invalidate();
            }
            let path = match special.get("path") {
                Some(path) => permission_text(path, validation),
                None => {
                    validation.invalidate();
                    None
                }
            };
            let subpath = optional_permission_text(special.get("subpath"), validation);
            path.zip(subpath)
                .map(|(path, subpath)| PermissionSpecialPathDisplay::Unknown { path, subpath })
        }
        _ => {
            validation.invalidate();
            None
        }
    }
}

fn exact_special(
    special: &Map<String, Value>,
    keys: &[&str],
    display: PermissionSpecialPathDisplay,
    validation: &mut PermissionDisplayValidation,
) -> Option<PermissionSpecialPathDisplay> {
    if !has_only_keys(special, keys) {
        validation.invalidate();
    }
    Some(display)
}

fn optional_permission_text(
    value: Option<&Value>,
    validation: &mut PermissionDisplayValidation,
) -> Option<Option<String>> {
    match value {
        None | Some(Value::Null) => Some(None),
        Some(value) => permission_text(value, validation).map(Some),
    }
}

fn permission_text(value: &Value, validation: &mut PermissionDisplayValidation) -> Option<String> {
    let Some(text) = value.as_str() else {
        validation.invalidate();
        return None;
    };
    if text.is_empty() {
        validation.invalidate();
    }
    let redacted = redact_protocol_text(text);
    if redacted != text {
        validation.invalidate();
    }
    Some(redacted)
}

fn has_only_keys(object: &Map<String, Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}
