//! Proven-owned artifacts and standard Git lock publication.
//!
//! Callers choose only `index` or detached `HEAD`; private artifact names are
//! native-issued, and every destructive action requires exact durable evidence.
#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use std::path::Path;

const MAX_OWNED_FILE_BYTES: usize = 64 * 1024 * 1024;
const ARTIFACT_PREFIX: &str = ".schoolx-git-";
const ARTIFACT_SUFFIX: &str = ".artifact";
const RANDOM_HEX_BYTES: usize = 32;

#[cfg(test)]
thread_local! {
    static FAIL_AFTER_RENAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn fail_next_publish_after_rename_for_test() {
    FAIL_AFTER_RENAME.with(|flag| flag.set(true));
}

#[cfg(test)]
fn take_publish_after_rename_failure() -> bool {
    FAIL_AFTER_RENAME.with(|flag| flag.replace(false))
}
mod recovery;
#[cfg(test)]
#[path = "owned_lock/tests.rs"]
mod tests;
/// Exact identity and contents of one app-owned regular file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnedFileIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) owner: u32,
    pub(crate) mode: u32,
    pub(crate) link_count: u64,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}
/// Serializable identity of the exact pinned Git admin directory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnedDirectoryIdentity {
    pub(crate) path: String,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) owner: u32,
    pub(crate) mode: u32,
}
/// Native-issued artifact name that can be durably journaled before creation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IssuedArtifactComponent {
    component: String,
}
impl IssuedArtifactComponent {
    pub(crate) fn component(&self) -> &str {
        &self.component
    }

    pub(crate) fn from_journal(component: &str) -> Result<Self, String> {
        validate_artifact_component(component)?;
        Ok(Self {
            component: component.to_string(),
        })
    }
}
fn validate_artifact_component(component: &str) -> Result<(), String> {
    let Some(hex) = component
        .strip_prefix(ARTIFACT_PREFIX)
        .and_then(|value| value.strip_suffix(ARTIFACT_SUFFIX))
    else {
        return Err("owned Git artifact name is outside the native namespace".to_string());
    };
    if hex.len() != RANDOM_HEX_BYTES * 2
        || !hex
            .bytes()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
    {
        return Err("owned Git artifact name has invalid random evidence".to_string());
    }
    Ok(())
}
/// A random private artifact before it is linked to a standard Git lock.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnedArtifact {
    pub(crate) component: String,
    pub(crate) identity: OwnedFileIdentity,
}
/// The only standard Git locks this module can acquire or publish.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StandardLockKind {
    Index,
    Head,
}
impl StandardLockKind {
    fn lock_component(self) -> &'static str {
        match self {
            Self::Index => "index.lock",
            Self::Head => "HEAD.lock",
        }
    }

    fn destination_component(self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Head => "HEAD",
        }
    }
}
/// Durable evidence that an artifact and standard lock are the same inode.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnedStandardLock {
    pub(crate) artifact_component: String,
    pub(crate) kind: StandardLockKind,
    pub(crate) identity: OwnedFileIdentity,
}
/// Durable evidence after a standard lock has been renamed to its destination.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnedPublishedArtifact {
    pub(crate) artifact_component: String,
    pub(crate) kind: StandardLockKind,
    pub(crate) identity: OwnedFileIdentity,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LockAcquisition {
    Acquired(OwnedStandardLock),
    AlreadyOwned(OwnedStandardLock),
    Foreign,
}
/// Live placement of an inode previously recorded as an owned standard lock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OwnedLockPlacement {
    Locked(OwnedStandardLock),
    Published(OwnedPublishedArtifact),
    Released(OwnedArtifact),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CleanupDisposition {
    Removed,
    AlreadyAbsent,
}
/// Whether a failed publish definitely preceded rename or may have renamed.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PublishFailure {
    BeforeRename(String),
    OutcomeUnknown(String),
}
impl PublishFailure {
    pub(crate) fn outcome_unknown(&self) -> bool {
        matches!(self, Self::OutcomeUnknown(_))
    }

    pub(crate) fn into_message(self) -> String {
        match self {
            Self::BeforeRename(message) | Self::OutcomeUnknown(message) => message,
        }
    }
}
/// Open handle and immutable pathname identity for one Git admin directory.
pub(crate) struct PinnedAdminDirectory {
    identity: OwnedDirectoryIdentity,
    #[cfg(unix)]
    expected_path: std::path::PathBuf,
    #[cfg(unix)]
    handle: std::fs::File,
}
#[cfg(unix)]
mod unix {
    use super::*;
    use rustix::fs::{AtFlags, FileType, Mode, OFlags};
    use sha2::{Digest, Sha256};
    use std::ffi::{CStr, CString};
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::fs::MetadataExt as _;
    mod cleanup;
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum NamedLockState {
        Absent,
        Exact,
        Foreign,
    }

    impl PinnedAdminDirectory {
        /// Pin an exact canonical admin directory without following its final component.
        pub(crate) fn pin(path: &Path) -> Result<Self, String> {
            if !path.is_absolute() {
                return Err("Git admin directory must be absolute before pinning".to_string());
            }
            let canonical = path
                .canonicalize()
                .map_err(|error| format!("failed to resolve Git admin directory: {error}"))?;
            if canonical != path {
                return Err("Git admin directory must be canonical before pinning".to_string());
            }
            let path_text = path
                .to_str()
                .ok_or_else(|| "Git admin directory is not valid UTF-8".to_string())?
                .to_string();
            let fd = rustix::fs::open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| format!("failed to pin Git admin directory: {error}"))?;
            let handle = fs::File::from(fd);
            let stat = rustix::fs::fstat(&handle).map_err(|error| {
                format!("failed to inspect pinned Git admin directory: {error}")
            })?;
            if !FileType::from_raw_mode(stat.st_mode).is_dir() {
                return Err("pinned Git admin handle is not a directory".to_string());
            }
            let identity = OwnedDirectoryIdentity {
                path: path_text,
                device: stat.st_dev as u64,
                inode: stat.st_ino as u64,
                owner: stat.st_uid as u32,
                mode: stat.st_mode as u32,
            };
            if identity.owner != current_owner() {
                return Err("Git admin directory is not owned by the current user".to_string());
            }
            let pinned = Self {
                identity,
                expected_path: canonical,
                handle,
            };
            pinned.revalidate()?;
            Ok(pinned)
        }

        pub(crate) fn identity(&self) -> &OwnedDirectoryIdentity {
            &self.identity
        }

        /// Revalidate the open handle and the exact named admin path.
        pub(crate) fn revalidate(&self) -> Result<(), String> {
            let open = rustix::fs::fstat(&self.handle)
                .map_err(|error| format!("failed to re-inspect pinned Git admin: {error}"))?;
            if !directory_matches(&open, &self.identity) {
                return Err("pinned Git admin directory identity changed".to_string());
            }
            let named = fs::symlink_metadata(&self.expected_path)
                .map_err(|error| format!("failed to re-inspect named Git admin: {error}"))?;
            if named.file_type().is_symlink()
                || !named.is_dir()
                || named.dev() != self.identity.device
                || named.ino() != self.identity.inode
                || named.uid() != self.identity.owner
                || named.mode() != self.identity.mode
            {
                return Err("Git admin directory path was moved or replaced".to_string());
            }
            let canonical = self
                .expected_path
                .canonicalize()
                .map_err(|error| format!("failed to resolve named Git admin: {error}"))?;
            if canonical != self.expected_path {
                return Err("Git admin directory path stopped being canonical".to_string());
            }
            Ok(())
        }

        /// Issue a mutation-free random component for the prepared journal claim.
        pub(crate) fn issue_artifact_component(&self) -> Result<IssuedArtifactComponent, String> {
            self.revalidate()?;
            Ok(IssuedArtifactComponent {
                component: random_artifact_component()?,
            })
        }

        /// Create and durably record the exact previously issued private artifact.
        pub(crate) fn create_artifact(
            &self,
            issued: &IssuedArtifactComponent,
            bytes: &[u8],
        ) -> Result<OwnedArtifact, String> {
            if bytes.len() > MAX_OWNED_FILE_BYTES {
                return Err(format!(
                    "owned Git artifact exceeds the {MAX_OWNED_FILE_BYTES}-byte limit"
                ));
            }
            self.revalidate()?;
            validate_artifact_component(&issued.component)?;
            let component = issued.component.clone();
            let component_c = component_cstr(&component)?;
            let fd = rustix::fs::openat(
                &self.handle,
                component_c.as_c_str(),
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|error| format!("failed to create owned Git artifact: {error}"))?;
            let mut file = fs::File::from(fd);
            rustix::fs::fchmod(&file, Mode::RUSR | Mode::WUSR)
                .map_err(|error| format!("failed to secure owned Git artifact: {error}"))?;
            file.write_all(bytes)
                .map_err(|error| format!("failed to write owned Git artifact: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("failed to sync owned Git artifact: {error}"))?;
            let stat = rustix::fs::fstat(&file)
                .map_err(|error| format!("failed to inspect owned Git artifact: {error}"))?;
            let expected = identity_from_stat(&stat, sha256(bytes))?;
            validate_identity_shape(&expected, 1)?;
            self.verify_named_matches(component_c.as_c_str(), &expected)?;
            rustix::fs::fsync(&self.handle).map_err(|error| {
                format!("failed to sync Git admin after artifact create: {error}")
            })?;
            self.revalidate()?;
            let current = self.read_owned_file(component_c.as_c_str())?;
            if current != expected {
                return Err("owned Git artifact changed after durable creation".to_string());
            }
            Ok(OwnedArtifact {
                component,
                identity: current,
            })
        }

        pub(crate) fn revalidate_artifact(&self, artifact: &OwnedArtifact) -> Result<(), String> {
            validate_artifact(artifact)?;
            self.revalidate()?;
            let component = component_cstr(&artifact.component)?;
            let current = self.read_owned_file(component.as_c_str())?;
            if current != artifact.identity {
                return Err("owned Git artifact evidence no longer matches".to_string());
            }
            self.revalidate()
        }

        /// Hard-link an artifact to a standard lock without replacing any existing entry.
        pub(crate) fn acquire_standard_lock(
            &self,
            artifact: &OwnedArtifact,
            kind: StandardLockKind,
        ) -> Result<LockAcquisition, String> {
            validate_artifact(artifact)?;
            self.revalidate()?;
            let artifact_name = component_cstr(&artifact.component)?;
            let current_artifact = self.read_owned_file(artifact_name.as_c_str())?;
            if !same_stable_file(&current_artifact, &artifact.identity) {
                return Err("owned Git artifact was replaced before lock acquisition".to_string());
            }
            let lock_name = literal_component(kind.lock_component())?;
            match self.classify_named_lock(lock_name, &current_artifact)? {
                NamedLockState::Exact => {
                    let owned = self.read_owned_pair(
                        artifact_name.as_c_str(),
                        lock_name,
                        kind,
                        &artifact.identity,
                    )?;
                    rustix::fs::fsync(&self.handle).map_err(|error| {
                        format!("failed to sync already-owned Git lock: {error}")
                    })?;
                    self.revalidate()?;
                    return Ok(LockAcquisition::AlreadyOwned(owned));
                }
                NamedLockState::Foreign => {
                    if current_artifact.link_count != 1 {
                        return Err(
                            "owned Git artifact has an unexplained link beside a foreign lock"
                                .to_string(),
                        );
                    }
                    return Ok(LockAcquisition::Foreign);
                }
                NamedLockState::Absent => {}
            }
            if current_artifact.link_count != 1 {
                return Err("owned Git artifact has an unexplained link count".to_string());
            }
            match rustix::fs::linkat(
                &self.handle,
                artifact_name.as_c_str(),
                &self.handle,
                lock_name,
                AtFlags::empty(),
            ) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => {
                    return match self.classify_named_lock(lock_name, &current_artifact)? {
                        NamedLockState::Exact => {
                            let owned = self.read_owned_pair(
                                artifact_name.as_c_str(),
                                lock_name,
                                kind,
                                &artifact.identity,
                            )?;
                            rustix::fs::fsync(&self.handle).map_err(|sync_error| {
                                format!("failed to sync raced owned Git lock: {sync_error}")
                            })?;
                            self.revalidate()?;
                            Ok(LockAcquisition::AlreadyOwned(owned))
                        }
                        NamedLockState::Foreign => Ok(LockAcquisition::Foreign),
                        NamedLockState::Absent => Err(
                            "standard Git lock disappeared during no-replace acquisition"
                                .to_string(),
                        ),
                    };
                }
                Err(error) => return Err(format!("failed to acquire standard Git lock: {error}")),
            }
            rustix::fs::fsync(&self.handle)
                .map_err(|error| format!("failed to sync acquired standard Git lock: {error}"))?;
            self.revalidate()?;
            Ok(LockAcquisition::Acquired(self.read_owned_pair(
                artifact_name.as_c_str(),
                lock_name,
                kind,
                &artifact.identity,
            )?))
        }

        pub(crate) fn revalidate_standard_lock(
            &self,
            owned: &OwnedStandardLock,
        ) -> Result<(), String> {
            validate_owned_lock(owned)?;
            self.revalidate()?;
            let artifact = component_cstr(&owned.artifact_component)?;
            let lock = literal_component(owned.kind.lock_component())?;
            let current =
                self.read_owned_pair(artifact.as_c_str(), lock, owned.kind, &owned.identity)?;
            if current != *owned {
                return Err("owned standard Git lock evidence no longer matches".to_string());
            }
            self.revalidate()
        }

        /// Classify an owned lock using only its artifact, lock, and destination evidence.
        pub(crate) fn classify_owned_lock(
            &self,
            owned: &OwnedStandardLock,
        ) -> Result<OwnedLockPlacement, String> {
            validate_owned_lock(owned)?;
            self.revalidate()?;
            let artifact_name = component_cstr(&owned.artifact_component)?;
            let artifact = self.read_owned_file(artifact_name.as_c_str())?;
            if !same_stable_file(&artifact, &owned.identity) {
                return Err(
                    "owned Git artifact was replaced during lock classification".to_string()
                );
            }
            let lock_name = literal_component(owned.kind.lock_component())?;
            match self.classify_named_lock(lock_name, &artifact)? {
                NamedLockState::Exact => {
                    let current = self.read_owned_pair(
                        artifact_name.as_c_str(),
                        lock_name,
                        owned.kind,
                        &owned.identity,
                    )?;
                    if current.identity != owned.identity {
                        return Err("owned standard Git lock changed after acquisition".to_string());
                    }
                    return Ok(OwnedLockPlacement::Locked(current));
                }
                NamedLockState::Foreign => {
                    return Err("foreign standard Git lock was preserved".to_string());
                }
                NamedLockState::Absent => {}
            }
            let destination = literal_component(owned.kind.destination_component())?;
            if self.named_is_same_inode(destination, &artifact)? {
                let published =
                    self.read_published_pair(artifact_name.as_c_str(), destination, owned.kind)?;
                if published.identity != owned.identity {
                    return Err("published Git destination changed after lock evidence".to_string());
                }
                return Ok(OwnedLockPlacement::Published(published));
            }
            if artifact.link_count == 1 {
                return Ok(OwnedLockPlacement::Released(OwnedArtifact {
                    component: owned.artifact_component.clone(),
                    identity: artifact,
                }));
            }
            Err("owned Git lock has an ambiguous live link placement".to_string())
        }

        /// Publish only after a caller-supplied preimage CAS proof succeeds under the lock.
        pub(crate) fn publish_after_cas<F>(
            &self,
            owned: &OwnedStandardLock,
            prove_preimage: F,
        ) -> Result<OwnedPublishedArtifact, PublishFailure>
        where
            F: FnOnce() -> Result<(), String>,
        {
            self.revalidate_standard_lock(owned)
                .map_err(PublishFailure::BeforeRename)?;
            prove_preimage().map_err(PublishFailure::BeforeRename)?;
            // The callback cannot mint a publish token. Rechecking immediately
            // after it is the sealed boundary consumed by the rename below.
            self.revalidate_standard_lock(owned)
                .map_err(PublishFailure::BeforeRename)?;
            let lock = literal_component(owned.kind.lock_component())
                .map_err(PublishFailure::BeforeRename)?;
            let destination = literal_component(owned.kind.destination_component())
                .map_err(PublishFailure::BeforeRename)?;
            rustix::fs::renameat(&self.handle, lock, &self.handle, destination).map_err(
                |error| {
                    PublishFailure::BeforeRename(format!(
                        "failed to publish owned standard Git lock: {error}"
                    ))
                },
            )?;
            #[cfg(test)]
            if take_publish_after_rename_failure() {
                return Err(PublishFailure::OutcomeUnknown(
                    "injected failure immediately after Git destination rename".to_string(),
                ));
            }
            rustix::fs::fsync(&self.handle).map_err(|error| {
                PublishFailure::OutcomeUnknown(format!(
                    "failed to sync published Git destination: {error}"
                ))
            })?;
            self.revalidate().map_err(PublishFailure::OutcomeUnknown)?;
            let artifact = component_cstr(&owned.artifact_component)
                .map_err(PublishFailure::OutcomeUnknown)?;
            let published = self
                .read_published_pair(artifact.as_c_str(), destination, owned.kind)
                .map_err(PublishFailure::OutcomeUnknown)?;
            if published.identity != owned.identity {
                return Err(PublishFailure::OutcomeUnknown(
                    "published Git destination does not match owned lock evidence".to_string(),
                ));
            }
            Ok(published)
        }

        /// Remove an exact unconsumed standard lock and return nlink=1 artifact evidence.
        pub(crate) fn release_standard_lock(
            &self,
            owned: &OwnedStandardLock,
        ) -> Result<OwnedArtifact, String> {
            match self.classify_owned_lock(owned)? {
                OwnedLockPlacement::Released(artifact) => Ok(artifact),
                OwnedLockPlacement::Published(_) => {
                    Err("published Git lock cannot be released as a guard".to_string())
                }
                OwnedLockPlacement::Locked(current) => {
                    self.revalidate_standard_lock(&current)?;
                    let lock = literal_component(current.kind.lock_component())?;
                    rustix::fs::unlinkat(&self.handle, lock, AtFlags::empty()).map_err(
                        |error| format!("failed to remove exact owned standard Git lock: {error}"),
                    )?;
                    rustix::fs::fsync(&self.handle).map_err(|error| {
                        format!("failed to sync standard Git lock cleanup: {error}")
                    })?;
                    self.revalidate()?;
                    let artifact_name = component_cstr(&current.artifact_component)?;
                    let identity = self.read_owned_file(artifact_name.as_c_str())?;
                    if !same_stable_file(&identity, &current.identity) || identity.link_count != 1 {
                        return Err(
                            "owned Git artifact did not return to one link after release"
                                .to_string(),
                        );
                    }
                    Ok(OwnedArtifact {
                        component: current.artifact_component,
                        identity,
                    })
                }
            }
        }

        /// Remove an exact private nlink=1 artifact, never a standard lock.
        pub(crate) fn remove_artifact(
            &self,
            artifact: &OwnedArtifact,
        ) -> Result<CleanupDisposition, String> {
            validate_artifact(artifact)?;
            self.revalidate()?;
            let component = component_cstr(&artifact.component)?;
            match rustix::fs::statat(
                &self.handle,
                component.as_c_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Err(rustix::io::Errno::NOENT) => {
                    rustix::fs::fsync(&self.handle).map_err(|error| {
                        format!("failed to sync absent owned Git artifact: {error}")
                    })?;
                    return Ok(CleanupDisposition::AlreadyAbsent);
                }
                Err(error) => {
                    return Err(format!("failed to inspect owned Git artifact: {error}"));
                }
                Ok(_) => {}
            }
            self.revalidate_artifact(artifact)?;
            rustix::fs::unlinkat(&self.handle, component.as_c_str(), AtFlags::empty())
                .map_err(|error| format!("failed to remove exact owned Git artifact: {error}"))?;
            rustix::fs::fsync(&self.handle)
                .map_err(|error| format!("failed to sync owned Git artifact cleanup: {error}"))?;
            self.revalidate()?;
            ensure_absent(&self.handle, component.as_c_str())?;
            Ok(CleanupDisposition::Removed)
        }

        /// Remove only the private artifact link left by a completed publish.
        /// External post-publish destination drift is preserved.
        pub(crate) fn remove_published_artifact(
            &self,
            published: &OwnedPublishedArtifact,
        ) -> Result<CleanupDisposition, String> {
            validate_published(published)?;
            self.revalidate()?;
            let artifact_name = component_cstr(&published.artifact_component)?;
            let artifact = match self.read_owned_file(artifact_name.as_c_str()) {
                Ok(identity) => identity,
                Err(error) if error.starts_with("absent owned Git file:") => {
                    rustix::fs::fsync(&self.handle).map_err(|sync_error| {
                        format!("failed to sync absent published artifact: {sync_error}")
                    })?;
                    return Ok(CleanupDisposition::AlreadyAbsent);
                }
                Err(error) => return Err(error),
            };
            if !same_stable_file(&artifact, &published.identity)
                || !matches!(artifact.link_count, 1 | 2)
            {
                return Err("published private artifact was replaced before cleanup".to_string());
            }
            if artifact.link_count == 2 {
                let destination = literal_component(published.kind.destination_component())?;
                if !self.named_is_same_inode(destination, &artifact)? {
                    return Err(
                        "published artifact has an unexplained second link before cleanup"
                            .to_string(),
                    );
                }
                let pair = self.read_published_pair(
                    artifact_name.as_c_str(),
                    destination,
                    published.kind,
                )?;
                if pair.identity != published.identity {
                    return Err("published destination evidence changed before cleanup".to_string());
                }
            }
            // Re-read the exact private name immediately before unlink. If the
            // destination drifted, the private inode now has nlink=1 and remains
            // safe to remove without touching the replacement destination.
            let latest = self.read_owned_file(artifact_name.as_c_str())?;
            if !same_stable_file(&latest, &published.identity)
                || !matches!(latest.link_count, 1 | 2)
            {
                return Err("published private artifact changed before cleanup".to_string());
            }
            rustix::fs::unlinkat(&self.handle, artifact_name.as_c_str(), AtFlags::empty())
                .map_err(|error| format!("failed to remove published private artifact: {error}"))?;
            rustix::fs::fsync(&self.handle)
                .map_err(|error| format!("failed to sync published artifact cleanup: {error}"))?;
            self.revalidate()?;
            ensure_absent(&self.handle, artifact_name.as_c_str())?;
            Ok(CleanupDisposition::Removed)
        }

        fn classify_named_lock(
            &self,
            lock: &CStr,
            artifact: &OwnedFileIdentity,
        ) -> Result<NamedLockState, String> {
            match rustix::fs::statat(&self.handle, lock, AtFlags::SYMLINK_NOFOLLOW) {
                Err(rustix::io::Errno::NOENT) => Ok(NamedLockState::Absent),
                Err(error) => Err(format!("failed to inspect standard Git lock: {error}")),
                Ok(stat)
                    if stat.st_dev as u64 != artifact.device || stat.st_ino != artifact.inode =>
                {
                    Ok(NamedLockState::Foreign)
                }
                Ok(_) => {
                    let current = self.read_owned_file(lock)?;
                    if same_stable_file(&current, artifact) {
                        Ok(NamedLockState::Exact)
                    } else {
                        Err("standard Git lock shares an inode but not owned evidence".to_string())
                    }
                }
            }
        }

        fn named_is_same_inode(
            &self,
            name: &CStr,
            expected: &OwnedFileIdentity,
        ) -> Result<bool, String> {
            match rustix::fs::statat(&self.handle, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => {
                    Ok(stat.st_dev as u64 == expected.device && stat.st_ino == expected.inode)
                }
                Err(rustix::io::Errno::NOENT) => Ok(false),
                Err(error) => Err(format!("failed to inspect Git destination: {error}")),
            }
        }

        fn read_owned_pair(
            &self,
            artifact_name: &CStr,
            lock_name: &CStr,
            kind: StandardLockKind,
            expected: &OwnedFileIdentity,
        ) -> Result<OwnedStandardLock, String> {
            let artifact = self.read_owned_file(artifact_name)?;
            let lock = self.read_owned_file(lock_name)?;
            if artifact != lock
                || artifact.link_count != 2
                || !same_stable_file(&artifact, expected)
            {
                return Err(
                    "owned artifact and standard lock are not an exact two-link inode pair"
                        .to_string(),
                );
            }
            Ok(OwnedStandardLock {
                artifact_component: artifact_name
                    .to_str()
                    .map_err(|_| "owned artifact component is not UTF-8".to_string())?
                    .to_string(),
                kind,
                identity: artifact,
            })
        }

        fn read_published_pair(
            &self,
            artifact_name: &CStr,
            destination_name: &CStr,
            kind: StandardLockKind,
        ) -> Result<OwnedPublishedArtifact, String> {
            let artifact = self.read_owned_file(artifact_name)?;
            let destination = self.read_owned_file(destination_name)?;
            if artifact != destination || artifact.link_count != 2 {
                return Err(
                    "owned artifact and Git destination are not an exact two-link inode pair"
                        .to_string(),
                );
            }
            Ok(OwnedPublishedArtifact {
                artifact_component: artifact_name
                    .to_str()
                    .map_err(|_| "owned artifact component is not UTF-8".to_string())?
                    .to_string(),
                kind,
                identity: artifact,
            })
        }

        fn verify_named_matches(
            &self,
            name: &CStr,
            expected: &OwnedFileIdentity,
        ) -> Result<(), String> {
            let stat = rustix::fs::statat(&self.handle, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| format!("failed to inspect named owned Git file: {error}"))?;
            if stat.st_dev as u64 != expected.device
                || stat.st_ino as u64 != expected.inode
                || stat.st_uid as u32 != expected.owner
                || stat.st_mode as u32 != expected.mode
                || stat.st_nlink as u64 != expected.link_count
                || stat.st_size as u64 != expected.size
            {
                return Err("named owned Git file does not match its open handle".to_string());
            }
            Ok(())
        }

        pub(super) fn read_owned_file(&self, name: &CStr) -> Result<OwnedFileIdentity, String> {
            let named = match rustix::fs::statat(&self.handle, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(rustix::io::Errno::NOENT) => {
                    return Err(format!("absent owned Git file:{}", name.to_string_lossy()));
                }
                Err(error) => return Err(format!("failed to inspect owned Git file: {error}")),
            };
            if !FileType::from_raw_mode(named.st_mode).is_file() {
                return Err("owned Git file is not a regular file".to_string());
            }
            if named.st_size < 0 || named.st_size as u64 > MAX_OWNED_FILE_BYTES as u64 {
                return Err(format!(
                    "owned Git file exceeds the {MAX_OWNED_FILE_BYTES}-byte limit"
                ));
            }
            let fd = rustix::fs::openat(
                &self.handle,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| format!("failed to open owned Git file: {error}"))?;
            let file = fs::File::from(fd);
            let before = rustix::fs::fstat(&file)
                .map_err(|error| format!("failed to inspect open owned Git file: {error}"))?;
            if !same_raw_file(&named, &before) {
                return Err("owned Git file changed while opening".to_string());
            }
            let mut bytes = Vec::with_capacity((before.st_size as usize).min(64 * 1024));
            (&file)
                .take((MAX_OWNED_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("failed to read owned Git file: {error}"))?;
            if bytes.len() > MAX_OWNED_FILE_BYTES {
                return Err(format!(
                    "owned Git file exceeds the {MAX_OWNED_FILE_BYTES}-byte limit"
                ));
            }
            let after = rustix::fs::fstat(&file)
                .map_err(|error| format!("failed to re-inspect open owned Git file: {error}"))?;
            let renamed = rustix::fs::statat(&self.handle, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| format!("failed to re-inspect named owned Git file: {error}"))?;
            if !same_raw_file(&before, &after)
                || !same_raw_file(&after, &renamed)
                || after.st_size as usize != bytes.len()
            {
                return Err("owned Git file changed while reading".to_string());
            }
            let identity = identity_from_stat(&after, sha256(&bytes))?;
            validate_identity_permissions(&identity)?;
            Ok(identity)
        }
    }

    fn validate_artifact(artifact: &OwnedArtifact) -> Result<(), String> {
        validate_artifact_component(&artifact.component)?;
        validate_identity_shape(&artifact.identity, 1)
    }

    fn validate_owned_lock(owned: &OwnedStandardLock) -> Result<(), String> {
        validate_artifact_component(&owned.artifact_component)?;
        validate_identity_shape(&owned.identity, 2)
    }

    fn validate_published(published: &OwnedPublishedArtifact) -> Result<(), String> {
        validate_artifact_component(&published.artifact_component)?;
        validate_identity_shape(&published.identity, 2)
    }

    fn validate_identity_shape(
        identity: &OwnedFileIdentity,
        expected_links: u64,
    ) -> Result<(), String> {
        validate_identity_permissions(identity)?;
        if identity.link_count != expected_links {
            return Err(format!(
                "owned Git evidence requires link count {expected_links}"
            ));
        }
        if identity.size > MAX_OWNED_FILE_BYTES as u64
            || identity.sha256.len() != 64
            || !identity
                .sha256
                .bytes()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
        {
            return Err("owned Git evidence has invalid size or digest".to_string());
        }
        Ok(())
    }

    fn validate_identity_permissions(identity: &OwnedFileIdentity) -> Result<(), String> {
        if !FileType::from_raw_mode(identity.mode as _).is_file()
            || identity.mode & 0o7777 != 0o600
            || identity.owner != current_owner()
            || identity.link_count == 0
        {
            return Err(
                "owned Git file must be a current-user regular file with mode 0600".to_string(),
            );
        }
        Ok(())
    }

    fn identity_from_stat(
        stat: &rustix::fs::Stat,
        digest: String,
    ) -> Result<OwnedFileIdentity, String> {
        let size = u64::try_from(stat.st_size)
            .map_err(|_| "owned Git file reported a negative size".to_string())?;
        Ok(OwnedFileIdentity {
            device: stat.st_dev as u64,
            inode: stat.st_ino,
            owner: stat.st_uid,
            mode: stat.st_mode as u32,
            link_count: stat.st_nlink as u64,
            size,
            sha256: digest,
        })
    }

    fn directory_matches(stat: &rustix::fs::Stat, expected: &OwnedDirectoryIdentity) -> bool {
        FileType::from_raw_mode(stat.st_mode).is_dir()
            && stat.st_dev as u64 == expected.device
            && stat.st_ino == expected.inode
            && stat.st_uid == expected.owner
            && stat.st_mode as u32 == expected.mode
    }

    fn same_raw_file(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
        left.st_dev == right.st_dev
            && left.st_ino == right.st_ino
            && left.st_uid == right.st_uid
            && left.st_mode == right.st_mode
            && left.st_nlink == right.st_nlink
            && left.st_size == right.st_size
    }

    fn same_stable_file(left: &OwnedFileIdentity, right: &OwnedFileIdentity) -> bool {
        left.device == right.device
            && left.inode == right.inode
            && left.owner == right.owner
            && left.mode == right.mode
            && left.size == right.size
            && left.sha256 == right.sha256
    }

    fn current_owner() -> u32 {
        rustix::process::geteuid().as_raw()
    }

    fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn random_artifact_component() -> Result<String, String> {
        let mut random = [0_u8; RANDOM_HEX_BYTES];
        getrandom::getrandom(&mut random)
            .map_err(|error| format!("failed to create random Git artifact name: {error}"))?;
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let component = format!("{ARTIFACT_PREFIX}{suffix}{ARTIFACT_SUFFIX}");
        validate_artifact_component(&component)?;
        Ok(component)
    }

    fn component_cstr(component: &str) -> Result<CString, String> {
        validate_artifact_component(component)?;
        CString::new(component).map_err(|_| "owned Git artifact name contains NUL".to_string())
    }

    fn literal_component(component: &'static str) -> Result<&'static CStr, String> {
        CStr::from_bytes_with_nul(match component {
            "index.lock" => b"index.lock\0",
            "HEAD.lock" => b"HEAD.lock\0",
            "index" => b"index\0",
            "HEAD" => b"HEAD\0",
            _ => return Err("unsupported standard Git component".to_string()),
        })
        .map_err(|_| "standard Git component is invalid".to_string())
    }

    fn ensure_absent(directory: &fs::File, name: &CStr) -> Result<(), String> {
        match rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => Ok(()),
            Ok(_) => Err("owned Git file remained after exact cleanup".to_string()),
            Err(error) => Err(format!("failed to verify owned Git file absence: {error}")),
        }
    }
}

#[cfg(not(unix))]
#[path = "owned_lock/non_unix.rs"]
mod non_unix;
