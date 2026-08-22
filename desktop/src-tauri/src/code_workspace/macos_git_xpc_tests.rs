use super::*;

#[test]
fn process_spec_rejects_executable_replacement() {
    let command = Command::new("/bin/echo");
    assert!(
        process_spec_from_command(&command).is_err_and(|error| error.contains("non-system Git"))
    );
}

#[test]
fn unknown_family_fails_closed() {
    let encoded = schoolx_git_xpc_prepare(255, String::new(), 1, 2, 0, 3, 4, 0, 0);
    assert!(encoded.contains("unknown typed Git family"));
}

#[test]
fn child_cleanup_requires_both_disposition_fields() {
    assert!(child_cleanup_is_proven(true, false));
    assert!(!child_cleanup_is_proven(false, false));
    assert!(!child_cleanup_is_proven(true, true));
}

#[test]
fn session_cleanup_requires_the_exact_clean_disposition() {
    for (cleanup_proven, authority_retained, expected) in [
        (true, false, true),
        (false, false, false),
        (true, true, false),
        (false, true, false),
    ] {
        let response = SessionResponse {
            ok: false,
            session_id: 41,
            session_cleanup_proven: cleanup_proven,
            session_authority_retained: authority_retained,
            error: "rejected".to_string(),
        };
        assert_eq!(session_cleanup_is_proven(&response), expected);
    }
}

#[test]
fn session_response_accepts_only_the_canonical_swift_shape() {
    let response: SessionResponse = serde_json::from_str(
        r#"{"ok":false,"sessionId":41,"sessionCleanupProven":true,"sessionAuthorityRetained":false,"error":"rejected"}"#,
    )
    .expect("Swift session disposition should match the Rust boundary");
    assert_eq!(response.session_id, 41);
    assert!(session_cleanup_is_proven(&response));

    assert!(serde_json::from_str::<SessionResponse>(
        r#"{"ok":false,"sessionId":41,"sessionCleanupProven":true,"sessionAuthorityRetained":false,"error":"rejected","unknown":1}"#,
    )
    .is_err());
    assert!(serde_json::from_str::<SessionResponse>(
        r#"{"ok":false,"sessionId":41,"sessionCleanupProven":true,"error":"rejected"}"#,
    )
    .is_err());
    assert!(serde_json::from_str::<SessionResponse>("not json").is_err());
}

#[test]
fn poll_response_accepts_swift_camel_case_fields() {
    let finished: PollResponse = serde_json::from_str(r#"{"state":"finished","rawStatus":0}"#)
        .expect("Swift Finished payload should match the Rust boundary");
    assert!(matches!(finished, PollResponse::Finished { raw_status: 0 }));

    let failed: PollResponse = serde_json::from_str(
        r#"{"state":"failed","error":"boom","childCleanupProven":true,"childAuthorityRetained":false}"#,
    )
    .expect("Swift Failed payload should match the Rust boundary");
    assert!(matches!(
        failed,
        PollResponse::Failed {
            error,
            child_cleanup_proven: true,
            child_authority_retained: false,
        } if error == "boom"
    ));
}

#[test]
fn poll_response_rejects_noncanonical_snake_case_fields() {
    assert!(
        serde_json::from_str::<PollResponse>(r#"{"state":"finished","raw_status":0}"#,).is_err()
    );
    assert!(serde_json::from_str::<PollResponse>(
        r#"{"state":"failed","error":"boom","child_cleanup_proven":true,"child_authority_retained":false}"#,
    )
    .is_err());
}

#[test]
fn identifier_exhaustion_does_not_wrap() {
    let counter = AtomicU64::new(u64::MAX);
    assert!(next_identifier(&counter, "test").is_err());
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn late_cleanup_callback_releases_only_the_exact_live_session() {
    ACTIVE_SESSION_ID.store(73, Ordering::Release);
    assert!(!schoolx_git_xpc_session_cleanup_proven(0));
    assert!(!schoolx_git_xpc_session_cleanup_proven(72));
    assert_eq!(ACTIVE_SESSION_ID.load(Ordering::Acquire), 73);
    assert!(schoolx_git_xpc_session_cleanup_proven(73));
    assert_eq!(ACTIVE_SESSION_ID.load(Ordering::Acquire), 0);
}
