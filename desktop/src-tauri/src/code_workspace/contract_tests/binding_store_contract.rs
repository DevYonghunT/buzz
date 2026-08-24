use super::*;

#[test]
fn binding_store_fixture_reloads_and_public_list_scrubs_recovery_baseline() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = directory.path().join("execution-root");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(directory.path())?;
    let payload = STORE_FIXTURE.replace("{{EXECUTION_ROOT}}", &root.to_string_lossy());
    fs::write(store.store_path(), payload).map_err(|error| error.to_string())?;

    let loaded = store.load()?;
    assert_eq!(loaded.version, CODE_THREAD_BINDING_SCHEMA_VERSION);
    assert_eq!(loaded.bindings.len(), 1);
    assert_eq!(loaded.preparations.len(), 1);
    let scope = CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: "a".repeat(64),
    };
    let public = store.list_preparations(&scope)?;
    assert_eq!(public.len(), 1);
    let public_value = serde_json::to_value(&public[0]).map_err(|error| error.to_string())?;
    assert!(public_value.get("recoveryThreadBaseline").is_none());
    assert!(public_value.get("mergeTargetRef").is_none());
    let mut expected = fixture(TAURI_CONTRACT)?["outputs"]["preparationPublicBaseline"].clone();
    expected["executionRoot"] = json!(root.to_string_lossy());
    assert_eq!(public_value, expected);

    let reopened = CodeThreadBindingStore::for_app_data(directory.path())?;
    assert_eq!(reopened.load()?, loaded);
    Ok(())
}
