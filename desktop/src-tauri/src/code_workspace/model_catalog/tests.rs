use std::collections::VecDeque;

use serde_json::json;
use tempfile::tempdir;

use super::*;

fn wire_model(model: &str, is_default: bool) -> Value {
    json!({
        "id": format!("id-{model}"),
        "model": model,
        "upgrade": null,
        "upgradeInfo": null,
        "availabilityNux": null,
        "displayName": format!("Display {model}"),
        "description": "Model description",
        "hidden": false,
        "supportedReasoningEfforts": [
            {"reasoningEffort": "minimal", "description": "Fast"},
            {"reasoningEffort": "xhigh", "description": "Deep"}
        ],
        "defaultReasoningEffort": "minimal",
        "inputModalities": ["text", "image"],
        "supportsPersonality": false,
        "additionalSpeedTiers": [],
        "serviceTiers": [],
        "defaultServiceTier": null,
        "isDefault": is_default
    })
}

fn wire_model_0_149(model: &str, is_default: bool) -> Value {
    let mut model = wire_model(model, is_default);
    model["modelSpecialty"] = Value::Null;
    model["multiAgentVersion"] = json!("v2");
    model["upgrade"] = json!("gpt-next");
    model["upgradeInfo"] = json!({
        "model": "gpt-next",
        "migrationMarkdown": null,
        "modelLink": null,
        "retirementAt": 1_800_000_000,
        "upgradeCopy": null
    });
    model
}

#[test]
fn catalog_accepts_the_audited_codex_0_149_model_fields_strictly() -> Result<(), String> {
    let catalog = collect_model_catalog(149, |_| {
        Ok(json!({
            "data": [wire_model_0_149("gpt-5.6-sol", true)],
            "nextCursor": null
        }))
    })?;
    assert_eq!(catalog.runtime_generation, 149);
    assert_eq!(catalog.models.len(), 1);
    assert_eq!(catalog.models[0].model, "gpt-5.6-sol");

    let mut future = wire_model_0_149("gpt-future", true);
    future["multiAgentVersion"] = json!("v3");
    let error = collect_model_catalog(150, |_| {
        Ok(json!({"data": [future.clone()], "nextCursor": null}))
    })
    .expect_err("future multi-agent versions must remain fail closed");
    assert!(error.contains("v3"));
    assert!(error.contains("unknown variant"));
    Ok(())
}

#[test]
fn catalog_paginates_with_native_hidden_and_limit_policy() -> Result<(), String> {
    let mut responses = VecDeque::from([
        json!({"data": [wire_model("gpt-a", true)], "nextCursor": "next"}),
        json!({"data": [wire_model("gpt-b", false)], "nextCursor": null}),
    ]);
    let mut params = Vec::new();
    let catalog = collect_model_catalog(7, |request| {
        params.push(request);
        responses
            .pop_front()
            .ok_or_else(|| "unexpected request".to_string())
    })?;
    assert_eq!(catalog.runtime_generation, 7);
    assert_eq!(catalog.models.len(), 2);
    assert_eq!(
        params,
        vec![
            json!({"includeHidden": false, "limit": 100}),
            json!({"cursor": "next", "includeHidden": false, "limit": 100})
        ]
    );
    Ok(())
}

#[test]
fn catalog_accepts_open_effort_strings_and_rejects_unadvertised_selection() -> Result<(), String> {
    let catalog = collect_model_catalog(3, |_| {
        Ok(json!({"data": [wire_model("gpt-a", true)], "nextCursor": null}))
    })?;
    catalog.require_selection(&CodeModelSelection {
        model: "gpt-a".to_string(),
        reasoning_effort: "xhigh".to_string(),
    })?;
    assert!(catalog
        .require_selection(&CodeModelSelection {
            model: "gpt-a".to_string(),
            reasoning_effort: "medium".to_string(),
        })
        .is_err());
    Ok(())
}

#[test]
fn catalog_rejects_repeated_cursor_hidden_and_duplicate_models() {
    let repeated = collect_model_catalog(1, |_| Ok(json!({"data": [], "nextCursor": "same"})));
    assert!(repeated.is_err());

    let mut hidden = wire_model("hidden", true);
    hidden["hidden"] = json!(true);
    assert!(collect_model_catalog(1, |_| {
        Ok(json!({"data": [hidden.clone()], "nextCursor": null}))
    })
    .is_err());

    assert!(collect_model_catalog(1, |_| {
        Ok(json!({
            "data": [wire_model("same", true), wire_model("same", false)],
            "nextCursor": null
        }))
    })
    .is_err());
}

#[test]
fn catalog_rejects_duplicate_ids_efforts_defaults_and_unsupported_default() {
    let mut duplicate_id = wire_model("gpt-b", false);
    duplicate_id["id"] = json!("id-gpt-a");
    assert!(collect_model_catalog(1, |_| {
        Ok(json!({
            "data": [wire_model("gpt-a", true), duplicate_id.clone()],
            "nextCursor": null
        }))
    })
    .is_err());

    let mut duplicate_effort = wire_model("gpt-a", true);
    duplicate_effort["supportedReasoningEfforts"][1]["reasoningEffort"] = json!("minimal");
    assert!(collect_model_catalog(1, |_| {
        Ok(json!({"data": [duplicate_effort.clone()], "nextCursor": null}))
    })
    .is_err());

    let mut unsupported_default = wire_model("gpt-a", true);
    unsupported_default["defaultReasoningEffort"] = json!("medium");
    assert!(collect_model_catalog(1, |_| {
        Ok(json!({"data": [unsupported_default.clone()], "nextCursor": null}))
    })
    .is_err());

    assert!(collect_model_catalog(1, |_| {
        Ok(json!({
            "data": [wire_model("gpt-a", true), wire_model("gpt-b", true)],
            "nextCursor": null
        }))
    })
    .is_err());
}

#[test]
fn catalog_rejects_unknown_fields_and_pagination_bounds() {
    assert!(collect_model_catalog(1, |_| {
        Ok(json!({
            "data": [wire_model("gpt-a", true)],
            "nextCursor": null,
            "unknown": true
        }))
    })
    .is_err());

    let oversized_page = (0..=MODEL_PAGE_LIMIT)
        .map(|index| wire_model(&format!("gpt-{index}"), index == 0))
        .collect::<Vec<_>>();
    assert!(collect_model_catalog(1, |_| {
        Ok(json!({"data": oversized_page.clone(), "nextCursor": null}))
    })
    .is_err());

    let mut page = 0usize;
    let page_bound = collect_model_catalog(1, |_| {
        page = page.saturating_add(1);
        Ok(json!({
            "data": [],
            "nextCursor": format!("cursor-{page}")
        }))
    });
    assert!(page_bound.is_err_and(|error| error.contains("page safety limit")));
}

#[test]
fn turn_selection_requires_a_complete_pair() -> Result<(), String> {
    assert!(turn_selection(Some("gpt-a"), None).is_err());
    assert!(turn_selection(None, Some("high")).is_err());
    assert_eq!(turn_selection(None, None)?, None);
    assert_eq!(
        turn_selection(Some("gpt-a"), Some("high"))?,
        Some(CodeModelSelection {
            model: "gpt-a".to_string(),
            reasoning_effort: "high".to_string(),
        })
    );
    Ok(())
}

#[test]
fn selection_store_round_trips_and_catalog_reconciles_stale_value() -> Result<(), String> {
    let directory = tempdir().map_err(|error| error.to_string())?;
    let store = CodeModelSelectionStore::for_app_data(directory.path())?;
    let selection = CodeModelSelection {
        model: "gpt-a".to_string(),
        reasoning_effort: "xhigh".to_string(),
    };
    store.save(&selection)?;
    assert_eq!(store.load()?, Some(selection.clone()));

    let catalog = collect_model_catalog(2, |_| {
        Ok(json!({"data": [wire_model("gpt-b", true)], "nextCursor": null}))
    })?;
    assert_eq!(catalog.reconcile_recent_selection(store.load()?), None);
    Ok(())
}

#[test]
fn selection_store_preserves_malformed_existing_bytes() -> Result<(), String> {
    let directory = tempdir().map_err(|error| error.to_string())?;
    let store = CodeModelSelectionStore::for_app_data(directory.path())?;
    fs::write(store.path(), b"{not-json").map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    assert!(store
        .save(&CodeModelSelection {
            model: "gpt-a".to_string(),
            reasoning_effort: "minimal".to_string(),
        })
        .is_err());
    assert_eq!(
        fs::read(store.path()).map_err(|error| error.to_string())?,
        b"{not-json"
    );
    Ok(())
}

#[test]
fn selection_store_rejects_oversized_bytes() -> Result<(), String> {
    let directory = tempdir().map_err(|error| error.to_string())?;
    let store = CodeModelSelectionStore::for_app_data(directory.path())?;
    fs::write(
        store.path(),
        vec![b'x'; MAX_MODEL_SELECTION_BYTES as usize + 1],
    )
    .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    assert!(store.load().is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn selection_store_rejects_non_private_file() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempdir().map_err(|error| error.to_string())?;
    let store = CodeModelSelectionStore::for_app_data(directory.path())?;
    store.save(&CodeModelSelection {
        model: "gpt-a".to_string(),
        reasoning_effort: "minimal".to_string(),
    })?;
    fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644))
        .map_err(|error| error.to_string())?;
    assert!(store.load().is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn selection_store_rejects_symlink_target() -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let directory = tempdir().map_err(|error| error.to_string())?;
    let store = CodeModelSelectionStore::for_app_data(directory.path())?;
    let outside = directory.path().join("outside.json");
    fs::write(&outside, b"outside").map_err(|error| error.to_string())?;
    symlink(&outside, store.path()).map_err(|error| error.to_string())?;
    assert!(store.load().is_err());
    Ok(())
}
