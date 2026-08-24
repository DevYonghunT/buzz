use super::*;

#[test]
fn missing_store_loads_as_empty_current_version() {
    let (_directory, store) = store();
    let index = store.load().expect("empty index");

    assert_eq!(index.version, CODE_THREAD_BINDING_SCHEMA_VERSION);
    assert!(index.bindings.is_empty());
    assert!(!store.store_path().exists());
}

#[test]
fn version_one_without_preparations_remains_readable() {
    let (_directory, store) = store();
    fs::write(store.store_path(), r#"{"version":1,"bindings":[]}"#)
        .expect("legacy version-one fixture");

    let index = store.load().expect("legacy version-one index");
    assert!(index.bindings.is_empty());
    assert!(index.preparations.is_empty());
}

#[cfg(unix)]
#[test]
fn code_directory_is_owner_only_without_changing_app_data_mode() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().expect("temp app data");
    let code_dir = directory.path().join(CODE_STORE_DIRECTORY);
    fs::create_dir(&code_dir).expect("existing code directory");
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755))
        .expect("app-data permissions");
    fs::set_permissions(&code_dir, fs::Permissions::from_mode(0o777)).expect("code permissions");

    CodeThreadBindingStore::for_app_data(directory.path()).expect("binding store");

    let app_mode = fs::metadata(directory.path())
        .expect("app-data metadata")
        .permissions()
        .mode()
        & 0o777;
    let code_mode = fs::metadata(&code_dir)
        .expect("code metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(app_mode, 0o755);
    assert_eq!(code_mode, 0o700);
}

#[test]
fn round_trip_reload_is_deterministic_and_owner_only() {
    let (directory, store) = store();
    let root_a = directory.path().join("root-a");
    let root_b = directory.path().join("root-b");
    fs::create_dir(&root_a).expect("managed root");
    fs::create_dir(&root_b).expect("local root");
    let root_a = root_a.canonicalize().expect("canonical managed root");
    let root_b = root_b.canonicalize().expect("canonical local root");
    let later = binding(
        &root_b,
        scope("community-b", "project", 'b'),
        "thread-b",
        CodeExecutionMode::Local,
        None,
    );
    let earlier = binding(
        &root_a,
        scope("community-a", "project", 'a'),
        "thread-a",
        CodeExecutionMode::Worktree,
        Some("11111111-1111-4111-8111-111111111111"),
    );

    store.upsert(later.clone()).expect("insert later");
    store.upsert(earlier.clone()).expect("insert earlier");
    let raw = fs::read_to_string(store.store_path()).expect("stored JSON");
    assert!(raw.find("thread-a").expect("thread-a") < raw.find("thread-b").expect("thread-b"));

    let reloaded = CodeThreadBindingStore::for_app_data(directory.path())
        .expect("reopened store")
        .load()
        .expect("reloaded index");
    assert_eq!(reloaded.bindings, vec![earlier, later]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(store.store_path())
            .expect("store metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn exact_upsert_is_idempotent() {
    let (directory, store) = store();
    let root = directory.path().join("worktree");
    fs::create_dir(&root).expect("managed root");
    let root = root.canonicalize().expect("canonical managed root");
    let record = binding(
        &root,
        scope("community", "project", 'a'),
        "thread-1",
        CodeExecutionMode::Worktree,
        Some("11111111-1111-4111-8111-111111111111"),
    );

    store.upsert(record.clone()).expect("first upsert");
    store.upsert(record.clone()).expect("idempotent upsert");

    assert_eq!(store.load().expect("index").bindings, vec![record]);
}

#[test]
fn list_and_lookup_are_scope_isolated() {
    let (directory, store) = store();
    let wanted_scope = scope("community-a", "project", 'a');
    for name in ["wanted", "other-community", "other-repository"] {
        fs::create_dir(directory.path().join(name)).expect("local scope root");
    }
    let wanted_root = directory
        .path()
        .join("wanted")
        .canonicalize()
        .expect("canonical wanted root");
    let other_community_root = directory
        .path()
        .join("other-community")
        .canonicalize()
        .expect("canonical other-community root");
    let other_repository_root = directory
        .path()
        .join("other-repository")
        .canonicalize()
        .expect("canonical other-repository root");
    let wanted = binding(
        &wanted_root,
        wanted_scope.clone(),
        "thread-wanted",
        CodeExecutionMode::Local,
        None,
    );
    let other_community = binding(
        &other_community_root,
        scope("community-b", "project", 'a'),
        "thread-other-community",
        CodeExecutionMode::Local,
        None,
    );
    let other_repository = binding(
        &other_repository_root,
        scope("community-a", "project", 'b'),
        "thread-other-repository",
        CodeExecutionMode::Local,
        None,
    );
    for record in [&wanted, &other_community, &other_repository] {
        store.upsert(record.clone()).expect("scope fixture");
    }
    fs::remove_dir(&other_community_root).expect("remove unrelated local checkout");

    assert_eq!(
        store.list(&wanted_scope).expect("scoped list"),
        vec![wanted.clone()]
    );
    assert_eq!(
        store
            .lookup(&CodeThreadBindingLookupInput {
                scope: wanted_scope,
                codex_thread_id: wanted.codex_thread_id.clone(),
            })
            .expect("matching lookup"),
        Some(wanted.clone())
    );
    assert!(store
        .lookup(&CodeThreadBindingLookupInput {
            scope: scope("community-b", "project", 'a'),
            codex_thread_id: wanted.codex_thread_id,
        })
        .expect("isolated lookup")
        .is_none());
}

#[test]
fn same_thread_cannot_be_rebound() {
    let (directory, store) = store();
    fs::create_dir(directory.path().join("first")).expect("first local root");
    fs::create_dir(directory.path().join("second")).expect("second local root");
    let first_root = directory
        .path()
        .join("first")
        .canonicalize()
        .expect("canonical first root");
    let second_root = directory
        .path()
        .join("second")
        .canonicalize()
        .expect("canonical second root");
    let first = binding(
        &first_root,
        scope("community-a", "project", 'a'),
        "thread-1",
        CodeExecutionMode::Local,
        None,
    );
    let conflicting = binding(
        &second_root,
        scope("community-b", "project", 'b'),
        "thread-1",
        CodeExecutionMode::Local,
        None,
    );
    store.upsert(first.clone()).expect("first binding");

    assert!(store.upsert(conflicting).is_err());
    assert_eq!(store.load().expect("preserved index").bindings, vec![first]);
}

#[test]
fn managed_worktree_cannot_bind_two_threads() {
    let (directory, store) = store();
    let root = directory.path().join("managed-root");
    fs::create_dir(&root).expect("managed root");
    let root = root.canonicalize().expect("canonical managed root");
    let first = binding(
        &root,
        scope("community-a", "project", 'a'),
        "thread-1",
        CodeExecutionMode::Worktree,
        Some("11111111-1111-4111-8111-111111111111"),
    );
    let same_id = binding(
        &directory.path().join("another-root"),
        scope("community-b", "project", 'b'),
        "thread-2",
        CodeExecutionMode::Worktree,
        Some("11111111-1111-4111-8111-111111111111"),
    );
    let same_root = binding(
        &root,
        scope("community-c", "project", 'c'),
        "thread-3",
        CodeExecutionMode::Worktree,
        Some("33333333-3333-4333-8333-333333333333"),
    );
    store.upsert(first.clone()).expect("first binding");

    assert!(store.upsert(same_id).is_err());
    assert!(store.upsert(same_root).is_err());
    assert_eq!(store.load().expect("preserved index").bindings, vec![first]);
}

#[test]
fn local_checkout_can_have_multiple_threads() {
    let (directory, store) = store();
    let root = directory.path().join("local-checkout");
    fs::create_dir(&root).expect("local checkout");
    let root = root.canonicalize().expect("canonical local checkout");
    let first = binding(
        &root,
        scope("community", "project", 'a'),
        "thread-1",
        CodeExecutionMode::Local,
        None,
    );
    let second = binding(
        &root,
        scope("community", "project", 'a'),
        "thread-2",
        CodeExecutionMode::Local,
        None,
    );

    store.upsert(first).expect("first local binding");
    store.upsert(second).expect("second local binding");
    assert_eq!(store.load().expect("local bindings").bindings.len(), 2);
}

#[test]
fn availability_precheck_is_global_for_managed_worktrees() {
    let (directory, store) = store();
    let root = directory.path().join("managed-root");
    fs::create_dir(&root).expect("managed root");
    fs::create_dir(directory.path().join("different-root")).expect("different managed root");
    fs::create_dir(directory.path().join("free-root")).expect("free managed root");
    let root = root.canonicalize().expect("canonical managed root");
    let different_root = directory
        .path()
        .join("different-root")
        .canonicalize()
        .expect("canonical different managed root");
    let free_root = directory
        .path()
        .join("free-root")
        .canonicalize()
        .expect("canonical free managed root");
    store
        .upsert(binding(
            &root,
            scope("community-a", "project-a", 'a'),
            "thread-1",
            CodeExecutionMode::Worktree,
            Some("11111111-1111-4111-8111-111111111111"),
        ))
        .expect("managed fixture");

    assert!(store
        .ensure_execution_available(&CodeExecutionAvailabilityInput {
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: different_root.to_string_lossy().into_owned(),
            worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
        })
        .is_err());
    assert!(store
        .ensure_execution_available(&CodeExecutionAvailabilityInput {
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: root.to_string_lossy().into_owned(),
            worktree_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
        })
        .is_err());
    assert!(store
        .ensure_execution_available(&CodeExecutionAvailabilityInput {
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: free_root.to_string_lossy().into_owned(),
            worktree_id: Some("33333333-3333-4333-8333-333333333333".to_string()),
        })
        .is_ok());
}

#[test]
fn availability_precheck_allows_shared_local_checkout() {
    let (directory, store) = store();
    let root = directory.path().join("local-root");
    fs::create_dir(&root).expect("local root");
    let root = root.canonicalize().expect("canonical local root");
    store
        .upsert(binding(
            &root,
            scope("community", "project", 'a'),
            "thread-1",
            CodeExecutionMode::Local,
            None,
        ))
        .expect("local fixture");

    store
        .ensure_execution_available(&CodeExecutionAvailabilityInput {
            execution_mode: CodeExecutionMode::Local,
            execution_root: root.to_string_lossy().into_owned(),
            worktree_id: None,
        })
        .expect("local roots are shareable");
}

#[test]
fn corrupt_store_fails_closed_and_is_preserved() {
    let (directory, store) = store();
    let corrupt = b"{ not valid JSON";
    fs::write(store.store_path(), corrupt).expect("corrupt fixture");
    let local_root = directory.path().join("local-checkout");
    fs::create_dir(&local_root).expect("local root");
    let local_root = local_root.canonicalize().expect("canonical local root");

    assert!(store.load().is_err());
    let attempted = CodeThreadBinding {
        community_id: "community".to_string(),
        project_dtag: "project".to_string(),
        repository_identity: repository_identity('a'),
        codex_thread_id: "thread".to_string(),
        execution_mode: CodeExecutionMode::Local,
        execution_root: local_root.to_string_lossy().into_owned(),
        base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        worktree_id: None,
    };
    assert!(store.upsert(attempted).is_err());
    assert_eq!(
        fs::read(store.store_path()).expect("preserved bytes"),
        corrupt
    );
}

#[test]
fn unversioned_and_unsupported_schemas_fail_closed() {
    let (_directory, store) = store();
    for fixture in [
        r#"{"bindings":[]}"#,
        r#"{"version":0,"bindings":[]}"#,
        r#"{"version":99,"bindings":[]}"#,
    ] {
        fs::write(store.store_path(), fixture).expect("schema fixture");
        assert!(store.load().is_err(), "fixture should fail: {fixture}");
        assert_eq!(
            fs::read_to_string(store.store_path()).expect("preserved schema"),
            fixture
        );
    }
}

#[test]
fn oversized_store_is_rejected_before_json_parsing() {
    let (_directory, store) = store();
    let oversized = vec![b' '; MAX_BINDING_STORE_BYTES as usize + 1];
    fs::write(store.store_path(), oversized).expect("oversized fixture");

    let error = store.load().expect_err("oversized store must fail");
    assert!(error.contains("byte limit"), "unexpected error: {error}");
}

#[test]
fn worktree_ids_require_canonical_hyphenated_uuids() {
    let (directory, store) = store();
    let simple_uuid = "11111111111141118111111111111111";
    let record = binding(
        &directory.path().join("managed-root"),
        scope("community", "project", 'a'),
        "thread-simple-uuid",
        CodeExecutionMode::Worktree,
        Some(simple_uuid),
    );

    assert!(store.upsert(record).is_err());
    assert!(store
        .ensure_execution_available(&CodeExecutionAvailabilityInput {
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: directory
                .path()
                .join("managed-root")
                .to_string_lossy()
                .into_owned(),
            worktree_id: Some(simple_uuid.to_string()),
        })
        .is_err());
}

#[test]
fn local_execution_root_must_exist_and_be_a_directory() {
    let (directory, store) = store();
    let missing = binding(
        &directory.path().join("missing-local-root"),
        scope("community", "project", 'a'),
        "thread-missing",
        CodeExecutionMode::Local,
        None,
    );
    assert!(store.upsert(missing).is_err());

    let file_root = directory.path().join("local-file");
    fs::write(&file_root, b"not a directory").expect("file root");
    let file_binding = binding(
        &file_root,
        scope("community", "project", 'a'),
        "thread-file",
        CodeExecutionMode::Local,
        None,
    );
    assert!(store.upsert(file_binding).is_err());
}

#[test]
fn moved_local_root_does_not_block_a_healthy_binding_in_another_scope() {
    let (directory, store) = store();
    let stale_scope = scope("community-stale", "project-stale", 'a');
    let stale_root = directory.path().join("stale-local-root");
    fs::create_dir(&stale_root).expect("stale local root");
    let stale_root = stale_root.canonicalize().expect("canonical stale root");
    let stale_binding = binding(
        &stale_root,
        stale_scope.clone(),
        "thread-stale",
        CodeExecutionMode::Local,
        None,
    );
    store
        .upsert(stale_binding.clone())
        .expect("persist stale binding before root moves");

    let moved_root = directory.path().join("moved-local-root");
    fs::rename(&stale_root, &moved_root).expect("move stale local root");
    assert!(!stale_root.exists());

    let healthy_scope = scope("community-healthy", "project-healthy", 'b');
    let healthy_root = directory.path().join("healthy-local-root");
    fs::create_dir(&healthy_root).expect("healthy local root");
    let healthy_root = healthy_root.canonicalize().expect("canonical healthy root");
    let healthy_binding = binding(
        &healthy_root,
        healthy_scope.clone(),
        "thread-healthy",
        CodeExecutionMode::Local,
        None,
    );
    store
        .upsert(healthy_binding.clone())
        .expect("persist healthy binding despite stale root");

    let reloaded = CodeThreadBindingStore::for_app_data(directory.path())
        .expect("reopen mixed-availability store");
    assert_eq!(
        reloaded.list(&healthy_scope).expect("list healthy scope"),
        vec![healthy_binding]
    );
    assert_eq!(
        reloaded.list(&stale_scope).expect("list stale scope"),
        vec![stale_binding.clone()]
    );
    assert!(reloaded
        .load()
        .expect("load mixed-availability index")
        .bindings
        .contains(&stale_binding));
}

#[test]
fn missing_managed_execution_root_remains_loadable() {
    let (directory, store) = store();
    let root = directory.path().join("pruned-managed-root");
    fs::create_dir(&root).expect("managed root");
    let root = root.canonicalize().expect("canonical managed root");
    let record = binding(
        &root,
        scope("community", "project", 'a'),
        "thread-pruned",
        CodeExecutionMode::Worktree,
        Some("11111111-1111-4111-8111-111111111111"),
    );

    store
        .upsert(record.clone())
        .expect("persist managed worktree");
    fs::remove_dir(&root).expect("simulate externally pruned worktree");
    assert_eq!(
        store.load().expect("recoverable binding").bindings,
        vec![record]
    );
}

#[test]
fn non_not_found_execution_root_errors_are_not_treated_as_missing() {
    let (directory, store) = store();
    let blocking_file = directory.path().join("blocking-file");
    fs::write(&blocking_file, b"not a parent directory").expect("blocking file");
    let record = binding(
        &blocking_file.join("managed-root"),
        scope("community", "project", 'a'),
        "thread-invalid-parent",
        CodeExecutionMode::Worktree,
        Some("11111111-1111-4111-8111-111111111111"),
    );

    let error = store.upsert(record).expect_err("invalid parent must fail");
    assert!(
        error.contains("failed to inspect SchoolX Code execution root"),
        "unexpected error: {error}"
    );
}

#[test]
fn mode_and_path_invariants_are_validated() {
    let (directory, store) = store();
    let local_root = directory.path().join("local");
    fs::create_dir(&local_root).expect("local root");
    let local_root = local_root.canonicalize().expect("canonical local root");
    let local_with_id = binding(
        &local_root,
        scope("community", "project", 'a'),
        "thread-local",
        CodeExecutionMode::Local,
        Some("11111111-1111-4111-8111-111111111111"),
    );
    let worktree_without_id = binding(
        &directory.path().join("worktree"),
        scope("community", "project", 'a'),
        "thread-worktree",
        CodeExecutionMode::Worktree,
        None,
    );
    let relative = binding(
        Path::new("relative"),
        scope("community", "project", 'a'),
        "thread-relative",
        CodeExecutionMode::Local,
        None,
    );

    assert!(store.upsert(local_with_id).is_err());
    assert!(store.upsert(worktree_without_id).is_err());
    assert!(store.upsert(relative).is_err());
    assert!(store.load().expect("still empty").bindings.is_empty());
}

#[cfg(unix)]
#[test]
fn execution_root_symlink_is_rejected() {
    use std::os::unix::fs::symlink;

    let (directory, store) = store();
    let real_root = directory.path().join("real-local-root");
    let linked_root = directory.path().join("linked-local-root");
    fs::create_dir(&real_root).expect("real local root");
    symlink(&real_root, &linked_root).expect("linked local root");
    let record = binding(
        &linked_root,
        scope("community", "project", 'a'),
        "thread-linked",
        CodeExecutionMode::Local,
        None,
    );

    assert!(store.upsert(record).is_err());
}

#[cfg(unix)]
#[test]
fn binding_file_symlink_is_rejected_without_touching_target() {
    use std::os::unix::fs::symlink;

    let (directory, store) = store();
    let external = directory.path().join("external.json");
    fs::write(&external, b"outside").expect("external fixture");
    symlink(&external, store.store_path()).expect("binding symlink");

    assert!(store.load().is_err());
    assert_eq!(fs::read(&external).expect("external target"), b"outside");
}

#[cfg(unix)]
#[test]
fn symlinked_code_parent_is_rejected() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temp app data");
    let external = directory.path().join("external-code");
    fs::create_dir(&external).expect("external code dir");
    symlink(&external, directory.path().join(CODE_STORE_DIRECTORY))
        .expect("code directory symlink");

    assert!(CodeThreadBindingStore::for_app_data(directory.path()).is_err());
}

#[cfg(unix)]
#[test]
fn code_parent_replaced_with_symlink_after_open_is_rejected() {
    use std::os::unix::fs::symlink;

    let (directory, store) = store();
    let external = directory.path().join("external-code");
    fs::create_dir(&external).expect("external code dir");
    fs::remove_dir(directory.path().join(CODE_STORE_DIRECTORY)).expect("remove code dir");
    symlink(&external, directory.path().join(CODE_STORE_DIRECTORY))
        .expect("replacement code symlink");

    assert!(store.load().is_err());
    assert!(fs::read_dir(&external)
        .expect("external directory")
        .next()
        .is_none());
}

#[cfg(unix)]
#[test]
fn symlinked_app_data_root_is_rejected() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temp root");
    let real = directory.path().join("real-app-data");
    let link = directory.path().join("linked-app-data");
    fs::create_dir(&real).expect("real app data");
    symlink(&real, &link).expect("app-data symlink");

    assert!(CodeThreadBindingStore::for_app_data(&link).is_err());
}
