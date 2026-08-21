use super::project_git_exec::{
    build_git_auth_config, clean_branch, run_git, validate_workspace_clone_url, GitAuthConfig,
    GitReadSnapshot, PinnedGitDirectory,
};
use super::project_repo_paths::find_local_repo_dir;
use crate::app_state::AppState;
use serde::Serialize;
use tauri::State;

/// Per-file cap on rendered patch lines. One regenerated lockfile or
/// minified bundle would otherwise produce tens of thousands of DOM nodes
/// in the diff view and freeze the webview.
const MAX_PATCH_LINES: usize = 2_000;
const MAX_PATCH_BYTES: usize = 256 * 1024;
const MAX_DIFF_FILES: usize = 250;
const CHANGES_DRIFT_ERROR: &str =
    "SchoolX Code Changes changed during inspection; retry after the workspace settles";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectRepoDiffFileInfo {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectRepoDiffInfo {
    pub files: Vec<ProjectRepoDiffFileInfo>,
    pub additions: usize,
    pub deletions: usize,
    pub commit_body: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CurrentRepoChangeStatus {
    Added,
    Modified,
    Deleted,
    TypeChanged,
    Unmerged,
    Untracked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CurrentRepoDiffFileInfo {
    pub(crate) path: String,
    pub(crate) status: CurrentRepoChangeStatus,
    pub(crate) binary: bool,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
    pub(crate) patch: String,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CurrentRepoDiffInfo {
    pub(crate) files: Vec<CurrentRepoDiffFileInfo>,
    pub(crate) total_files: usize,
    pub(crate) files_truncated: bool,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrackedInventoryEntry {
    path: String,
    status: CurrentRepoChangeStatus,
    binary: bool,
    additions: usize,
    deletions: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CurrentInventoryEntry {
    Tracked(TrackedInventoryEntry),
    Untracked { path: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CurrentChangesManifest {
    entries: Vec<CurrentInventoryEntry>,
}

#[derive(Debug)]
enum CurrentChangesReadError {
    Drift,
    Fatal(String),
}

impl From<String> for CurrentChangesReadError {
    fn from(error: String) -> Self {
        Self::Fatal(error)
    }
}

fn clean_target_ref(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        value.starts_with("refs/")
            && !value.contains("..")
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
    })
}

pub(crate) fn clean_commit(value: Option<String>) -> Option<String> {
    value
        .filter(|value| matches!(value.len(), 40 | 64))
        .filter(|value| value.chars().all(|c| c.is_ascii_hexdigit()))
}

fn fetch_target(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch: Option<&str>,
    target_ref: Option<&str>,
    target_commit: Option<&str>,
) -> Result<(), String> {
    if let Some(target_ref) = target_ref {
        if run_git(
            &["fetch", "--depth=100", "origin", target_ref],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            run_git(
                &["checkout", "--detach", "FETCH_HEAD"],
                Some(repo_dir),
                auth,
            )?;
            return Ok(());
        }
    } else if let Some(target_commit) = target_commit {
        if run_git(
            &["fetch", "--depth=100", "origin", target_commit],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            run_git(
                &["checkout", "--detach", "FETCH_HEAD"],
                Some(repo_dir),
                auth,
            )?;
            return Ok(());
        }
    }

    if let Some(target_commit) = target_commit {
        if run_git(
            &["fetch", "--depth=100", "origin", target_commit],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            run_git(
                &["checkout", "--detach", "FETCH_HEAD"],
                Some(repo_dir),
                auth,
            )?;
            return Ok(());
        }
    }

    if let Some(branch) = branch {
        let refspec = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
        run_git(
            &["fetch", "--depth=100", "origin", &refspec],
            Some(repo_dir),
            auth,
        )?;
        run_git(
            &["checkout", "--detach", &format!("origin/{branch}")],
            Some(repo_dir),
            auth,
        )?;
        return Ok(());
    }

    run_git(
        &["fetch", "--depth=100", "origin", "HEAD"],
        Some(repo_dir),
        auth,
    )?;
    run_git(
        &["checkout", "--detach", "FETCH_HEAD"],
        Some(repo_dir),
        auth,
    )?;
    Ok(())
}

fn diff_base_ref(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_branch: Option<&str>,
) -> Option<String> {
    let base_branch = base_branch?;
    let refspec = format!("refs/heads/{base_branch}:refs/remotes/origin/{base_branch}");
    run_git(
        &["fetch", "--depth=100", "origin", &refspec],
        Some(repo_dir),
        auth,
    )
    .ok()?;
    Some(format!("origin/{base_branch}"))
}

fn parse_count(value: &str) -> usize {
    value.parse::<usize>().unwrap_or_default()
}

fn parse_numstat(output: &str) -> Vec<(String, usize, usize)> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let additions = parse_count(parts.next()?);
            let deletions = parse_count(parts.next()?);
            let path = parts.next()?.to_string();
            Some((path, additions, deletions))
        })
        .take(MAX_DIFF_FILES)
        .collect()
}

fn safe_repo_relative_path(value: &str) -> bool {
    use std::path::Component;

    !value.is_empty()
        && !value
            .chars()
            .any(|character| matches!(character, '\0' | '\n' | '\r' | '\t'))
        && std::path::Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn require_nul_terminated(output: &str, label: &str) -> Result<(), String> {
    if !output.is_empty() && !output.ends_with('\0') {
        return Err(format!("Git returned unterminated {label}"));
    }
    Ok(())
}

fn parse_strict_numstat_counts(
    additions: &str,
    deletions: &str,
) -> Result<(usize, usize, bool), String> {
    match (additions, deletions) {
        ("-", "-") => Ok((0, 0, true)),
        ("-", _) | (_, "-") => Err("Git returned one-sided binary change statistics".to_string()),
        _ => {
            let additions = additions
                .parse::<usize>()
                .map_err(|_| "Git returned malformed addition statistics".to_string())?;
            let deletions = deletions
                .parse::<usize>()
                .map_err(|_| "Git returned malformed deletion statistics".to_string())?;
            Ok((additions, deletions, false))
        }
    }
}

fn parse_current_numstat_z(output: &str) -> Result<Vec<TrackedInventoryEntry>, String> {
    require_nul_terminated(output, "change statistics")?;
    let mut paths = std::collections::BTreeSet::new();
    output
        .split_terminator('\0')
        .map(|entry| {
            let mut parts = entry.splitn(3, '\t');
            let additions = parts
                .next()
                .ok_or_else(|| "Git returned malformed change statistics".to_string())?;
            let deletions = parts
                .next()
                .ok_or_else(|| "Git returned malformed change statistics".to_string())?;
            let path = parts
                .next()
                .ok_or_else(|| "Git returned malformed change statistics".to_string())?;
            if !safe_repo_relative_path(path) {
                return Err("Git reported an unsafe changed path".to_string());
            }
            if !paths.insert(path.to_string()) {
                return Err("Git reported duplicate change statistics".to_string());
            }
            let (additions, deletions, binary) = parse_strict_numstat_counts(additions, deletions)?;
            Ok(TrackedInventoryEntry {
                path: path.to_string(),
                status: CurrentRepoChangeStatus::Modified,
                binary,
                additions,
                deletions,
            })
        })
        .collect()
}

fn parse_current_status(value: &str) -> Result<CurrentRepoChangeStatus, String> {
    match value {
        "A" => Ok(CurrentRepoChangeStatus::Added),
        "M" => Ok(CurrentRepoChangeStatus::Modified),
        "D" => Ok(CurrentRepoChangeStatus::Deleted),
        "T" => Ok(CurrentRepoChangeStatus::TypeChanged),
        "U" => Ok(CurrentRepoChangeStatus::Unmerged),
        _ => Err(format!("Git returned unsupported change status {value:?}")),
    }
}

fn parse_current_name_status_z(
    output: &str,
) -> Result<Vec<(String, CurrentRepoChangeStatus)>, String> {
    require_nul_terminated(output, "change status")?;
    let fields = output.split_terminator('\0').collect::<Vec<_>>();
    if fields.len() % 2 != 0 {
        return Err("Git returned malformed change status fields".to_string());
    }
    let mut paths = std::collections::BTreeSet::new();
    fields
        .chunks_exact(2)
        .map(|fields| {
            let status = parse_current_status(fields[0])?;
            let path = fields[1];
            if !safe_repo_relative_path(path) {
                return Err("Git reported an unsafe status path".to_string());
            }
            if !paths.insert(path.to_string()) {
                return Err("Git reported duplicate change status".to_string());
            }
            Ok((path.to_string(), status))
        })
        .collect()
}

fn parse_current_paths(output: &str, label: &str) -> Result<Vec<String>, String> {
    require_nul_terminated(output, label)?;
    let mut paths = std::collections::BTreeSet::new();
    for path in output.split_terminator('\0') {
        if !safe_repo_relative_path(path) {
            return Err(format!("Git reported an unsafe {label}"));
        }
        if !paths.insert(path.to_string()) {
            return Err(format!("Git reported a duplicate {label}"));
        }
    }
    Ok(paths.into_iter().collect())
}

fn parse_current_untracked_paths(output: &str) -> Result<Vec<String>, String> {
    parse_current_paths(output, "untracked path")
}

fn parse_current_unmerged_paths(output: &str) -> Result<Vec<String>, String> {
    parse_current_paths(output, "unmerged path")
}

fn join_current_manifest(
    numstat: Vec<TrackedInventoryEntry>,
    statuses: Vec<(String, CurrentRepoChangeStatus)>,
    unmerged: Vec<String>,
    untracked: Vec<String>,
) -> Result<CurrentChangesManifest, CurrentChangesReadError> {
    let unmerged = unmerged
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut numstat = numstat
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut entries = std::collections::BTreeMap::new();
    let mut remaining_unmerged = unmerged.clone();
    for (path, reported_status) in statuses {
        let status = if unmerged.contains(&path) {
            remaining_unmerged.remove(&path);
            CurrentRepoChangeStatus::Unmerged
        } else {
            reported_status
        };
        let mut entry = match numstat.remove(&path) {
            Some(entry) => entry,
            None if status == CurrentRepoChangeStatus::Unmerged => TrackedInventoryEntry {
                path: path.clone(),
                status,
                binary: false,
                additions: 0,
                deletions: 0,
            },
            None => return Err(CurrentChangesReadError::Drift),
        };
        entry.status = status;
        entries.insert(path, CurrentInventoryEntry::Tracked(entry));
    }
    if !numstat.is_empty() || !remaining_unmerged.is_empty() {
        return Err(CurrentChangesReadError::Drift);
    }
    for path in untracked {
        if entries.contains_key(&path) {
            return Err(CurrentChangesReadError::Drift);
        }
        entries.insert(path.clone(), CurrentInventoryEntry::Untracked { path });
    }
    Ok(CurrentChangesManifest {
        entries: entries.into_values().collect(),
    })
}

fn parse_untracked_patch_output(output: String) -> Result<(String, usize, usize, bool), String> {
    if output.is_empty() {
        return Ok((String::new(), 0, 0, false));
    }
    const DIFF_HEADER: &str = "diff --git a/dev/fd/0 b/dev/fd/0";
    const BINARY_MARKER: &str = "Binary files /dev/null and b/dev/fd/0 differ";
    const REGULAR_MODE: &str = "new file mode 100644";
    const EXECUTABLE_MODE: &str = "new file mode 100755";

    let mut header_lines = Vec::new();
    let mut has_hunk = false;
    for line in output.lines() {
        if line.starts_with("@@") {
            has_hunk = true;
            break;
        }
        header_lines.push(line);
    }
    if header_lines.first().copied() != Some(DIFF_HEADER) {
        return Err("Git returned an unexpected untracked patch header".to_string());
    }
    if !matches!(
        header_lines.get(1).copied(),
        Some(REGULAR_MODE | EXECUTABLE_MODE)
    ) {
        return Err("Git returned an unexpected untracked file mode".to_string());
    }
    let valid_index_line = |line: &str| {
        line.strip_prefix("index ")
            .and_then(|hashes| hashes.split_once(".."))
            .is_some_and(|(old, new)| {
                !old.is_empty()
                    && old.chars().all(|character| character == '0')
                    && !new.is_empty()
                    && new.chars().all(|character| character.is_ascii_hexdigit())
            })
    };
    let exact_binary_markers = header_lines
        .iter()
        .filter(|line| **line == BINARY_MARKER)
        .count();
    let has_other_binary_marker = header_lines
        .iter()
        .any(|line| line.starts_with("Binary files ") && *line != BINARY_MARKER);
    if has_other_binary_marker || exact_binary_markers > 1 {
        return Err("Git returned a malformed untracked binary marker".to_string());
    }
    if exact_binary_markers == 1 {
        if has_hunk
            || header_lines.len() != 4
            || !header_lines
                .get(2)
                .is_some_and(|line| valid_index_line(line))
            || header_lines.get(3).copied() != Some(BINARY_MARKER)
        {
            return Err("Git mixed binary and textual untracked patch data".to_string());
        }
        return Ok((output, 0, 0, true));
    }
    if !has_hunk {
        if header_lines.len() == 2
            || (header_lines.len() == 3
                && header_lines
                    .get(2)
                    .is_some_and(|line| valid_index_line(line)))
        {
            return Ok((output, 0, 0, false));
        }
        return Err("Git returned an unclassified untracked patch".to_string());
    }
    if header_lines.len() != 5
        || !header_lines
            .get(2)
            .is_some_and(|line| valid_index_line(line))
        || header_lines.get(3).copied() != Some("--- /dev/null")
        || header_lines.get(4).copied() != Some("+++ b/dev/fd/0")
    {
        return Err("Git returned malformed textual untracked patch headers".to_string());
    }
    let (additions, deletions) = count_patch_changes(&output);
    if deletions != 0 {
        return Err("Git returned deletions for an untracked file".to_string());
    }
    Ok((output, additions, deletions, false))
}

fn count_patch_changes(patch: &str) -> (usize, usize) {
    let mut in_hunk = false;
    patch.lines().fold((0, 0), |(additions, deletions), line| {
        if line.starts_with("diff --git ") {
            in_hunk = false;
            return (additions, deletions);
        }
        if line.starts_with("@@") {
            in_hunk = true;
            return (additions, deletions);
        }
        if !in_hunk {
            return (additions, deletions);
        }
        if line.starts_with('+') {
            (additions.saturating_add(1), deletions)
        } else if line.starts_with('-') {
            (additions, deletions.saturating_add(1))
        } else {
            (additions, deletions)
        }
    })
}

fn empty_tree_ref(repo_dir: &std::path::Path, auth: &GitAuthConfig) -> Result<String, String> {
    run_git(
        &["hash-object", "-t", "tree", "/dev/null"],
        Some(repo_dir),
        auth,
    )
    .map(|output| output.trim().to_string())
}

fn diff_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_ref: Option<String>,
) -> String {
    if let Some(base_ref) = base_ref {
        return if run_git(&["merge-base", &base_ref, "HEAD"], Some(repo_dir), auth).is_ok() {
            format!("{base_ref}...HEAD")
        } else {
            format!("{base_ref}..HEAD")
        };
    }

    empty_tree_ref(repo_dir, auth)
        .map(|empty_tree| format!("{empty_tree}..HEAD"))
        .unwrap_or_else(|_| "HEAD^..HEAD".to_string())
}

/// Range for a single commit against its parent, used by the commit detail
/// view. Root commits fall back to the empty tree so the whole initial tree
/// renders as additions. Errors when the commit is not reachable in the
/// available history — diffing an unrelated ref instead would be misleading.
fn commit_parent_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    commit: &str,
) -> Result<String, String> {
    run_git(
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{commit}^{{commit}}"),
        ],
        Some(repo_dir),
        auth,
    )
    .map_err(|_| format!("commit {commit} was not found in the repository history"))?;
    let parent = format!("{commit}^");
    if run_git(
        &["rev-parse", "--verify", "--quiet", &parent],
        Some(repo_dir),
        auth,
    )
    .is_ok()
    {
        return Ok(format!("{parent}..{commit}"));
    }
    let empty_tree = empty_tree_ref(repo_dir, auth)?;
    Ok(format!("{empty_tree}..{commit}"))
}

fn local_ref_exists(repo_dir: &std::path::Path, auth: &GitAuthConfig, ref_name: &str) -> bool {
    run_git(
        &["rev-parse", "--verify", "--quiet", ref_name],
        Some(repo_dir),
        auth,
    )
    .is_ok()
}

fn local_target_ref(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch: Option<&str>,
    target_commit: Option<&str>,
) -> String {
    if let Some(target_commit) = target_commit {
        if local_ref_exists(repo_dir, auth, target_commit) {
            return target_commit.to_string();
        }
    }
    if let Some(branch) = branch {
        if local_ref_exists(repo_dir, auth, branch) {
            return branch.to_string();
        }
        let origin_branch = format!("origin/{branch}");
        if local_ref_exists(repo_dir, auth, &origin_branch) {
            return origin_branch;
        }
    }
    "HEAD".to_string()
}

fn local_base_ref(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch: Option<&str>,
    target_branch: Option<&str>,
) -> Option<String> {
    let branch = branch?;
    let origin_branch = format!("origin/{branch}");
    if local_ref_exists(repo_dir, auth, &origin_branch) {
        return Some(origin_branch);
    }
    if target_branch == Some(branch) {
        return None;
    }
    local_ref_exists(repo_dir, auth, branch).then_some(branch.to_string())
}

fn local_diff_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_branch: Option<&str>,
    target_branch: Option<&str>,
    base_commit: Option<&str>,
    target_commit: Option<&str>,
) -> String {
    let target_ref = local_target_ref(repo_dir, auth, target_branch, target_commit);
    if let Some(base_commit) = base_commit {
        if base_commit != target_ref && local_ref_exists(repo_dir, auth, base_commit) {
            return if run_git(
                &["merge-base", base_commit, &target_ref],
                Some(repo_dir),
                auth,
            )
            .is_ok()
            {
                format!("{base_commit}...{target_ref}")
            } else {
                format!("{base_commit}..{target_ref}")
            };
        }
    }
    if let Some(base_ref) = local_base_ref(repo_dir, auth, base_branch, target_branch) {
        return if run_git(
            &["merge-base", &base_ref, &target_ref],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            format!("{base_ref}...{target_ref}")
        } else {
            format!("{base_ref}..{target_ref}")
        };
    }
    // With no base at all, a bare commit means "diff against its parent"
    // (commit detail view) rather than against the whole tree.
    if base_commit.is_none() && base_branch.is_none() {
        if let Some(target_commit) = target_commit {
            if local_ref_exists(repo_dir, auth, target_commit) {
                if let Ok(range) = commit_parent_range(repo_dir, auth, target_commit) {
                    return range;
                }
            }
        }
    }
    empty_tree_ref(repo_dir, auth)
        .map(|empty_tree| format!("{empty_tree}..{target_ref}"))
        .unwrap_or_else(|_| format!("{target_ref}^..{target_ref}"))
}

/// Caps a patch at [`MAX_PATCH_LINES`], reporting whether it was cut.
fn truncate_patch(patch: String) -> (String, bool) {
    let mut retained_end: usize = 0;
    let mut lines = patch.split_inclusive('\n');
    for _ in 0..MAX_PATCH_LINES {
        let Some(line) = lines.next() else {
            break;
        };
        retained_end = retained_end.saturating_add(line.len());
    }
    let line_cut = lines.next().is_some().then_some(retained_end);
    let byte_cut = (patch.len() > MAX_PATCH_BYTES).then(|| {
        let mut cut = MAX_PATCH_BYTES;
        while !patch.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        cut
    });
    let cut = match (line_cut, byte_cut) {
        (Some(line), Some(bytes)) => Some(line.min(bytes)),
        (Some(line), None) => Some(line),
        (None, Some(bytes)) => Some(bytes),
        (None, None) => None,
    };
    match cut {
        Some(cut) => (patch[..cut].to_string(), true),
        None => (patch, false),
    }
}

fn diff_from_repo(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    range: &str,
    target_commit: Option<&str>,
) -> Result<ProjectRepoDiffInfo, String> {
    let commit_body = target_commit
        .map(|commit| {
            run_git(
                &[
                    "show",
                    "--no-patch",
                    "--format=%b",
                    "--end-of-options",
                    commit,
                ],
                Some(repo_dir),
                auth,
            )
            .map(|body| body.trim_end().to_string())
        })
        .transpose()?
        .filter(|body| !body.is_empty());
    let numstat = run_git(&["diff", "--numstat", range], Some(repo_dir), auth)?;
    let files = parse_numstat(&numstat)
        .into_iter()
        .map(|(path, additions, deletions)| {
            let patch = run_git(
                &[
                    "diff",
                    "--no-ext-diff",
                    "--find-renames",
                    "--find-copies",
                    "--unified=80",
                    "--src-prefix=a/",
                    "--dst-prefix=b/",
                    range,
                    "--",
                    &path,
                ],
                Some(repo_dir),
                auth,
            )
            .unwrap_or_default();
            let (patch, truncated) = truncate_patch(patch);
            ProjectRepoDiffFileInfo {
                path,
                additions,
                deletions,
                patch,
                truncated,
            }
        })
        .collect::<Vec<_>>();
    Ok(ProjectRepoDiffInfo {
        additions: files.iter().map(|file| file.additions).sum(),
        deletions: files.iter().map(|file| file.deletions).sum(),
        commit_body,
        files,
    })
}

/// Read the current tracked, staged, and untracked changes in one already
/// validated execution root relative to its immutable persisted base commit.
#[cfg(test)]
pub(crate) fn current_changes_from_repo_after_pin(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_ref: &str,
    repository_identity: &str,
    after_pin: impl FnOnce() -> Result<(), String>,
) -> Result<CurrentRepoDiffInfo, String> {
    let expected_execution_root = repo_dir
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize the test execution root: {error}"))?;
    let pinned = PinnedGitDirectory::pin(&expected_execution_root)?;
    after_pin()?;
    current_changes_from_pinned_repo(
        &pinned,
        auth,
        &expected_execution_root.to_string_lossy(),
        repository_identity,
        base_ref,
    )
}

pub(crate) fn current_changes_from_pinned_repo(
    pinned: &PinnedGitDirectory,
    auth: &GitAuthConfig,
    expected_execution_root: &str,
    expected_repository_identity: &str,
    base_ref: &str,
) -> Result<CurrentRepoDiffInfo, String> {
    current_changes_from_pinned_repo_with_hook(
        pinned,
        auth,
        expected_execution_root,
        expected_repository_identity,
        base_ref,
        MAX_DIFF_FILES,
        None,
    )
}

#[cfg(test)]
pub(crate) fn current_changes_from_repo_with_limit(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_ref: &str,
    repository_identity: &str,
    file_limit: usize,
) -> Result<CurrentRepoDiffInfo, String> {
    let expected_execution_root = repo_dir
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize the test execution root: {error}"))?;
    let pinned = PinnedGitDirectory::pin(&expected_execution_root)?;
    current_changes_from_pinned_repo_with_hook(
        &pinned,
        auth,
        &expected_execution_root.to_string_lossy(),
        repository_identity,
        base_ref,
        file_limit,
        None,
    )
}

#[cfg(test)]
pub(crate) fn current_changes_from_repo_after_untracked_list(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_ref: &str,
    repository_identity: &str,
    mut hook: impl FnMut() -> Result<(), String>,
) -> Result<CurrentRepoDiffInfo, String> {
    let expected_execution_root = repo_dir
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize the test execution root: {error}"))?;
    let pinned = PinnedGitDirectory::pin(&expected_execution_root)?;
    current_changes_from_pinned_repo_with_hook(
        &pinned,
        auth,
        &expected_execution_root.to_string_lossy(),
        repository_identity,
        base_ref,
        MAX_DIFF_FILES,
        Some(&mut hook),
    )
}

fn current_changes_from_pinned_repo_with_hook(
    pinned: &PinnedGitDirectory,
    auth: &GitAuthConfig,
    expected_execution_root: &str,
    expected_repository_identity: &str,
    base_ref: &str,
    file_limit: usize,
    mut before_untracked_patches: Option<&mut dyn FnMut() -> Result<(), String>>,
) -> Result<CurrentRepoDiffInfo, String> {
    for attempt in 0..=1 {
        let result = if let Some(hook) = before_untracked_patches.as_mut() {
            current_changes_attempt(
                pinned,
                auth,
                expected_execution_root,
                expected_repository_identity,
                base_ref,
                file_limit,
                Some(&mut **hook),
            )
        } else {
            current_changes_attempt(
                pinned,
                auth,
                expected_execution_root,
                expected_repository_identity,
                base_ref,
                file_limit,
                None,
            )
        };
        match result {
            Ok(changes) => return Ok(changes),
            Err(CurrentChangesReadError::Drift) if attempt == 0 => continue,
            Err(CurrentChangesReadError::Drift) => return Err(CHANGES_DRIFT_ERROR.to_string()),
            Err(CurrentChangesReadError::Fatal(error)) => return Err(error),
        }
    }
    Err(CHANGES_DRIFT_ERROR.to_string())
}

fn read_current_manifest(
    snapshot: &mut GitReadSnapshot<'_>,
    base_ref: &str,
) -> Result<CurrentChangesManifest, CurrentChangesReadError> {
    let numstat = parse_current_numstat_z(&snapshot.tracked_numstat(base_ref)?)?;
    let statuses = parse_current_name_status_z(&snapshot.tracked_name_status(base_ref)?)?;
    let unmerged = parse_current_unmerged_paths(&snapshot.tracked_unmerged_paths()?)?;
    let untracked = parse_current_untracked_paths(&snapshot.untracked_paths()?)?;
    join_current_manifest(numstat, statuses, unmerged, untracked)
}

fn classify_patch_read_error(
    snapshot: &mut GitReadSnapshot<'_>,
    base_ref: &str,
    initial_manifest: &CurrentChangesManifest,
    original_error: String,
) -> CurrentChangesReadError {
    match read_current_manifest(snapshot, base_ref) {
        Ok(final_manifest) if final_manifest != *initial_manifest => CurrentChangesReadError::Drift,
        Err(CurrentChangesReadError::Drift) => CurrentChangesReadError::Drift,
        Ok(_) | Err(CurrentChangesReadError::Fatal(_)) => {
            CurrentChangesReadError::Fatal(original_error)
        }
    }
}

fn current_changes_attempt(
    pinned: &PinnedGitDirectory,
    auth: &GitAuthConfig,
    expected_execution_root: &str,
    expected_repository_identity: &str,
    base_ref: &str,
    file_limit: usize,
    mut before_untracked_patches: Option<&mut dyn FnMut() -> Result<(), String>>,
) -> Result<CurrentRepoDiffInfo, CurrentChangesReadError> {
    let mut snapshot = GitReadSnapshot::new(
        pinned,
        auth,
        expected_execution_root,
        expected_repository_identity,
        base_ref,
    )?;
    let initial_manifest = read_current_manifest(&mut snapshot, base_ref)?;
    if let Some(hook) = before_untracked_patches.as_mut() {
        hook()?;
    }

    let total_files = initial_manifest.entries.len();
    let mut files = Vec::with_capacity(total_files.min(file_limit));
    for entry in initial_manifest.entries.iter().take(file_limit) {
        let file = match entry {
            CurrentInventoryEntry::Tracked(entry) => {
                let patch = match snapshot.tracked_patch(base_ref, &entry.path) {
                    Ok(patch) => patch,
                    Err(error) => {
                        return Err(classify_patch_read_error(
                            &mut snapshot,
                            base_ref,
                            &initial_manifest,
                            error,
                        ));
                    }
                };
                if !entry.binary && entry.status != CurrentRepoChangeStatus::Unmerged {
                    let observed = count_patch_changes(&patch);
                    if observed != (entry.additions, entry.deletions) {
                        return Err(CurrentChangesReadError::Drift);
                    }
                }
                let (patch, truncated) = truncate_patch(patch);
                CurrentRepoDiffFileInfo {
                    path: entry.path.clone(),
                    status: entry.status,
                    binary: entry.binary,
                    additions: entry.additions,
                    deletions: entry.deletions,
                    patch,
                    truncated,
                }
            }
            CurrentInventoryEntry::Untracked { path } => {
                let output = match snapshot.untracked_patch(path) {
                    Ok(output) => output,
                    Err(error) => {
                        return Err(classify_patch_read_error(
                            &mut snapshot,
                            base_ref,
                            &initial_manifest,
                            error,
                        ));
                    }
                };
                let (patch, additions, deletions, binary) = parse_untracked_patch_output(output)?;
                let (patch, truncated) = truncate_patch(patch);
                CurrentRepoDiffFileInfo {
                    path: path.clone(),
                    status: CurrentRepoChangeStatus::Untracked,
                    binary,
                    additions,
                    deletions,
                    patch,
                    truncated,
                }
            }
        };
        files.push(file);
    }

    let final_manifest = read_current_manifest(&mut snapshot, base_ref)?;
    if final_manifest != initial_manifest {
        return Err(CurrentChangesReadError::Drift);
    }

    Ok(CurrentRepoDiffInfo {
        total_files,
        files_truncated: total_files > files.len(),
        additions: files
            .iter()
            .fold(0usize, |total, file| total.saturating_add(file.additions)),
        deletions: files
            .iter()
            .fold(0usize, |total, file| total.saturating_add(file.deletions)),
        files,
    })
}

#[tauri::command]
pub async fn get_project_repo_diff(
    clone_url: String,
    default_branch: Option<String>,
    base_branch: Option<String>,
    target_ref: Option<String>,
    target_commit: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectRepoDiffInfo, String> {
    validate_workspace_clone_url(&clone_url, &state)?;
    let auth = build_git_auth_config(&state)?;
    let branch = clean_branch(default_branch);
    let base_branch = clean_branch(base_branch);
    let target_ref = clean_target_ref(target_ref);
    let target_commit = clean_commit(target_commit);

    tauri::async_runtime::spawn_blocking(move || {
        let temp_dir = tempfile::tempdir().map_err(|error| format!("create temp dir: {error}"))?;
        let repo_dir = temp_dir.path().join("repo");
        let repo_path = repo_dir
            .to_str()
            .ok_or_else(|| "temporary repository path is not UTF-8".to_string())?;
        run_git(
            &[
                "clone",
                "--filter=blob:none",
                "--no-checkout",
                &clone_url,
                repo_path,
            ],
            None,
            &auth,
        )?;
        fetch_target(
            &repo_dir,
            &auth,
            branch.as_deref(),
            target_ref.as_deref(),
            target_commit.as_deref(),
        )?;
        // A commit with no base branch or target ref means "diff this commit
        // against its parent" (commit detail view), not "diff HEAD against a
        // base".
        let range = match (&target_ref, &base_branch, &target_commit) {
            (None, None, Some(commit)) => commit_parent_range(&repo_dir, &auth, commit)?,
            _ => diff_range(
                &repo_dir,
                &auth,
                diff_base_ref(&repo_dir, &auth, base_branch.as_deref()),
            ),
        };
        let commit_body_ref = if target_ref.is_none() && base_branch.is_none() {
            target_commit.as_deref()
        } else {
            None
        };
        diff_from_repo(&repo_dir, &auth, &range, commit_body_ref)
    })
    .await
    .map_err(|error| format!("repo diff task failed: {error}"))?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_project_local_repo_diff(
    repos_dir: Option<String>,
    project_dtag: String,
    clone_url: Option<String>,
    default_branch: Option<String>,
    base_branch: Option<String>,
    base_commit: Option<String>,
    target_commit: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<ProjectRepoDiffInfo>, String> {
    let auth = build_git_auth_config(&state)?;
    let branch = clean_branch(default_branch);
    let base_branch = clean_branch(base_branch);
    let base_commit = clean_commit(base_commit);
    let target_commit = clean_commit(target_commit);

    tauri::async_runtime::spawn_blocking(move || {
        let Some(repo_dir) =
            find_local_repo_dir(repos_dir.as_deref(), &project_dtag, clone_url.as_deref())?
        else {
            return Ok(None);
        };
        let range = local_diff_range(
            &repo_dir,
            &auth,
            base_branch.as_deref(),
            branch.as_deref(),
            base_commit.as_deref(),
            target_commit.as_deref(),
        );
        let commit_body_ref = if base_commit.is_none() && base_branch.is_none() {
            target_commit.as_deref()
        } else {
            None
        };
        diff_from_repo(&repo_dir, &auth, &range, commit_body_ref).map(Some)
    })
    .await
    .map_err(|error| format!("local repo diff task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::{
        count_patch_changes, join_current_manifest, parse_current_name_status_z,
        parse_current_numstat_z, parse_current_unmerged_paths, parse_current_untracked_paths,
        parse_strict_numstat_counts, parse_untracked_patch_output, truncate_patch,
        CurrentChangesReadError, CurrentInventoryEntry, CurrentRepoChangeStatus, MAX_PATCH_BYTES,
        MAX_PATCH_LINES,
    };

    #[test]
    fn patch_counts_header_shaped_content_inside_hunks() {
        let patch = concat!(
            "diff --git a/file b/file\n",
            "--- a/file\n",
            "+++ b/file\n",
            "@@ -1,2 +1,3 @@\n",
            "--- removed content\n",
            "+++ added content\n",
            "+normal addition\n",
        );

        assert_eq!(count_patch_changes(patch), (2, 1));
    }

    #[test]
    fn patch_counts_each_type_change_section_without_counting_its_headers() {
        let patch = concat!(
            "diff --git a/file b/file\n",
            "deleted file mode 100644\n",
            "--- a/file\n",
            "+++ /dev/null\n",
            "@@ -1 +0,0 @@\n",
            "-regular\n",
            "diff --git a/file b/file\n",
            "new file mode 120000\n",
            "--- /dev/null\n",
            "+++ b/file\n",
            "@@ -0,0 +1 @@\n",
            "+target\n",
        );

        assert_eq!(count_patch_changes(patch), (1, 1));
    }

    #[test]
    fn current_numstat_is_strict_and_preserves_binary_meaning() {
        assert_eq!(parse_strict_numstat_counts("-", "-"), Ok((0, 0, true)));
        assert_eq!(parse_strict_numstat_counts("12", "3"), Ok((12, 3, false)));
        assert!(parse_strict_numstat_counts("-", "3").is_err());
        assert!(parse_strict_numstat_counts("12", "-").is_err());
        assert!(parse_strict_numstat_counts("twelve", "3").is_err());

        let entries = parse_current_numstat_z(concat!("-\t-\tbinary.dat\0", "0\t0\tmode-only\0",))
            .expect("strict numstat should parse valid binary and zero-count entries");
        assert_eq!(entries.len(), 2);
        assert!(entries[0].binary);
        assert_eq!((entries[0].additions, entries[0].deletions), (0, 0));
        assert!(!entries[1].binary);
        assert!(parse_current_numstat_z("1\t0\tunterminated").is_err());
        assert!(parse_current_numstat_z("1\t0\0").is_err());
        assert!(parse_current_numstat_z(concat!("1\t0\tdup\0", "1\t0\tdup\0")).is_err());
        assert_eq!(
            parse_current_untracked_paths("replacement-\u{fffd}.txt\0")
                .expect("valid UTF-8 replacement characters are legal path content"),
            vec!["replacement-\u{fffd}.txt".to_string()]
        );
    }

    #[test]
    fn current_name_status_is_closed_and_manifest_is_complete_sorted() {
        let statuses = parse_current_name_status_z(concat!(
            "M\0zeta.txt\0",
            "A\0alpha.txt\0",
            "D\0deleted.txt\0",
            "T\0typed.txt\0",
            "M\0conflict.txt\0",
        ))
        .expect("supported statuses should parse");
        assert_eq!(statuses[0].1, CurrentRepoChangeStatus::Modified);
        assert_eq!(statuses[1].1, CurrentRepoChangeStatus::Added);
        assert_eq!(statuses[2].1, CurrentRepoChangeStatus::Deleted);
        assert_eq!(statuses[3].1, CurrentRepoChangeStatus::TypeChanged);
        assert_eq!(statuses[4].1, CurrentRepoChangeStatus::Modified);
        assert_eq!(
            parse_current_name_status_z("U\0unmerged.txt\0")
                .expect("explicit unmerged status should remain supported")[0]
                .1,
            CurrentRepoChangeStatus::Unmerged
        );
        assert!(parse_current_name_status_z("R100\0old\0new\0").is_err());
        assert!(parse_current_name_status_z("M\0missing-terminator").is_err());

        let numstat = parse_current_numstat_z(concat!(
            "1\t0\tzeta.txt\0",
            "2\t0\talpha.txt\0",
            "0\t1\tdeleted.txt\0",
            "0\t0\ttyped.txt\0",
        ))
        .expect("tracked statistics should parse");
        let untracked = parse_current_untracked_paths("middle.txt\0beta.txt\0")
            .expect("untracked inventory should parse");
        let unmerged = parse_current_unmerged_paths("conflict.txt\0")
            .expect("unmerged inventory should parse");
        let manifest = join_current_manifest(numstat, statuses, unmerged, untracked)
            .expect("matching full inventories should join");
        let paths = manifest
            .entries
            .iter()
            .map(|entry| match entry {
                CurrentInventoryEntry::Tracked(entry) => entry.path.as_str(),
                CurrentInventoryEntry::Untracked { path } => path.as_str(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "alpha.txt",
                "beta.txt",
                "conflict.txt",
                "deleted.txt",
                "middle.txt",
                "typed.txt",
                "zeta.txt",
            ]
        );
        assert!(manifest.entries.iter().any(|entry| matches!(
            entry,
            CurrentInventoryEntry::Tracked(entry)
                if entry.path == "conflict.txt"
                    && entry.status == CurrentRepoChangeStatus::Unmerged
        )));
    }

    #[test]
    fn current_manifest_mismatches_fail_as_snapshot_drift() {
        let numstat =
            parse_current_numstat_z("1\t0\tonly-stat.txt\0").expect("test statistics should parse");
        assert!(matches!(
            join_current_manifest(numstat, Vec::new(), Vec::new(), Vec::new()),
            Err(CurrentChangesReadError::Drift)
        ));

        let statuses =
            parse_current_name_status_z("M\0only-status.txt\0").expect("test status should parse");
        assert!(matches!(
            join_current_manifest(Vec::new(), statuses, Vec::new(), Vec::new()),
            Err(CurrentChangesReadError::Drift)
        ));
    }

    #[test]
    fn untracked_patch_classification_is_strict_and_ignores_hunk_marker_text() {
        let patch = concat!(
            "diff --git a/dev/fd/0 b/dev/fd/0\n",
            "new file mode 100644\n",
            "index 0000000..1111111\n",
            "--- /dev/null\n",
            "+++ b/dev/fd/0\n",
            "@@ -0,0 +1 @@\n",
            "+text\n",
        );
        assert_eq!(
            parse_untracked_patch_output(patch.to_string()),
            Ok((patch.to_string(), 1, 0, false))
        );
        let marker_in_hunk =
            patch.replace("+text", "+Binary files /dev/null and b/dev/fd/0 differ");
        assert!(parse_untracked_patch_output(marker_in_hunk).is_ok_and(
            |(_, additions, deletions, binary)| additions == 1 && deletions == 0 && !binary
        ));

        let binary_patch = concat!(
            "diff --git a/dev/fd/0 b/dev/fd/0\n",
            "new file mode 100644\n",
            "index 0000000..1111111\n",
            "Binary files /dev/null and b/dev/fd/0 differ\n",
        );
        assert_eq!(
            parse_untracked_patch_output(binary_patch.to_string()),
            Ok((binary_patch.to_string(), 0, 0, true))
        );
        let empty_patch = concat!(
            "diff --git a/dev/fd/0 b/dev/fd/0\n",
            "new file mode 100644\n",
        );
        assert_eq!(
            parse_untracked_patch_output(empty_patch.to_string()),
            Ok((empty_patch.to_string(), 0, 0, false))
        );
        let empty_patch_with_index = concat!(
            "diff --git a/dev/fd/0 b/dev/fd/0\n",
            "new file mode 100644\n",
            "index 000000000..e69de29bb\n",
        );
        assert_eq!(
            parse_untracked_patch_output(empty_patch_with_index.to_string()),
            Ok((empty_patch_with_index.to_string(), 0, 0, false))
        );
        assert!(parse_untracked_patch_output(
            binary_patch.replace("b/dev/fd/0 differ", "b/unexpected differ")
        )
        .is_err());
        assert!(
            parse_untracked_patch_output(format!("{binary_patch}@@ -0,0 +1 @@\n+text\n")).is_err()
        );
    }

    #[test]
    fn patch_truncation_respects_exact_line_and_utf8_byte_boundaries() {
        let exact = "line\n".repeat(MAX_PATCH_LINES);
        assert_eq!(truncate_patch(exact.clone()), (exact, false));

        let over = "line\n".repeat(MAX_PATCH_LINES + 1);
        let (retained, truncated) = truncate_patch(over);
        assert!(truncated);
        assert_eq!(retained.lines().count(), MAX_PATCH_LINES);

        let multibyte = "€".repeat((MAX_PATCH_BYTES / '€'.len_utf8()) + 1);
        let (retained, truncated) = truncate_patch(multibyte);
        assert!(truncated);
        assert!(retained.len() <= MAX_PATCH_BYTES);
        assert!(retained.chars().all(|character| character == '€'));
    }
}
