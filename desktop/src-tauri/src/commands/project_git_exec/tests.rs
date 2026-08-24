use super::{
    capture_git_child_bytes, clean_branch, clean_target_ref, credential_helper_config_value,
    git_needs_credentials, git_subcommand, read_pipe_bounded, validate_clone_url,
    validate_clone_url_against_relay, validate_local_clone_url,
};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[test]
fn credential_helper_config_value_uses_forward_slashes() {
    let path = std::path::PathBuf::from(r"C:\Users\x\AppData\Local\Buzz\git-credential-nostr.exe");
    assert_eq!(
        credential_helper_config_value(&path),
        "C:/Users/x/AppData/Local/Buzz/git-credential-nostr.exe",
    );
}

#[test]
fn git_subcommand_skips_global_config_options() {
    assert_eq!(
        git_subcommand(&[
            "-c",
            "user.name=Buzz User",
            "-c",
            "user.email=user@example.com",
            "merge",
            "HEAD",
        ]),
        Some("merge")
    );
    assert_eq!(
        git_subcommand(&["--config=credential.useHttpPath=true", "fetch", "origin"]),
        Some("fetch")
    );
    assert_eq!(
        git_subcommand(&["--literal-pathspecs", "diff", "HEAD", "--", ":(glob)*"]),
        Some("diff")
    );
}

#[test]
fn pipe_capture_stops_at_its_streaming_byte_limit() {
    let abort = Arc::new(AtomicBool::new(false));
    let result = read_pipe_bounded(
        Some(std::io::Cursor::new(vec![b'x'; 1024 * 1024])),
        64 * 1024,
        "stdout",
        Arc::clone(&abort),
        None,
    );
    assert!(result.is_err());
    assert!(abort.load(Ordering::Acquire));
}

#[cfg(unix)]
#[test]
fn combined_output_budget_kills_descendant_pipe_holders() -> Result<(), String> {
    use std::os::unix::process::CommandExt;

    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("trap '' TERM; (trap '' TERM; sleep 30) & printf 0123456789; printf abcdefghij >&2")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .map_err(|error| format!("spawn capture fixture: {error}"))?;
    let started = std::time::Instant::now();
    let result = capture_git_child_bytes(
        child,
        &[],
        std::time::Duration::from_secs(2),
        64,
        64,
        Some(12),
    );
    assert!(result.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    Ok(())
}

#[test]
fn remote_and_promisor_operations_receive_credentials() {
    assert!(git_needs_credentials(&["fetch", "origin"]));
    assert!(git_needs_credentials(&[
        "-c",
        "user.name=Buzz User",
        "merge",
        "HEAD"
    ]));
    assert!(!git_needs_credentials(&["rev-parse", "HEAD"]));
}

#[test]
fn clean_branch_accepts_plain_and_prefixed_names() {
    assert_eq!(
        clean_branch(Some("refs/heads/feature/x-1".into())),
        Some("feature/x-1".to_string())
    );
    assert_eq!(
        clean_branch(Some(" main ".into())),
        Some("main".to_string())
    );
}

#[test]
fn clean_branch_rejects_flag_shaped_and_traversal_values() {
    assert_eq!(clean_branch(Some("--upload-pack=/tmp/evil".into())), None);
    assert_eq!(clean_branch(Some("-x".into())), None);
    assert_eq!(clean_branch(Some("a/../b".into())), None);
    assert_eq!(clean_branch(Some("/leading".into())), None);
    assert_eq!(clean_branch(Some("trailing/".into())), None);
    assert_eq!(clean_branch(Some("bad name".into())), None);
    assert_eq!(clean_branch(None), None);
}

#[test]
fn clean_target_ref_accepts_only_tags_and_pull_request_refs() {
    assert_eq!(
        clean_target_ref(Some("refs/tags/v1.0.0".into())),
        Some("refs/tags/v1.0.0".to_string())
    );
    assert_eq!(
        clean_target_ref(Some("refs/nostr/abc123".into())),
        Some("refs/nostr/abc123".to_string())
    );
    assert_eq!(clean_target_ref(Some("refs/heads/main".into())), None);
    assert_eq!(clean_target_ref(Some("refs/tags/../main".into())), None);
}

#[test]
fn validate_clone_url_requires_buzz_repo_shape() {
    let owner = "a".repeat(64);
    assert!(validate_clone_url(&format!("https://relay.example/git/{owner}/repo")).is_ok());
    assert!(validate_clone_url(&format!("https://relay.example/prefix/git/{owner}/repo")).is_ok());
    assert!(validate_clone_url("https://relay.example/git/short/repo").is_err());
    assert!(validate_clone_url("https://evil.example/has/git/inpath").is_err());
    assert!(validate_clone_url(&format!("ssh://relay.example/git/{owner}/repo")).is_err());
    assert!(validate_clone_url(&format!(
        "https://relay.example/git/{owner}/repo/unexpected"
    ))
    .is_err());
}

#[test]
fn workspace_clone_url_requires_exact_relay_origin_and_prefix() {
    let owner = "a".repeat(64);
    let valid = format!("https://relay.example/prefix/git/{owner}/repo");
    assert!(validate_clone_url_against_relay(&valid, "https://relay.example/prefix").is_ok());
    assert!(validate_clone_url_against_relay(&valid, "http://relay.example/prefix").is_err());
    assert!(validate_clone_url_against_relay(&valid, "https://relay.example:8443/prefix").is_err());
    assert!(validate_clone_url_against_relay(&valid, "https://relay.example/other").is_err());
    assert!(validate_clone_url_against_relay(
        &format!("https://evil.example/prefix/git/{owner}/repo"),
        "https://relay.example/prefix",
    )
    .is_err());
}

#[test]
fn local_clone_url_allows_only_public_github_https_urls() {
    assert!(validate_local_clone_url("https://github.com/block/buzz").is_ok());
    assert!(validate_local_clone_url("https://github.com/block/buzz.git").is_ok());
    assert!(validate_local_clone_url("http://github.com/block/buzz").is_err());
    assert!(validate_local_clone_url("https://github.com/block/buzz/issues").is_err());
    assert!(validate_local_clone_url("https://user@github.com/block/buzz").is_err());
    assert!(validate_local_clone_url("https://github.com.evil.test/block/buzz").is_err());
    assert!(validate_local_clone_url("https://gitlab.com/block/buzz").is_err());
}
