//! Test-only crash checkpoints around durable Git transaction boundaries.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::code_workspace::git_write) enum TransactionFaultBoundary {
    PreparedPersisted,
    BlobObjectWritten,
    TreeObjectWritten,
    TreeEvidencePersisted,
    CommitObjectWritten,
    ObjectWrittenPersisted,
    IndexArtifactDurable,
    HeadArtifactDurable,
    ArtifactsReadyPersisted,
    IndexLockDurable,
    HeadLockDurable,
    LocksReadyPersisted,
    BeforeIndexPublish,
    IndexPublishDurable,
    IndexPublishedPersisted,
    BeforeHeadPublish,
    HeadPublishDurable,
    HeadPublishedPersisted,
    CompletedReceiptPersisted,
    CleanupCompleted,
    ResponseReady,
    AcknowledgementPersisted,
}

#[cfg(test)]
pub(super) const TEST_FAULT_BOUNDARY_ENV: &str = "SCHOOLX_CODE_GIT_WRITE_FAULT_BOUNDARY_V1";
#[cfg(test)]
pub(super) const INJECTED_CRASH_EXIT_CODE: i32 = 86;

pub(in crate::code_workspace::git_write) fn checkpoint(
    boundary: TransactionFaultBoundary,
) -> Result<(), String> {
    #[cfg(test)]
    {
        let Some(encoded) = std::env::var_os(TEST_FAULT_BOUNDARY_ENV) else {
            return Ok(());
        };
        let encoded = encoded
            .into_string()
            .map_err(|_| "Git transaction fault boundary is not UTF-8".to_string())?;
        let target: TransactionFaultBoundary = serde_json::from_str(&encoded)
            .map_err(|error| format!("invalid Git transaction fault boundary: {error}"))?;
        if target == boundary {
            std::process::exit(INJECTED_CRASH_EXIT_CODE);
        }
    }
    #[cfg(not(test))]
    let _ = boundary;
    Ok(())
}
