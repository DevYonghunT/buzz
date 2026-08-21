//! Crash-safe adoption of a durable planned artifact component.

use super::*;

impl PinnedAdminDirectory {
    /// Create a planned artifact or adopt the exact file left by a lost response.
    #[cfg(unix)]
    pub(crate) fn recover_or_create_artifact(
        &self,
        issued: &IssuedArtifactComponent,
        bytes: &[u8],
    ) -> Result<OwnedArtifact, String> {
        if bytes.len() > MAX_OWNED_FILE_BYTES {
            return Err(format!(
                "owned Git artifact exceeds the {MAX_OWNED_FILE_BYTES}-byte limit"
            ));
        }
        validate_artifact_component(issued.component())?;
        match self.create_artifact(issued, bytes) {
            Ok(artifact) => Ok(artifact),
            Err(create_error) => self
                .adopt_planned_artifact(issued, bytes)
                .map_err(|adopt_error| {
                    format!(
                        "failed to create planned Git artifact: {create_error}; existing entry was not adoptable: {adopt_error}"
                    )
                }),
        }
    }

    #[cfg(unix)]
    fn adopt_planned_artifact(
        &self,
        issued: &IssuedArtifactComponent,
        bytes: &[u8],
    ) -> Result<OwnedArtifact, String> {
        use std::ffi::CString;
        use std::fs;

        use rustix::fs::{Mode, OFlags};
        use sha2::{Digest, Sha256};

        self.revalidate()?;
        let name = CString::new(issued.component())
            .map_err(|_| "planned Git artifact component contains NUL".to_string())?;
        let expected_digest = hex::encode(Sha256::digest(bytes));
        let before = self.read_owned_file(name.as_c_str())?;
        if before.link_count != 1
            || before.size != bytes.len() as u64
            || before.sha256 != expected_digest
        {
            return Err(
                "planned Git artifact does not match owner/mode/link/size/digest evidence"
                    .to_string(),
            );
        }
        let descriptor = rustix::fs::openat(
            &self.handle,
            name.as_c_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|error| format!("failed to reopen planned Git artifact: {error}"))?;
        fs::File::from(descriptor)
            .sync_all()
            .map_err(|error| format!("failed to sync recovered Git artifact: {error}"))?;
        rustix::fs::fsync(&self.handle)
            .map_err(|error| format!("failed to sync recovered Git artifact parent: {error}"))?;
        self.revalidate()?;
        let after = self.read_owned_file(name.as_c_str())?;
        if after != before {
            return Err("planned Git artifact changed while it was adopted".to_string());
        }
        Ok(OwnedArtifact {
            component: issued.component().to_string(),
            identity: after,
        })
    }

    #[cfg(not(unix))]
    pub(crate) fn recover_or_create_artifact(
        &self,
        _issued: &IssuedArtifactComponent,
        _bytes: &[u8],
    ) -> Result<OwnedArtifact, String> {
        Err("proven-owned Git locks are unsupported on this platform".to_string())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn recovery_adopts_only_exact_planned_bytes() -> Result<(), String> {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let admin = root.path().join("admin");
        fs::create_dir(&admin).map_err(|error| error.to_string())?;
        let admin = admin.canonicalize().map_err(|error| error.to_string())?;
        let pinned = PinnedAdminDirectory::pin(&admin)?;

        let exact_name = pinned.issue_artifact_component()?;
        let created = pinned.create_artifact(&exact_name, b"exact\n")?;
        let adopted = pinned.recover_or_create_artifact(&exact_name, b"exact\n")?;
        assert_eq!(adopted, created);

        let foreign_name = pinned.issue_artifact_component()?;
        let foreign_path = admin.join(foreign_name.component());
        fs::write(&foreign_path, b"foreign\n").map_err(|error| error.to_string())?;
        fs::set_permissions(&foreign_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
        assert!(pinned
            .recover_or_create_artifact(&foreign_name, b"expected\n")
            .is_err());
        assert_eq!(
            fs::read(&foreign_path).map_err(|error| error.to_string())?,
            b"foreign\n"
        );
        Ok(())
    }
}
