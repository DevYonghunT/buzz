//! Restartable cleanup for an exact journaled standard-lock/artifact pair.

use super::*;

impl PinnedAdminDirectory {
    /// Remove only the exact owned guard pair, accepting a fully absent retry.
    pub(crate) fn cleanup_standard_lock(
        &self,
        owned: &OwnedStandardLock,
    ) -> Result<CleanupDisposition, String> {
        validate_owned_lock(owned)?;
        self.revalidate()?;
        let artifact_name = component_cstr(&owned.artifact_component)?;
        let lock_name = literal_component(owned.kind.lock_component())?;
        let artifact = match self.read_owned_file(artifact_name.as_c_str()) {
            Ok(artifact) => artifact,
            Err(error) if error.starts_with("absent owned Git file:") => {
                return self.cleanup_lock_without_artifact(lock_name, owned);
            }
            Err(error) => return Err(error),
        };
        if !same_stable_file(&artifact, &owned.identity) {
            return Err("owned Git artifact was replaced before convergent cleanup".to_string());
        }
        match self.classify_named_lock(lock_name, &artifact)? {
            NamedLockState::Exact => {
                let current = self.read_owned_pair(
                    artifact_name.as_c_str(),
                    lock_name,
                    owned.kind,
                    &owned.identity,
                )?;
                let artifact = self.release_standard_lock(&current)?;
                self.remove_artifact(&artifact)?;
                Ok(CleanupDisposition::Removed)
            }
            NamedLockState::Absent if artifact.link_count == 1 => {
                self.remove_artifact(&OwnedArtifact {
                    component: owned.artifact_component.clone(),
                    identity: artifact,
                })?;
                Ok(CleanupDisposition::Removed)
            }
            NamedLockState::Absent => {
                Err("owned guard artifact has an unexplained live link".to_string())
            }
            NamedLockState::Foreign => {
                Err("foreign standard Git lock was preserved during cleanup".to_string())
            }
        }
    }

    fn cleanup_lock_without_artifact(
        &self,
        lock_name: &CStr,
        owned: &OwnedStandardLock,
    ) -> Result<CleanupDisposition, String> {
        let lock = match self.read_owned_file(lock_name) {
            Ok(lock) => lock,
            Err(error) if error.starts_with("absent owned Git file:") => {
                rustix::fs::fsync(&self.handle).map_err(|sync_error| {
                    format!("failed to sync absent owned guard cleanup: {sync_error}")
                })?;
                self.revalidate()?;
                return Ok(CleanupDisposition::AlreadyAbsent);
            }
            Err(error) => return Err(error),
        };
        if !same_stable_file(&lock, &owned.identity) || lock.link_count != 1 {
            return Err("orphaned standard Git lock is foreign or ambiguously linked".to_string());
        }
        rustix::fs::unlinkat(&self.handle, lock_name, AtFlags::empty())
            .map_err(|error| format!("failed to remove exact orphaned Git lock: {error}"))?;
        rustix::fs::fsync(&self.handle)
            .map_err(|error| format!("failed to sync orphaned Git lock cleanup: {error}"))?;
        self.revalidate()?;
        ensure_absent(&self.handle, lock_name)?;
        Ok(CleanupDisposition::Removed)
    }
}
