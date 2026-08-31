//! Initializes an owner-controlled empty project repository with a README.
//!
//! The command is deliberately narrow: it only creates a commit in a pristine
//! unborn checkout, and it only retries a local commit whose complete tree and
//! identity match the commit this module creates. Existing user commits and
//! working-tree changes are never published or replaced implicitly.

use nostr::Keys;
use serde::Serialize;
use std::io::Write;
use tauri::State;

use crate::app_state::AppState;
use crate::product::PRODUCT_NAME;

use super::project_git::{first_output_line, normalize_branch_option};
use super::project_git_exec::{
    build_git_auth_config_for_keys, clone_url_owner, run_git, validate_workspace_clone_url,
    GitAuthConfig,
};
use super::project_git_push::push_project_local_repository_blocking;
use super::project_git_workflow::clone_project_repository_blocking;

const INITIAL_COMMIT_MESSAGE: &str = "Initial commit";
const README_PATH: &str = "README.md";
const NOREPLY_DOMAIN: &str = "users.noreply.buzz";

/// Outcome of ensuring a newly-created project has its first remote commit.
#[derive(Debug, Serialize)]
pub struct ProjectRepoInitializeResult {
    pub path: String,
    pub cloned: bool,
    pub initialized: bool,
    pub pushed: bool,
    pub branch: String,
    pub commit: String,
    pub message: String,
}

#[derive(Debug)]
struct RemoteHead {
    branch: String,
    commit: String,
}

fn normalized_repository_id(project_dtag: &str) -> Result<String, String> {
    let project_dtag = project_dtag.trim();
    if project_dtag.is_empty()
        || project_dtag.len() > 64
        || project_dtag.starts_with('.')
        || project_dtag.contains("..")
        || !project_dtag.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(
            "Repository ID must be 1–64 letters, numbers, dots, underscores, or hyphens; it must not start with a dot or contain consecutive dots."
                .to_string(),
        );
    }
    Ok(project_dtag.to_string())
}

fn normalized_initial_branch(default_branch: Option<&str>) -> Result<String, String> {
    match default_branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(branch) => normalize_branch_option(Some(branch))
            .ok_or_else(|| "Invalid default branch for repository initialization.".to_string()),
        None => Ok("main".to_string()),
    }
}

fn repository_owner_for_viewer(
    clone_url: &str,
    project_dtag: &str,
    keys: &Keys,
) -> Result<String, String> {
    let owner = clone_url_owner(clone_url)
        .ok_or_else(|| "Could not resolve the repository owner from its clone URL.".to_string())?;
    let repository_id = url::Url::parse(clone_url)
        .ok()
        .and_then(|url| {
            url.path_segments()?
                .rfind(|segment| !segment.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| "Could not resolve the repository ID from its clone URL.".to_string())?;
    if repository_id != project_dtag {
        return Err(
            "The repository ID does not match the project clone URL; SchoolX did not initialize it."
                .to_string(),
        );
    }
    let viewer = keys.public_key().to_hex();
    if !owner.eq_ignore_ascii_case(&viewer) {
        return Err("Only the repository owner can create its first commit.".to_string());
    }
    Ok(viewer)
}

fn readme_content(project_dtag: &str) -> String {
    format!("# {project_dtag}\n")
}

fn commit_author(owner: &str) -> (String, String) {
    (
        format!("{PRODUCT_NAME} User"),
        format!("{owner}@{NOREPLY_DOMAIN}"),
    )
}

fn local_head(repo_dir: &std::path::Path, auth: &GitAuthConfig) -> Option<String> {
    run_git(&["rev-parse", "--verify", "HEAD"], Some(repo_dir), auth)
        .ok()
        .and_then(|output| first_output_line(&output))
}

fn worktree_is_pristine(repo_dir: &std::path::Path, auth: &GitAuthConfig) -> Result<bool, String> {
    run_git(
        &["status", "--porcelain", "--untracked-files=all"],
        Some(repo_dir),
        auth,
    )
    .map(|output| output.trim().is_empty())
}

fn parse_remote_heads(output: &str) -> Vec<RemoteHead> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let commit = parts.next()?.trim().to_ascii_lowercase();
            let reference = parts.next()?.trim();
            if parts.next().is_some()
                || !matches!(commit.len(), 40 | 64)
                || !commit
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
            {
                return None;
            }
            let branch = reference.strip_prefix("refs/heads/")?;
            normalize_branch_option(Some(branch)).map(|branch| RemoteHead { branch, commit })
        })
        .collect()
}

fn remote_heads(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
) -> Result<Vec<RemoteHead>, String> {
    run_git(
        &["ls-remote", "--heads", "--end-of-options", "origin"],
        Some(repo_dir),
        auth,
    )
    .map(|output| parse_remote_heads(&output))
}

fn remote_branch_head<'a>(heads: &'a [RemoteHead], branch: &str) -> Option<&'a str> {
    heads
        .iter()
        .find(|head| head.branch == branch)
        .map(|head| head.commit.as_str())
}

fn generated_initial_commit_matches(
    repo_dir: &std::path::Path,
    project_dtag: &str,
    owner: &str,
    auth: &GitAuthConfig,
) -> bool {
    if !worktree_is_pristine(repo_dir, auth).unwrap_or(false) {
        return false;
    }

    let parents = match run_git(
        &["rev-list", "--parents", "--max-count=1", "HEAD"],
        Some(repo_dir),
        auth,
    ) {
        Ok(output) => output.split_whitespace().count(),
        Err(_) => return false,
    };
    if parents != 1 {
        return false;
    }

    let files = match run_git(
        &["ls-tree", "-r", "--name-only", "HEAD"],
        Some(repo_dir),
        auth,
    ) {
        Ok(output) => output,
        Err(_) => return false,
    };
    if files.lines().collect::<Vec<_>>() != [README_PATH] {
        return false;
    }

    let content = match run_git(
        &["show", format!("HEAD:{README_PATH}").as_str()],
        Some(repo_dir),
        auth,
    ) {
        Ok(output) => output,
        Err(_) => return false,
    };
    if content != readme_content(project_dtag) {
        return false;
    }

    let (author_name, author_email) = commit_author(owner);
    let identity = match run_git(
        &[
            "show",
            "-s",
            "--format=%an%x00%ae%x00%cn%x00%ce%x00%s",
            "HEAD",
        ],
        Some(repo_dir),
        auth,
    ) {
        Ok(output) => output,
        Err(_) => return false,
    };
    let mut fields = identity.trim_end_matches(['\r', '\n']).split('\0');
    fields.next() == Some(author_name.as_str())
        && fields.next() == Some(author_email.as_str())
        && fields.next() == Some(author_name.as_str())
        && fields.next() == Some(author_email.as_str())
        && fields.next() == Some(INITIAL_COMMIT_MESSAGE)
        && fields.next().is_none()
}

fn remove_generated_readme(repo_dir: &std::path::Path, expected_content: &str) {
    let path = repo_dir.join(README_PATH);
    if std::fs::read_to_string(&path).is_ok_and(|content| content == expected_content) {
        let _ = std::fs::remove_file(path);
    }
}

fn create_initial_commit(
    repo_dir: &std::path::Path,
    project_dtag: &str,
    owner: &str,
    auth: &GitAuthConfig,
) -> Result<String, String> {
    if local_head(repo_dir, auth).is_some() {
        return Err("The local repository already has a commit.".to_string());
    }
    if !worktree_is_pristine(repo_dir, auth)? {
        return Err(
            "The empty repository contains local files or staged changes; SchoolX left them unchanged."
                .to_string(),
        );
    }

    let content = readme_content(project_dtag);
    let readme_path = repo_dir.join(README_PATH);
    let write_result = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&readme_path)
        .and_then(|mut file| file.write_all(content.as_bytes()));
    if let Err(error) = write_result {
        remove_generated_readme(repo_dir, &content);
        return Err(format!("create {README_PATH}: {error}"));
    }

    if let Err(error) = run_git(&["add", "--", README_PATH], Some(repo_dir), auth) {
        let _ = run_git(&["read-tree", "--empty"], Some(repo_dir), auth);
        remove_generated_readme(repo_dir, &content);
        return Err(error);
    }

    let (author_name, author_email) = commit_author(owner);
    let name_config = format!("user.name={author_name}");
    let email_config = format!("user.email={author_email}");
    if let Err(error) = run_git(
        &[
            "-c",
            name_config.as_str(),
            "-c",
            email_config.as_str(),
            "commit",
            "--no-verify",
            "--no-gpg-sign",
            "-m",
            INITIAL_COMMIT_MESSAGE,
        ],
        Some(repo_dir),
        auth,
    ) {
        let _ = run_git(&["read-tree", "--empty"], Some(repo_dir), auth);
        remove_generated_readme(repo_dir, &content);
        return Err(error);
    }

    local_head(repo_dir, auth)
        .ok_or_else(|| "The initial commit was created but HEAD could not be resolved.".to_string())
}

fn checkout_remote_winner(
    repo_dir: &std::path::Path,
    project_dtag: &str,
    owner: &str,
    branch: &str,
    expected_remote_commit: &str,
    auth: &GitAuthConfig,
) -> Result<String, String> {
    let current_head = local_head(repo_dir, auth);
    if current_head.as_deref() == Some(expected_remote_commit) {
        return Ok(expected_remote_commit.to_string());
    }
    if !worktree_is_pristine(repo_dir, auth)? {
        return Err(
            "The remote repository is initialized, but the local checkout has changes; SchoolX left them unchanged."
                .to_string(),
        );
    }
    if current_head.is_some()
        && !generated_initial_commit_matches(repo_dir, project_dtag, owner, auth)
    {
        return Err(
            "The remote repository and local checkout have different commits; reconcile them in Terminal."
                .to_string(),
        );
    }

    run_git(
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            "--end-of-options",
            "origin",
            branch,
        ],
        Some(repo_dir),
        auth,
    )?;
    let fetched = run_git(&["rev-parse", "FETCH_HEAD"], Some(repo_dir), auth)
        .ok()
        .and_then(|output| first_output_line(&output))
        .ok_or_else(|| "Could not resolve the initialized remote branch.".to_string())?;
    run_git(
        &["checkout", "-B", branch, "FETCH_HEAD", "--"],
        Some(repo_dir),
        auth,
    )?;
    Ok(fetched)
}

fn initialized_result(
    repo_dir: &std::path::Path,
    cloned: bool,
    initialized: bool,
    pushed: bool,
    branch: &str,
    commit: String,
    message: String,
) -> ProjectRepoInitializeResult {
    ProjectRepoInitializeResult {
        path: repo_dir.display().to_string(),
        cloned,
        initialized,
        pushed,
        branch: branch.to_string(),
        commit,
        message,
    }
}

fn initialize_local_repository_blocking(
    repo_dir: &std::path::Path,
    cloned: bool,
    project_dtag: &str,
    clone_url: &str,
    branch: &str,
    owner: &str,
    auth: &GitAuthConfig,
) -> Result<ProjectRepoInitializeResult, String> {
    let heads = remote_heads(repo_dir, auth)?;
    if let Some(remote_commit) = remote_branch_head(&heads, branch) {
        let commit =
            checkout_remote_winner(repo_dir, project_dtag, owner, branch, remote_commit, auth)?;
        return Ok(initialized_result(
            repo_dir,
            cloned,
            false,
            false,
            branch,
            commit,
            format!("Repository already has a first commit on {branch}."),
        ));
    }
    if !heads.is_empty() {
        return Err(format!(
            "The remote repository already has branches, but {branch} was not found."
        ));
    }

    let commit = match local_head(repo_dir, auth) {
        Some(commit) => {
            if !generated_initial_commit_matches(repo_dir, project_dtag, owner, auth) {
                return Err(
                    "The local repository already has a commit that SchoolX did not create; push it manually."
                        .to_string(),
                );
            }
            commit
        }
        None => create_initial_commit(repo_dir, project_dtag, owner, auth)?,
    };

    match push_project_local_repository_blocking(
        repo_dir,
        clone_url.to_string(),
        Some(branch.to_string()),
        None,
        auth,
    ) {
        Ok(result) => Ok(initialized_result(
            repo_dir,
            cloned,
            true,
            result.pushed,
            &result.branch,
            result.commit,
            result.message,
        )),
        Err(push_error) => match remote_heads(repo_dir, auth) {
            Ok(heads) if remote_branch_head(&heads, branch) == Some(commit.as_str()) => {
                Ok(initialized_result(
                    repo_dir,
                    cloned,
                    true,
                    true,
                    branch,
                    commit,
                    format!(
                        "Pushed {branch} to remote; the remote commit was verified after the push response failed."
                    ),
                ))
            }
            Ok(heads) => {
                if let Some(remote_commit) = remote_branch_head(&heads, branch) {
                    let remote_commit = remote_commit.to_string();
                    let commit = checkout_remote_winner(
                        repo_dir,
                        project_dtag,
                        owner,
                        branch,
                        &remote_commit,
                        auth,
                    )?;
                    Ok(initialized_result(
                        repo_dir,
                        cloned,
                        false,
                        false,
                        branch,
                        commit,
                        format!(
                            "Another client initialized {branch} first; the local checkout now follows that commit."
                        ),
                    ))
                } else {
                    Err(format!(
                        "Failed to push the initial commit: {push_error}. The local SchoolX initial commit was kept so Retry can publish it."
                    ))
                }
            }
            Err(verification_error) => Err(format!(
                "Failed to push the initial commit: {push_error}. Remote verification also failed: {verification_error}. The local SchoolX initial commit was kept so Retry can publish it."
            )),
        },
    }
}

/// Creates and publishes the first commit for an owner-controlled empty
/// project, cloning its local checkout first when necessary.
#[tauri::command]
pub async fn initialize_project_repository(
    repos_dir: Option<String>,
    project_dtag: String,
    clone_url: String,
    default_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectRepoInitializeResult, String> {
    validate_workspace_clone_url(&clone_url, &state)?;
    let project_dtag = normalized_repository_id(&project_dtag)?;
    let branch = normalized_initial_branch(default_branch.as_deref())?;
    let keys = state.signing_keys()?;
    let owner = repository_owner_for_viewer(&clone_url, &project_dtag, &keys)?;
    let auth = build_git_auth_config_for_keys(&keys)?;

    tauri::async_runtime::spawn_blocking(move || {
        let clone_result = clone_project_repository_blocking(
            repos_dir.as_deref(),
            &project_dtag,
            &clone_url,
            Some(&branch),
            &auth,
        )?;
        let repo_dir = std::path::PathBuf::from(&clone_result.path);
        initialize_local_repository_blocking(
            &repo_dir,
            clone_result.cloned,
            &project_dtag,
            &clone_url,
            &branch,
            &owner,
            &auth,
        )
    })
    .await
    .map_err(|error| format!("repo initialization task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::{
        commit_author, create_initial_commit, initialize_local_repository_blocking,
        normalized_initial_branch, normalized_repository_id, repository_owner_for_viewer,
        README_PATH,
    };
    use crate::commands::project_git_exec::{build_test_git_auth_config, run_git};
    use nostr::Keys;

    struct RepositoryFixture {
        _root: tempfile::TempDir,
        checkout: std::path::PathBuf,
        remote: std::path::PathBuf,
        auth: crate::commands::project_git_exec::GitAuthConfig,
        owner: String,
    }

    impl RepositoryFixture {
        fn new() -> Result<Self, String> {
            let auth = build_test_git_auth_config()?;
            let root = tempfile::tempdir().map_err(|error| error.to_string())?;
            let remote = root.path().join("remote.git");
            let checkout = root.path().join("checkout");
            let remote_path = remote
                .to_str()
                .ok_or_else(|| "remote path is not UTF-8".to_string())?;
            let checkout_path = checkout
                .to_str()
                .ok_or_else(|| "checkout path is not UTF-8".to_string())?;
            run_git(&["init", "--bare", "--", remote_path], None, &auth)?;
            run_git(&["init", "--", checkout_path], None, &auth)?;
            run_git(
                &["symbolic-ref", "HEAD", "refs/heads/main"],
                Some(&checkout),
                &auth,
            )?;
            run_git(
                &["remote", "add", "origin", remote_path],
                Some(&checkout),
                &auth,
            )?;
            Ok(Self {
                _root: root,
                checkout,
                remote,
                auth,
                owner: "a".repeat(64),
            })
        }

        fn remote_path(&self) -> Result<&str, String> {
            self.remote
                .to_str()
                .ok_or_else(|| "remote path is not UTF-8".to_string())
        }
    }

    #[test]
    fn validates_initialization_inputs_and_owner() {
        assert_eq!(
            normalized_repository_id(" schoolx-demo "),
            Ok("schoolx-demo".into())
        );
        assert!(normalized_repository_id("../demo").is_err());
        assert!(normalized_repository_id("한글").is_err());
        assert_eq!(normalized_initial_branch(None), Ok("main".into()));
        assert_eq!(
            normalized_initial_branch(Some("refs/heads/feature/demo")),
            Ok("feature/demo".into())
        );
        assert!(normalized_initial_branch(Some("--upload-pack=evil")).is_err());

        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let clone_url = format!("https://relay.example/git/{owner}/demo");
        assert_eq!(
            repository_owner_for_viewer(&clone_url, "demo", &keys),
            Ok(owner)
        );
        assert!(repository_owner_for_viewer(&clone_url, "other", &keys).is_err());
        let other_url = format!("https://relay.example/git/{}/demo", "b".repeat(64));
        assert!(repository_owner_for_viewer(&other_url, "demo", &keys).is_err());
    }

    #[test]
    fn initializes_and_idempotently_reuses_the_remote_commit() -> Result<(), String> {
        let fixture = RepositoryFixture::new()?;
        let result = initialize_local_repository_blocking(
            &fixture.checkout,
            false,
            "demo",
            fixture.remote_path()?,
            "main",
            &fixture.owner,
            &fixture.auth,
        )?;
        assert!(result.initialized);
        assert!(result.pushed);
        assert_eq!(result.branch, "main");
        assert_eq!(
            std::fs::read_to_string(fixture.checkout.join(README_PATH))
                .map_err(|error| error.to_string())?,
            "# demo\n"
        );
        let (author_name, author_email) = commit_author(&fixture.owner);
        assert_eq!(
            run_git(
                &["show", "-s", "--format=%an%x00%ae%x00%s", "HEAD"],
                Some(&fixture.checkout),
                &fixture.auth,
            )?
            .trim_end(),
            format!("{author_name}\0{author_email}\0Initial commit")
        );

        let repeated = initialize_local_repository_blocking(
            &fixture.checkout,
            false,
            "demo",
            fixture.remote_path()?,
            "main",
            &fixture.owner,
            &fixture.auth,
        )?;
        assert!(!repeated.initialized);
        assert!(!repeated.pushed);
        assert_eq!(repeated.commit, result.commit);
        assert_eq!(
            run_git(
                &["rev-list", "--count", "HEAD"],
                Some(&fixture.checkout),
                &fixture.auth,
            )?
            .trim(),
            "1"
        );
        Ok(())
    }

    #[test]
    fn retries_a_verified_local_schoolx_initial_commit() -> Result<(), String> {
        let fixture = RepositoryFixture::new()?;
        let local_commit =
            create_initial_commit(&fixture.checkout, "demo", &fixture.owner, &fixture.auth)?;

        let result = initialize_local_repository_blocking(
            &fixture.checkout,
            false,
            "demo",
            fixture.remote_path()?,
            "main",
            &fixture.owner,
            &fixture.auth,
        )?;
        assert!(result.initialized);
        assert!(result.pushed);
        assert_eq!(result.commit, local_commit);
        Ok(())
    }

    #[test]
    fn follows_a_remote_commit_that_won_the_initialization_race() -> Result<(), String> {
        let fixture = RepositoryFixture::new()?;
        let winner_root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let winner = winner_root.path().join("winner");
        let winner_path = winner
            .to_str()
            .ok_or_else(|| "winner path is not UTF-8".to_string())?;
        run_git(&["init", "--", winner_path], None, &fixture.auth)?;
        run_git(
            &["symbolic-ref", "HEAD", "refs/heads/main"],
            Some(&winner),
            &fixture.auth,
        )?;
        std::fs::write(winner.join(README_PATH), "# winner\n")
            .map_err(|error| error.to_string())?;
        run_git(&["add", "--", README_PATH], Some(&winner), &fixture.auth)?;
        run_git(
            &[
                "-c",
                "user.name=Winner",
                "-c",
                "user.email=winner@example.com",
                "commit",
                "-m",
                "Winner commit",
            ],
            Some(&winner),
            &fixture.auth,
        )?;
        run_git(
            &["remote", "add", "origin", fixture.remote_path()?],
            Some(&winner),
            &fixture.auth,
        )?;
        run_git(
            &["push", "--end-of-options", "origin", "HEAD:main"],
            Some(&winner),
            &fixture.auth,
        )?;

        let result = initialize_local_repository_blocking(
            &fixture.checkout,
            false,
            "demo",
            fixture.remote_path()?,
            "main",
            &fixture.owner,
            &fixture.auth,
        )?;
        assert!(!result.initialized);
        assert!(!result.pushed);
        assert_eq!(
            std::fs::read_to_string(fixture.checkout.join(README_PATH))
                .map_err(|error| error.to_string())?,
            "# winner\n"
        );
        assert_eq!(
            run_git(
                &["rev-list", "--count", "HEAD"],
                Some(&fixture.checkout),
                &fixture.auth,
            )?
            .trim(),
            "1"
        );
        Ok(())
    }

    #[test]
    fn refuses_to_touch_an_unborn_checkout_with_user_files() -> Result<(), String> {
        let fixture = RepositoryFixture::new()?;
        let user_file = fixture.checkout.join("notes.txt");
        std::fs::write(&user_file, "keep me\n").map_err(|error| error.to_string())?;

        let error = initialize_local_repository_blocking(
            &fixture.checkout,
            false,
            "demo",
            fixture.remote_path()?,
            "main",
            &fixture.owner,
            &fixture.auth,
        )
        .expect_err("dirty unborn checkout must be rejected");
        assert!(error.contains("local files or staged changes"));
        assert_eq!(
            std::fs::read_to_string(user_file).map_err(|error| error.to_string())?,
            "keep me\n"
        );
        assert!(run_git(
            &["rev-parse", "--verify", "HEAD"],
            Some(&fixture.checkout),
            &fixture.auth,
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn refuses_to_publish_an_unrecognized_local_root_commit() -> Result<(), String> {
        let fixture = RepositoryFixture::new()?;
        std::fs::write(fixture.checkout.join("custom.txt"), "custom\n")
            .map_err(|error| error.to_string())?;
        run_git(
            &["add", "--", "custom.txt"],
            Some(&fixture.checkout),
            &fixture.auth,
        )?;
        run_git(
            &[
                "-c",
                "user.name=User",
                "-c",
                "user.email=user@example.com",
                "commit",
                "-m",
                "User commit",
            ],
            Some(&fixture.checkout),
            &fixture.auth,
        )?;

        let error = initialize_local_repository_blocking(
            &fixture.checkout,
            false,
            "demo",
            fixture.remote_path()?,
            "main",
            &fixture.owner,
            &fixture.auth,
        )
        .expect_err("user commit must not be auto-pushed");
        assert!(error.contains("did not create"));
        assert!(run_git(
            &[
                format!("--git-dir={}", fixture.remote_path()?).as_str(),
                "show-ref",
            ],
            None,
            &fixture.auth,
        )
        .is_err());
        Ok(())
    }
}
