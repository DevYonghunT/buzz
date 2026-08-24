use super::*;

pub(super) struct TestRepository {
    pub(super) _directory: tempfile::TempDir,
    pub(super) root: PathBuf,
}

pub(super) fn test_git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let executable = crate::managed_agents::resolve_command("git")
        .ok_or_else(|| "git executable was not found".to_string())?;
    let output = Command::new(executable)
        .arg("--no-pager")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| format!("failed to run test git: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub(super) fn create_repository() -> Result<TestRepository, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = directory.path().join("repository");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    test_git(&root, &["init", "--initial-branch=main"])?;
    fs::write(root.join("README.md"), "first\n").map_err(|error| error.to_string())?;
    test_git(&root, &["add", "README.md"])?;
    test_git(
        &root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
    )?;
    Ok(TestRepository {
        _directory: directory,
        root,
    })
}

pub(super) fn test_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

pub(super) fn test_commit_file(
    repository_root: &Path,
    path: &str,
    contents: &str,
    message: &str,
) -> Result<String, String> {
    fs::write(repository_root.join(path), contents).map_err(|error| error.to_string())?;
    test_git(repository_root, &["add", "--", path])?;
    test_git(
        repository_root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            message,
        ],
    )?;
    Ok(test_line(&test_git(
        repository_root,
        &["rev-parse", "HEAD"],
    )?))
}

pub(super) fn proof_observation(
    source_root: &Path,
    managed_root: &Path,
) -> Result<Vec<Vec<u8>>, String> {
    Ok(vec![
        test_git(
            source_root,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )?,
        test_git(managed_root, &["rev-parse", "HEAD"])?,
        test_git(managed_root, &["status", "--porcelain=v1", "-z"])?,
        fs::read(managed_root.join(".git")).map_err(|error| error.to_string())?,
    ])
}
