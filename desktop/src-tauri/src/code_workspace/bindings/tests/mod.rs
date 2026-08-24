use super::*;

fn repository_identity(marker: char) -> String {
    marker.to_string().repeat(64)
}

fn scope(community: &str, project: &str, marker: char) -> CodeThreadBindingScope {
    CodeThreadBindingScope {
        community_id: community.to_string(),
        project_dtag: project.to_string(),
        repository_identity: repository_identity(marker),
    }
}

fn binding(
    root: &Path,
    scope: CodeThreadBindingScope,
    thread_id: &str,
    mode: CodeExecutionMode,
    worktree_id: Option<&str>,
) -> CodeThreadBinding {
    CodeThreadBinding {
        community_id: scope.community_id,
        project_dtag: scope.project_dtag,
        repository_identity: scope.repository_identity,
        codex_thread_id: thread_id.to_string(),
        execution_mode: mode,
        execution_root: root.to_string_lossy().into_owned(),
        base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        worktree_id: worktree_id.map(str::to_string),
    }
}

fn local_descriptor(root: &Path, marker: char) -> CodeWorktreeDescriptor {
    CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Local,
        repository_identity: repository_identity(marker),
        execution_root: root.to_string_lossy().into_owned(),
        base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        worktree_id: None,
    }
}

fn store() -> (tempfile::TempDir, CodeThreadBindingStore) {
    let directory = tempfile::tempdir().expect("temp app data");
    let store = CodeThreadBindingStore::for_app_data(directory.path()).expect("binding store");
    (directory, store)
}

mod preparations;
mod store;
