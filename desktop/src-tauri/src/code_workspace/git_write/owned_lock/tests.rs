#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::PathBuf;

use super::*;

struct Fixture {
    _root: tempfile::TempDir,
    admin: PathBuf,
    pinned: PinnedAdminDirectory,
}

impl Fixture {
    fn new() -> Result<Self, String> {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let admin = root.path().join("admin");
        fs::create_dir(&admin).map_err(|error| error.to_string())?;
        let admin = admin.canonicalize().map_err(|error| error.to_string())?;
        let pinned = PinnedAdminDirectory::pin(&admin)?;
        Ok(Self {
            _root: root,
            admin,
            pinned,
        })
    }

    fn path(&self, component: &str) -> PathBuf {
        self.admin.join(component)
    }

    fn create_artifact(&self, bytes: &[u8]) -> Result<OwnedArtifact, String> {
        let issued = self.pinned.issue_artifact_component()?;
        self.pinned.create_artifact(&issued, bytes)
    }
}

fn acquired(value: LockAcquisition) -> Result<OwnedStandardLock, String> {
    match value {
        LockAcquisition::Acquired(owned) => Ok(owned),
        other => Err(format!("expected a newly acquired lock, got {other:?}")),
    }
}

#[test]
fn issued_component_is_serializable_and_mutation_free_until_create() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let issued = fixture.pinned.issue_artifact_component()?;
    assert_eq!(
        fs::read_dir(&fixture.admin)
            .map_err(|error| error.to_string())?
            .count(),
        0
    );
    let encoded = serde_json::to_vec(&issued).map_err(|error| error.to_string())?;
    let encoded: serde_json::Value =
        serde_json::from_slice(&encoded).map_err(|error| error.to_string())?;
    let component = encoded
        .get("component")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "serialized issued component was missing".to_string())?;
    let recovered = IssuedArtifactComponent::from_journal(component)?;
    assert_eq!(recovered, issued);

    let artifact = fixture
        .pinned
        .create_artifact(&recovered, b"journaled candidate\n")?;
    assert_eq!(artifact.component, issued.component());
    assert_eq!(MAX_OWNED_FILE_BYTES, 64 * 1024 * 1024);
    Ok(())
}

#[test]
fn journal_rehydrate_rejects_non_native_components_without_mutation() -> Result<(), String> {
    let fixture = Fixture::new()?;
    for component in [
        "index.lock",
        ".schoolx-git-../index.lock.artifact",
        ".schoolx-git-ABCDEF.artifact",
    ] {
        assert!(IssuedArtifactComponent::from_journal(component).is_err());
    }
    assert_eq!(
        fs::read_dir(&fixture.admin)
            .map_err(|error| error.to_string())?
            .count(),
        0
    );
    Ok(())
}

#[test]
fn artifact_hard_link_moves_from_one_to_two_links_and_is_idempotent() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let artifact = fixture.create_artifact(b"candidate index\n")?;
    assert_eq!(artifact.identity.link_count, 1);
    assert_eq!(artifact.identity.mode & 0o7777, 0o600);
    let artifact_path = fixture.path(&artifact.component);
    let lock_path = fixture.path("index.lock");

    let owned = acquired(
        fixture
            .pinned
            .acquire_standard_lock(&artifact, StandardLockKind::Index)?,
    )?;
    assert_eq!(owned.identity.link_count, 2);
    let artifact_stat = fs::metadata(&artifact_path).map_err(|error| error.to_string())?;
    let lock_stat = fs::metadata(&lock_path).map_err(|error| error.to_string())?;
    assert_eq!(artifact_stat.dev(), lock_stat.dev());
    assert_eq!(artifact_stat.ino(), lock_stat.ino());
    assert_eq!(artifact_stat.nlink(), 2);
    assert_eq!(lock_stat.nlink(), 2);

    match fixture
        .pinned
        .acquire_standard_lock(&artifact, StandardLockKind::Index)?
    {
        LockAcquisition::AlreadyOwned(retried) => assert_eq!(retried, owned),
        other => return Err(format!("owned lock retry was not idempotent: {other:?}")),
    }

    let released = fixture.pinned.release_standard_lock(&owned)?;
    assert!(!lock_path.exists());
    assert_eq!(released.identity.link_count, 1);
    assert_eq!(
        fs::metadata(&artifact_path)
            .map_err(|error| error.to_string())?
            .nlink(),
        1
    );
    assert_eq!(
        fixture.pinned.remove_artifact(&released)?,
        CleanupDisposition::Removed
    );
    assert!(!artifact_path.exists());
    Ok(())
}

#[test]
fn preexisting_foreign_lock_is_classified_and_preserved() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let artifact = fixture.create_artifact(b"owned bytes\n")?;
    let lock = fixture.path("index.lock");
    let foreign = b"foreign lock bytes\n";
    fs::write(&lock, foreign).map_err(|error| error.to_string())?;

    assert_eq!(
        fixture
            .pinned
            .acquire_standard_lock(&artifact, StandardLockKind::Index)?,
        LockAcquisition::Foreign
    );
    assert_eq!(fs::read(&lock).map_err(|error| error.to_string())?, foreign);
    assert_eq!(
        fixture.pinned.remove_artifact(&artifact)?,
        CleanupDisposition::Removed
    );
    assert_eq!(fs::read(&lock).map_err(|error| error.to_string())?, foreign);
    Ok(())
}

#[test]
fn replacement_of_acquired_lock_blocks_cleanup_and_preserves_foreign_bytes() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let artifact = fixture.create_artifact(b"owned HEAD guard\n")?;
    let owned = acquired(
        fixture
            .pinned
            .acquire_standard_lock(&artifact, StandardLockKind::Head)?,
    )?;
    let lock = fixture.path("HEAD.lock");
    fs::remove_file(&lock).map_err(|error| error.to_string())?;
    let foreign = b"foreign replacement\n";
    fs::write(&lock, foreign).map_err(|error| error.to_string())?;

    let error = fixture
        .pinned
        .cleanup_standard_lock(&owned)
        .expect_err("foreign replacement must block cleanup");
    assert!(error.contains("foreign standard Git lock"), "{error}");
    assert_eq!(fs::read(&lock).map_err(|error| error.to_string())?, foreign);
    assert_eq!(
        fs::read(fixture.path(&artifact.component)).map_err(|error| error.to_string())?,
        b"owned HEAD guard\n"
    );
    Ok(())
}

#[test]
fn restartable_cleanup_accepts_each_exact_partial_state() -> Result<(), String> {
    let lock_only = Fixture::new()?;
    let artifact = lock_only.create_artifact(b"guard\n")?;
    let owned = acquired(
        lock_only
            .pinned
            .acquire_standard_lock(&artifact, StandardLockKind::Head)?,
    )?;
    fs::remove_file(lock_only.path(&artifact.component)).map_err(|error| error.to_string())?;
    assert_eq!(
        lock_only.pinned.cleanup_standard_lock(&owned)?,
        CleanupDisposition::Removed
    );
    assert!(!lock_only.path("HEAD.lock").exists());
    assert_eq!(
        lock_only.pinned.cleanup_standard_lock(&owned)?,
        CleanupDisposition::AlreadyAbsent
    );

    let artifact_only = Fixture::new()?;
    let artifact = artifact_only.create_artifact(b"guard\n")?;
    let owned = acquired(
        artifact_only
            .pinned
            .acquire_standard_lock(&artifact, StandardLockKind::Index)?,
    )?;
    fs::remove_file(artifact_only.path("index.lock")).map_err(|error| error.to_string())?;
    assert_eq!(
        artifact_only.pinned.cleanup_standard_lock(&owned)?,
        CleanupDisposition::Removed
    );
    assert!(!artifact_only.path(&artifact.component).exists());
    Ok(())
}

#[test]
fn artifact_symlink_wrong_mode_and_wrong_digest_are_rejected() -> Result<(), String> {
    let symlink_fixture = Fixture::new()?;
    let symlink_artifact = symlink_fixture.create_artifact(b"owned\n")?;
    let target = symlink_fixture.path("foreign-target");
    fs::write(&target, b"foreign\n").map_err(|error| error.to_string())?;
    fs::remove_file(symlink_fixture.path(&symlink_artifact.component))
        .map_err(|error| error.to_string())?;
    std::os::unix::fs::symlink(&target, symlink_fixture.path(&symlink_artifact.component))
        .map_err(|error| error.to_string())?;
    let symlink_error = symlink_fixture
        .pinned
        .revalidate_artifact(&symlink_artifact)
        .expect_err("artifact symlink must be rejected");
    assert!(
        symlink_error.contains("not a regular file"),
        "{symlink_error}"
    );
    assert_eq!(
        fs::read(&target).map_err(|error| error.to_string())?,
        b"foreign\n"
    );

    let mode_fixture = Fixture::new()?;
    let mode_artifact = mode_fixture.create_artifact(b"owned\n")?;
    fs::set_permissions(
        mode_fixture.path(&mode_artifact.component),
        fs::Permissions::from_mode(0o644),
    )
    .map_err(|error| error.to_string())?;
    let mode_error = mode_fixture
        .pinned
        .revalidate_artifact(&mode_artifact)
        .expect_err("wrong artifact mode must be rejected");
    assert!(mode_error.contains("mode 0600"), "{mode_error}");

    let digest_fixture = Fixture::new()?;
    let artifact = digest_fixture.create_artifact(b"owned\n")?;
    let mut wrong_digest = artifact.clone();
    wrong_digest.identity.sha256 = "0".repeat(64);
    let digest_error = digest_fixture
        .pinned
        .revalidate_artifact(&wrong_digest)
        .expect_err("wrong artifact digest must be rejected");
    assert!(
        digest_error.contains("evidence no longer matches"),
        "{digest_error}"
    );
    Ok(())
}

#[test]
fn publish_consumes_exact_lock_and_installs_exact_destination_bytes() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let destination = fixture.path("index");
    fs::write(&destination, b"old index\n").map_err(|error| error.to_string())?;
    let artifact = fixture.create_artifact(b"new candidate index\n")?;
    let artifact_path = fixture.path(&artifact.component);
    let owned = acquired(
        fixture
            .pinned
            .acquire_standard_lock(&artifact, StandardLockKind::Index)?,
    )?;

    let published = fixture
        .pinned
        .publish_after_cas(&owned, || {
            let current = fs::read(&destination).map_err(|error| error.to_string())?;
            if current != b"old index\n" {
                return Err("destination preimage changed".to_string());
            }
            Ok(())
        })
        .map_err(PublishFailure::into_message)?;
    assert!(!fixture.path("index.lock").exists());
    assert_eq!(
        fs::read(&destination).map_err(|error| error.to_string())?,
        b"new candidate index\n"
    );
    let artifact_stat = fs::metadata(&artifact_path).map_err(|error| error.to_string())?;
    let destination_stat = fs::metadata(&destination).map_err(|error| error.to_string())?;
    assert_eq!(artifact_stat.dev(), destination_stat.dev());
    assert_eq!(artifact_stat.ino(), destination_stat.ino());
    assert_eq!(artifact_stat.nlink(), 2);
    fixture.pinned.revalidate()?;
    match fixture.pinned.classify_owned_lock(&owned)? {
        OwnedLockPlacement::Published(classified) => assert_eq!(classified, published),
        other => return Err(format!("publish was not recoverably classified: {other:?}")),
    }

    assert_eq!(
        fixture.pinned.remove_published_artifact(&published)?,
        CleanupDisposition::Removed
    );
    assert!(!artifact_path.exists());
    assert_eq!(
        fs::read(&destination).map_err(|error| error.to_string())?,
        b"new candidate index\n"
    );
    assert_eq!(
        fs::metadata(&destination)
            .map_err(|error| error.to_string())?
            .nlink(),
        1
    );
    Ok(())
}

#[test]
fn failed_cas_callback_preserves_owned_lock_and_destination() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let destination = fixture.path("HEAD");
    fs::write(&destination, b"old-head\n").map_err(|error| error.to_string())?;
    let artifact = fixture.create_artifact(b"new-head\n")?;
    let owned = acquired(
        fixture
            .pinned
            .acquire_standard_lock(&artifact, StandardLockKind::Head)?,
    )?;
    let error = fixture
        .pinned
        .publish_after_cas(&owned, || Err("preimage mismatch".to_string()))
        .expect_err("failed CAS proof must block publish");
    assert_eq!(error.into_message(), "preimage mismatch");
    assert_eq!(
        fs::read(&destination).map_err(|error| error.to_string())?,
        b"old-head\n"
    );
    fixture.pinned.revalidate_standard_lock(&owned)?;
    Ok(())
}

#[test]
fn post_rename_failure_is_typed_and_live_publish_remains_classifiable() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let destination = fixture.path("index");
    fs::write(&destination, b"old index\n").map_err(|error| error.to_string())?;
    let artifact = fixture.create_artifact(b"new index\n")?;
    let owned = acquired(
        fixture
            .pinned
            .acquire_standard_lock(&artifact, StandardLockKind::Index)?,
    )?;
    fail_next_publish_after_rename_for_test();
    let failure = fixture
        .pinned
        .publish_after_cas(&owned, || Ok(()))
        .expect_err("injected post-rename fault must report an unknown outcome");
    assert!(failure.outcome_unknown());
    assert_eq!(
        fs::read(&destination).map_err(|error| error.to_string())?,
        b"new index\n"
    );
    let published = match fixture.pinned.classify_owned_lock(&owned)? {
        OwnedLockPlacement::Published(published) => published,
        other => return Err(format!("post-rename state was not published: {other:?}")),
    };
    fixture.pinned.remove_published_artifact(&published)?;
    Ok(())
}

#[test]
fn named_admin_replacement_is_rejected_before_mutation() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let issued = fixture.pinned.issue_artifact_component()?;
    let old_admin = fixture
        .admin
        .parent()
        .ok_or_else(|| "admin has no parent".to_string())?
        .join("admin-old");
    fs::rename(&fixture.admin, &old_admin).map_err(|error| error.to_string())?;
    fs::create_dir(&fixture.admin).map_err(|error| error.to_string())?;

    let error = fixture
        .pinned
        .create_artifact(&issued, b"must not be written\n")
        .expect_err("named parent replacement must fail closed");
    assert!(error.contains("moved or replaced"), "{error}");
    assert_eq!(
        fs::read_dir(&fixture.admin)
            .map_err(|error| error.to_string())?
            .count(),
        0
    );
    assert_eq!(
        fs::read_dir(&old_admin)
            .map_err(|error| error.to_string())?
            .count(),
        0
    );
    Ok(())
}
