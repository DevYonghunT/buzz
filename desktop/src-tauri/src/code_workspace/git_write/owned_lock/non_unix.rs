use super::*;

const UNSUPPORTED: &str = "proven-owned Git locks are unsupported on this platform";

impl PinnedAdminDirectory {
    pub(crate) fn pin(_path: &Path) -> Result<Self, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn identity(&self) -> &OwnedDirectoryIdentity {
        &self.identity
    }

    pub(crate) fn revalidate(&self) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn issue_artifact_component(&self) -> Result<IssuedArtifactComponent, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn create_artifact(
        &self,
        _issued: &IssuedArtifactComponent,
        _bytes: &[u8],
    ) -> Result<OwnedArtifact, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn revalidate_artifact(&self, _artifact: &OwnedArtifact) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn acquire_standard_lock(
        &self,
        _artifact: &OwnedArtifact,
        _kind: StandardLockKind,
    ) -> Result<LockAcquisition, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn revalidate_standard_lock(
        &self,
        _owned: &OwnedStandardLock,
    ) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn classify_owned_lock(
        &self,
        _owned: &OwnedStandardLock,
    ) -> Result<OwnedLockPlacement, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn publish_after_cas<F>(
        &self,
        _owned: &OwnedStandardLock,
        _prove_preimage: F,
    ) -> Result<OwnedPublishedArtifact, PublishFailure>
    where
        F: FnOnce() -> Result<(), String>,
    {
        Err(PublishFailure::BeforeRename(UNSUPPORTED.to_string()))
    }

    pub(crate) fn release_standard_lock(
        &self,
        _owned: &OwnedStandardLock,
    ) -> Result<OwnedArtifact, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn cleanup_standard_lock(
        &self,
        _owned: &OwnedStandardLock,
    ) -> Result<CleanupDisposition, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn remove_artifact(
        &self,
        _artifact: &OwnedArtifact,
    ) -> Result<CleanupDisposition, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(crate) fn remove_published_artifact(
        &self,
        _published: &OwnedPublishedArtifact,
    ) -> Result<CleanupDisposition, String> {
        Err(UNSUPPORTED.to_string())
    }
}
