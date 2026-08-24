//! Shared git subprocess plumbing for the project commands.
//!
//! Runs the system `git` with an ephemeral, env-only auth configuration:
//! the identity nsec is handed to `git-credential-nostr` via environment
//! variables so nothing key-related ever touches disk or global git config.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

mod auth;

#[cfg(test)]
pub(crate) use auth::build_test_git_auth_config;
#[cfg(test)]
use auth::validate_clone_url;
pub(crate) use auth::{
    build_git_auth_config, build_git_auth_config_for_keys, build_git_clone_auth_config,
    clean_branch, clean_target_ref, clone_url_owner, validate_local_clone_url,
    validate_local_clone_url_for_workspace, validate_workspace_clone_url,
};
use auth::{configure_git_auth, one_git_value};
#[cfg(test)]
use auth::{credential_helper_config_value, validate_clone_url_against_relay};

/// Wall-clock cap for a single git invocation. Remote operations talk to
/// relay-supplied clone URLs, so a slow or adversarial remote must not pin
/// `spawn_blocking` threads indefinitely.
const LOCAL_GIT_TIMEOUT: Duration = Duration::from_secs(60);
const REMOTE_GIT_TIMEOUT: Duration = Duration::from_secs(300);
const GIT_STDOUT_LIMIT: usize = 16 * 1024 * 1024;
const GIT_STDERR_LIMIT: usize = 1024 * 1024;
const CHANGES_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);
const CHANGES_SNAPSHOT_COMMAND_LIMIT: usize = 264;
const CHANGES_SNAPSHOT_OUTPUT_LIMIT: usize = 32 * 1024 * 1024;

fn git_subcommand<'a>(args: &'a [&str]) -> Option<&'a str> {
    let mut index = 0;
    while let Some(argument) = args.get(index).copied() {
        match argument {
            "-c" | "--config" | "-C" | "--git-dir" | "--work-tree" => index += 2,
            "--no-pager" | "--paginate" | "--end-of-options" => index += 1,
            argument
                if argument.starts_with("--config=")
                    || argument.starts_with("--git-dir=")
                    || argument.starts_with("--work-tree=") =>
            {
                index += 1;
            }
            argument if argument.starts_with('-') => index += 1,
            subcommand => return Some(subcommand),
        }
    }
    None
}

fn git_needs_credentials(args: &[&str]) -> bool {
    matches!(
        git_subcommand(args),
        Some("clone" | "fetch" | "push" | "pull" | "ls-remote" | "merge")
    )
}

pub(crate) struct GitAuthConfig {
    git_path: std::path::PathBuf,
    credential_helper: Option<std::path::PathBuf>,
    nsec: String,
    allow_file_transport: bool,
}

fn read_pipe_bounded(
    pipe: Option<impl Read>,
    limit: usize,
    label: &'static str,
    abort: Arc<AtomicBool>,
    shared_remaining: Option<Arc<AtomicUsize>>,
) -> Result<Vec<u8>, String> {
    let Some(mut pipe) = pipe else {
        return Ok(Vec::new());
    };
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = pipe.read(&mut buffer).map_err(|error| {
            abort.store(true, Ordering::Release);
            format!("failed to read git {label}: {error}")
        })?;
        if count == 0 {
            return Ok(bytes);
        }
        let remaining = limit.saturating_sub(bytes.len());
        let pipe_retained = count.min(remaining);
        let retained = shared_remaining.as_ref().map_or(pipe_retained, |shared| {
            claim_capture_bytes(shared, pipe_retained)
        });
        bytes.extend_from_slice(&buffer[..retained]);
        if retained < pipe_retained {
            abort.store(true, Ordering::Release);
            return Err("git output exceeded the Changes snapshot byte budget".to_string());
        }
        if count > remaining {
            abort.store(true, Ordering::Release);
            return Err(format!(
                "git {label} exceeded its {limit}-byte safety limit"
            ));
        }
    }
}

fn claim_capture_bytes(remaining: &AtomicUsize, requested: usize) -> usize {
    let mut current = remaining.load(Ordering::Acquire);
    loop {
        let claimed = current.min(requested);
        match remaining.compare_exchange_weak(
            current,
            current.saturating_sub(claimed),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return claimed,
            Err(updated) => current = updated,
        }
    }
}

pub(crate) fn run_git(
    args: &[&str],
    cwd: Option<&std::path::Path>,
    auth: &GitAuthConfig,
) -> Result<String, String> {
    run_git_accepting(args, cwd, auth, &[])
}

/// Run a read-only git command that uses documented non-zero statuses for
/// successful output. `git diff --no-index`, for example, returns 1 when it
/// found differences even though stdout contains the requested patch.
pub(crate) fn run_git_accepting(
    args: &[&str],
    cwd: Option<&std::path::Path>,
    auth: &GitAuthConfig,
    accepted_exit_codes: &[i32],
) -> Result<String, String> {
    let needs_credentials = git_needs_credentials(args);
    let timeout = if needs_credentials {
        REMOTE_GIT_TIMEOUT
    } else {
        LOCAL_GIT_TIMEOUT
    };
    run_git_accepting_limited(
        args,
        cwd,
        auth,
        accepted_exit_codes,
        timeout,
        GIT_STDOUT_LIMIT,
        GIT_STDERR_LIMIT,
    )
}

fn run_git_accepting_limited(
    args: &[&str],
    cwd: Option<&Path>,
    auth: &GitAuthConfig,
    accepted_exit_codes: &[i32],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<String, String> {
    let mut command = Command::new(&auth.git_path);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let needs_credentials = git_needs_credentials(args);
    configure_git_auth(&mut command, auth, needs_credentials);
    if matches!(
        git_subcommand(args),
        Some("diff" | "ls-files" | "rev-parse")
    ) {
        command.env("GIT_OPTIONAL_LOCKS", "0");
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    crate::util::configure_no_window(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let child = command
        .spawn()
        .map_err(|error| format!("failed to run git: {error}"))?;

    capture_git_child(
        child,
        accepted_exit_codes,
        timeout,
        stdout_limit,
        stderr_limit,
    )
}

fn capture_git_child(
    child: std::process::Child,
    accepted_exit_codes: &[i32],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<String, String> {
    capture_git_child_bytes(
        child,
        accepted_exit_codes,
        timeout,
        stdout_limit,
        stderr_limit,
        None,
    )
    .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

struct CapturedGitOutput {
    stdout: Vec<u8>,
    captured_bytes: usize,
}

fn capture_git_child_bytes(
    mut child: std::process::Child,
    accepted_exit_codes: &[i32],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    total_limit: Option<usize>,
) -> Result<CapturedGitOutput, String> {
    // Drain the pipes on background threads so a chatty git process can't
    // deadlock on a full pipe while we poll for exit below.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let abort = Arc::new(AtomicBool::new(false));
    let stdout_abort = Arc::clone(&abort);
    let stderr_abort = Arc::clone(&abort);
    let shared_remaining = total_limit.map(|limit| Arc::new(AtomicUsize::new(limit)));
    let stdout_shared = shared_remaining.as_ref().map(Arc::clone);
    let stderr_shared = shared_remaining.as_ref().map(Arc::clone);
    let stdout_thread = std::thread::spawn(move || {
        read_pipe_bounded(
            stdout_pipe,
            stdout_limit,
            "stdout",
            stdout_abort,
            stdout_shared,
        )
    });
    let stderr_thread = std::thread::spawn(move || {
        read_pipe_bounded(
            stderr_pipe,
            stderr_limit,
            "stderr",
            stderr_abort,
            stderr_shared,
        )
    });

    let started = Instant::now();
    let mut forced_error = None;
    let status = loop {
        if abort.load(Ordering::Acquire) {
            forced_error = Some("git output capture failed".to_string());
            terminate_git_process_tree(&mut child);
            break None;
        }
        match poll_git_child(&mut child) {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() > timeout {
                    forced_error = Some(format!("git timed out after {}s", timeout.as_secs()));
                    terminate_git_process_tree(&mut child);
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                forced_error = Some(format!("failed to wait for git: {error}"));
                terminate_git_process_tree(&mut child);
                break None;
            }
        }
    };

    let stdout = stdout_thread
        .join()
        .map_err(|_| "git stdout reader panicked".to_string())?;
    let stderr = stderr_thread
        .join()
        .map_err(|_| "git stderr reader panicked".to_string())?;
    if let Some(error) = forced_error {
        return match (stdout, stderr) {
            (Err(pipe_error), _) | (_, Err(pipe_error)) => Err(pipe_error),
            (Ok(_), Ok(_)) => Err(error),
        };
    }
    let stdout = stdout?;
    let stderr = stderr?;
    let status = status.ok_or_else(|| "git exited without a status".to_string())?;
    if !status.success()
        && !status
            .code()
            .is_some_and(|code| accepted_exit_codes.contains(&code))
    {
        let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("git exited with status {status}")
        } else {
            stderr
        });
    }
    let captured_bytes = total_limit.map_or_else(
        || stdout.len().saturating_add(stderr.len()),
        |limit| {
            limit.saturating_sub(
                shared_remaining
                    .as_ref()
                    .map_or(0, |remaining| remaining.load(Ordering::Acquire)),
            )
        },
    );
    Ok(CapturedGitOutput {
        stdout,
        captured_bytes,
    })
}

#[cfg(all(target_os = "macos", not(test)))]
fn capture_macos_git_child_bytes(
    mut child: crate::code_workspace::PinnedGitChild,
    accepted_exit_codes: &[i32],
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    total_limit: Option<usize>,
) -> Result<CapturedGitOutput, String> {
    let stdout_pipe = Some(child.take_stdout()?);
    let stderr_pipe = Some(child.take_stderr()?);
    let abort = Arc::new(AtomicBool::new(false));
    let stdout_abort = Arc::clone(&abort);
    let stderr_abort = Arc::clone(&abort);
    let shared_remaining = total_limit.map(|limit| Arc::new(AtomicUsize::new(limit)));
    let stdout_shared = shared_remaining.as_ref().map(Arc::clone);
    let stderr_shared = shared_remaining.as_ref().map(Arc::clone);
    let stdout_thread = std::thread::spawn(move || {
        read_pipe_bounded(
            stdout_pipe,
            stdout_limit,
            "stdout",
            stdout_abort,
            stdout_shared,
        )
    });
    let stderr_thread = std::thread::spawn(move || {
        read_pipe_bounded(
            stderr_pipe,
            stderr_limit,
            "stderr",
            stderr_abort,
            stderr_shared,
        )
    });

    let started = Instant::now();
    let mut forced_error = None;
    let status = loop {
        if abort.load(Ordering::Acquire) {
            forced_error = Some("git output capture failed".to_string());
            let _ = child.terminate();
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() <= timeout => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                forced_error = Some(format!("git timed out after {}s", timeout.as_secs()));
                let _ = child.terminate();
                break None;
            }
            Err(error) => {
                forced_error = Some(format!("failed to wait for git: {error}"));
                let _ = child.terminate();
                break None;
            }
        }
    };

    let stdout = stdout_thread
        .join()
        .map_err(|_| "git stdout reader panicked".to_string())?;
    let stderr = stderr_thread
        .join()
        .map_err(|_| "git stderr reader panicked".to_string())?;
    if let Some(error) = forced_error {
        return match (stdout, stderr) {
            (Err(pipe_error), _) | (_, Err(pipe_error)) => Err(pipe_error),
            (Ok(_), Ok(_)) => Err(error),
        };
    }
    let stdout = stdout?;
    let stderr = stderr?;
    let status = status.ok_or_else(|| "git exited without a status".to_string())?;
    if !status.success()
        && !status
            .code()
            .is_some_and(|code| accepted_exit_codes.contains(&code))
    {
        let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("git exited with status {status}")
        } else {
            stderr
        });
    }
    let captured_bytes = total_limit.map_or_else(
        || stdout.len().saturating_add(stderr.len()),
        |limit| {
            limit.saturating_sub(
                shared_remaining
                    .as_ref()
                    .map_or(0, |remaining| remaining.load(Ordering::Acquire)),
            )
        },
    );
    Ok(CapturedGitOutput {
        stdout,
        captured_bytes,
    })
}

fn terminate_git_process_tree(child: &mut std::process::Child) {
    kill_dedicated_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn poll_git_child(
    child: &mut std::process::Child,
) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
    let pid = rustix::process::Pid::from_raw(child.id() as i32)
        .ok_or_else(|| std::io::Error::other("git child PID was invalid"))?;
    let observed = rustix::process::waitid(
        rustix::process::WaitId::Pid(pid),
        rustix::process::WaitIdOptions::EXITED
            | rustix::process::WaitIdOptions::NOHANG
            | rustix::process::WaitIdOptions::NOWAIT,
    )
    .map_err(std::io::Error::from)?;
    if observed.is_none() {
        return Ok(None);
    }
    // WNOWAIT keeps the leader reserved while the entire dedicated group is
    // killed, closing descendant-held pipes without a PID/PGID reuse window.
    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    child.wait().map(Some)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn poll_git_child(
    child: &mut std::process::Child,
) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
    child.try_wait()
}

#[cfg(unix)]
fn kill_dedicated_process_group(raw_pid: u32) {
    if let Some(pid) = rustix::process::Pid::from_raw(raw_pid as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

#[cfg(not(unix))]
fn kill_dedicated_process_group(raw_pid: u32) {
    let _ = crate::managed_agents::terminate_process(raw_pid);
}

/// An open execution-root directory used as every child process' cwd for one
/// SchoolX Code Changes snapshot. The descriptor prevents a path replacement
/// from redirecting later Git commands to another directory.
#[cfg(unix)]
pub(crate) struct PinnedGitDirectory {
    expected_path: PathBuf,
    descriptor: std::fs::File,
    device: u64,
    inode: u64,
    git_entry: std::fs::File,
    git_entry_device: u64,
    git_entry_inode: u64,
    #[cfg(target_os = "linux")]
    launch: crate::code_workspace::git_launch::GitLaunchAuthority,
    #[cfg(all(target_os = "macos", not(test)))]
    session: crate::code_workspace::macos_git_xpc::MacGitAuthoritySession,
}

#[cfg(unix)]
impl PinnedGitDirectory {
    pub(crate) fn pin(expected_path: &Path) -> Result<Self, String> {
        use std::os::fd::AsFd;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

        if !expected_path.is_absolute() {
            return Err("SchoolX Code Git root must be absolute before it is pinned".to_string());
        }
        let descriptor = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(expected_path)
            .map_err(|error| {
                format!(
                    "failed to pin SchoolX Code Git root {}: {error}",
                    expected_path.display()
                )
            })?;
        let metadata = descriptor
            .metadata()
            .map_err(|error| format!("failed to inspect pinned SchoolX Code Git root: {error}"))?;
        if !metadata.is_dir() {
            return Err("pinned SchoolX Code Git root is not a directory".to_string());
        }
        let git_entry = rustix::fs::openat(
            descriptor.as_fd(),
            ".git",
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        )
        .map(std::fs::File::from)
        .map_err(|error| format!("failed to pin SchoolX Code .git entry: {error}"))?;
        let git_entry_metadata = git_entry
            .metadata()
            .map_err(|error| format!("failed to inspect pinned .git entry: {error}"))?;
        if !(git_entry_metadata.is_dir() || git_entry_metadata.is_file()) {
            return Err("SchoolX Code .git entry was not a directory or gitfile".to_string());
        }
        #[cfg(target_os = "linux")]
        let launch = crate::code_workspace::git_launch::GitLaunchAuthority::admit(&descriptor)?;
        #[cfg(all(target_os = "macos", not(test)))]
        let session = {
            drop(crate::code_workspace::git_write::macos_root_trusted_git()?);
            crate::code_workspace::macos_git_xpc::MacGitAuthoritySession::begin()?
        };
        let pinned = Self {
            expected_path: expected_path.to_path_buf(),
            device: metadata.dev(),
            inode: metadata.ino(),
            descriptor,
            git_entry_device: git_entry_metadata.dev(),
            git_entry_inode: git_entry_metadata.ino(),
            git_entry,
            #[cfg(target_os = "linux")]
            launch,
            #[cfg(all(target_os = "macos", not(test)))]
            session,
        };
        pinned.verify_named_identity()?;
        Ok(pinned)
    }

    fn verify_named_identity(&self) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;

        let named = self.expected_path.symlink_metadata().map_err(|error| {
            format!(
                "failed to verify pinned SchoolX Code Git root {}: {error}",
                self.expected_path.display()
            )
        })?;
        if named.file_type().is_symlink()
            || !named.is_dir()
            || named.dev() != self.device
            || named.ino() != self.inode
        {
            return Err(
                "SchoolX Code Git root moved or was replaced during inspection".to_string(),
            );
        }
        let open = self
            .descriptor
            .metadata()
            .map_err(|error| format!("failed to recheck pinned SchoolX Code Git root: {error}"))?;
        if !open.is_dir() || open.dev() != self.device || open.ino() != self.inode {
            return Err("SchoolX Code Git root handle changed during inspection".to_string());
        }
        let git_entry = self.git_entry.metadata().map_err(|error| {
            format!("failed to recheck pinned SchoolX Code .git entry: {error}")
        })?;
        let named_git = self
            .expected_path
            .join(".git")
            .symlink_metadata()
            .map_err(|error| format!("failed to verify pinned SchoolX Code .git entry: {error}"))?;
        if named_git.file_type().is_symlink()
            || named_git.dev() != self.git_entry_device
            || named_git.ino() != self.git_entry_inode
            || git_entry.dev() != self.git_entry_device
            || git_entry.ino() != self.git_entry_inode
        {
            return Err(
                "SchoolX Code .git entry moved or was replaced during inspection".to_string(),
            );
        }
        Ok(())
    }

    fn spawn_read(
        &self,
        git_executable: &Path,
        command: crate::code_workspace::CodePinnedReadCommand,
        disabled_filter_keys: Vec<String>,
    ) -> Result<crate::code_workspace::PinnedGitChild, String> {
        self.verify_named_identity()?;
        #[cfg(target_os = "linux")]
        return crate::code_workspace::spawn_pinned_read_git_helper(
            &self.descriptor,
            &self.expected_path,
            git_executable,
            &self.launch,
            command,
            disabled_filter_keys,
        );
        #[cfg(not(target_os = "linux"))]
        crate::code_workspace::spawn_pinned_read_git_helper(
            &self.descriptor,
            &self.expected_path,
            git_executable,
            #[cfg(all(target_os = "macos", not(test)))]
            &self.session,
            command,
            disabled_filter_keys,
        )
    }
}

#[cfg(unix)]
struct PinnedCommonDirectory {
    expected_path: PathBuf,
    descriptor: std::fs::File,
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl PinnedCommonDirectory {
    fn pin(expected_path: &Path) -> Result<Self, String> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

        if !expected_path.is_absolute() {
            return Err("Git common directory was not absolute".to_string());
        }
        let descriptor = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(expected_path)
            .map_err(|error| format!("failed to pin Git common directory: {error}"))?;
        let metadata = descriptor
            .metadata()
            .map_err(|error| format!("failed to inspect pinned Git common directory: {error}"))?;
        if !metadata.is_dir() {
            return Err("pinned Git common directory was not a directory".to_string());
        }
        let pinned = Self {
            expected_path: expected_path.to_path_buf(),
            device: metadata.dev(),
            inode: metadata.ino(),
            descriptor,
        };
        pinned.verify()?;
        Ok(pinned)
    }

    fn verify(&self) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;

        let named = self
            .expected_path
            .symlink_metadata()
            .map_err(|error| format!("failed to verify Git common directory: {error}"))?;
        let open = self
            .descriptor
            .metadata()
            .map_err(|error| format!("failed to recheck Git common directory: {error}"))?;
        if named.file_type().is_symlink()
            || !named.is_dir()
            || named.dev() != self.device
            || named.ino() != self.inode
            || open.dev() != self.device
            || open.ino() != self.inode
        {
            return Err("Git common directory moved or was replaced during inspection".to_string());
        }
        Ok(())
    }
}

#[cfg(not(unix))]
pub(crate) struct PinnedGitDirectory;

#[cfg(not(unix))]
impl PinnedGitDirectory {
    pub(crate) fn pin(_expected_path: &Path) -> Result<Self, String> {
        Err(
            "SchoolX Code Changes requires secure pinned-directory support on this platform"
                .to_string(),
        )
    }

    fn verify_named_identity(&self) -> Result<(), String> {
        Err(
            "SchoolX Code Changes requires secure pinned-directory support on this platform"
                .to_string(),
        )
    }

    fn spawn_read(
        &self,
        _git_executable: &Path,
        _command: crate::code_workspace::CodePinnedReadCommand,
        _disabled_filter_keys: Vec<String>,
    ) -> Result<std::process::Child, String> {
        self.verify_named_identity()?;
        Err("SchoolX Code Changes directory is unavailable".to_string())
    }
}

/// Run one complete pinned Git read snapshot under an explicit macOS session
/// end fence. The nested directory object borrows the ambient session, so a
/// successful result is not exposed until the signed service proves release.
pub(crate) fn with_pinned_git_directory<T>(
    expected_path: &Path,
    operation: impl FnOnce(&PinnedGitDirectory) -> Result<T, String>,
) -> Result<T, String> {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        crate::code_workspace::macos_git_xpc::with_authority_session(|_| {
            let pinned = PinnedGitDirectory::pin(expected_path)?;
            operation(&pinned)
        })
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let pinned = PinnedGitDirectory::pin(expected_path)?;
        operation(&pinned)
    }
}

#[cfg(not(unix))]
struct PinnedCommonDirectory;

#[cfg(not(unix))]
impl PinnedCommonDirectory {
    fn pin(_expected_path: &Path) -> Result<Self, String> {
        Err("secure Git common-directory pinning is unavailable".to_string())
    }

    fn verify(&self) -> Result<(), String> {
        Err("secure Git common-directory pinning is unavailable".to_string())
    }
}

/// Shared wall-clock, process-count, and captured-output budget for one
/// Changes snapshot. Each command also uses the same pinned directory handle.
pub(crate) struct GitReadSnapshot<'a> {
    directory: &'a PinnedGitDirectory,
    git_executable: PathBuf,
    started: Instant,
    remaining_commands: usize,
    remaining_output: usize,
    disabled_filter_keys: Vec<String>,
    common_directory: Option<PinnedCommonDirectory>,
}

impl<'a> GitReadSnapshot<'a> {
    pub(crate) fn new(
        directory: &'a PinnedGitDirectory,
        auth: &'a GitAuthConfig,
        expected_execution_root: &str,
        expected_repository_identity: &str,
        base_commit: &str,
    ) -> Result<Self, String> {
        directory.verify_named_identity()?;
        let git_executable = auth
            .git_path
            .canonicalize()
            .map_err(|error| format!("failed to pin the Git executable: {error}"))?;
        if !git_executable.is_file() {
            return Err("pinned Git executable is not a regular file".to_string());
        }
        let mut snapshot = Self {
            directory,
            git_executable,
            started: Instant::now(),
            remaining_commands: CHANGES_SNAPSHOT_COMMAND_LIMIT,
            remaining_output: CHANGES_SNAPSHOT_OUTPUT_LIMIT,
            disabled_filter_keys: Vec::new(),
            common_directory: None,
        };
        let top_level = snapshot
            .run_command_bytes(crate::code_workspace::CodePinnedReadCommand::TopLevel, &[])?;
        if one_git_value(&top_level, "Git top-level")? != expected_execution_root {
            return Err("pinned SchoolX Code Git root changed repository identity".to_string());
        }
        let common_dir = snapshot
            .run_command_bytes(crate::code_workspace::CodePinnedReadCommand::CommonDir, &[])?;
        let common_dir = one_git_value(&common_dir, "Git common directory")?;
        if crate::code_workspace::repository_identity(Path::new(&common_dir))?
            != expected_repository_identity
        {
            return Err("pinned SchoolX Code Git common directory changed identity".to_string());
        }
        snapshot.common_directory = Some(PinnedCommonDirectory::pin(Path::new(&common_dir))?);
        let resolved_base = snapshot.run_command_bytes(
            crate::code_workspace::CodePinnedReadCommand::VerifyCommit {
                commit: base_commit.to_string(),
            },
            &[],
        )?;
        if one_git_value(&resolved_base, "SchoolX Code base commit")? != base_commit {
            return Err("pinned SchoolX Code base commit changed".to_string());
        }
        let local_config = snapshot.run_command_bytes(
            crate::code_workspace::CodePinnedReadCommand::LocalConfig,
            &[],
        )?;
        let mut overrides = std::collections::BTreeSet::new();
        let worktree_config_enabled =
            crate::code_workspace::collect_local_filter_overrides(&local_config, &mut overrides)?;
        if worktree_config_enabled {
            let worktree_config = snapshot.run_command_bytes(
                crate::code_workspace::CodePinnedReadCommand::WorktreeConfigNames,
                &[],
            )?;
            crate::code_workspace::collect_filter_override_names(&worktree_config, &mut overrides)?;
        }
        snapshot.disabled_filter_keys = overrides.into_iter().collect();
        Ok(snapshot)
    }

    pub(crate) fn tracked_numstat(&mut self, base_commit: &str) -> Result<String, String> {
        self.run_command(
            crate::code_workspace::CodePinnedReadCommand::TrackedNumstat {
                base_commit: base_commit.to_string(),
            },
            &[],
        )
    }

    pub(crate) fn tracked_name_status(&mut self, base_commit: &str) -> Result<String, String> {
        self.run_command(
            crate::code_workspace::CodePinnedReadCommand::TrackedNameStatus {
                base_commit: base_commit.to_string(),
            },
            &[],
        )
    }

    pub(crate) fn tracked_unmerged_paths(&mut self) -> Result<String, String> {
        self.run_command(
            crate::code_workspace::CodePinnedReadCommand::TrackedUnmergedPaths,
            &[],
        )
    }

    pub(crate) fn tracked_patch(
        &mut self,
        base_commit: &str,
        path: &str,
    ) -> Result<String, String> {
        self.run_command(
            crate::code_workspace::CodePinnedReadCommand::TrackedPatch {
                base_commit: base_commit.to_string(),
                path: path.to_string(),
            },
            &[],
        )
    }

    pub(crate) fn untracked_paths(&mut self) -> Result<String, String> {
        self.run_command(
            crate::code_workspace::CodePinnedReadCommand::UntrackedPaths,
            &[],
        )
    }

    pub(crate) fn untracked_patch(&mut self, path: &str) -> Result<String, String> {
        self.run_command(
            crate::code_workspace::CodePinnedReadCommand::UntrackedPatch {
                path: path.to_string(),
            },
            &[1],
        )
    }

    fn run_command(
        &mut self,
        command: crate::code_workspace::CodePinnedReadCommand,
        accepted_exit_codes: &[i32],
    ) -> Result<String, String> {
        self.run_command_bytes(command, accepted_exit_codes)
            .and_then(|output| {
                String::from_utf8(output)
                    .map_err(|error| format!("Git returned non-UTF-8 Changes output: {error}"))
            })
    }

    fn run_command_bytes(
        &mut self,
        command: crate::code_workspace::CodePinnedReadCommand,
        accepted_exit_codes: &[i32],
    ) -> Result<Vec<u8>, String> {
        if self.remaining_commands == 0 {
            return Err("SchoolX Code Changes exceeded its Git command budget".to_string());
        }
        let remaining_time = CHANGES_SNAPSHOT_TIMEOUT
            .checked_sub(self.started.elapsed())
            .ok_or_else(|| "SchoolX Code Changes exceeded its deadline".to_string())?;
        if remaining_time.is_zero() {
            return Err("SchoolX Code Changes exceeded its deadline".to_string());
        }
        if self.remaining_output == 0 {
            return Err("SchoolX Code Changes exceeded its captured-output budget".to_string());
        }
        self.remaining_commands = self.remaining_commands.saturating_sub(1);
        if let Some(common_directory) = &self.common_directory {
            common_directory.verify()?;
        }
        let child = self.directory.spawn_read(
            &self.git_executable,
            command,
            self.disabled_filter_keys.clone(),
        )?;
        #[cfg(all(target_os = "macos", not(test)))]
        let output = capture_macos_git_child_bytes(
            child,
            accepted_exit_codes,
            remaining_time.min(LOCAL_GIT_TIMEOUT),
            self.remaining_output.min(GIT_STDOUT_LIMIT),
            GIT_STDERR_LIMIT,
            Some(self.remaining_output),
        )?;
        #[cfg(any(not(target_os = "macos"), test))]
        let output = capture_git_child_bytes(
            child,
            accepted_exit_codes,
            remaining_time.min(LOCAL_GIT_TIMEOUT),
            self.remaining_output.min(GIT_STDOUT_LIMIT),
            GIT_STDERR_LIMIT,
            Some(self.remaining_output),
        )?;
        #[cfg(test)]
        let stdout = output
            .stdout
            .strip_prefix(b"\nrunning 1 test\n")
            .unwrap_or(&output.stdout)
            .to_vec();
        #[cfg(not(test))]
        let stdout = output.stdout;
        self.remaining_output = self.remaining_output.saturating_sub(output.captured_bytes);
        self.directory.verify_named_identity()?;
        if let Some(common_directory) = &self.common_directory {
            common_directory.verify()?;
        }
        Ok(stdout)
    }
}

#[cfg(test)]
mod tests;
