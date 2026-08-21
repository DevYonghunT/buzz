use std::fs;
use std::path::{Path, PathBuf};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::fs::MetadataExt as _;

use super::identity::{pin_regular_file, verify_regular_file, FileIdentity};

const MAX_GIT_EXECUTABLE_BYTES: usize = 128 * 1024 * 1024;
const SYSTEM_GIT: &str = "/usr/bin/git";

/// Opaque authority for the root-trusted Git executable selected by the
/// SchoolX Code native boundary.
///
/// The executable path is intentionally not caller-selectable. Every launch
/// must revalidate the same pinned identity immediately before spawning.
#[derive(Clone, Debug)]
#[cfg(target_os = "linux")]
pub(in crate::code_workspace) struct RootTrustedGit {
    identity: FileIdentity,
}

#[cfg(target_os = "linux")]
impl RootTrustedGit {
    /// Pin the platform Git executable using the existing root-owned,
    /// non-writable namespace and exact-file-identity policy.
    pub(in crate::code_workspace) fn pin() -> Result<Self, String> {
        pin_git_executable().map(|identity| Self { identity })
    }

    /// Reuse exact Git evidence already selected for a typed write journal.
    pub(super) fn from_identity(identity: FileIdentity) -> Result<Self, String> {
        verify_git_executable(&identity)?;
        Ok(Self { identity })
    }

    /// Revalidate the root-controlled namespace and exact executable bytes.
    pub(in crate::code_workspace) fn revalidate(&self) -> Result<(), String> {
        verify_git_executable(&self.identity)
    }

    /// Return the already validated absolute executable path.
    pub(in crate::code_workspace) fn path(&self) -> &Path {
        Path::new(&self.identity.path)
    }
}

/// Pin a Git executable whose namespace and bytes cannot be changed by the
/// unprivileged desktop user. Package-manager and root activity is explicitly
/// part of the trusted computing base for this boundary.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn pin_git_executable() -> Result<FileIdentity, String> {
    reject_privileged_desktop()?;
    let candidates = git_candidates();
    let mut failures = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        match pin_candidate(&candidate) {
            Ok(identity) => return Ok(identity),
            Err(error) => failures.push(format!("{}: {error}", candidate.display())),
        }
    }
    Err(format!(
        "no root-trusted Git executable was available ({})",
        failures.join("; ")
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn pin_git_executable() -> Result<FileIdentity, String> {
    Err("SchoolX Code Git writes support only macOS and Linux".to_string())
}

/// Revalidate both the exact executable identity and the root-controlled
/// namespace immediately before a typed Git exec.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn verify_git_executable(identity: &FileIdentity) -> Result<(), String> {
    reject_privileged_desktop()?;
    verify_trusted_canonical_path(Path::new(&identity.path))?;
    verify_regular_file(identity, MAX_GIT_EXECUTABLE_BYTES)
        .map_err(|error| format!("pinned Git executable identity changed: {error}"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn verify_git_executable(_identity: &FileIdentity) -> Result<(), String> {
    Err("SchoolX Code Git writes support only macOS and Linux".to_string())
}

#[cfg(target_os = "macos")]
fn git_candidates() -> Vec<PathBuf> {
    // /usr/bin/git is the Apple-controlled xcode-select shim. The helper's
    // cleared environment prevents DEVELOPER_DIR from redirecting it.
    vec![PathBuf::from(SYSTEM_GIT)]
}

#[cfg(target_os = "linux")]
fn git_candidates() -> Vec<PathBuf> {
    let system = PathBuf::from(SYSTEM_GIT);
    let mut candidates = vec![system.clone()];
    if let Some(resolved) = crate::managed_agents::resolve_command("git") {
        if resolved != system {
            candidates.push(resolved);
        }
    }
    candidates
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn pin_candidate(candidate: &Path) -> Result<FileIdentity, String> {
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize Git executable: {error}"))?;
    verify_trusted_canonical_path(&canonical)?;
    let identity = pin_regular_file(&canonical, MAX_GIT_EXECUTABLE_BYTES)
        .map_err(|error| format!("failed to pin Git executable: {error}"))?;
    verify_git_executable(&identity)?;
    Ok(identity)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reject_privileged_desktop() -> Result<(), String> {
    if rustix::process::geteuid().as_raw() == 0 {
        return Err("SchoolX Code Git writes are unavailable when the desktop runs as root".into());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_trusted_canonical_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Git executable trust requires an absolute path".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to re-canonicalize Git executable: {error}"))?;
    if canonical != path {
        return Err("Git executable evidence is not a canonical path".to_string());
    }

    let executable = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect Git executable trust: {error}"))?;
    verify_trusted_component(path, &executable, false)?;
    if !executable.is_file() || executable.mode() & 0o111 == 0 {
        return Err("root-trusted Git executable is not an executable regular file".to_string());
    }
    if executable.mode() & 0o6000 != 0 {
        return Err("root-trusted Git executable has set-id permission bits".to_string());
    }

    let parent = path
        .parent()
        .ok_or_else(|| "Git executable has no parent directory".to_string())?;
    for ancestor in parent.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|error| {
            format!(
                "failed to inspect Git executable ancestor {}: {error}",
                ancestor.display()
            )
        })?;
        verify_trusted_component(ancestor, &metadata, true)?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_trusted_component(
    path: &Path,
    metadata: &fs::Metadata,
    directory: bool,
) -> Result<(), String> {
    let expected_type = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    if metadata.file_type().is_symlink() || !expected_type {
        return Err(format!(
            "Git executable trust component {} has an unsafe type",
            path.display()
        ));
    }
    if metadata.uid() != 0 {
        return Err(format!(
            "Git executable trust component {} is not root-owned",
            path.display()
        ));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "Git executable trust component {} is group- or other-writable",
            path.display()
        ));
    }
    match rustix::fs::accessat(
        rustix::fs::CWD,
        path,
        rustix::fs::Access::WRITE_OK,
        rustix::fs::AtFlags::EACCESS,
    ) {
        Ok(()) => Err(format!(
            "Git executable trust component {} is writable by the desktop user",
            path.display()
        )),
        Err(rustix::io::Errno::ACCESS | rustix::io::Errno::PERM | rustix::io::Errno::ROFS) => {
            Ok(())
        }
        Err(error) => Err(format!(
            "failed to prove Git executable trust component {} non-writable: {error}",
            path.display()
        )),
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn platform_git_is_canonical_root_trusted_and_revalidates() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() == 0 {
            assert!(pin_git_executable().is_err_and(|error| error.contains("runs as root")));
            return Ok(());
        }
        let identity = pin_git_executable()?;
        assert_eq!(
            Path::new(&identity.path),
            Path::new(&identity.path)
                .canonicalize()
                .map_err(|error| error.to_string())?
        );
        assert_eq!(identity.owner, 0);
        verify_git_executable(&identity)
    }

    #[test]
    fn candidate_rejects_user_controlled_or_writable_namespace() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Ok(());
        }
        let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
        let candidate = sandbox.path().join("git");
        fs::write(&candidate, b"not trusted\n").map_err(|error| error.to_string())?;
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
        let error = pin_candidate(&candidate)
            .expect_err("a user-controlled Git executable unexpectedly passed trust checks");
        assert!(
            error.contains("not root-owned")
                || error.contains("group- or other-writable")
                || error.contains("writable by the desktop user")
        );
        Ok(())
    }

    #[test]
    fn executable_identity_rejects_stable_path_replacement() -> Result<(), String> {
        let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
        let candidate = sandbox.path().join("git");
        let replacement = sandbox.path().join("replacement");
        fs::write(&candidate, b"first executable bytes\n").map_err(|error| error.to_string())?;
        fs::write(&replacement, b"second executable bytes\n").map_err(|error| error.to_string())?;
        let identity = pin_regular_file(&candidate, 1024)?;
        fs::rename(&replacement, &candidate).map_err(|error| error.to_string())?;
        let error = verify_regular_file(&identity, 1024)
            .expect_err("a replacement at the pinned executable path was accepted");
        assert!(error.contains("identity changed"));
        Ok(())
    }

    #[test]
    fn effective_write_probe_rejects_acl_or_mode_access() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Ok(());
        }
        let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
        let error = match rustix::fs::accessat(
            rustix::fs::CWD,
            sandbox.path(),
            rustix::fs::Access::WRITE_OK,
            rustix::fs::AtFlags::EACCESS,
        ) {
            Ok(()) => "writable by the desktop user".to_string(),
            Err(error) => {
                return Err(format!(
                    "test directory was unexpectedly read-only: {error}"
                ))
            }
        };
        assert!(error.contains("writable"));
        Ok(())
    }
}
