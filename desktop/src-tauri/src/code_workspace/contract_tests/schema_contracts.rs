use super::*;

#[test]
fn schema_manifest_freezes_the_audited_codex_0_145_0_contract() -> Result<(), String> {
    let manifest = fixture(SCHEMA_MANIFEST)?;
    assert_eq!(manifest["snapshotSchemaVersion"], 1);
    assert_eq!(
        manifest["source"],
        json!({
            "cliVersion": "codex-cli 0.145.0",
            "generator": "app-server generate-json-schema",
            "experimental": false,
            "generatedFileCount": 273,
            "canonicalization": "jq -S -c output including final LF",
            "aggregateFormat": "relative paths sorted bytewise, then <canonical-file-sha256><two spaces><relative-path><LF>",
            "fullGeneratedSetSha256": "757aa191b6d452c6e6d05f6c1f1cb093b9f673da2d185a29ee8d5d96feae67a8",
            "selectedLeafSchemasSha256": "b8d695b56e3ea5255857e2eb2dc9685d5ad65b735f276a5c743363d792677c73",
            "selectedSchemaCount": 66,
            "selectedSchemasSha256": "1ce5af96175ce83bb1d91db7939e8dcc243984255cf44777f19e58e0afe6549a",
            "selectedSchemaArtifact": "codex-0.145.0-selected-schemas.tar.gz.base64",
            "selectedSchemaArtifactEncoding": "base64(gzip(tar(canonical jq -S -c files)))",
            "executableSha256": "1da3f4e0e96028b8a771814293c3033dafd1971f943f6c7e79b0897fe705f590",
            "executableHashIsInformational": true
        })
    );
    assert_eq!(
        manifest["runtimeVersionRequirement"],
        "codex-cli 0.145.<numeric patch>"
    );
    assert_eq!(manifest["provenSnapshotVersion"], "codex-cli 0.145.0");
    assert_eq!(manifest["clientNotifications"]["initialized"], Value::Null);

    let schemas = manifest["schemas"]
        .as_array()
        .ok_or_else(|| "missing schemas".to_string())?;
    assert_eq!(schemas.len(), 66);
    assert_eq!(manifest["source"]["selectedSchemaCount"], 66);
    let paths = schemas
        .iter()
        .map(|schema| schema[1].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(paths.iter().copied().collect::<HashSet<_>>().len(), 66);
    assert!(schemas.iter().all(|schema| {
        schema[0]
            .as_str()
            .is_some_and(|hash| hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit()))
    }));
    assert_eq!(
        manifest_aggregate(&manifest["schemas"])?,
        manifest["source"]["selectedSchemasSha256"]
    );
    assert_eq!(
        manifest["source"]["selectedSchemasSha256"],
        "1ce5af96175ce83bb1d91db7939e8dcc243984255cf44777f19e58e0afe6549a"
    );
    assert_eq!(
        manifest["source"]["selectedLeafSchemasSha256"],
        "b8d695b56e3ea5255857e2eb2dc9685d5ad65b735f276a5c743363d792677c73"
    );
    assert_eq!(
        manifest["source"]["fullGeneratedSetSha256"],
        "757aa191b6d452c6e6d05f6c1f1cb093b9f673da2d185a29ee8d5d96feae67a8"
    );

    let methods = manifest["methods"]
        .as_object()
        .ok_or_else(|| "missing method map".to_string())?;
    assert_eq!(methods.len(), 14);
    assert_eq!(
        manifest["notifications"].as_object().map(|map| map.len()),
        Some(23)
    );
    assert_eq!(
        manifest["serverRequests"].as_object().map(|map| map.len()),
        Some(3)
    );
    assert_eq!(
        manifest["dispatchSchemas"].as_array().map(Vec::len),
        Some(4)
    );
    Ok(())
}

#[test]
fn schema_manifest_freezes_the_audited_codex_0_149_0_contract() -> Result<(), String> {
    let manifest = fixture(SCHEMA_MANIFEST_0_149)?;
    assert_eq!(manifest["snapshotSchemaVersion"], 1);
    assert_eq!(
        manifest["source"],
        json!({
            "cliVersion": "codex-cli 0.149.0",
            "generator": "app-server generate-json-schema",
            "experimental": false,
            "generatedFileCount": 291,
            "canonicalization": "jq -S -c output including final LF",
            "aggregateFormat": "relative paths sorted bytewise, then <canonical-file-sha256><two spaces><relative-path><LF>",
            "fullGeneratedSetSha256": "cb215283cdd5a870f56ffae341e7809bbb9640eebc19db6974dc1cf54a66851f",
            "selectedLeafSchemasSha256": "e63d7bffa5e8b25c99d24af47b185e002c98356d03ab0552c5dbb11a95474afb",
            "selectedSchemaCount": 66,
            "selectedSchemasSha256": "5a8cb724fc8073bb4bedad9f6ad18f470edae6f3b38f31fe8d65a87898bf674b",
            "selectedSchemaArtifact": "codex-0.149.0-selected-schemas.tar.gz.base64",
            "selectedSchemaArtifactEncoding": "base64(gzip(tar(canonical jq -S -c files)))",
            "executableSha256": "f4a74117b8142cda581c95ff753abf4508b5636d89682c1ed77e4a9249af8963",
            "executableHashIsInformational": true
        })
    );
    assert_eq!(
        manifest["runtimeVersionRequirement"],
        "codex-cli 0.149.<numeric patch>"
    );
    assert_eq!(manifest["provenSnapshotVersion"], "codex-cli 0.149.0");

    let schemas = manifest["schemas"]
        .as_array()
        .ok_or_else(|| "missing Codex 0.149 schemas".to_string())?;
    assert_eq!(schemas.len(), 66);
    assert_eq!(manifest["source"]["selectedSchemaCount"], 66);
    assert_eq!(
        manifest_aggregate(&manifest["schemas"])?,
        manifest["source"]["selectedSchemasSha256"]
    );
    assert_eq!(
        manifest["source"]["selectedSchemasSha256"],
        "5a8cb724fc8073bb4bedad9f6ad18f470edae6f3b38f31fe8d65a87898bf674b"
    );
    assert_eq!(
        manifest["source"]["selectedLeafSchemasSha256"],
        "e63d7bffa5e8b25c99d24af47b185e002c98356d03ab0552c5dbb11a95474afb"
    );
    assert_eq!(
        manifest["source"]["fullGeneratedSetSha256"],
        "cb215283cdd5a870f56ffae341e7809bbb9640eebc19db6974dc1cf54a66851f"
    );
    assert_eq!(
        manifest["compatibilityWithBaseline"],
        json!({
            "baselineVersion": "codex-cli 0.145.0",
            "retainedSelectedSchemaPaths": 66,
            "exactUnchangedSelectedSchemas": 45,
            "exactChangedSelectedSchemas": 21,
            "structurallyUnchangedSelectedSchemas": 45,
            "structurallyChangedSelectedSchemas": 21,
            "numericRepresentationOnlyChanges": 0,
            "schoolxRequestPropertiesRemoved": [],
            "requiredThreadFieldsAdded": ["projectId"],
            "strictModelParserPropertiesAdded": {
                "Model": ["modelSpecialty", "multiAgentVersion"],
                "ModelUpgradeInfo": ["retirementAt"]
            }
        })
    );
    Ok(())
}

fn schema_properties(schema: &Value, label: &str) -> Result<BTreeSet<String>, String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{label} has no object properties"))
        .map(|properties| properties.keys().cloned().collect())
}

fn schema_required(schema: &Value, label: &str) -> Result<BTreeSet<String>, String> {
    match schema.get("required") {
        Some(required) => string_set(required, label),
        None => Ok(BTreeSet::new()),
    }
}

fn schema_definition<'a>(
    schemas: &'a BTreeMap<String, (Value, Vec<u8>)>,
    path: &str,
    definition: &str,
) -> Result<&'a Value, String> {
    schemas
        .get(path)
        .and_then(|(schema, _)| schema.get("definitions"))
        .and_then(|definitions| definitions.get(definition))
        .ok_or_else(|| format!("{path} is missing definition {definition}"))
}

fn schemas_are_structurally_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left.as_f64() == right.as_f64(),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| schemas_are_structurally_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left)| {
                    right
                        .get(key)
                        .is_some_and(|right| schemas_are_structurally_equal(left, right))
                })
        }
        _ => left == right,
    }
}

#[test]
fn codex_0_149_schema_delta_preserves_schoolx_requests_and_freezes_parser_additions(
) -> Result<(), String> {
    let baseline_manifest = fixture(SCHEMA_MANIFEST)?;
    let next_manifest = fixture(SCHEMA_MANIFEST_0_149)?;
    assert_eq!(baseline_manifest["methods"], next_manifest["methods"]);
    assert_eq!(
        baseline_manifest["serverRequests"],
        next_manifest["serverRequests"]
    );

    let baseline = selected_schema_artifact(SELECTED_SCHEMA_ARCHIVE)?;
    let next = selected_schema_artifact(SELECTED_SCHEMA_ARCHIVE_0_149)?;
    assert_eq!(
        baseline.keys().collect::<Vec<_>>(),
        next.keys().collect::<Vec<_>>()
    );
    let exact_unchanged = baseline
        .iter()
        .filter(|(path, (_, bytes))| next.get(*path).is_some_and(|(_, next)| next == bytes))
        .count();
    let structurally_unchanged = baseline
        .iter()
        .filter(|(path, (schema, _))| {
            next.get(*path)
                .is_some_and(|(next_schema, _)| schemas_are_structurally_equal(schema, next_schema))
        })
        .count();
    assert_eq!(exact_unchanged, 45);
    assert_eq!(baseline.len().saturating_sub(exact_unchanged), 21);
    assert_eq!(structurally_unchanged, 45);
    assert_eq!(baseline.len().saturating_sub(structurally_unchanged), 21);
    assert_eq!(structurally_unchanged.saturating_sub(exact_unchanged), 0);

    for schemas in next_manifest["methods"]
        .as_object()
        .ok_or_else(|| "Codex 0.149 manifest has no method map".to_string())?
        .values()
    {
        let request_path = schemas
            .as_array()
            .and_then(|schemas| schemas.first())
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex method has no request schema".to_string())?;
        let old = &baseline
            .get(request_path)
            .ok_or_else(|| format!("baseline is missing {request_path}"))?
            .0;
        let new = &next
            .get(request_path)
            .ok_or_else(|| format!("Codex 0.149 is missing {request_path}"))?
            .0;
        let removed = schema_properties(old, request_path)?
            .difference(&schema_properties(new, request_path)?)
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            removed.is_empty(),
            "request properties removed from {request_path}: {removed:?}"
        );
        assert_eq!(
            schema_required(old, request_path)?,
            schema_required(new, request_path)?,
            "request required fields drifted for {request_path}"
        );
    }

    let old_thread = schema_definition(&baseline, "v2/ThreadReadResponse.json", "Thread")?;
    let new_thread = schema_definition(&next, "v2/ThreadReadResponse.json", "Thread")?;
    let added_thread_required = schema_required(new_thread, "Codex 0.149 Thread")?
        .difference(&schema_required(old_thread, "Codex 0.145 Thread")?)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        added_thread_required,
        BTreeSet::from(["projectId".to_string()])
    );

    let old_model = schema_definition(&baseline, "v2/ModelListResponse.json", "Model")?;
    let new_model = schema_definition(&next, "v2/ModelListResponse.json", "Model")?;
    let added_model_properties = schema_properties(new_model, "Codex 0.149 Model")?
        .difference(&schema_properties(old_model, "Codex 0.145 Model")?)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        added_model_properties,
        BTreeSet::from([
            "modelSpecialty".to_string(),
            "multiAgentVersion".to_string()
        ])
    );
    let old_upgrade =
        schema_definition(&baseline, "v2/ModelListResponse.json", "ModelUpgradeInfo")?;
    let new_upgrade = schema_definition(&next, "v2/ModelListResponse.json", "ModelUpgradeInfo")?;
    let added_upgrade_properties = schema_properties(new_upgrade, "Codex 0.149 ModelUpgradeInfo")?
        .difference(&schema_properties(
            old_upgrade,
            "Codex 0.145 ModelUpgradeInfo",
        )?)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        added_upgrade_properties,
        BTreeSet::from(["retirementAt".to_string()])
    );
    Ok(())
}

fn assert_selected_schema_artifact_recomputes_every_manifest_hash(
    schema_manifest: &str,
    selected_schema_archive: &str,
) -> Result<(), String> {
    let manifest = fixture(schema_manifest)?;
    let archived = selected_schema_artifact(selected_schema_archive)?;
    let entries = manifest["schemas"]
        .as_array()
        .ok_or_else(|| "manifest schemas must be an array".to_string())?;
    assert_eq!(archived.len(), entries.len());
    let expected_paths = entries
        .iter()
        .filter_map(|entry| entry.get(1).and_then(Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        archived.keys().cloned().collect::<BTreeSet<_>>(),
        expected_paths
    );

    let dispatch_schemas = string_set(&manifest["dispatchSchemas"], "dispatch schemas")?;
    let mut aggregate = String::new();
    let mut leaf_aggregate = String::new();
    for entry in entries {
        let expected_hash = entry[0]
            .as_str()
            .ok_or_else(|| "manifest schema hash must be a string".to_string())?;
        let path = entry[1]
            .as_str()
            .ok_or_else(|| "manifest schema path must be a string".to_string())?;
        let (schema, raw) = archived
            .get(path)
            .ok_or_else(|| format!("selected schema artifact is missing {path}"))?;
        if raw.last() != Some(&b'\n') || raw[..raw.len().saturating_sub(1)].contains(&b'\n') {
            return Err(format!(
                "archived schema is not canonical one-line JSON: {path}"
            ));
        }
        assert_eq!(
            &canonical_schema_bytes(schema.clone())?,
            raw,
            "archived schema key order drifted for {path}"
        );
        let actual_hash = sha256_hex(raw);
        assert_eq!(actual_hash, expected_hash, "schema hash drifted for {path}");
        aggregate.push_str(&actual_hash);
        aggregate.push_str("  ");
        aggregate.push_str(path);
        aggregate.push('\n');
        if !dispatch_schemas.contains(path) {
            leaf_aggregate.push_str(&actual_hash);
            leaf_aggregate.push_str("  ");
            leaf_aggregate.push_str(path);
            leaf_aggregate.push('\n');
        }
    }
    assert_eq!(
        sha256_hex(aggregate.as_bytes()),
        manifest["source"]["selectedSchemasSha256"]
    );
    assert_eq!(
        sha256_hex(leaf_aggregate.as_bytes()),
        manifest["source"]["selectedLeafSchemasSha256"]
    );
    Ok(())
}

#[test]
fn codex_0_145_selected_schema_artifact_recomputes_every_manifest_hash() -> Result<(), String> {
    assert_selected_schema_artifact_recomputes_every_manifest_hash(
        SCHEMA_MANIFEST,
        SELECTED_SCHEMA_ARCHIVE,
    )
}

#[test]
fn codex_0_149_selected_schema_artifact_recomputes_every_manifest_hash() -> Result<(), String> {
    assert_selected_schema_artifact_recomputes_every_manifest_hash(
        SCHEMA_MANIFEST_0_149,
        SELECTED_SCHEMA_ARCHIVE_0_149,
    )
}

fn assert_wire_fixture_conforms_to_the_curated_schema_shapes(
    schema_manifest: &str,
    wire_fixture: &str,
    selected_schema_archive: &str,
) -> Result<(), String> {
    let manifest = fixture(schema_manifest)?;
    let wire = fixture(wire_fixture)?;
    let selected_schemas = selected_schema_artifact(selected_schema_archive)?;
    let mut referenced_schemas = BTreeSet::new();

    assert_eq!(wire["initialize"]["request"]["method"], "initialize");
    let initialize_schemas = manifest["methods"]["initialize"]
        .as_array()
        .filter(|schemas| schemas.len() == 2)
        .ok_or_else(|| "initialize schema map must contain params and response".to_string())?;
    for (schema, value) in [
        (
            initialize_schemas[0]
                .as_str()
                .ok_or_else(|| "initialize params schema must be a string".to_string())?,
            &wire["initialize"]["request"]["params"],
        ),
        (
            initialize_schemas[1]
                .as_str()
                .ok_or_else(|| "initialize response schema must be a string".to_string())?,
            &wire["initialize"]["response"],
        ),
    ] {
        assert_wire_schema(&manifest, &selected_schemas, schema, value, "initialize")?;
        referenced_schemas.insert(schema.to_string());
    }
    assert_eq!(
        wire["initialize"]["initializedNotification"]["method"],
        "initialized"
    );

    let method_fixtures = [
        ("model/list", "modelList"),
        ("thread/archive", "threadArchive"),
        ("thread/fork", "threadFork"),
        ("thread/start", "threadStart"),
        ("thread/list", "threadList"),
        ("thread/loaded/list", "threadLoadedList"),
        ("thread/read", "threadRead"),
        ("thread/name/set", "threadNameSet"),
        ("thread/resume", "threadResume"),
        ("thread/unarchive", "threadUnarchive"),
        ("turn/start", "turnStart"),
        ("turn/steer", "turnSteer"),
        ("turn/interrupt", "turnInterrupt"),
    ];
    for (method, fixture_key) in method_fixtures {
        assert_eq!(wire[fixture_key]["method"], method);
        let schemas = manifest["methods"][method]
            .as_array()
            .filter(|schemas| schemas.len() == 2)
            .ok_or_else(|| format!("{method} schema map must contain params and response"))?;
        for (schema, value) in [
            (
                schemas[0]
                    .as_str()
                    .ok_or_else(|| format!("{method} params schema must be a string"))?,
                &wire[fixture_key]["params"],
            ),
            (
                schemas[1]
                    .as_str()
                    .ok_or_else(|| format!("{method} response schema must be a string"))?,
                &wire[fixture_key]["result"],
            ),
        ] {
            assert_wire_schema(&manifest, &selected_schemas, schema, value, method)?;
            referenced_schemas.insert(schema.to_string());
        }
    }

    let notification_map = manifest["notifications"]
        .as_object()
        .ok_or_else(|| "missing notification schema map".to_string())?;
    let mut notification_methods = BTreeSet::new();
    for notification in wire["notifications"]
        .as_array()
        .ok_or_else(|| "wire notifications must be an array".to_string())?
    {
        let method = notification["method"]
            .as_str()
            .ok_or_else(|| "wire notification method must be a string".to_string())?;
        let schema = notification_map
            .get(method)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing notification schema for {method}"))?;
        assert_wire_schema(
            &manifest,
            &selected_schemas,
            schema,
            &notification["params"],
            method,
        )?;
        notification_methods.insert(method.to_string());
        referenced_schemas.insert(schema.to_string());
    }
    assert_eq!(
        notification_methods,
        notification_map.keys().cloned().collect()
    );
    let fork_started = wire["notifications"]
        .as_array()
        .and_then(|notifications| {
            notifications
                .iter()
                .find(|notification| notification["method"] == "thread/started")
        })
        .ok_or_else(|| "missing representative fork thread/started notification".to_string())?;
    assert_eq!(fork_started["params"]["thread"]["id"], "thread-2");
    assert_eq!(fork_started["params"]["thread"]["forkedFromId"], "thread-1");
    assert_eq!(fork_started["params"]["thread"]["turns"], json!([]));

    let server_request_map = manifest["serverRequests"]
        .as_object()
        .ok_or_else(|| "missing server request schema map".to_string())?;
    let mut server_request_methods = BTreeSet::new();
    for approval in wire["approvals"]
        .as_array()
        .ok_or_else(|| "wire approvals must be an array".to_string())?
    {
        let method = approval["request"]["method"]
            .as_str()
            .ok_or_else(|| "approval method must be a string".to_string())?;
        let schemas = server_request_map
            .get(method)
            .and_then(Value::as_array)
            .filter(|schemas| schemas.len() == 2)
            .ok_or_else(|| format!("missing approval schema pair for {method}"))?;
        for (schema, value) in [
            (
                schemas[0]
                    .as_str()
                    .ok_or_else(|| format!("{method} params schema must be a string"))?,
                &approval["request"]["params"],
            ),
            (
                schemas[1]
                    .as_str()
                    .ok_or_else(|| format!("{method} response schema must be a string"))?,
                &approval["wireResult"],
            ),
        ] {
            assert_wire_schema(&manifest, &selected_schemas, schema, value, method)?;
            referenced_schemas.insert(schema.to_string());
        }
        server_request_methods.insert(method.to_string());
    }
    assert_eq!(
        server_request_methods,
        server_request_map.keys().cloned().collect()
    );
    assert_eq!(
        referenced_schemas,
        manifest["schemaShapes"]
            .as_object()
            .ok_or_else(|| "missing curated schema shapes".to_string())?
            .keys()
            .cloned()
            .collect()
    );

    let facts = &manifest["structuralFacts"];
    for thread in [
        &wire["threadFork"]["result"]["thread"],
        &wire["threadStart"]["result"]["thread"],
        &wire["threadRead"]["result"]["thread"],
        &wire["threadResume"]["result"]["thread"],
    ] {
        assert_required_fields(thread, &facts["threadRequired"], "Codex Thread")?;
        if facts["threadSourceRequiredBySchoolX"] == true && thread.get("threadSource").is_none() {
            return Err("SchoolX wire thread is missing threadSource".to_string());
        }
    }
    for response in [
        &wire["threadFork"]["result"],
        &wire["threadStart"]["result"],
        &wire["threadResume"]["result"],
    ] {
        assert_required_fields(
            response,
            &facts["threadOpenResponseRequired"],
            "Codex thread open response",
        )?;
    }
    for turn in [
        &wire["turnStart"]["result"]["turn"],
        &wire["threadFork"]["result"]["thread"]["turns"][0],
        &wire["threadResume"]["result"]["thread"]["turns"][0],
    ] {
        assert_required_fields(turn, &facts["turnRequired"], "Codex Turn")?;
        let status = turn["status"]
            .as_str()
            .ok_or_else(|| "Codex Turn status must be a string".to_string())?;
        if !string_set(&facts["turnStatuses"], "turn statuses")?.contains(status) {
            return Err(format!(
                "unsupported Codex Turn status in fixture: {status}"
            ));
        }
    }
    for item in [
        &wire["threadFork"]["result"]["thread"]["turns"][0]["items"][0],
        &wire["threadResume"]["result"]["thread"]["turns"][0]["items"][0],
    ] {
        assert_required_fields(
            item,
            &facts["agentMessageItemRequired"],
            "Codex agentMessage item",
        )?;
    }
    Ok(())
}

#[test]
fn codex_0_145_wire_fixture_conforms_to_the_curated_schema_shapes() -> Result<(), String> {
    assert_wire_fixture_conforms_to_the_curated_schema_shapes(
        SCHEMA_MANIFEST,
        WIRE_FIXTURE,
        SELECTED_SCHEMA_ARCHIVE,
    )
}

#[test]
fn codex_0_149_wire_fixture_conforms_to_the_curated_schema_shapes() -> Result<(), String> {
    assert_wire_fixture_conforms_to_the_curated_schema_shapes(
        SCHEMA_MANIFEST_0_149,
        WIRE_FIXTURE_0_149,
        SELECTED_SCHEMA_ARCHIVE_0_149,
    )
}
