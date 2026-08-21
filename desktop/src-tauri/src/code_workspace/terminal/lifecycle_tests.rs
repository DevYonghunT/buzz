use super::*;

fn scope() -> CodeThreadBindingScope {
    CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: "a".repeat(64),
    }
}

#[test]
fn terminate_owner_waits_for_exact_actor_ack_and_absence_is_idempotent() -> Result<(), String> {
    let manager = CodeTerminalManager::new();
    let owner = SessionOwner {
        scope: scope(),
        thread_id: "thread-1".to_string(),
    };
    let session_id = "session-1".to_string();
    let (control_tx, _control_rx) = mpsc::sync_channel(1);
    let (terminate_tx, terminate_rx) = mpsc::channel();
    lock_manager(&manager.inner)?.sessions.insert(
        session_id.clone(),
        SessionEntry {
            owner: owner.clone(),
            control_tx,
            terminate_tx,
            closing: false,
        },
    );

    let actor_manager = Arc::clone(&manager.inner);
    let actor = thread::spawn(move || -> Result<(), String> {
        let control = terminate_rx.recv().map_err(|error| error.to_string())?;
        lock_manager(&actor_manager)?.sessions.remove(&session_id);
        if let Some(reply) = control.reply {
            let _ = reply.send(Ok(()));
        }
        Ok(())
    });

    manager.terminate_owner(&owner.scope, &owner.thread_id)?;
    actor
        .join()
        .map_err(|_| "synthetic terminal actor panicked".to_string())??;
    assert!(lock_manager(&manager.inner)?.sessions.is_empty());
    manager.terminate_owner(&owner.scope, &owner.thread_id)?;
    Ok(())
}

#[test]
fn terminate_owner_refuses_an_owner_still_opening() -> Result<(), String> {
    let manager = CodeTerminalManager::new();
    let owner = SessionOwner {
        scope: scope(),
        thread_id: "thread-1".to_string(),
    };
    lock_manager(&manager.inner)?
        .opening_owners
        .insert(owner.clone());
    assert!(manager
        .terminate_owner(&owner.scope, &owner.thread_id)
        .is_err());
    Ok(())
}
