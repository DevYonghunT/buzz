use super::{
    CurrentChangesManifest, CurrentChangesReadError, CurrentInventoryEntry,
    CurrentRepoChangeStatus, TrackedInventoryEntry, MAX_DIFF_FILES,
};

fn parse_count(value: &str) -> usize {
    value.parse::<usize>().unwrap_or_default()
}

pub(super) fn parse_numstat(output: &str) -> Vec<(String, usize, usize)> {
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

pub(super) fn parse_strict_numstat_counts(
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

pub(super) fn parse_current_numstat_z(output: &str) -> Result<Vec<TrackedInventoryEntry>, String> {
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

pub(super) fn parse_current_name_status_z(
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

pub(super) fn parse_current_untracked_paths(output: &str) -> Result<Vec<String>, String> {
    parse_current_paths(output, "untracked path")
}

pub(super) fn parse_current_unmerged_paths(output: &str) -> Result<Vec<String>, String> {
    parse_current_paths(output, "unmerged path")
}

pub(super) fn join_current_manifest(
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

pub(super) fn parse_untracked_patch_output(
    output: String,
) -> Result<(String, usize, usize, bool), String> {
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

pub(super) fn count_patch_changes(patch: &str) -> (usize, usize) {
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
