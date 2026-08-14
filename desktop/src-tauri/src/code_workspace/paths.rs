use std::path::Path;

/// Resolve the execution root before it crosses the app-server boundary.
///
/// A thread is bound to this canonical directory. Resolving symlinks here
/// prevents the frontend from presenting one path while granting Codex access
/// to a different target.
pub(crate) fn canonical_workspace_root(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("SchoolX Code workspace root cannot be empty".to_string());
    }
    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return Err("SchoolX Code workspace root must be an absolute path".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to resolve SchoolX Code workspace root: {error}"))?;
    if !canonical.is_dir() {
        return Err("SchoolX Code workspace root must be a directory".to_string());
    }
    Ok(canonical.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_an_existing_workspace() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let resolved = canonical_workspace_root(&directory.path().to_string_lossy())?;
        let expected = directory
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        assert_eq!(resolved, expected.to_string_lossy());
        Ok(())
    }

    #[test]
    fn rejects_relative_and_missing_workspaces() {
        assert!(canonical_workspace_root("relative/path").is_err());
        assert!(canonical_workspace_root("/schoolx/definitely/missing").is_err());
    }
}
