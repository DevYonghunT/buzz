use super::*;

fn thread(
    id: &str,
    source: Value,
    parent_thread_id: Option<&str>,
    forked_from_id: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "cwd": "/tmp/schoolx-code",
        "source": source,
        "status": { "type": "idle" },
        "parentThreadId": parent_thread_id,
        "forkedFromId": forked_from_id,
    })
}

fn parse_threads(values: Vec<Value>) -> Result<Vec<CodeAuthoritativeThread>, String> {
    parse_authoritative_thread_list(
        json!({ "data": values, "nextCursor": null, "backwardsCursor": null }),
        CodeThreadMembership::Active,
    )
    .map(|page| page.data)
}

#[test]
fn lifecycle_input_and_archive_wire_are_exact() -> Result<(), String> {
    let input = CodeThreadLifecycleInput {
        scope: CodeThreadBindingScope {
            community_id: "community-1".to_string(),
            project_dtag: "project-1".to_string(),
            repository_identity: "a".repeat(64),
        },
        thread_id: "thread-1".to_string(),
    };
    assert_eq!(input.rpc_params()?, json!({ "threadId": "thread-1" }));
    let scope = input.scope;
    for forged in [
        json!({ "scope": scope.clone(), "threadId": "thread-1", "cwd": "/forged" }),
        json!({ "scope": scope.clone(), "threadId": "thread-1", "path": "/forged" }),
        json!({ "scope": scope, "threadId": "thread-1", "operationId": "forged" }),
    ] {
        assert!(serde_json::from_value::<CodeThreadLifecycleInput>(forged).is_err());
    }
    parse_thread_archive(json!({}))?;
    assert!(parse_thread_archive(json!({ "thread": "unexpected" })).is_err());
    let unarchived = parse_thread_unarchive(json!({ "thread": { "id": "thread-1" } }))?;
    assert_eq!(unarchived.id, "thread-1");
    assert!(parse_thread_unarchive(json!({})).is_err());
    Ok(())
}

#[test]
fn authoritative_list_params_have_every_source_and_no_cwd_or_search() -> Result<(), String> {
    let active = authoritative_thread_list_params(CodeThreadMembership::Active, None)?;
    assert_eq!(
        active["sourceKinds"],
        json!(SUPPORTED_CODEX_THREAD_SOURCE_KINDS)
    );
    assert_eq!(active["archived"], false);
    assert_eq!(active["limit"], CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT);
    assert_eq!(active["useStateDbOnly"], false);
    assert_eq!(active["sortDirection"], "desc");
    assert_eq!(active["sortKey"], "created_at");
    assert!(active.get("cwd").is_none());
    assert!(active.get("searchTerm").is_none());

    let archived =
        authoritative_thread_list_params(CodeThreadMembership::Archived, Some("archive-next"))?;
    assert_eq!(archived["archived"], true);
    assert_eq!(archived["cursor"], "archive-next");
    Ok(())
}

#[test]
fn strict_source_parser_accepts_every_pinned_shape() -> Result<(), String> {
    let values = vec![
        thread("thread-cli", json!("cli"), None, None),
        thread("thread-vscode", json!("vscode"), None, None),
        thread("thread-exec", json!("exec"), None, None),
        thread("thread-app", json!("appServer"), None, None),
        thread(
            "thread-review",
            json!({ "subAgent": "review" }),
            Some("thread-app"),
            None,
        ),
        thread(
            "thread-compact",
            json!({ "subAgent": "compact" }),
            Some("thread-app"),
            None,
        ),
        thread(
            "thread-memory",
            json!({ "subAgent": "memory_consolidation" }),
            Some("thread-app"),
            None,
        ),
        thread(
            "thread-spawn",
            json!({
                "subAgent": {
                    "thread_spawn": {
                        "depth": 1,
                        "parent_thread_id": "thread-app",
                        "agent_nickname": null,
                        "agent_path": null,
                        "agent_role": null
                    }
                }
            }),
            Some("thread-app"),
            None,
        ),
        thread(
            "thread-other",
            json!({ "subAgent": { "other": "fixture" } }),
            Some("thread-app"),
            None,
        ),
    ];
    assert_eq!(parse_threads(values)?.len(), 9);
    for rejected in [
        json!("unknown"),
        json!({ "custom": "extension" }),
        json!("newSource"),
        json!({ "subAgent": "newSubAgent" }),
        json!({ "subAgent": { "thread_spawn": { "depth": 1 } } }),
    ] {
        assert!(parse_threads(vec![thread("thread-rejected", rejected, None, None)]).is_err());
    }
    for parentless in [
        json!({ "subAgent": "review" }),
        json!({ "subAgent": { "other": "fixture" } }),
    ] {
        assert!(parse_threads(vec![thread("thread-parentless", parentless, None, None,)]).is_err());
    }
    Ok(())
}

#[test]
fn deferred_schoolx_thread_accepts_real_0_149_vscode_source_only_with_marker() -> Result<(), String>
{
    let deferred = |source: &str, thread_source: Option<&str>| {
        json!({
            "thread": {
                "id": "thread-deferred",
                "sessionId": "thread-deferred",
                "cwd": "/tmp/schoolx-code",
                "source": source,
                "status": { "type": "idle" },
                "ephemeral": false,
                "parentThreadId": null,
                "forkedFromId": null,
                "threadSource": thread_source,
                "turns": []
            }
        })
    };
    let marker = "schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb";
    for source in ["appServer", "vscode"] {
        assert_eq!(
            parse_authoritative_deferred_bound_thread_read(deferred(source, Some(marker)))?.id,
            "thread-deferred"
        );
    }
    for rejected in [
        deferred("cli", Some(marker)),
        deferred("vscode", None),
        deferred("vscode", Some("schoolx-code/not-a-uuid")),
    ] {
        assert!(parse_authoritative_deferred_bound_thread_read(rejected).is_err());
    }
    Ok(())
}

#[test]
fn graph_rejects_duplicates_missing_conflicts_and_cycles() -> Result<(), String> {
    let duplicate = parse_threads(vec![
        thread("thread-a", json!("appServer"), None, None),
        thread("thread-a", json!("appServer"), None, None),
    ])?;
    assert!(CodeAuthoritativeThreadGraph::from_threads(duplicate).is_err());

    let missing = parse_threads(vec![thread(
        "thread-child",
        json!({ "subAgent": "review" }),
        Some("thread-missing"),
        None,
    )])?;
    assert!(CodeAuthoritativeThreadGraph::from_threads(missing).is_err());

    assert!(parse_threads(vec![thread(
        "thread-conflict",
        json!("appServer"),
        Some("thread-a"),
        Some("thread-b"),
    )])
    .is_err());

    assert!(parse_threads(vec![thread(
        "thread-source-conflict",
        json!({
            "subAgent": {
                "thread_spawn": {
                    "depth": 1,
                    "parent_thread_id": "thread-source-parent"
                }
            }
        }),
        Some("thread-metadata-parent"),
        None,
    )])
    .is_err());

    let cycle = parse_threads(vec![
        thread(
            "thread-a",
            json!({ "subAgent": "review" }),
            Some("thread-b"),
            None,
        ),
        thread(
            "thread-b",
            json!({ "subAgent": "compact" }),
            Some("thread-a"),
            None,
        ),
    ])?;
    assert!(CodeAuthoritativeThreadGraph::from_threads(cycle).is_err());

    let oversized = (0..=MAX_AUTHORITATIVE_THREADS).map(|index| CodeAuthoritativeThread {
        id: format!("thread-{index}"),
        membership: CodeThreadMembership::Active,
        cwd: "/tmp/schoolx-code".to_string(),
        parent_thread_id: None,
        forked_from_id: None,
        status: CodePinnedThreadStatus::Idle,
    });
    assert!(CodeAuthoritativeThreadGraph::from_threads(oversized)
        .is_err_and(|error| error.contains("thread safety limit")));
    Ok(())
}

#[test]
fn graph_leaf_gate_rejects_transitive_descendant_and_accepts_leaf() -> Result<(), String> {
    let threads = parse_threads(vec![
        thread("thread-root", json!("appServer"), None, None),
        thread(
            "thread-child",
            json!({ "subAgent": "review" }),
            Some("thread-root"),
            None,
        ),
        thread(
            "thread-leaf",
            json!({ "subAgent": "compact" }),
            Some("thread-child"),
            None,
        ),
    ])?;
    let graph = CodeAuthoritativeThreadGraph::from_threads(threads)?;
    assert!(graph.ensure_leaf("thread-root").is_err());
    assert_eq!(
        graph.ensure_leaf("thread-leaf")?,
        CodeThreadMembership::Active
    );
    assert!(graph.ensure_leaf("thread-missing").is_err());
    Ok(())
}

#[test]
fn authoritative_read_requires_pinned_idle_status_and_turn_status() -> Result<(), String> {
    assert_eq!(
        authoritative_thread_read_params("thread-1")?,
        json!({ "threadId": "thread-1", "includeTurns": true })
    );
    let idle = parse_authoritative_thread_read(json!({
        "thread": {
            "id": "thread-1",
            "cwd": "/tmp/schoolx-code",
            "source": "appServer",
            "status": { "type": "idle" },
            "turns": [{ "id": "turn-1", "status": "completed", "items": [] }]
        }
    }))?;
    idle.ensure_quiescent()?;

    let active = parse_authoritative_thread_read(json!({
        "thread": {
            "id": "thread-1",
            "cwd": "/tmp/schoolx-code",
            "source": "appServer",
            "status": { "type": "active", "activeFlags": [] },
            "turns": [{ "id": "turn-2", "status": "inProgress", "items": [] }]
        }
    }))?;
    assert!(active.ensure_quiescent().is_err());

    assert!(parse_authoritative_thread_read(json!({
        "thread": {
            "id": "thread-1",
            "cwd": "/tmp/schoolx-code",
            "source": "appServer",
            "status": { "type": "newStatus" },
            "turns": []
        }
    }))
    .is_err());
    assert!(parse_authoritative_thread_read(json!({
        "thread": {
            "id": "thread-1",
            "cwd": "/tmp/schoolx-code",
            "source": "appServer",
            "status": { "type": "idle" },
            "turns": [{ "id": "turn-3", "status": "futureStatus", "items": [] }]
        }
    }))
    .is_err());
    assert!(parse_authoritative_thread_read(json!({
        "thread": {
            "id": "thread-parentless",
            "cwd": "/tmp/schoolx-code",
            "source": { "subAgent": "review" },
            "status": { "type": "idle" },
            "parentThreadId": null,
            "forkedFromId": null,
            "turns": []
        }
    }))
    .is_err());
    Ok(())
}
