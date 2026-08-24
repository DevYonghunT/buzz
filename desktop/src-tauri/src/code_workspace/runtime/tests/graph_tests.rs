use super::*;

#[test]
fn authoritative_graph_paginates_both_memberships_and_loaded_deferred_threads() -> Result<(), String>
{
    let thread = |id: &str, parent: Option<&str>| {
        json!({
            "id": id,
            "cwd": "/tmp/schoolx-code",
            "source": "appServer",
            "status": { "type": "idle" },
            "parentThreadId": parent,
            "forkedFromId": null
        })
    };
    let mut list_calls = 0_usize;
    let mut loaded_calls = 0_usize;
    let mut read_calls = 0_usize;
    let deferred_targets = HashSet::from(["thread-deferred".to_string()]);
    let graph =
        collect_authoritative_thread_graph(&deferred_targets, |method, params| match method {
            "thread/list" => {
                list_calls = list_calls.saturating_add(1);
                assert!(params.get("cwd").is_none());
                assert!(params.get("searchTerm").is_none());
                assert_eq!(
                    params["sourceKinds"],
                    json!(
                        super::super::super::thread_lifecycle::SUPPORTED_CODEX_THREAD_SOURCE_KINDS
                    )
                );
                let archived = params["archived"].as_bool().unwrap_or(false);
                let cursor = params.get("cursor").and_then(Value::as_str);
                Ok(match (archived, cursor) {
                    (false, None) => json!({
                        "data": [thread("thread-root", None)],
                        "nextCursor": "active-next"
                    }),
                    (false, Some("active-next")) => json!({
                        "data": [thread("thread-child", Some("thread-root"))],
                        "nextCursor": null
                    }),
                    (true, None) => json!({
                        "data": [thread("thread-archived-a", None)],
                        "nextCursor": "archived-next"
                    }),
                    (true, Some("archived-next")) => json!({
                        "data": [thread("thread-archived-b", None)],
                        "nextCursor": null
                    }),
                    _ => return Err("unexpected authoritative list page".to_string()),
                })
            }
            "thread/loaded/list" => {
                loaded_calls = loaded_calls.saturating_add(1);
                let cursor = params.get("cursor").and_then(Value::as_str);
                Ok(match cursor {
                    None => json!({ "data": ["thread-deferred"], "nextCursor": "loaded-next" }),
                    Some("loaded-next") => json!({ "data": [], "nextCursor": null }),
                    _ => return Err("unexpected loaded list page".to_string()),
                })
            }
            "thread/read" if params["threadId"] == "thread-deferred" => {
                read_calls = read_calls.saturating_add(1);
                assert_eq!(
                    params,
                    json!({ "threadId": "thread-deferred", "includeTurns": false })
                );
                Ok(json!({
                    "thread": {
                        "id": "thread-deferred",
                        "sessionId": "thread-deferred",
                        "cwd": "/tmp/schoolx-code",
                        "source": "vscode",
                        "threadSource": "schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb",
                        "status": { "type": "idle" },
                        "ephemeral": false,
                        "parentThreadId": null,
                        "forkedFromId": null,
                        "turns": []
                    }
                }))
            }
            _ => Err(format!("unexpected authoritative method {method}")),
        })?;

    assert_eq!(list_calls, 4);
    assert_eq!(loaded_calls, 2);
    assert_eq!(read_calls, 1);
    for active in ["thread-root", "thread-child", "thread-deferred"] {
        assert_eq!(graph.membership(active), Some(CodeThreadMembership::Active));
    }
    for archived in ["thread-archived-a", "thread-archived-b"] {
        assert_eq!(
            graph.membership(archived),
            Some(CodeThreadMembership::Archived)
        );
    }
    assert_eq!(
        graph.membership("thread-deferred"),
        Some(CodeThreadMembership::Active)
    );
    assert!(graph.ensure_leaf("thread-root").is_err());
    Ok(())
}
#[test]
fn authoritative_graph_admits_only_the_exact_list_absent_pending_fork() -> Result<(), String> {
    let destination = tempfile::tempdir().map_err(|error| error.to_string())?;
    let destination_root = destination
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
    let pending = [CodePendingForkExpectation {
        preparation_id: preparation_id.to_string(),
        source_thread_id: "thread-source".to_string(),
        execution_root: destination_root.clone(),
        recovery_thread_baseline: vec!["thread-before".to_string()],
    }];
    let graph = collect_authoritative_thread_graph_with_pending_forks(
        &HashSet::from(["thread-source".to_string()]),
        &pending,
        |method, params| match method {
            "thread/list" if params["archived"] == false => Ok(json!({
                "data": [{
                    "id": "thread-source",
                    "cwd": "/tmp/source",
                    "source": "appServer",
                    "status": { "type": "idle" }
                }],
                "nextCursor": null
            })),
            "thread/list" => Ok(json!({ "data": [], "nextCursor": null })),
            "thread/loaded/list" => Ok(json!({ "data": ["thread-child"], "nextCursor": null })),
            "thread/read" => {
                assert_eq!(
                    params,
                    json!({ "threadId": "thread-child", "includeTurns": false })
                );
                Ok(json!({
                    "thread": {
                        "id": "thread-child",
                        "sessionId": "thread-child",
                        "cwd": destination_root,
                        "source": "vscode",
                        "threadSource": format!("schoolx-code/{preparation_id}"),
                        "status": { "type": "idle" },
                        "ephemeral": false,
                        "parentThreadId": null,
                        "forkedFromId": "thread-source",
                        "turns": []
                    }
                }))
            }
            _ => Err(format!("unexpected authoritative method {method}")),
        },
    )?;

    assert_eq!(
        graph.membership("thread-child"),
        Some(CodeThreadMembership::Active)
    );
    assert!(graph.ensure_leaf("thread-source").is_err());

    let wrong_marker = collect_authoritative_thread_graph_with_pending_forks(
        &HashSet::from(["thread-source".to_string()]),
        &pending,
        |method, params| match method {
            "thread/list" if params["archived"] == false => Ok(json!({
                "data": [{
                    "id": "thread-source",
                    "cwd": "/tmp/source",
                    "source": "appServer",
                    "status": { "type": "idle" }
                }],
                "nextCursor": null
            })),
            "thread/list" => Ok(json!({ "data": [], "nextCursor": null })),
            "thread/loaded/list" => Ok(json!({ "data": ["thread-child"], "nextCursor": null })),
            "thread/read" => Ok(json!({
                "thread": {
                    "id": "thread-child",
                    "sessionId": "thread-child",
                    "cwd": destination_root,
                    "source": "appServer",
                    "threadSource": "schoolx-code/wrong-preparation",
                    "status": { "type": "idle" },
                    "ephemeral": false,
                    "parentThreadId": null,
                    "forkedFromId": "thread-source",
                    "turns": []
                }
            })),
            _ => Err(format!("unexpected authoritative method {method}")),
        },
    );
    assert!(wrong_marker.is_err_and(|error| error.contains("did not match a pending fork")));
    Ok(())
}

#[test]
fn authoritative_graph_rejects_membership_duplicates_and_cursor_cycles() {
    let duplicate =
        collect_authoritative_thread_graph(&HashSet::new(), |method, params| match method {
            "thread/list" => Ok(json!({
                "data": [{
                    "id": "thread-duplicate",
                    "cwd": "/tmp/schoolx-code",
                    "source": "appServer",
                    "status": { "type": "idle" }
                }],
                "nextCursor": null
            })),
            "thread/loaded/list" => Ok(json!({ "data": [], "nextCursor": null })),
            _ => Err(format!("unexpected method {method}: {params}")),
        });
    assert!(duplicate.is_err_and(|error| error.contains("duplicate thread id")));

    let cycle =
        collect_authoritative_thread_graph(&HashSet::new(), |method, params| match method {
            "thread/list" => Ok(json!({
                "data": [],
                "nextCursor": "same-cursor"
            })),
            _ => Err(format!("unexpected method {method}: {params}")),
        });
    assert!(cycle.is_err_and(|error| error.contains("repeated a cursor")));
}

#[test]
fn authoritative_graph_rejects_unbound_or_nonempty_list_absent_loaded_threads() {
    let foreign = collect_authoritative_thread_graph(&HashSet::new(), |method, _params| {
        Ok(match method {
            "thread/list" => json!({ "data": [], "nextCursor": null }),
            "thread/loaded/list" => {
                json!({ "data": ["thread-foreign"], "nextCursor": null })
            }
            _ => return Err(format!("unexpected method {method}")),
        })
    });
    assert!(foreign.is_err_and(|error| error.contains("absent from both")));

    let allowed = HashSet::from(["thread-nonempty".to_string()]);
    let nonempty = collect_authoritative_thread_graph(&allowed, |method, params| {
        Ok(match method {
            "thread/list" => json!({ "data": [], "nextCursor": null }),
            "thread/loaded/list" => {
                json!({ "data": ["thread-nonempty"], "nextCursor": null })
            }
            "thread/read" if params["threadId"] == "thread-nonempty" => json!({
                "thread": {
                    "id": "thread-nonempty",
                    "sessionId": "thread-nonempty",
                    "cwd": "/tmp/schoolx-code",
                    "source": "appServer",
                    "status": { "type": "idle" },
                    "ephemeral": false,
                    "parentThreadId": null,
                    "forkedFromId": null,
                    "turns": [{ "id": "turn-1", "status": "completed", "items": [] }]
                }
            }),
            _ => return Err(format!("unexpected method {method}")),
        })
    });
    assert!(nonempty.is_err_and(|error| error.contains("quiescent SchoolX root or fork")));
}

#[test]
fn authoritative_graph_rejects_page_bound_exhaustion() {
    let mut page = 0_usize;
    let result =
        collect_authoritative_thread_graph(&HashSet::new(), |method, params| match method {
            "thread/list" => {
                page = page.saturating_add(1);
                Ok(json!({
                    "data": [],
                    "nextCursor": format!("cursor-{page}")
                }))
            }
            _ => Err(format!("unexpected method {method}: {params}")),
        });
    assert_eq!(page, MAX_AUTHORITATIVE_PAGES);
    assert!(result.is_err_and(|error| error.contains("page safety limit")));
}
