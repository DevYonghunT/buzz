use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use super::super::engine::{validate_path, RawChange};
use super::super::git_command::{
    pin_directory, verify_named_directory_identity, GitConfigKey, GitConfigScope, GitWriteCommand,
    PinnedGitWriteRepository,
};
use super::output_line;

pub(super) fn validate_oid(value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err("Git returned an invalid object id".to_string())
    }
}

pub(super) fn reject_existing_lock(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "Git {} is busy; SchoolX will not remove or steal an existing lock",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("repository lock")
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect Git lock: {error}")),
    }
}

pub(super) fn reject_alternate_object_store(common: &Path) -> Result<(), String> {
    let objects = common.join("objects");
    let objects_identity = pin_directory(&objects)
        .map_err(|error| format!("failed to pin primary Git object database: {error}"))?;
    verify_named_directory_identity(&objects_identity)?;
    let alternates = objects.join("info").join("alternates");
    match fs::symlink_metadata(&alternates) {
        Ok(_) => {
            Err("Git alternate/shared object databases are read-only in this release".to_string())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect Git alternates: {error}")),
    }
}

pub(super) fn validate_local_config(bytes: &[u8]) -> Result<bool, String> {
    let records = parse_config_records(bytes)?;
    validate_config_records(&records)?;
    let mut worktree_config = false;
    for (key, value) in records {
        if key == "extensions.worktreeconfig" {
            worktree_config = parse_git_bool(&value)?;
        }
        if key == "extensions.objectformat" && !matches!(value.as_str(), "sha1" | "sha256") {
            return Err("Unsupported Git object format blocks writes".to_string());
        }
        if key == "extensions.refstorage" && value != "files" {
            return Err("Only the loose-files Git ref backend is writable".to_string());
        }
    }
    Ok(worktree_config)
}

pub(super) fn validate_worktree_config(bytes: &[u8]) -> Result<(), String> {
    validate_config_records(&parse_config_records(bytes)?)
}

fn parse_config_records(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    let mut records = Vec::new();
    for raw in bytes.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()) {
        let text = std::str::from_utf8(raw)
            .map_err(|_| "Repository config contains non-UTF-8 data".to_string())?;
        let (key, value) = text
            .split_once('\n')
            .or_else(|| text.split_once('='))
            .ok_or_else(|| "Repository config returned an ambiguous record".to_string())?;
        if key.is_empty() || key.chars().any(char::is_control) || value.contains('\0') {
            return Err("Repository config returned an unsafe record".to_string());
        }
        records.push((key.to_ascii_lowercase(), value.to_string()));
    }
    Ok(records)
}

fn validate_config_records(records: &[(String, String)]) -> Result<(), String> {
    for (key, value) in records {
        let executable = key.starts_with("filter.")
            || key == "diff.external"
            || (key.starts_with("diff.")
                && (key.ends_with(".command") || key.ends_with(".textconv")))
            || key == "core.fsmonitor"
            || key == "core.hookspath"
            || key == "core.alternaterefscommand"
            || key == "core.attributesfile"
            || key == "core.worktree"
            || key == "extensions.partialclone"
            || key.starts_with("include.")
            || key.starts_with("includeif.");
        if executable {
            return Err(format!(
                "Repository config {key} is outside the safe Git write subset"
            ));
        }
        if matches!(
            key.as_str(),
            "core.sparsecheckout" | "core.sparsecheckoutcone"
        ) && parse_git_bool(value)?
        {
            return Err("Sparse Git worktrees are read-only in this release".to_string());
        }
        if key == "index.sparse" && parse_git_bool(value)? {
            return Err("Sparse Git indexes are read-only in this release".to_string());
        }
    }
    Ok(())
}

fn parse_git_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" | "" => Ok(false),
        _ => Err("Repository config contains an invalid boolean value".to_string()),
    }
}

pub(super) fn validate_autocrlf(
    runner: &PinnedGitWriteRepository,
    worktree_config_enabled: bool,
) -> Result<(), String> {
    let mut scopes = vec![GitConfigScope::Local];
    if worktree_config_enabled {
        scopes.push(GitConfigScope::Worktree);
    }
    for scope in scopes {
        let output = runner.run(GitWriteCommand::ConfigValue {
            scope,
            key: GitConfigKey::CoreAutocrlf,
        })?;
        if output.code == 0 {
            let value = output_line(&output)?;
            if parse_git_bool(value)? {
                return Err(
                    "core.autocrlf must be false for raw-byte whole-file staging".to_string(),
                );
            }
        }
    }
    Ok(())
}

pub(super) fn reject_ambiguous_paths(
    staged: &[RawChange],
    unstaged: &[RawChange],
) -> Result<(), String> {
    let mut normalized = BTreeMap::<String, String>::new();
    for path in staged.iter().chain(unstaged).map(|change| &change.path) {
        let folded = path.to_lowercase();
        if let Some(existing) = normalized.insert(folded, path.clone()) {
            if existing != *path {
                return Err("Case-normalization-colliding Git paths are read-only".to_string());
            }
        }
    }
    Ok(())
}

pub(super) fn validate_path_attributes(
    runner: &PinnedGitWriteRepository,
    path: &str,
) -> Result<(), String> {
    let output = runner.run(GitWriteCommand::CheckAttributes {
        path: path.to_string(),
    })?;
    let fields = output.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
    let fields = fields.strip_suffix(&[&[][..]]).unwrap_or(fields.as_slice());
    if fields.len() % 3 != 0 {
        return Err("Git attributes returned an ambiguous record".to_string());
    }
    for record in fields.chunks_exact(3) {
        let attribute = std::str::from_utf8(record[1])
            .map_err(|_| "Git attribute name was non-UTF-8".to_string())?;
        let value = std::str::from_utf8(record[2])
            .map_err(|_| "Git attribute value was non-UTF-8".to_string())?;
        if matches!(
            attribute,
            "filter" | "text" | "eol" | "ident" | "working-tree-encoding"
        ) && !matches!(value, "unspecified" | "unset")
        {
            return Err(format!(
                "Git attribute {attribute} transforms {path}; raw-byte staging is unavailable"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_stage_entries(bytes: &[u8]) -> Result<(), String> {
    let mut paths = BTreeSet::new();
    for raw in bytes.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()) {
        let text = std::str::from_utf8(raw)
            .map_err(|_| "Git index contains a non-UTF-8 path".to_string())?;
        let (metadata, path) = text
            .split_once('\t')
            .ok_or_else(|| "Git index returned an ambiguous entry".to_string())?;
        validate_path(path)?;
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 4 || fields[0] != "H" || fields[3] != "0" {
            return Err(
                "Unmerged, sparse, skip-worktree, assume-unchanged, or intent-to-add index entries are read-only"
                    .to_string(),
            );
        }
        if !matches!(fields[1], "100644" | "100755") {
            return Err(
                "Symlink, gitlink, and special-file index entries are read-only".to_string(),
            );
        }
        validate_oid(fields[2])?;
        if fields[2].bytes().all(|byte| byte == b'0') || !paths.insert(path.to_string()) {
            return Err("Git index contains an ambiguous stage-0 entry".to_string());
        }
    }
    Ok(())
}

pub(super) fn read_no_follow(path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
    #[cfg(unix)]
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| format!("failed to securely read Git evidence: {error}"))?;
    #[cfg(not(unix))]
    return Err("secure Git evidence reads are unavailable on this platform".to_string());
    #[cfg(unix)]
    {
        let metadata = file
            .metadata()
            .map_err(|error| format!("failed to inspect Git evidence: {error}"))?;
        if !metadata.is_file() || metadata.len() > max_bytes as u64 {
            return Err("Git evidence is not a bounded regular file".to_string());
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("failed to read Git evidence: {error}"))?;
        if bytes.len() > max_bytes {
            return Err("Git evidence exceeded its read limit".to_string());
        }
        Ok(bytes)
    }
}
