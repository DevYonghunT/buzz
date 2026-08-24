#[cfg(target_os = "linux")]
use super::process::*;
use super::{proof_refs::*, *};

#[cfg(test)]
pub(in crate::code_workspace::bindings::removal::physical) fn execute_helper() -> Result<(), String>
{
    let encoded = std::env::var(HELPER_ENV)
        .map_err(|_| "removal Git helper request was missing or not UTF-8".to_string())?;
    if encoded.len() > MAX_HELPER_REQUEST_BYTES {
        return Err(format!(
            "removal Git helper request exceeds {MAX_HELPER_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: RemovalGitEnvelope = serde_json::from_str(&encoded)
        .map_err(|error| format!("invalid removal Git helper request: {error}"))?;
    validate_helper_envelope(&envelope)?;
    let stdin = std::io::stdin();
    let stat = rustix::fs::fstat(stdin.as_fd())
        .map_err(|error| format!("failed to inspect removal Git helper directory: {error}"))?;
    if !FileType::from_raw_mode(stat.st_mode).is_dir()
        || stat.st_dev as u64 != envelope.target_device
        || stat.st_ino as u64 != envelope.target_inode
    {
        return Err("removal Git helper directory identity did not match".to_string());
    }
    rustix::process::fchdir(stdin.as_fd())
        .map_err(|error| format!("failed to enter removal Git helper directory: {error}"))?;
    let current = fs::metadata(".")
        .map_err(|error| format!("failed to verify removal Git helper cwd: {error}"))?;
    if current.dev() != envelope.target_device || current.ino() != envelope.target_inode {
        return Err("removal Git helper changed to a different directory".to_string());
    }
    let mut command = helper_git_command(&envelope.request)?;
    if let RemovalGitRequest::BlobTypes { object_ids, .. } = &envelope.request {
        command.stdin(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start removal blob inspector: {error}"))?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| "removal blob inspector did not expose stdin".to_string())?;
        for object_id in object_ids {
            writeln!(input, "{object_id}")
                .map_err(|error| format!("failed to write removal blob request: {error}"))?;
        }
        drop(input);
        let status = child
            .wait()
            .map_err(|error| format!("failed to wait for removal blob inspector: {error}"))?;
        if !status.success() {
            return Err(format!(
                "removal blob inspector exited with status {status}"
            ));
        }
        // The helper normally replaces itself with Git. This one operation
        // needs a pipe, so terminate the isolated helper after Git succeeds;
        // returning would let the Rust test harness append bytes to stdout.
        std::process::exit(0);
    }
    let error = command.exec();
    Err(format!("failed to execute removal Git helper: {error}"))
}

#[cfg(target_os = "linux")]
pub(super) fn run_direct_git(
    launch: &RemovalGitLaunchAuthority,
    target: &fs::File,
    request: &RemovalGitRequest,
    deadline: Instant,
) -> Result<CapturedChild, String> {
    let authority = &launch.direct;
    let timeout = remaining_timeout(deadline)?;
    if Path::new(request.git_executable()) != authority.path() {
        return Err(
            "removal Git request did not match its root-trusted launch authority".to_string(),
        );
    }

    let (stdin, input) = match request {
        RemovalGitRequest::BlobTypes { object_ids, .. } => {
            let (reader, writer) = std::io::pipe()
                .map_err(|error| format!("failed to create removal blob input pipe: {error}"))?;
            let mut payload = Vec::with_capacity(object_ids.len().saturating_mul(66));
            for object_id in object_ids {
                payload.extend_from_slice(object_id.as_bytes());
                payload.push(b'\n');
            }
            (Stdio::from(reader), Some((writer, payload)))
        }
        _ => (Stdio::null(), None),
    };
    let mut command = authority.command();
    configure_helper_git_command(command.command_mut(), request)?;
    let mut child = authority.spawn(target, command, stdin)?;
    let input_writer = input.map(|(mut writer, payload)| {
        std::thread::spawn(move || {
            writer
                .write_all(&payload)
                .map_err(|error| format!("failed to write removal blob request: {error}"))
        })
    });
    let captured = capture_child(&mut child, timeout);
    let written = match input_writer {
        Some(writer) => writer
            .join()
            .map_err(|_| "removal blob input writer panicked".to_string())?,
        None => Ok(()),
    };
    match (captured, written) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(captured), Ok(())) => Ok(captured),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn strip_removal_test_harness_output(output: Vec<u8>) -> Vec<u8> {
    #[cfg(test)]
    {
        output
            .strip_prefix(b"\nrunning 1 test\n")
            .unwrap_or(&output)
            .to_vec()
    }
    #[cfg(not(test))]
    output
}

#[cfg(all(test, target_os = "linux"))]
#[test]
pub(super) fn linux_onnxruntime_startup_diagnostic_filter_is_exact() {
    let diagnostic =
        b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: 15\n";
    assert!(
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(
            diagnostic.to_vec()
        )
        .is_empty()
    );
    assert!(
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(
            b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: 0".to_vec(),
        )
        .is_empty()
    );

    let followed_by_error = [diagnostic.as_slice(), b"fatal: protected helper error\n"].concat();
    assert_eq!(
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(
            followed_by_error
        ),
        b"fatal: protected helper error\n"
    );
    assert_eq!(
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(
            b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: forged\n"
                .to_vec(),
        ),
        b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: forged\n"
    );
}

pub(super) fn validate_helper_envelope(envelope: &RemovalGitEnvelope) -> Result<(), String> {
    if envelope.version != HELPER_VERSION {
        return Err(format!(
            "unsupported removal Git helper version {}",
            envelope.version
        ));
    }
    let expected = envelope.request.expected_target_path();
    if !expected.is_absolute() {
        return Err("removal Git helper target must be absolute".to_string());
    }
    let git = Path::new(envelope.request.git_executable());
    if !git.is_absolute() {
        return Err("removal Git helper executable must be absolute".to_string());
    }
    let canonical_git = git
        .canonicalize()
        .map_err(|error| format!("failed to resolve removal Git executable: {error}"))?;
    if canonical_git != git || !canonical_git.is_file() {
        return Err("removal Git helper executable is not canonical".to_string());
    }
    match &envelope.request {
        RemovalGitRequest::Status {
            disabled_filter_keys,
            ..
        } => validate_filter_keys(disabled_filter_keys),
        RemovalGitRequest::ReadProofRef { removal_id, .. } => {
            validate_native_removal_id(removal_id)
        }
        RemovalGitRequest::CreateProofRef {
            removal_id,
            target_commit,
            zero_oid,
            ..
        } => {
            validate_native_removal_id(removal_id)?;
            validate_commit_id(target_commit)?;
            if zero_oid != zero_oid_for(target_commit)? {
                return Err("removal Git helper zero object id is invalid".to_string());
            }
            Ok(())
        }
        RemovalGitRequest::DeleteProofRef {
            removal_id,
            target_commit,
            ..
        } => {
            validate_native_removal_id(removal_id)?;
            validate_commit_id(target_commit)
        }
        RemovalGitRequest::HeadEntries { head_commit, .. } => validate_commit_id(head_commit),
        RemovalGitRequest::BlobTypes { object_ids, .. } => {
            if object_ids.is_empty() || object_ids.len() > MAX_OBJECT_TYPE_BATCH {
                return Err("removal Git helper blob batch has an invalid size".to_string());
            }
            let mut previous = None;
            for object_id in object_ids {
                validate_commit_id(object_id)?;
                if previous.is_some_and(|value: &String| value >= object_id) {
                    return Err(
                        "removal Git helper blob batch must be strictly ordered".to_string()
                    );
                }
                previous = Some(object_id);
            }
            Ok(())
        }
        RemovalGitRequest::LocalConfig { .. }
        | RemovalGitRequest::WorktreeConfigNames { .. }
        | RemovalGitRequest::IndexEntries { .. }
        | RemovalGitRequest::RefFormat { .. } => Ok(()),
    }
}

/// Decode and revalidate one closed removal-Git envelope inside the signed
/// macOS service, then derive its fixed `/usr/bin/git` process specification.
#[cfg(target_os = "macos")]
pub(in crate::code_workspace) fn prepare_macos_removal_git(
    payload: &str,
    cwd: DescriptorObservation,
    stdin: DescriptorObservation,
) -> Result<MacGitProcessSpec, String> {
    if payload.len() > MAX_HELPER_REQUEST_BYTES {
        return Err(format!(
            "removal Git helper request exceeds {MAX_HELPER_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: RemovalGitEnvelope = serde_json::from_str(payload)
        .map_err(|error| format!("invalid removal Git helper request: {error}"))?;
    validate_helper_envelope(&envelope)?;
    macos_git_xpc::validate_directory_observation(
        cwd,
        envelope.target_device,
        envelope.target_inode,
        None,
        "removal Git cwd",
    )?;
    if envelope.request.git_executable() != MACOS_SYSTEM_GIT {
        return Err("macOS removal Git request did not select /usr/bin/git".to_string());
    }
    let trusted_git = crate::code_workspace::git_write::macos_root_trusted_git()?;
    match &envelope.request {
        RemovalGitRequest::BlobTypes { object_ids, .. } => {
            let expected_size = object_ids.iter().try_fold(0_u64, |total, object_id| {
                total
                    .checked_add(object_id.len() as u64 + 1)
                    .ok_or_else(|| "removal blob input size overflowed".to_string())
            })?;
            macos_git_xpc::validate_bounded_regular_observation(
                stdin,
                expected_size,
                "removal blob input",
            )?;
            if stdin.size != expected_size {
                return Err("macOS removal blob input size did not match its request".to_string());
            }
        }
        _ => macos_git_xpc::validate_null_observation(stdin, "removal Git input")?,
    }
    let mut command = Command::new(trusted_git);
    configure_helper_git_command(&mut command, &envelope.request)?;
    macos_git_xpc::process_spec_from_command(&command)
}

#[cfg(all(test, target_os = "macos"))]
mod macos_xpc_tests {
    use super::*;

    #[test]
    fn prepare_revalidates_removal_git_descriptors() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Ok(());
        }
        let target = tempfile::tempdir().map_err(|error| error.to_string())?;
        let metadata = target
            .path()
            .metadata()
            .map_err(|error| error.to_string())?;
        let envelope = RemovalGitEnvelope {
            version: HELPER_VERSION,
            target_device: metadata.dev(),
            target_inode: metadata.ino(),
            request: RemovalGitRequest::LocalConfig {
                git_executable: MACOS_SYSTEM_GIT.to_string(),
                expected_target_path: path_string(target.path(), "test target")?,
            },
        };
        let payload = serde_json::to_string(&envelope).map_err(|error| error.to_string())?;
        let cwd = DescriptorObservation {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            size: 0,
        };
        let null = fs::metadata("/dev/null").map_err(|error| error.to_string())?;
        let stdin = DescriptorObservation {
            device: null.dev(),
            inode: null.ino(),
            mode: null.mode(),
            size: null.size(),
        };
        prepare_macos_removal_git(&payload, cwd, stdin)?;
        assert!(prepare_macos_removal_git(
            &payload,
            DescriptorObservation {
                inode: cwd.inode.saturating_add(1),
                ..cwd
            },
            stdin,
        )
        .is_err_and(|error| error.contains("descriptor identity")));
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn helper_git_command(request: &RemovalGitRequest) -> Result<Command, String> {
    let git = Path::new(request.git_executable());
    let mut command = Command::new(git);
    configure_helper_git_command(&mut command, request)?;
    Ok(command)
}

pub(super) fn configure_helper_git_command(
    command: &mut Command,
    request: &RemovalGitRequest,
) -> Result<(), String> {
    command.arg("--no-pager");
    let (mutating, filters): (bool, &[String]) = match request {
        RemovalGitRequest::LocalConfig { .. } => {
            command.args(["config", "--local", "--includes", "--null", "--list"]);
            (false, &[])
        }
        RemovalGitRequest::WorktreeConfigNames { .. } => {
            command.args([
                "config",
                "--worktree",
                "--includes",
                "--null",
                "--name-only",
                "--list",
            ]);
            (false, &[])
        }
        RemovalGitRequest::IndexEntries { .. } => {
            command.args([
                "--literal-pathspecs",
                "ls-files",
                "--cached",
                "--stage",
                "-z",
                "--",
            ]);
            (false, &[])
        }
        RemovalGitRequest::HeadEntries { head_commit, .. } => {
            command.args([
                "--literal-pathspecs",
                "ls-tree",
                "-r",
                "-z",
                "--full-tree",
                head_commit,
                "--",
            ]);
            (false, &[])
        }
        RemovalGitRequest::BlobTypes { .. } => {
            command
                .arg("cat-file")
                .arg("--batch-check=%(objectname) %(objecttype)");
            (false, &[])
        }
        RemovalGitRequest::RefFormat { .. } => {
            command.args([
                "config",
                "--local",
                "--get",
                "--default",
                "files",
                "extensions.refStorage",
            ]);
            (false, &[])
        }
        RemovalGitRequest::Status {
            disabled_filter_keys,
            ..
        } => {
            command.args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
            (false, disabled_filter_keys)
        }
        RemovalGitRequest::ReadProofRef { removal_id, .. } => {
            let proof_ref = proof_ref_from_id(removal_id);
            command.args([
                "--git-dir=.",
                "for-each-ref",
                "--format=%(refname)\t%(objectname)\t%(symref)",
                "--count=2",
                &proof_ref,
            ]);
            (false, &[])
        }
        RemovalGitRequest::CreateProofRef {
            removal_id,
            target_commit,
            zero_oid,
            ..
        } => {
            command.args([
                "--git-dir=.",
                "update-ref",
                "--no-deref",
                &proof_ref_from_id(removal_id),
                target_commit,
                zero_oid,
            ]);
            (true, &[])
        }
        RemovalGitRequest::DeleteProofRef {
            removal_id,
            target_commit,
            ..
        } => {
            command.args([
                "--git-dir=.",
                "update-ref",
                "--no-deref",
                "-d",
                &proof_ref_from_id(removal_id),
                target_commit,
            ]);
            (true, &[])
        }
    };
    configure_git_environment(command, mutating, filters);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    Ok(())
}

pub(super) fn configure_git_environment(command: &mut Command, mutating: bool, filters: &[String]) {
    let inherited = [
        "PATH",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
    ]
    .into_iter()
    .filter_map(|name| std::env::var_os(name).map(|value| (name, value)))
    .collect::<Vec<_>>();
    command.env_clear();
    for (name, value) in inherited {
        command.env(name, value);
    }
    command
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_GRAFT_FILE", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", if mutating { "1" } else { "0" });
    let mut static_config = vec![
        ("credential.helper", ""),
        ("advice.graftFileDeprecated", "false"),
        ("core.hooksPath", "/dev/null"),
        ("core.fsmonitor", "false"),
        ("protocol.allow", "never"),
        ("core.logAllRefUpdates", "false"),
    ];
    if mutating {
        static_config.push(("core.fsync", "reference"));
        static_config.push(("core.fsyncMethod", "fsync"));
    }
    command.env(
        "GIT_CONFIG_COUNT",
        (static_config.len() + filters.len()).to_string(),
    );
    for (index, &(key, value)) in static_config.iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    for (offset, key) in filters.iter().enumerate() {
        let index = static_config.len() + offset;
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(
            format!("GIT_CONFIG_VALUE_{index}"),
            if key.ends_with(".required") {
                "false"
            } else {
                ""
            },
        );
    }
}

pub(super) fn validate_filter_keys(keys: &[String]) -> Result<(), String> {
    if keys.len() > MAX_FILTER_KEYS {
        return Err(format!(
            "removal Git helper filter keys exceed {MAX_FILTER_KEYS}"
        ));
    }
    for key in keys {
        let lower = key.to_ascii_lowercase();
        if key.len() > 4096
            || key.chars().any(char::is_control)
            || !lower.starts_with("filter.")
            || ![".clean", ".smudge", ".process", ".required"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
        {
            return Err("removal Git helper filter key is invalid".to_string());
        }
    }
    Ok(())
}

pub(super) fn validate_native_removal_id(value: &str) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|error| format!("removal Git helper id is invalid: {error}"))?;
    if parsed.get_version() != Some(uuid::Version::Random)
        || parsed.hyphenated().to_string() != value
    {
        return Err("removal Git helper id must be a canonical UUID v4".to_string());
    }
    Ok(())
}

pub(super) fn proof_ref_from_id(removal_id: &str) -> String {
    format!("{PROOF_REF_PREFIX}{removal_id}")
}
