use super::*;

#[cfg(target_os = "macos")]
#[test]
fn macos_xpc_prepare_revalidates_git_write_descriptors() -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;

    if rustix::process::geteuid().as_raw() == 0 {
        return Ok(());
    }
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root_identity = pin_directory(root.path())?;
    let envelope = HelperEnvelope {
        version: HELPER_VERSION,
        root: root_identity.clone(),
        git: pin_git_executable()?,
        authority: None,
        command: GitWriteCommand::TopLevel,
    };
    let payload = serde_json::to_string(&envelope).map_err(|error| error.to_string())?;
    let cwd = crate::code_workspace::macos_git_xpc::DescriptorObservation {
        device: root_identity.device,
        inode: root_identity.inode,
        mode: root_identity.mode,
        size: 0,
    };
    let null = fs::metadata("/dev/null").map_err(|error| error.to_string())?;
    let stdin = crate::code_workspace::macos_git_xpc::DescriptorObservation {
        device: null.dev(),
        inode: null.ino(),
        mode: null.mode(),
        size: null.size(),
    };
    prepare_macos_git_write(&payload, cwd, stdin)?;

    let replaced = crate::code_workspace::macos_git_xpc::DescriptorObservation {
        inode: cwd.inode.saturating_add(1),
        ..cwd
    };
    assert!(prepare_macos_git_write(&payload, replaced, stdin)
        .is_err_and(|error| error.contains("descriptor identity")));
    Ok(())
}

#[cfg(unix)]
#[test]
#[ignore = "private Git write helper subprocess entry"]
fn helper_subprocess_entry() {
    match execute_helper() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(unix)]
#[test]
fn pinned_helper_runs_closed_read_and_rejects_replaced_input() -> Result<(), String> {
    let repository = tempfile::tempdir().map_err(|error| error.to_string())?;
    let output = Command::new("git")
        .current_dir(repository.path())
        .args(["init", "-q"])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    let source = repository.path().join("source");
    fs::write(&source, b"before\n").map_err(|error| error.to_string())?;
    let evidence = pin_input_file(&source, 1024)?;
    let pinned = PinnedGitWriteRepository::pin(repository.path())?;
    let top = pinned.run(GitWriteCommand::TopLevel)?;
    assert_eq!(top.code, 0);
    assert_eq!(
        String::from_utf8(top.stdout)
            .map_err(|error| error.to_string())?
            .trim(),
        repository
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
    );
    let error = match pinned.run(GitWriteCommand::HashObject {
        write: true,
        source: evidence.clone(),
    }) {
        Ok(_) => return Err("an unbound discovery runner wrote an object".to_string()),
        Err(error) => error,
    };
    assert!(error.contains("requires exact linked-worktree repository authority"));

    fs::write(&source, b"after\n").map_err(|error| error.to_string())?;
    let error = match pinned.run(GitWriteCommand::HashObject {
        write: false,
        source: evidence,
    }) {
        Ok(_) => return Err("replaced content unexpectedly passed identity checks".to_string()),
        Err(error) => error,
    };
    assert_eq!(
        error,
        "typed Git command failed: Git artifact identity changed after it was frozen"
    );
    Ok(())
}

#[cfg(not(unix))]
#[test]
fn git_write_surface_fails_closed_without_descriptor_support() {
    let root = Path::new(r"C:\schoolx\repository");
    let error = match PinnedGitWriteRepository::pin(root) {
        Ok(_) => panic!("Git write authority was created without descriptor-bound Unix support"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        "SchoolX Code Git writes require descriptor-bound Unix support"
    );
    assert!(pin_input_file(&root.join("message.txt"), 1024)
        .is_err_and(|error| error.contains("secure Git file pinning is unavailable")));
    assert!(pin_directory(root)
        .is_err_and(|error| error.contains("secure Git directory pinning is unavailable")));
}

#[test]
fn command_builder_locks_helpers_protocols_and_diff_drivers() -> Result<(), String> {
    let envelope = HelperEnvelope {
        version: HELPER_VERSION,
        root: DirectoryIdentity {
            path: "/pinned/root".to_string(),
            device: 1,
            inode: 2,
            owner: 3,
            mode: 0o040000,
            link_count: 1,
        },
        git: FileIdentity {
            path: "/pinned/git".to_string(),
            device: 4,
            inode: 5,
            owner: 6,
            mode: 0o100000,
            link_count: 1,
            size: 7,
            digest: "a".repeat(64),
        },
        authority: Some(RepositoryAuthority {
            worktree_git_file: FileIdentity {
                path: "/pinned/root/.git".to_string(),
                device: 1,
                inode: 8,
                owner: 3,
                mode: 0o100644,
                link_count: 1,
                size: 9,
                digest: "b".repeat(64),
            },
            admin_gitdir_file: FileIdentity {
                path: "/pinned/common/worktrees/task/gitdir".to_string(),
                device: 1,
                inode: 12,
                owner: 3,
                mode: 0o100644,
                link_count: 1,
                size: 24,
                digest: "c".repeat(64),
            },
            admin_commondir_file: FileIdentity {
                path: "/pinned/common/worktrees/task/commondir".to_string(),
                device: 1,
                inode: 13,
                owner: 3,
                mode: 0o100644,
                link_count: 1,
                size: 6,
                digest: "d".repeat(64),
            },
            admin: DirectoryIdentity {
                path: "/pinned/common/worktrees/task".to_string(),
                device: 1,
                inode: 9,
                owner: 3,
                mode: 0o040755,
                link_count: 1,
            },
            common: DirectoryIdentity {
                path: "/pinned/common".to_string(),
                device: 1,
                inode: 10,
                owner: 3,
                mode: 0o040755,
                link_count: 1,
            },
            object_database: DirectoryIdentity {
                path: "/pinned/common/objects".to_string(),
                device: 1,
                inode: 11,
                owner: 3,
                mode: 0o040755,
                link_count: 1,
            },
        }),
        command: GitWriteCommand::DiffPatch {
            staged: true,
            path: "src/main.rs".to_string(),
        },
    };
    let (command, input) = build_git_command(&envelope)?;
    assert!(input.is_none());

    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    for locked in [
        "core.fsmonitor=false",
        "core.hooksPath=/dev/null",
        "credential.helper=",
        "credential.interactive=false",
        "protocol.allow=never",
        "protocol.file.allow=never",
        "protocol.git.allow=never",
        "protocol.http.allow=never",
        "protocol.https.allow=never",
        "protocol.ssh.allow=never",
        "protocol.ext.allow=never",
        "core.fsync=added,reference",
        "core.fsyncMethod=fsync",
    ] {
        assert!(arguments
            .windows(2)
            .any(|pair| pair[0] == "-c" && pair[1] == locked));
    }
    assert!(arguments.ends_with(&[
        "diff".to_string(),
        "--no-ext-diff".to_string(),
        "--no-textconv".to_string(),
        "--no-renames".to_string(),
        "--unified=80".to_string(),
        "--cached".to_string(),
        "--".to_string(),
        "src/main.rs".to_string(),
    ]));

    let environment = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for (key, expected) in [
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_SYSTEM", "/dev/null"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_ATTR_NOSYSTEM", "1"),
        ("GIT_GRAFT_FILE", "/dev/null"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_ASKPASS", "false"),
        ("SSH_ASKPASS", "false"),
        ("GCM_INTERACTIVE", "never"),
        ("GIT_SSH_COMMAND", "false"),
        ("GIT_PROTOCOL_FROM_USER", "0"),
        ("GIT_ALLOW_PROTOCOL", ""),
        ("GIT_NO_REPLACE_OBJECTS", "1"),
        ("GIT_NO_LAZY_FETCH", "1"),
        ("GIT_LITERAL_PATHSPECS", "1"),
        ("GIT_DIR", "/pinned/common/worktrees/task"),
        ("GIT_COMMON_DIR", "/pinned/common"),
        ("GIT_WORK_TREE", "/pinned/root"),
    ] {
        assert_eq!(environment.get(key), Some(&Some(expected.to_string())));
    }
    for forbidden in [
        "HOME",
        "PATH",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "SSH_AUTH_SOCK",
        "BUZZ_PRIVATE_KEY",
        "BUZZ_AUTH_TAG",
    ] {
        assert!(!environment.contains_key(forbidden));
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn bound_helper_rejects_worktree_git_file_and_object_database_replacement() -> Result<(), String> {
    let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
    let main = sandbox.path().join("main");
    let worktree = sandbox.path().join("managed");
    fs::create_dir(&main).map_err(|error| error.to_string())?;
    run_git(&main, &["init", "-q"])?;
    run_git(&main, &["config", "user.name", "SchoolX Test"])?;
    run_git(&main, &["config", "user.email", "schoolx@example.test"])?;
    fs::write(main.join("tracked.txt"), b"base\n").map_err(|error| error.to_string())?;
    run_git(&main, &["add", "tracked.txt"])?;
    run_git(&main, &["commit", "-q", "-m", "base"])?;
    let worktree_text = worktree
        .to_str()
        .ok_or_else(|| "test worktree path is not UTF-8".to_string())?;
    run_git(
        &main,
        &["worktree", "add", "-q", "--detach", worktree_text, "HEAD"],
    )?;

    let pinned = PinnedGitWriteRepository::pin(&worktree)?;
    let admin = resolve_git_path(&worktree, &run_git(&worktree, &["rev-parse", "--git-dir"])?)?;
    let common = resolve_git_path(
        &worktree,
        &run_git(&worktree, &["rev-parse", "--git-common-dir"])?,
    )?;
    let marker = pin_input_file(&worktree.join(".git"), 32 * 1024)?;
    let admin_identity = pin_directory(&admin)?;
    let common_identity = pin_directory(&common)?;
    let objects_identity = pin_directory(&common.join("objects"))?;
    let pinned = pinned.bind_repository_authority(
        marker.clone(),
        admin_identity,
        common_identity,
        objects_identity,
    )?;
    let head = run_git(&worktree, &["rev-parse", "HEAD"])?;
    assert_eq!(
        String::from_utf8(pinned.run(GitWriteCommand::HeadCommit)?.stdout)
            .map_err(|error| error.to_string())?
            .trim(),
        head,
    );

    let objects = common.join("objects");
    let original_objects = common.join("objects-schoolx-test-original");
    fs::rename(&objects, &original_objects).map_err(|error| error.to_string())?;
    fs::create_dir(&objects).map_err(|error| error.to_string())?;
    let error = match pinned.run(GitWriteCommand::HeadCommit) {
        Ok(_) => return Err("replaced object database was accepted".to_string()),
        Err(error) => error,
    };
    assert!(error.contains("moved or was replaced"));
    fs::remove_dir(&objects).map_err(|error| error.to_string())?;
    fs::rename(&original_objects, &objects).map_err(|error| error.to_string())?;

    let admin_gitdir = admin.join("gitdir");
    let original_admin_gitdir = fs::read(&admin_gitdir).map_err(|error| error.to_string())?;
    fs::write(&admin_gitdir, b"/foreign/worktree/.git\n").map_err(|error| error.to_string())?;
    let error = match pinned.run(GitWriteCommand::HeadCommit) {
        Ok(_) => return Err("replaced Git-admin backlink was accepted".to_string()),
        Err(error) => error,
    };
    assert!(error.contains("identity changed"));
    fs::write(&admin_gitdir, original_admin_gitdir).map_err(|error| error.to_string())?;

    let replacement = worktree.join("git-file-replacement");
    fs::write(
        &replacement,
        fs::read(&marker.path).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::rename(&replacement, worktree.join(".git")).map_err(|error| error.to_string())?;
    let error = match pinned.run(GitWriteCommand::HeadCommit) {
        Ok(_) => return Err("replaced worktree .git file was accepted".to_string()),
        Err(error) => error,
    };
    assert!(error.contains("identity changed"));
    Ok(())
}

#[cfg(unix)]
fn resolve_git_path(root: &Path, value: &str) -> Result<std::path::PathBuf, String> {
    let path = Path::new(value.trim());
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    path.canonicalize().map_err(|error| error.to_string())
}

#[cfg(unix)]
fn run_git(root: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| error.to_string())
}
