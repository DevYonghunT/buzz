use super::*;

#[test]
fn worktree_removal_decision_gates_are_frozen_with_the_public_surface_open() -> Result<(), String> {
    let contract = fixture(WORKTREE_REMOVAL_GATE_CONTRACT)?;
    let tauri_contract = fixture(TAURI_CONTRACT)?;
    let gate_order = [
        "mergedAuthority",
        "durableRemovalJournal",
        "bindingTranscriptSemantics",
        "pinnedDeletionBoundary",
    ];

    assert_eq!(contract["version"], 1);
    assert_eq!(
        contract["status"],
        "authorityProofJournalPhysicalRemovalImplementedPublicSurfaceOpen"
    );
    assert_eq!(
        keys(&contract)?,
        [
            "acceptanceCases",
            "currentInventory",
            "currentStoreVersion",
            "designDocument",
            "forbiddenOperations",
            "futureReceipt",
            "futureStoreVersion",
            "futureSurface",
            "gateOrder",
            "gates",
            "journalStates",
            "physicalRemovalOrder",
            "status",
            "version",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );
    assert_eq!(
        contract["currentStoreVersion"],
        CODE_THREAD_BINDING_SCHEMA_VERSION
    );
    assert_eq!(contract["futureStoreVersion"], 4);
    assert_eq!(contract["gateOrder"], json!(gate_order));
    assert_eq!(
        keys(&contract["gates"])?,
        gate_order.into_iter().map(str::to_string).collect()
    );
    for gate in gate_order {
        assert_eq!(contract["gates"][gate]["state"], "implementedClosed");
        assert!(
            WORKTREE_REMOVAL_GATE_DESIGN.contains(&format!("Gate `{gate}`")),
            "removal design is missing the {gate} section"
        );
    }
    assert!(WORKTREE_REMOVAL_GATE_DESIGN.contains("CodeWorktreeRemoveInput"));
    assert_eq!(
        contract["designDocument"],
        "docs/schoolx-2/SCHOOLX_CODE_WORKTREE_REMOVAL_GATES.md"
    );

    let surface = &contract["futureSurface"];
    assert_eq!(surface["commandName"], "code_worktree_remove");
    assert_eq!(surface["topLevelArgs"], json!(["input"]));
    assert_eq!(surface["inputFields"], json!(["scope", "threadId"]));
    assert_eq!(surface["operationId"], "nativeCanonicalUuid");
    assert_eq!(surface["registered"], true);
    assert_eq!(surface["frontendMethodExposed"], true);
    assert_eq!(surface["buttonRendered"], true);
    assert_eq!(
        keys(&tauri_contract["strictInputs"]["worktreeRemove"])?,
        surface["inputFields"]
            .as_array()
            .ok_or_else(|| "removal input fields must be an array".to_string())?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    );
    assert_eq!(
        contract["futureReceipt"]["fields"],
        json!([
            "removalId",
            "scope",
            "threadId",
            "worktreeId",
            "headCommit",
            "mergedIntoRef",
            "mergedIntoCommit",
            "transcriptDisposition",
            "executionDisposition"
        ])
    );
    assert_eq!(
        contract["futureReceipt"]["transcriptDisposition"],
        "preserved"
    );
    assert_eq!(contract["futureReceipt"]["executionDisposition"], "removed");
    assert_eq!(
        keys(&tauri_contract["outputs"]["worktreeRemovalReceipt"])?,
        string_set(
            &contract["futureReceipt"]["fields"],
            "removal receipt fields"
        )?
    );

    assert_eq!(
        contract["gates"]["mergedAuthority"]["targetRefNamespace"],
        "refs/heads/"
    );
    assert_eq!(
        contract["gates"]["mergedAuthority"]["storeCollection"],
        "mergeTargets"
    );
    assert_eq!(
        contract["gates"]["mergedAuthority"]["captureImplemented"],
        true
    );
    assert_eq!(
        contract["gates"]["mergedAuthority"]["proofImplemented"],
        true
    );
    assert_eq!(
        contract["gates"]["mergedAuthority"]["publicProofSurface"],
        false
    );
    assert_eq!(
        contract["gates"]["mergedAuthority"]["legacyBindings"],
        "authorityAbsent"
    );
    assert_eq!(
        contract["gates"]["mergedAuthority"]["proof"],
        "mergeBaseIsAncestor"
    );
    assert_eq!(
        contract["gates"]["mergedAuthority"]["proofSnapshotFields"],
        json!([
            "repositoryIdentity",
            "worktreeId",
            "headCommit",
            "targetRef",
            "targetCommit"
        ])
    );
    assert_eq!(
        contract["gates"]["mergedAuthority"]["rejectedEvidence"],
        json!([
            "headEqualsBaseRef",
            "inventoryInspection",
            "callerRefOrCommit",
            "tagOrRawObjectId",
            "remoteTrackingRef",
            "otherContainingRef",
            "squashOrCherryPickEquivalence",
            "networkOrPullRequestClaim",
            "replacementOrGraftAncestry"
        ])
    );
    assert_eq!(
        contract["gates"]["durableRemovalJournal"]["states"],
        json!(["claimed", "removing", "removed"])
    );
    assert_eq!(
        contract["gates"]["durableRemovalJournal"]["retryKey"],
        json!(["scope", "threadId"])
    );
    assert_eq!(
        contract["gates"]["durableRemovalJournal"]["recordFields"],
        json!([
            "state",
            "removalId",
            "binding",
            "threadLifecycleAtClaim",
            "mergeProof",
            "physicalManifestDigest",
            "physical",
            "transcriptDisposition",
            "executionDisposition"
        ])
    );
    assert_eq!(
        contract["gates"]["durableRemovalJournal"]["physicalFields"],
        contract["gates"]["pinnedDeletionBoundary"]["requiredPinnedCoordinates"]
    );
    assert_eq!(
        contract["gates"]["durableRemovalJournal"]["casImplemented"],
        true
    );
    assert_eq!(
        contract["gates"]["durableRemovalJournal"]["physicalMutationImplemented"],
        true
    );
    assert_eq!(
        contract["gates"]["bindingTranscriptSemantics"]["finalBindingDisposition"],
        "retiredIntoPermanentRemovalTombstone"
    );
    assert_eq!(
        contract["gates"]["bindingTranscriptSemantics"]["transcriptDisposition"],
        "preserved"
    );
    assert_eq!(
        contract["gates"]["bindingTranscriptSemantics"]["tombstoneExecutable"],
        false
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["currentPinnedGitHelperReusable"],
        false
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["physicalMutationImplemented"],
        true
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["publicRemovalEntrypoint"],
        true
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["manifestStorage"],
        "digestAddressedStrictV1Sidecar"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["manifestIdentityPolicy"],
        "deviceInodeBirthTimeAndSupportedGeneration"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["manifestPathPolicy"],
        "sameMountHandleRelativeNamedDirectoryAndFileIdentity"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["verifiedAbsenceCapability"],
        "opaqueSingleUse"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["proofRefNamespace"],
        "refs/schoolx/removal-claims/"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["proofRefTarget"],
        "targetCommit"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["proofRefCleanup"],
        "durableExactCompareAndDelete"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["manifestCleanupMarker"],
        "durableSidecarAbsenceAfterDurableProofRefAbsence"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["removedCleanupOfflinePolicy"],
        "preserveSidecarAndDefer"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["gitAdminLockPolicy"],
        "lockedMarkerOrLockfileReject"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["objectStoragePolicy"],
        "primarySameMountNoFollowNoAlternates"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["refStoragePolicy"],
        "filesBackendWithLooseProofRef"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["mountBoundaryPolicy"],
        "sameMountIdentityNoNestedMounts"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["proofRefRepresentation"],
        "directLooseRegularNoFollow"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["proofRefDurability"],
        "referenceFileAndDirectoryFsync"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["partialDeletionPolicy"],
        "knownPrefixOnly"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["startupRecoveryBefore"],
        json!([
            "runtimeEmitterStart",
            "lifecycleReconciliation",
            "startRecovery",
            "forkRecovery"
        ])
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["pendingRemovalConflictGates"],
        json!(["archivedRename", "turnInterrupt"])
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["physicalManifestPolicy"],
        "dotGitAndTrackedEntriesOnly"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["quarantinePolicy"],
        "atomicNoReplaceParentRelativeRename"
    );
    assert_eq!(
        contract["gates"]["pinnedDeletionBoundary"]["requiredPinnedCoordinates"],
        json!([
            "managedRootParent",
            "managedRoot",
            "quarantineName",
            "gitAdminParent",
            "gitAdminEntry"
        ])
    );
    assert_eq!(
        contract["journalStates"],
        json!([
            {
                "state": "claimed",
                "meaning": "durableProofWithZeroDeletionMutation",
                "rollback": "definitelyNotStartedOnly"
            },
            {
                "state": "removing",
                "meaning": "firstMutationMayHaveOccurred",
                "rollback": "never"
            },
            {
                "state": "removed",
                "meaning": "verifiedAbsenceAndPermanentTranscriptTombstone",
                "rollback": "never"
            }
        ])
    );
    assert_eq!(
        contract["physicalRemovalOrder"],
        json!([
            "loadArchivedAuthority",
            "proveQuiescenceAndCaptureManifest",
            "persistDigestAddressedManifestSidecar",
            "persistClaimed",
            "revalidateAuthorityAndPersistRemoving",
            "pinExactProofRefAndRevalidate",
            "renameRootToQuarantine",
            "deleteManifestFromQuarantine",
            "deleteExactGitAdminEntry",
            "verifyAbsenceAndSiblings",
            "atomicallyRetireBindingIntoTombstone",
            "compareDeleteExactProofRef",
            "durablyRetireManifestSidecar"
        ])
    );

    assert_eq!(contract["currentInventory"]["preserved"], true);
    assert_eq!(contract["currentInventory"]["canRemove"], "eligibleOnly");
    assert_eq!(
        contract["currentInventory"]["archivedBlocker"],
        "mergeProofUnavailableUnlessProven"
    );
    assert_eq!(
        contract["forbiddenOperations"],
        json!([
            "force",
            "gitClean",
            "gitReset",
            "gitWorktreeRemove",
            "gitWorktreePrune",
            "broadRemoveDirAll",
            "implicitArchiveCleanup",
            "implicitForkCleanup",
            "inventoryReceiptReuse",
            "frontendPathOrProofClaim",
            "fetchOrNetworkProof",
            "threadOrTranscriptDelete"
        ])
    );
    assert_eq!(
        contract["acceptanceCases"],
        json!({
            "mergedAuthority": [
                "headEqualsTarget",
                "headAncestorViaMergeCommit",
                "unmergedHead",
                "squashOrCherryPickOnly",
                "legacyAuthorityAbsent",
                "headOrTargetDrift",
                "replacementOrGraftOnly",
                "timeoutOrMissingObject",
                "zeroMutation"
            ],
            "journal": [
                "claimAdmissionFailureZeroMutation",
                "claimedDefinitelyNotStartedCancellation",
                "removingNeverRollsBack",
                "crashAtEveryMutationBoundary",
                "responseLossReturnsSameRemoval",
                "finalStoreFailureRetriesFinalization",
                "startupRecoveryPrecedesOtherReconciliation",
                "pendingRemovalGatesRenameAndInterrupt"
            ],
            "semantics": [
                "liveBindingRetainedBeforeVerifiedAbsence",
                "removedTombstonePreservesTranscriptCoordinate",
                "removedIdentityCannotBeReused",
                "removedTaskCannotExecuteOrUnarchive",
                "noCodexThreadMutation"
            ],
            "deletionBoundary": [
                "ignoredFileRejects",
                "untrackedOrEmptyDirectoryRejects",
                "specialOrCrossDeviceEntryRejects",
                "missingOrAlternateObjectRejects",
                "nonPrefixPartialDeletionRejects",
                "manifestSidecarReplacementFailsClosed",
                "offlineCommonDirCleanupDefers",
                "lockedGitAdminRejects",
                "symlinkIsUnlinkedNotFollowed",
                "originalNameReplacementSurvives",
                "quarantineOrAdminReplacementFailsClosed",
                "proofRefReplacementSurvivesCleanup",
                "siblingRootsRemainUnchanged",
                "unsupportedPlatformZeroMutation"
            ]
        })
    );

    let destructive_verbs = [
        "remove", "delete", "destroy", "cleanup", "clean", "prune", "purge", "discard",
    ];
    let has_worktree_mutation = |command: &str| {
        command.starts_with("code_worktree")
            && destructive_verbs.iter().any(|verb| command.contains(verb))
    };
    let fixture_commands = tauri_contract["commands"]
        .as_array()
        .ok_or_else(|| "missing Tauri command contract".to_string())?;
    let fixture_mutations = fixture_commands
        .iter()
        .filter_map(|command| command["name"].as_str())
        .filter(|name| has_worktree_mutation(name))
        .collect::<Vec<_>>();
    assert_eq!(fixture_mutations, vec!["code_worktree_remove"]);
    let registered_mutations = registered_code_commands()?
        .into_iter()
        .filter(|command| has_worktree_mutation(command))
        .collect::<Vec<_>>();
    assert_eq!(registered_mutations, vec!["code_worktree_remove"]);
    assert!(tauri_contract["strictInputs"]
        .as_object()
        .is_some_and(|inputs| inputs.contains_key("worktreeRemove")));
    assert!(tauri_contract["outputs"]
        .as_object()
        .is_some_and(|outputs| outputs.contains_key("worktreeRemovalReceipt")));
    assert!(COMMAND_SOURCE.contains("code_worktree_remove"));
    assert!(!WORKTREE_INVENTORY_COMMAND_SOURCE.contains("code_worktree_remove"));
    Ok(())
}
