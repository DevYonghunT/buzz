//! Audience policy for derived content such as summaries and reports.
//!
//! A source link is provenance, not an authorization boundary. Before derived
//! content is published, every class of target reader must be able to read
//! every source. Open channels therefore have two simultaneous rules: humans
//! may read at community scope, while managed agents still require explicit
//! active membership. This module models and tests that zero-I/O policy; relay
//! integrations are responsible for loading current visibility, membership,
//! and server-resolved principal classes immediately before publication.

use std::collections::BTreeSet;

use crate::PublicKey;

/// Stable byte representation used when comparing channel readers.
pub type AudienceMember = [u8; 32];

/// Human readers for one channel at a point in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HumanAudience {
    /// Every authenticated human in the community may read.
    Community,
    /// Only the listed active human members may read.
    Members(BTreeSet<AudienceMember>),
}

impl HumanAudience {
    fn permits_target(&self, target: &Self) -> bool {
        match (self, target) {
            (Self::Community, _) => true,
            (Self::Members(_), Self::Community) => false,
            (Self::Members(source), Self::Members(target)) => target.is_subset(source),
        }
    }
}

/// Effective readers for one channel at a point in time.
///
/// Human visibility and managed-agent capability are intentionally separate:
/// an open channel uses [`HumanAudience::Community`] but still lists only the
/// managed agents with active membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelAudience {
    humans: HumanAudience,
    managed_agents: BTreeSet<AudienceMember>,
}

impl ChannelAudience {
    /// Construct an open-channel audience.
    pub fn open(managed_agents: impl IntoIterator<Item = PublicKey>) -> Self {
        Self::open_member_bytes(managed_agents.into_iter().map(|pubkey| pubkey.to_bytes()))
    }

    /// Construct an open-channel audience from normalized managed-agent keys.
    pub fn open_member_bytes(managed_agents: impl IntoIterator<Item = AudienceMember>) -> Self {
        Self {
            humans: HumanAudience::Community,
            managed_agents: managed_agents.into_iter().collect(),
        }
    }

    /// Construct a private-channel audience.
    pub fn private(
        humans: impl IntoIterator<Item = PublicKey>,
        managed_agents: impl IntoIterator<Item = PublicKey>,
    ) -> Self {
        Self::private_member_bytes(
            humans.into_iter().map(|pubkey| pubkey.to_bytes()),
            managed_agents.into_iter().map(|pubkey| pubkey.to_bytes()),
        )
    }

    /// Construct a private-channel audience from normalized principal keys.
    pub fn private_member_bytes(
        humans: impl IntoIterator<Item = AudienceMember>,
        managed_agents: impl IntoIterator<Item = AudienceMember>,
    ) -> Self {
        Self {
            humans: HumanAudience::Members(humans.into_iter().collect()),
            managed_agents: managed_agents.into_iter().collect(),
        }
    }

    /// Human readers represented by this audience.
    pub const fn humans(&self) -> &HumanAudience {
        &self.humans
    }

    /// Explicitly authorized managed-agent readers.
    pub const fn managed_agents(&self) -> &BTreeSet<AudienceMember> {
        &self.managed_agents
    }

    /// Return whether every target reader can also read this source.
    pub fn permits_target(&self, target: &Self) -> bool {
        self.humans.permits_target(&target.humans)
            && target.managed_agents.is_subset(&self.managed_agents)
    }
}

/// Why a derived-content audience check failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudiencePolicyError {
    /// Publishing without a source would make the result untraceable.
    #[error("derived content must reference at least one source")]
    MissingSources,
    /// At least one target reader cannot read one of the sources.
    #[error("target audience is broader than source audience at index {source_index}")]
    TargetBroaderThanSource {
        /// Zero-based source index that rejected the target audience.
        source_index: usize,
    },
}

/// Validate that `target` is no broader than every source audience.
///
/// For multiple sources this requires both human and managed-agent readers to
/// fit the intersection of all source audiences. The function rejects an empty
/// source list so a caller cannot publish unprovenanceable output.
pub fn validate_derived_content_audience(
    target: &ChannelAudience,
    sources: &[ChannelAudience],
) -> Result<(), AudiencePolicyError> {
    if sources.is_empty() {
        return Err(AudiencePolicyError::MissingSources);
    }

    for (source_index, source) in sources.iter().enumerate() {
        if !source.permits_target(target) {
            return Err(AudiencePolicyError::TargetBroaderThanSource { source_index });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(byte: u8) -> AudienceMember {
        [byte; 32]
    }

    #[test]
    fn private_source_rejects_open_target() {
        let source = ChannelAudience::private_member_bytes([member(1)], []);
        assert_eq!(
            validate_derived_content_audience(&ChannelAudience::open_member_bytes([]), &[source]),
            Err(AudiencePolicyError::TargetBroaderThanSource { source_index: 0 })
        );
    }

    #[test]
    fn private_target_must_fit_every_source_intersection() {
        let source_a = ChannelAudience::private_member_bytes([member(1), member(2)], [member(8)]);
        let source_b =
            ChannelAudience::private_member_bytes([member(2), member(3)], [member(8), member(9)]);

        assert!(validate_derived_content_audience(
            &ChannelAudience::private_member_bytes([member(2)], [member(8)]),
            &[source_a.clone(), source_b.clone()],
        )
        .is_ok());
        assert_eq!(
            validate_derived_content_audience(
                &ChannelAudience::private_member_bytes([member(1), member(2)], [member(8)]),
                &[source_a, source_b],
            ),
            Err(AudiencePolicyError::TargetBroaderThanSource { source_index: 1 })
        );
    }

    #[test]
    fn open_source_keeps_humans_open_but_restricts_agents() {
        let source = ChannelAudience::open_member_bytes([member(7)]);

        assert!(validate_derived_content_audience(
            &ChannelAudience::open_member_bytes([member(7)]),
            std::slice::from_ref(&source),
        )
        .is_ok());
        assert_eq!(
            validate_derived_content_audience(
                &ChannelAudience::open_member_bytes([member(8)]),
                &[source],
            ),
            Err(AudiencePolicyError::TargetBroaderThanSource { source_index: 0 })
        );
    }

    #[test]
    fn open_source_allows_a_narrower_private_human_target() {
        assert!(validate_derived_content_audience(
            &ChannelAudience::private_member_bytes([member(1)], []),
            &[ChannelAudience::open_member_bytes([])],
        )
        .is_ok());
    }

    #[test]
    fn sources_are_required() {
        assert_eq!(
            validate_derived_content_audience(
                &ChannelAudience::private_member_bytes([member(1)], []),
                &[],
            ),
            Err(AudiencePolicyError::MissingSources)
        );
    }
}
