//! Fail-closed lifecycle for a persistent macOS Git XPC authority session.

use super::*;

impl SessionInner {
    fn lifecycle(&self) -> MutexGuard<'_, SessionLifecycle> {
        match self.lifecycle.lock() {
            Ok(lifecycle) => lifecycle,
            Err(poisoned) => {
                let mut lifecycle = poisoned.into_inner();
                if lifecycle.poison_reason.is_none() {
                    lifecycle.poison_reason = Some(
                        "macOS XPC Git session state mutex was poisoned; authority retained"
                            .to_string(),
                    );
                }
                lifecycle
            }
        }
    }

    pub(super) fn on_owner_thread(&self) -> bool {
        std::thread::current().id() == self.owner_thread
    }

    pub(super) fn ensure_owner_or_poison(&self, operation: &str) -> Result<(), String> {
        if self.on_owner_thread() {
            return Ok(());
        }
        let error = format!(
            "macOS XPC Git {operation} moved outside its admitted thread; session {} is fail-closed",
            self.session_id
        );
        self.poison(error.clone());
        Err(error)
    }

    pub(super) fn poison(&self, error: String) {
        let mut lifecycle = self.lifecycle();
        if lifecycle.poison_reason.is_none() {
            lifecycle.poison_reason = Some(error);
        }
    }

    pub(super) fn can_reuse_from(&self, thread: &ThreadId) -> Result<bool, String> {
        if thread != &self.owner_thread {
            let error = format!(
                "macOS XPC Git session {} cannot be reused from another thread",
                self.session_id
            );
            self.poison(error.clone());
            return Err(error);
        }
        let lifecycle = self.lifecycle();
        if let Some(error) = &lifecycle.poison_reason {
            return Err(error.clone());
        }
        if lifecycle.end_complete {
            return Ok(false);
        }
        if lifecycle.close_requested || lifecycle.end_started {
            return Err(format!(
                "macOS XPC Git session {} is already closing",
                self.session_id
            ));
        }
        Ok(true)
    }

    pub(super) fn acquire_child(self: &Arc<Self>, request_id: u64) -> Result<ChildPermit, String> {
        self.ensure_owner_or_poison("child launch")?;
        let mut lifecycle = self.lifecycle();
        if let Some(error) = &lifecycle.poison_reason {
            return Err(error.clone());
        }
        if lifecycle.close_requested || lifecycle.end_started || lifecycle.end_complete {
            return Err(format!(
                "macOS XPC Git session {} is not open for a child",
                self.session_id
            ));
        }
        if lifecycle.active_child.is_some() {
            return Err(format!(
                "macOS XPC Git session {} already has an active child",
                self.session_id
            ));
        }
        let authority = Arc::new(ChildAuthority {
            request_id,
            cleanup_proven: AtomicBool::new(false),
        });
        lifecycle.active_child = Some(Arc::clone(&authority));
        Ok(ChildPermit {
            session: Arc::clone(self),
            authority,
        })
    }

    fn complete_child(&self, authority: &Arc<ChildAuthority>) -> Result<(), String> {
        self.ensure_owner_or_poison("child cleanup")?;
        let should_end = {
            let mut lifecycle = self.lifecycle();
            match lifecycle.active_child.as_ref() {
                Some(active) if Arc::ptr_eq(active, authority) => {
                    lifecycle.active_child = None;
                }
                None if authority.cleanup_proven.load(Ordering::Acquire) => return Ok(()),
                _ => {
                    let error = format!(
                        "macOS XPC Git child authority changed unexpectedly in session {}; authority retained",
                        self.session_id
                    );
                    if lifecycle.poison_reason.is_none() {
                        lifecycle.poison_reason = Some(error.clone());
                    }
                    return Err(error);
                }
            }
            lifecycle.close_requested
                && !lifecycle.close_in_progress
                && !lifecycle.end_started
                && !lifecycle.end_complete
        };
        if should_end {
            self.send_session_end()
        } else {
            Ok(())
        }
    }

    pub(super) fn request_close(&self) -> Result<(), String> {
        self.ensure_owner_or_poison("session close")?;
        let active_child = {
            let mut lifecycle = self.lifecycle();
            if let Some(error) = &lifecycle.poison_reason {
                return Err(error.clone());
            }
            if lifecycle.end_complete {
                return Ok(());
            }
            lifecycle.close_requested = true;
            if lifecycle.close_in_progress {
                return Err(format!(
                    "macOS XPC Git session {} close is already in progress",
                    self.session_id
                ));
            }
            lifecycle.close_in_progress = true;
            lifecycle.active_child.clone()
        };

        let mut cancellation_error = None;
        if let Some(authority) = active_child {
            if !authority.cleanup_proven.load(Ordering::Acquire) {
                let encoded = ffi::schoolx_git_xpc_cancel(authority.request_id);
                let response: CancelResponse = match serde_json::from_str(&encoded) {
                    Ok(response) => response,
                    Err(error) => {
                        let diagnostic = format!(
                            "invalid macOS XPC Git cancel response during session end: {error}; authority retained"
                        );
                        self.poison(diagnostic.clone());
                        return Err(diagnostic);
                    }
                };
                if !child_cleanup_is_proven(
                    response.child_cleanup_proven,
                    response.child_authority_retained,
                ) {
                    let diagnostic = format!(
                        "{}; session {} is fail-closed",
                        response_diagnostic(
                            "macOS XPC Git child cancellation had an ambiguous disposition",
                            &response.error,
                        ),
                        self.session_id
                    );
                    self.poison(diagnostic.clone());
                    return Err(diagnostic);
                }
                if !response.ok {
                    cancellation_error = Some(response_diagnostic(
                        "macOS XPC Git child cancellation reported failure after cleanup",
                        &response.error,
                    ));
                }
                authority.cleanup_proven.store(true, Ordering::Release);
                self.complete_child(&authority)?;
            }
        }

        let end_result = self.send_session_end();
        {
            let mut lifecycle = self.lifecycle();
            lifecycle.close_in_progress = false;
        }
        match (cancellation_error, end_result) {
            (None, result) => result,
            (Some(error), Ok(())) => Err(error),
            (Some(error), Err(end_error)) => Err(format!(
                "{error}; additionally failed to end macOS XPC Git session: {end_error}"
            )),
        }
    }

    fn send_session_end(&self) -> Result<(), String> {
        {
            let mut lifecycle = self.lifecycle();
            if let Some(error) = &lifecycle.poison_reason {
                return Err(error.clone());
            }
            if lifecycle.end_complete {
                return Ok(());
            }
            if lifecycle.active_child.is_some() {
                let error = format!(
                    "macOS XPC Git session {} cannot end before child cleanup proof",
                    self.session_id
                );
                lifecycle.poison_reason = Some(error.clone());
                return Err(error);
            }
            if lifecycle.end_started {
                return Err(format!(
                    "macOS XPC Git session {} end is already in progress",
                    self.session_id
                ));
            }
            lifecycle.end_started = true;
        }

        let encoded = ffi::schoolx_git_xpc_session_end(self.session_id);
        let response: SessionResponse = match serde_json::from_str(&encoded) {
            Ok(response) => response,
            Err(error) => {
                let diagnostic = format!(
                    "invalid macOS XPC Git session-end response: {error}; session {} is fail-closed",
                    self.session_id
                );
                self.poison(diagnostic.clone());
                return Err(diagnostic);
            }
        };
        if response.session_id != self.session_id || !session_cleanup_is_proven(&response) {
            let diagnostic = format!(
                "{}; session {} is fail-closed",
                response_diagnostic(
                    "macOS XPC Git session end had an ambiguous disposition",
                    &response.error,
                ),
                self.session_id
            );
            self.poison(diagnostic.clone());
            return Err(diagnostic);
        }

        if let Err(error) = release_global_session(self.session_id) {
            self.poison(error.clone());
            return Err(error);
        }
        {
            let mut lifecycle = self.lifecycle();
            lifecycle.end_complete = true;
            lifecycle.end_started = false;
        }
        if response.ok {
            Ok(())
        } else {
            Err(response_diagnostic(
                "macOS XPC Git session end reported failure after cleanup",
                &response.error,
            ))
        }
    }
}

impl ChildPermit {
    pub(super) fn ensure_owner(&self, operation: &str) -> Result<(), String> {
        self.session.ensure_owner_or_poison(operation)
    }

    pub(super) fn prove_cleanup(&mut self) -> Result<(), String> {
        self.ensure_owner("child completion")?;
        if self.authority.cleanup_proven.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.session.complete_child(&self.authority)
    }

    pub(super) fn poison(&self, error: String) {
        self.session.poison(error);
    }

    pub(super) fn cleanup_proven(&self) -> bool {
        self.authority.cleanup_proven.load(Ordering::Acquire)
    }
}

impl Drop for ChildPermit {
    fn drop(&mut self) {
        if !self.session.on_owner_thread() {
            self.session.poison(format!(
                "macOS XPC Git child {} was dropped outside its admitted thread; authority retained",
                self.authority.request_id
            ));
        } else if !self.cleanup_proven() {
            self.session.poison(format!(
                "macOS XPC Git child {} lost its permit without cleanup proof; authority retained",
                self.authority.request_id
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session() -> Arc<SessionInner> {
        Arc::new(SessionInner {
            session_id: 41,
            owner_thread: std::thread::current().id(),
            lifecycle: Mutex::new(SessionLifecycle::default()),
        })
    }

    #[test]
    fn child_permit_requires_proof_before_next_child() -> Result<(), String> {
        let session = test_session();
        let mut first = session.acquire_child(1)?;
        assert!(session
            .acquire_child(2)
            .is_err_and(|error| error.contains("active child")));
        first.prove_cleanup()?;
        drop(first);
        let mut second = session.acquire_child(2)?;
        second.prove_cleanup()?;
        Ok(())
    }

    #[test]
    fn unproven_child_drop_poison_retains_session() -> Result<(), String> {
        let session = test_session();
        drop(session.acquire_child(1)?);
        assert!(session
            .acquire_child(2)
            .is_err_and(|error| error.contains("without cleanup proof")));
        Ok(())
    }

    #[test]
    fn wrong_thread_use_poison_retains_session() -> Result<(), String> {
        let session = test_session();
        let moved = Arc::clone(&session);
        let worker = std::thread::spawn(move || moved.ensure_owner_or_poison("test use"));
        let worker_result = worker
            .join()
            .map_err(|_| "wrong-thread authority test panicked".to_string())?;
        assert!(worker_result.is_err());
        assert!(session
            .acquire_child(1)
            .is_err_and(|error| error.contains("outside its admitted thread")));
        Ok(())
    }

    #[test]
    fn dropping_fresh_handle_does_not_close_a_live_clone() {
        let inner = test_session();
        let scope = Arc::new(SessionScope {
            inner: Arc::clone(&inner),
        });
        let fresh = MacGitAuthoritySession {
            scope,
            explicit_end: true,
        };
        let clone = fresh.clone();
        drop(fresh);
        assert_eq!(Arc::strong_count(&clone.scope), 1);
        assert!(inner.lifecycle().poison_reason.is_none());
        inner.lifecycle().end_complete = true;
        drop(clone);
    }

    #[test]
    fn ending_with_a_live_clone_poison_blocks_later_child_launch() {
        let inner = test_session();
        let scope = Arc::new(SessionScope {
            inner: Arc::clone(&inner),
        });
        let fresh = MacGitAuthoritySession {
            scope,
            explicit_end: true,
        };
        let clone = fresh.clone();
        assert!(fresh
            .end()
            .is_err_and(|error| error.contains("live authority handles")));
        assert!(inner
            .acquire_child(7)
            .is_err_and(|error| error.contains("live authority handles")));
        drop(clone);
    }
}
