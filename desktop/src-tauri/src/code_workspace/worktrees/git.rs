#[cfg(unix)]
use super::pinned_command::*;
#[cfg(not(unix))]
use super::pinned_verify::*;
use super::*;

#[cfg(not(unix))]
pub(super) fn run_git(
    cwd: &Path,
    args: &[OsString],
    operation: GitOperation,
) -> Result<Vec<u8>, String> {
    run_git_until(cwd, args, operation, None)
}

#[cfg(not(unix))]
pub(super) fn run_git_until(
    cwd: &Path,
    args: &[OsString],
    operation: GitOperation,
    deadline: Option<Instant>,
) -> Result<Vec<u8>, String> {
    let disabled_filter_keys = if operation.may_run_repository_filters() {
        repository_filter_overrides_until(cwd, deadline)?
    } else {
        Vec::new()
    };
    run_git_with_filter_overrides_until(cwd, args, operation, &disabled_filter_keys, deadline)
}

pub(super) fn repository_filter_overrides(cwd: &Path) -> Result<Vec<String>, String> {
    repository_filter_overrides_until(cwd, None)
}

pub(super) fn repository_filter_overrides_until(
    cwd: &Path,
    deadline: Option<Instant>,
) -> Result<Vec<String>, String> {
    let mut overrides = BTreeSet::new();
    #[cfg(unix)]
    let local_output = run_pinned_read_until(
        cwd,
        CodePinnedReadCommand::LocalConfig,
        Vec::new(),
        deadline,
    )?;
    #[cfg(not(unix))]
    let local_output = run_git_with_filter_overrides_until(
        cwd,
        &[
            OsString::from("config"),
            OsString::from("--local"),
            OsString::from("--includes"),
            OsString::from("--null"),
            OsString::from("--list"),
        ],
        GitOperation::ReadOnly,
        &[],
        deadline,
    )?;
    let worktree_config_enabled = collect_local_filter_overrides(&local_output, &mut overrides)?;
    if worktree_config_enabled {
        #[cfg(unix)]
        let worktree_output = run_pinned_read_until(
            cwd,
            CodePinnedReadCommand::WorktreeConfigNames,
            Vec::new(),
            deadline,
        )?;
        #[cfg(not(unix))]
        let worktree_output = run_git_with_filter_overrides_until(
            cwd,
            &[
                OsString::from("config"),
                OsString::from("--worktree"),
                OsString::from("--includes"),
                OsString::from("--null"),
                OsString::from("--name-only"),
                OsString::from("--list"),
            ],
            GitOperation::ReadOnly,
            &[],
            deadline,
        )?;
        collect_filter_override_names(&worktree_output, &mut overrides)?;
    }
    Ok(overrides.into_iter().collect())
}

pub(crate) fn collect_local_filter_overrides(
    output: &[u8],
    overrides: &mut BTreeSet<String>,
) -> Result<bool, String> {
    let mut worktree_config_enabled = false;
    for record in output
        .split(|byte| *byte == b'\0')
        .filter(|record| !record.is_empty())
    {
        let separator = record
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| "Git config record did not contain a value separator".to_string())?;
        let key = std::str::from_utf8(&record[..separator])
            .map_err(|error| format!("Git config key was not UTF-8: {error}"))?;
        collect_filter_override(key, overrides)?;
        if key.eq_ignore_ascii_case("extensions.worktreeConfig") {
            worktree_config_enabled = parse_git_boolean(&record[separator + 1..])?;
        }
    }
    Ok(worktree_config_enabled)
}

pub(crate) fn collect_filter_override_names(
    output: &[u8],
    overrides: &mut BTreeSet<String>,
) -> Result<(), String> {
    for raw_key in output
        .split(|byte| *byte == b'\0')
        .filter(|key| !key.is_empty())
    {
        let key = std::str::from_utf8(raw_key)
            .map_err(|error| format!("Git config key was not UTF-8: {error}"))?;
        collect_filter_override(key, overrides)?;
    }
    Ok(())
}

pub(super) fn collect_filter_override(
    key: &str,
    overrides: &mut BTreeSet<String>,
) -> Result<(), String> {
    let normalized = key.to_ascii_lowercase();
    let is_filter_command = normalized.starts_with("filter.")
        && [".clean", ".smudge", ".process"]
            .iter()
            .any(|suffix| normalized.ends_with(suffix));
    if !is_filter_command {
        return Ok(());
    }
    if key.chars().any(char::is_control) {
        return Err("Git filter config key contained control characters".to_string());
    }
    let section_end = key
        .rfind('.')
        .ok_or_else(|| "Git filter config key was malformed".to_string())?;
    let filter_prefix = &key[..section_end];
    overrides.insert(key.to_string());
    overrides.insert(format!("{filter_prefix}.required"));
    Ok(())
}

pub(super) fn parse_git_boolean(value: &[u8]) -> Result<bool, String> {
    let value = std::str::from_utf8(value)
        .map_err(|error| format!("Git worktreeConfig value was not UTF-8: {error}"))?;
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err("Git extensions.worktreeConfig was not a valid boolean".to_string()),
    }
}

#[cfg(not(unix))]
pub(super) fn run_git_with_filter_overrides_until(
    cwd: &Path,
    args: &[OsString],
    operation: GitOperation,
    disabled_filter_keys: &[String],
    deadline: Option<Instant>,
) -> Result<Vec<u8>, String> {
    run_git_executable_with_filter_overrides(
        &git_executable()?,
        cwd,
        args,
        operation,
        disabled_filter_keys,
        deadline,
    )
}

#[cfg(any(not(unix), test))]
pub(super) fn git_executable() -> Result<PathBuf, String> {
    let executable = crate::managed_agents::resolve_command("git")
        .ok_or_else(|| "git executable was not found".to_string())?;
    executable
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize git executable: {error}"))
}
