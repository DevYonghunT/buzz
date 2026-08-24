use super::*;

#[test]
fn v4_removal_wire_is_strict_tagged_and_preserves_malformed_bytes() -> Result<(), String> {
    let (_directory, store, seed) = seed_active()?;
    archive(&store, &seed.lookup)?;
    let claimed = claim_with_id(&store, &seed.claim, REMOVAL_ID)?;
    assert!(matches!(claimed, CodeWorktreeRemovalRecord::Claimed(_)));

    let valid_bytes = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let valid: Value = serde_json::from_slice(&valid_bytes).map_err(|error| error.to_string())?;
    let removal = &valid["removals"][0];
    assert_eq!(
        object_keys(removal)?,
        BTreeSet::from([
            "binding".to_string(),
            "executionDisposition".to_string(),
            "mergeProof".to_string(),
            "physical".to_string(),
            "physicalManifestDigest".to_string(),
            "removalId".to_string(),
            "state".to_string(),
            "threadLifecycleAtClaim".to_string(),
            "transcriptDisposition".to_string(),
        ])
    );
    assert_eq!(removal["state"], "claimed");
    assert_eq!(removal["threadLifecycleAtClaim"], "archived");
    assert_eq!(removal["transcriptDisposition"], "preserved");
    assert_eq!(removal["executionDisposition"], "removed");
    assert_eq!(
        removal["binding"].as_object().map(|value| value.len()),
        Some(8)
    );
    assert_eq!(
        object_keys(&removal["mergeProof"])?,
        BTreeSet::from([
            "headCommit".to_string(),
            "repositoryIdentity".to_string(),
            "targetCommit".to_string(),
            "targetRef".to_string(),
            "worktreeId".to_string(),
        ])
    );
    assert_eq!(
        object_keys(&removal["physical"])?,
        BTreeSet::from([
            "gitAdminEntry".to_string(),
            "gitAdminParent".to_string(),
            "managedRoot".to_string(),
            "managedRootParent".to_string(),
            "quarantineName".to_string(),
        ])
    );
    assert_eq!(store.load()?.removals, vec![claimed]);

    let mut fixtures = Vec::new();
    let mut unknown_top = valid.clone();
    unknown_top["removals"][0]["unexpected"] = json!(true);
    fixtures.push(unknown_top);
    let mut unknown_state = valid.clone();
    unknown_state["removals"][0]["state"] = json!("paused");
    fixtures.push(unknown_state);
    let mut missing_state = valid.clone();
    missing_state["removals"][0]
        .as_object_mut()
        .ok_or_else(|| "removal must be an object".to_string())?
        .remove("state");
    fixtures.push(missing_state);
    let mut missing_binding = valid.clone();
    missing_binding["removals"][0]
        .as_object_mut()
        .ok_or_else(|| "removal must be an object".to_string())?
        .remove("binding");
    fixtures.push(missing_binding);
    let mut bad_id = valid.clone();
    bad_id["removals"][0]["removalId"] = json!("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA");
    fixtures.push(bad_id);
    let mut canonical_non_v4_id = valid.clone();
    canonical_non_v4_id["removals"][0]["removalId"] = json!("aaaaaaaa-aaaa-1aaa-8aaa-aaaaaaaaaaaa");
    fixtures.push(canonical_non_v4_id);
    let mut bad_lifecycle = valid.clone();
    bad_lifecycle["removals"][0]["threadLifecycleAtClaim"] = json!("active");
    fixtures.push(bad_lifecycle);
    let mut bad_transcript = valid.clone();
    bad_transcript["removals"][0]["transcriptDisposition"] = json!("deleted");
    fixtures.push(bad_transcript);
    let mut bad_execution = valid.clone();
    bad_execution["removals"][0]["executionDisposition"] = json!("available");
    fixtures.push(bad_execution);
    let mut bad_digest = valid.clone();
    bad_digest["removals"][0]["physicalManifestDigest"] = json!("not-a-digest");
    fixtures.push(bad_digest);
    let mut bad_ref = valid.clone();
    bad_ref["removals"][0]["mergeProof"]["targetRef"] = json!("origin/main");
    fixtures.push(bad_ref);
    let mut mixed_object_format = valid.clone();
    mixed_object_format["removals"][0]["mergeProof"]["targetCommit"] = json!("2".repeat(64));
    fixtures.push(mixed_object_format);
    let mut bad_quarantine = valid.clone();
    bad_quarantine["removals"][0]["physical"]["quarantineName"] = json!("other");
    fixtures.push(bad_quarantine);
    let mut duplicate = valid.clone();
    let duplicate_record = duplicate["removals"][0].clone();
    duplicate["removals"]
        .as_array_mut()
        .ok_or_else(|| "removals must be an array".to_string())?
        .push(duplicate_record);
    fixtures.push(duplicate);
    let mut active_pending = valid.clone();
    active_pending["lifecycles"][0]["lifecycle"] = json!({ "state": "active" });
    fixtures.push(active_pending);
    let mut missing_live = valid.clone();
    missing_live["bindings"] = json!([]);
    missing_live["lifecycles"] = json!([]);
    missing_live["mergeTargets"] = json!([]);
    fixtures.push(missing_live);

    for fixture in fixtures {
        let bytes = write_json(store.store_path(), &fixture)?;
        assert!(
            store.load().is_err(),
            "fixture unexpectedly loaded: {fixture}"
        );
        assert_eq!(
            fs::read(store.store_path()).map_err(|error| error.to_string())?,
            bytes
        );
    }

    let valid_text = String::from_utf8(valid_bytes).map_err(|error| error.to_string())?;
    let duplicate_members = [
        valid_text.replacen(
            "\"state\": \"claimed\",",
            "\"state\": \"claimed\",\n      \"state\": \"claimed\",",
            1,
        ),
        valid_text.replacen(
            &format!("\"headCommit\": \"{}\",", "1".repeat(40)),
            &format!(
                "\"headCommit\": \"{}\",\n        \"headCommit\": \"{}\",",
                "1".repeat(40),
                "1".repeat(40)
            ),
            1,
        ),
    ];
    let mut duplicate_members = duplicate_members
        .map(String::into_bytes)
        .into_iter()
        .collect::<Vec<_>>();
    duplicate_members.push(json_with_duplicate_removals(&valid)?);
    for bytes in duplicate_members {
        fs::write(store.store_path(), &bytes).map_err(|error| error.to_string())?;
        assert!(store.load().is_err());
        assert_eq!(
            fs::read(store.store_path()).map_err(|error| error.to_string())?,
            bytes
        );
    }
    Ok(())
}
