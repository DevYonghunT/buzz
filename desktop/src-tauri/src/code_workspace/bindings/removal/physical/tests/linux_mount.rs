#[cfg(target_os = "linux")]
use super::*;

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
pub(super) fn linux_privileged_same_filesystem_bind_managed_root_rejects_claim(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    assert_linux_claim_self_bind_rejected(
        &fixture,
        &fixture.managed_root,
        "managed worktree root crosses a nested mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
pub(super) fn linux_privileged_same_filesystem_bind_tracked_entry_rejects_claim(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    assert_linux_claim_self_bind_rejected(
        &fixture,
        &fixture.managed_root.join("README.md"),
        "rejects a nested-mount manifest file",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
pub(super) fn linux_privileged_same_filesystem_bind_git_admin_entry_rejects_claim(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let admin = linked_admin_entry(&fixture.managed_root)?;
    assert_linux_claim_self_bind_rejected(
        &fixture,
        &admin,
        "Git-admin entry crosses a nested mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
pub(super) fn linux_privileged_same_filesystem_bind_primary_objects_rejects_claim(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let objects = fixture.source_root.join(".git").join("objects");
    assert_linux_claim_self_bind_rejected(
        &fixture,
        &objects,
        "Git primary object directory crosses a nested mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
pub(super) fn linux_privileged_same_filesystem_bind_sidecar_directory_preserves_removing(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let removing = prepare_durable_removing(&fixture)?;
    let directory = fixture.app_data.join("code").join("removal-manifests-v1");
    assert_linux_removing_self_bind_rejected(
        &fixture,
        &removing,
        &directory,
        "removal manifest directory crosses a nested mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
pub(super) fn linux_privileged_same_filesystem_bind_sidecar_file_preserves_removing(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let removing = prepare_durable_removing(&fixture)?;
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!(
            "{}.json",
            removing.authority().physical_manifest_digest
        ));
    assert_linux_removing_self_bind_rejected(
        &fixture,
        &removing,
        &sidecar,
        "removal manifest sidecar crosses a mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
pub(super) fn linux_privileged_same_filesystem_bind_managed_root_is_sticky_removing(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let removing = prepare_durable_removing(&fixture)?;
    assert_linux_removing_self_bind_rejected(
        &fixture,
        &removing,
        &fixture.managed_root,
        "replacement or ambiguous state; recovery is sticky",
    )
}
