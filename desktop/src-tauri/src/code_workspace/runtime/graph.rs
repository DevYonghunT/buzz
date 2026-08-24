use super::*;

#[cfg(test)]
pub(super) fn collect_authoritative_thread_graph(
    deferred_targets: &HashSet<String>,
    request: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Result<CodeAuthoritativeThreadGraph, String> {
    collect_authoritative_thread_graph_with_pending_forks(deferred_targets, &[], request)
}

pub(super) fn collect_authoritative_thread_graph_with_pending_forks(
    deferred_targets: &HashSet<String>,
    pending_forks: &[CodePendingForkExpectation],
    mut request: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Result<CodeAuthoritativeThreadGraph, String> {
    validate_pending_fork_expectations(pending_forks)?;
    let mut threads = Vec::new();
    for membership in [CodeThreadMembership::Active, CodeThreadMembership::Archived] {
        let mut cursor = None;
        let mut seen_cursors = HashSet::new();
        for _ in 0..MAX_AUTHORITATIVE_PAGES {
            let params = authoritative_thread_list_params(membership, cursor.as_deref())?;
            let page =
                parse_authoritative_thread_list(request("thread/list", params)?, membership)?;
            if threads.len().saturating_add(page.data.len()) > MAX_AUTHORITATIVE_THREADS {
                return Err(format!(
                    "Codex authoritative graph exceeds the {MAX_AUTHORITATIVE_THREADS}-thread safety limit"
                ));
            }
            threads.extend(page.data);
            match page.next_cursor {
                Some(next_cursor) => {
                    if !seen_cursors.insert(next_cursor.clone()) {
                        return Err(format!(
                            "Codex {membership:?} thread pagination repeated a cursor"
                        ));
                    }
                    cursor = Some(next_cursor);
                }
                None => {
                    cursor = None;
                    break;
                }
            }
        }
        if cursor.is_some() {
            return Err(format!(
                "Codex {membership:?} thread pagination exceeded the {MAX_AUTHORITATIVE_PAGES}-page safety limit"
            ));
        }
    }

    // Codex 0.145 can defer a just-created thread in memory, omitting it from
    // thread/list. Exhaustively merge loaded ids from the same generation;
    // exact bound ids and Starting fork journals use separate strict parsers.
    let mut listed_ids = threads
        .iter()
        .map(|thread| thread.id.clone())
        .collect::<HashSet<_>>();
    let mut loaded_ids = HashSet::new();
    let mut matched_pending_forks = HashSet::new();
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();
    for _ in 0..MAX_AUTHORITATIVE_PAGES {
        let params = protocol::loaded_thread_list_params(cursor.as_deref())?;
        let page = protocol::parse_loaded_thread_list(request("thread/loaded/list", params)?)?;
        if page.data.len() > CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT as usize {
            return Err(format!(
                "Codex loaded thread list exceeded the {CODE_AUTHORITATIVE_THREAD_PAGE_LIMIT}-thread page limit"
            ));
        }
        for thread_id in page.data {
            if !loaded_ids.insert(thread_id.clone()) {
                return Err(format!(
                    "Codex loaded thread list contained duplicate thread id {thread_id}"
                ));
            }
            if loaded_ids.len() > MAX_AUTHORITATIVE_THREADS {
                return Err(format!(
                    "Codex loaded thread inventory exceeds the {MAX_AUTHORITATIVE_THREADS}-thread safety limit"
                ));
            }
            if listed_ids.contains(&thread_id) {
                continue;
            }
            if !deferred_targets.contains(&thread_id) && pending_forks.is_empty() {
                return Err(format!(
                    "Codex loaded thread {thread_id} was absent from both authoritative memberships"
                ));
            }
            let params = protocol::thread_read_params(&thread_id)?;
            let value = request("thread/read", params)?;
            let thread = if deferred_targets.contains(&thread_id) {
                parse_authoritative_deferred_bound_thread_read(value)?
            } else {
                let mut matches = Vec::new();
                for expectation in pending_forks {
                    if matched_pending_forks.contains(&expectation.preparation_id) {
                        continue;
                    }
                    if let Ok(candidate) =
                        parse_authoritative_pending_fork_thread_read(value.clone(), expectation)
                    {
                        matches.push((expectation.preparation_id.clone(), candidate));
                    }
                }
                match matches.len() {
                    1 => {
                        let (preparation_id, candidate) = matches.remove(0);
                        matched_pending_forks.insert(preparation_id);
                        candidate
                    }
                    0 => {
                        return Err(format!(
                            "Codex loaded thread {thread_id} was absent from both authoritative memberships and did not match a pending fork"
                        ));
                    }
                    count => {
                        return Err(format!(
                            "Codex loaded thread {thread_id} matched {count} pending fork journals"
                        ));
                    }
                }
            };
            if thread.id != thread_id {
                return Err("Codex loaded-thread read returned a different thread id".to_string());
            }
            if threads.len() >= MAX_AUTHORITATIVE_THREADS {
                return Err(format!(
                    "Codex authoritative graph exceeds the {MAX_AUTHORITATIVE_THREADS}-thread safety limit"
                ));
            }
            listed_ids.insert(thread_id);
            threads.push(thread);
        }
        match page.next_cursor {
            Some(next_cursor) => {
                if !seen_cursors.insert(next_cursor.clone()) {
                    return Err("Codex loaded thread pagination repeated a cursor".to_string());
                }
                cursor = Some(next_cursor);
            }
            None => {
                cursor = None;
                break;
            }
        }
    }
    if cursor.is_some() {
        return Err(format!(
            "Codex loaded thread pagination exceeded the {MAX_AUTHORITATIVE_PAGES}-page safety limit"
        ));
    }
    CodeAuthoritativeThreadGraph::from_threads(threads)
}

pub(super) fn validate_pending_fork_expectations(
    pending_forks: &[CodePendingForkExpectation],
) -> Result<(), String> {
    let mut preparation_ids = HashSet::with_capacity(pending_forks.len());
    for expectation in pending_forks {
        protocol::validate_id("pending fork preparation", &expectation.preparation_id)?;
        protocol::validate_id("pending fork source", &expectation.source_thread_id)?;
        if canonical_workspace_root(&expectation.execution_root)? != expectation.execution_root {
            return Err("SchoolX pending fork root is not canonical".to_string());
        }
        if !preparation_ids.insert(expectation.preparation_id.as_str()) {
            return Err(
                "SchoolX pending fork expectations contain a duplicate journal".to_string(),
            );
        }
        let mut previous = None;
        for thread_id in &expectation.recovery_thread_baseline {
            protocol::validate_id("pending fork baseline thread", thread_id)?;
            if previous.is_some_and(|candidate: &String| candidate >= thread_id) {
                return Err(
                    "SchoolX pending fork recovery baseline is not strictly sorted".to_string(),
                );
            }
            previous = Some(thread_id);
        }
    }
    Ok(())
}

pub(super) fn validate_deferred_targets(
    deferred_target_ids: &[String],
) -> Result<HashSet<String>, String> {
    let mut deferred_targets = HashSet::with_capacity(deferred_target_ids.len());
    for thread_id in deferred_target_ids {
        protocol::validate_id("deferred authoritative target", thread_id)?;
        if !deferred_targets.insert(thread_id.clone()) {
            return Err("Codex deferred target list contained a duplicate id".to_string());
        }
    }
    Ok(deferred_targets)
}

pub(super) fn validate_recovery_candidate_root(
    candidate: &CodeRecoveryThread,
    expected_root: &str,
) -> Result<(), String> {
    if recovery_candidate_matches_root(candidate, expected_root)? {
        Ok(())
    } else {
        Err("Codex exact-root thread list returned a thread outside the requested root".to_string())
    }
}

pub(super) fn recovery_candidate_matches_root(
    candidate: &CodeRecoveryThread,
    expected_root: &str,
) -> Result<bool, String> {
    let Some(reported_root) = candidate.thread.cwd.as_deref() else {
        return Err("Codex recovery thread did not report an execution root".to_string());
    };
    match canonical_workspace_root(reported_root) {
        Ok(reported_root) => Ok(reported_root == expected_root),
        // Loaded threads are global to the app-server and can legitimately
        // point at a checkout that was removed. Such a thread cannot match the
        // live canonical SchoolX root and is ignored.
        Err(_) if reported_root != expected_root => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn respond_to_pending_approval(
    approvals: &PendingApprovalStore,
    input: &CodeApprovalResponseInput,
    respond: impl FnOnce(Value, Value) -> Result<(), String>,
) -> Result<(), String> {
    let reservation = approvals.reserve_response(input)?;
    let (request_id, result) = reservation.wire_response();
    match respond(request_id, result) {
        Ok(()) => approvals.commit_response(&reservation),
        Err(response_error) => match approvals.restore_response(&reservation) {
            Ok(()) => Err(response_error),
            Err(restore_error) => Err(format!(
                "{response_error}; failed to restore pending Codex approval: {restore_error}"
            )),
        },
    }
}

pub(super) fn refresh_process_health(
    inner: &mut RuntimeInner,
    approvals: &PendingApprovalStore,
    events: &EventBridge,
) {
    let failure = inner
        .process
        .as_mut()
        .and_then(RuntimeProcess::health_error);
    if let Some(error) = failure {
        let stop_error = stop_runtime_process(inner).err();
        let _ = events.clear_activity(inner.generation);
        approvals.clear_generation(inner.generation);
        inner.phase = CodeRuntimePhase::Failed;
        inner.initialized = None;
        inner.last_error = Some(match stop_error {
            Some(stop_error) => {
                format!("{error}; failed to verify app-server shutdown: {stop_error}")
            }
            None => error,
        });
    }
}

pub(super) fn stop_runtime_process(inner: &mut RuntimeInner) -> Result<(), String> {
    let Some(mut process) = inner.process.take() else {
        return Ok(());
    };
    match process.stop() {
        Ok(()) => Ok(()),
        Err(error) => {
            inner.process = Some(process);
            Err(error)
        }
    }
}

pub(super) fn status_from_inner(
    inner: &RuntimeInner,
    queued_notifications: usize,
) -> CodeRuntimeStatus {
    let initialized = inner.initialized.as_ref();
    let probe = inner
        .probe
        .as_ref()
        .map(CodeRuntimeProbe::redacted_for_egress);
    CodeRuntimeStatus {
        phase: inner.phase,
        generation: inner.generation,
        executable: probe.as_ref().and_then(|probe| probe.executable.clone()),
        version: probe.as_ref().and_then(|probe| probe.version.clone()),
        pid: inner.process.as_ref().map(|process| process.child.id()),
        user_agent: initialized.map(|result| protocol::redact_protocol_text(&result.user_agent)),
        codex_home: initialized.map(|result| protocol::redact_protocol_text(&result.codex_home)),
        platform_family: initialized
            .map(|result| protocol::redact_protocol_text(&result.platform_family)),
        platform_os: initialized.map(|result| protocol::redact_protocol_text(&result.platform_os)),
        queued_notifications,
        last_error: inner
            .last_error
            .as_deref()
            .map(protocol::redact_protocol_text),
    }
}
