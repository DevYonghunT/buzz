//! Linux-only, descriptor-bound launch authority for typed SchoolX Code Git.
//!
//! Rust's safe process API accepts a path for `current_dir`, not a directory
//! descriptor. On the pinned Linux release/toolchain tuple, the hard-coded
//! `/proc/self/fd/<n>` magic-link namespace lets the child-side chdir action
//! resolve an inherited, close-on-exec directory descriptor without clearing
//! `FD_CLOEXEC` or exposing an ambient descriptor after Git is executed.

use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::git_write::RootTrustedGit;

const PROC_SELF_FD: &str = "/proc/self/fd";
const FIRST_NON_STDIO_FD: i32 = 3;
const CAPABILITY_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const CAPABILITY_PROBE_OUTPUT_LIMIT: usize = 64 * 1024;

/// A command whose executable is fixed to the pinned root-trusted Git.
///
/// Callers may add only their already typed arguments and environment. There
/// is no program setter on `Command`, and the launcher installs cwd and stdio
/// itself immediately before spawning.
pub(crate) struct TrustedGitCommand {
    inner: Command,
}

impl TrustedGitCommand {
    /// Access command configuration without exposing a way to replace Git.
    pub(crate) fn command_mut(&mut self) -> &mut Command {
        &mut self.inner
    }
}

/// Root-trusted Git plus the Linux descriptor-spawn capability.
#[derive(Clone)]
pub(crate) struct GitLaunchAuthority {
    git: RootTrustedGit,
}

impl GitLaunchAuthority {
    /// Admit the launcher only after the live Linux procfs descriptor namespace
    /// and one already-open probe directory have passed the same checks used at
    /// every spawn. Call this before any new journal claim or filesystem/Git
    /// mutation.
    pub(crate) fn admit(probe_directory: &File) -> Result<Self, String> {
        let git = RootTrustedGit::pin()?;
        Self::admit_with_git(probe_directory, git)
    }

    /// Admit a Git identity that was already pinned by a typed caller.
    ///
    /// Git write journals retain that exact identity as durable evidence, so
    /// this constructor avoids selecting the root-trusted executable twice.
    pub(in crate::code_workspace) fn admit_with_git(
        probe_directory: &File,
        git: RootTrustedGit,
    ) -> Result<Self, String> {
        drop(DescriptorCwd::pin(probe_directory)?);
        git.revalidate()?;
        let authority = Self { git };
        authority.probe_runtime(probe_directory)?;
        Ok(authority)
    }

    /// Start a cleared-environment command fixed to the pinned Git identity.
    pub(crate) fn command(&self) -> TrustedGitCommand {
        let mut inner = Command::new(self.git.path());
        inner.env_clear();
        TrustedGitCommand { inner }
    }

    /// Return the fixed root-trusted Git path for legacy typed request
    /// validation. The path is never accepted as launch authority on Linux.
    pub(crate) fn path(&self) -> &Path {
        self.git.path()
    }

    /// Spawn Git with `directory` as its descriptor-bound cwd.
    ///
    /// The returned ordinary `Child` preserves the existing bounded capture,
    /// timeout, and process-group cleanup implementations at each typed caller.
    /// `stdin` is installed independently from the cwd descriptor.
    pub(crate) fn spawn(
        &self,
        directory: &File,
        command: TrustedGitCommand,
        stdin: Stdio,
    ) -> Result<Child, String> {
        self.spawn_with_hook(directory, command, stdin, || Ok(()))
    }

    fn spawn_with_hook<F>(
        &self,
        directory: &File,
        mut command: TrustedGitCommand,
        stdin: Stdio,
        after_cwd_pinned: F,
    ) -> Result<Child, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let cwd = DescriptorCwd::pin(directory)?;
        self.git.revalidate()?;
        command
            .inner
            .current_dir(cwd.path())
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::util::configure_no_window(&mut command.inner);
        command.inner.process_group(0);
        after_cwd_pinned()?;
        command
            .inner
            .spawn()
            .map_err(|error| format!("failed to spawn descriptor-bound Git: {error}"))
    }

    fn probe_runtime(&self, directory: &File) -> Result<(), String> {
        let mut command = self.command();
        command.command_mut().arg("--version").env("LC_ALL", "C");
        let child = self.spawn(directory, command, Stdio::null())?;
        let (status, stdout, stderr) = capture_capability_probe(child)?;
        self.git.revalidate()?;
        if !status.success()
            || !stderr.is_empty()
            || !stdout.starts_with(b"git version ")
            || stdout.len() > CAPABILITY_PROBE_OUTPUT_LIMIT
        {
            return Err("Linux descriptor-bound Git runtime probe failed closed".to_string());
        }
        Ok(())
    }
}

fn capture_capability_probe(
    mut child: Child,
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "descriptor-bound Git probe stdout was unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "descriptor-bound Git probe stderr was unavailable".to_string())?;
    let stdout_reader = std::thread::spawn(move || read_probe_pipe(stdout));
    let stderr_reader = std::thread::spawn(move || read_probe_pipe(stderr));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < CAPABILITY_PROBE_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                terminate_probe(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("Linux descriptor-bound Git runtime probe timed out".to_string());
            }
            Err(error) => {
                terminate_probe(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!(
                    "failed to wait for descriptor-bound Git runtime probe: {error}"
                ));
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "descriptor-bound Git probe stdout reader panicked".to_string())??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "descriptor-bound Git probe stderr reader panicked".to_string())??;
    Ok((status, stdout, stderr))
}

fn read_probe_pipe(mut pipe: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    pipe.by_ref()
        .take((CAPABILITY_PROBE_OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read descriptor-bound Git probe output: {error}"))?;
    if bytes.len() > CAPABILITY_PROBE_OUTPUT_LIMIT {
        return Err("descriptor-bound Git probe output exceeded its limit".to_string());
    }
    Ok(bytes)
}

fn terminate_probe(child: &mut Child) {
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Owned duplicate that remains live until `Command::spawn` has completed.
struct DescriptorCwd {
    descriptor: OwnedFd,
    proc_path: PathBuf,
}

impl DescriptorCwd {
    fn pin(source: &File) -> Result<Self, String> {
        let descriptor = rustix::io::fcntl_dupfd_cloexec(source, FIRST_NON_STDIO_FD)
            .map_err(|error| format!("failed to duplicate pinned Git cwd: {error}"))?;
        let raw = descriptor.as_raw_fd();
        if raw < FIRST_NON_STDIO_FD {
            return Err("descriptor-bound Git cwd collided with standard I/O".to_string());
        }
        let flags = rustix::io::fcntl_getfd(&descriptor)
            .map_err(|error| format!("failed to inspect pinned Git cwd flags: {error}"))?;
        if !flags.contains(rustix::io::FdFlags::CLOEXEC) {
            return Err("descriptor-bound Git cwd was not close-on-exec".to_string());
        }

        let expected = rustix::fs::fstat(&descriptor)
            .map_err(|error| format!("failed to inspect pinned Git cwd: {error}"))?;
        if !rustix::fs::FileType::from_raw_mode(expected.st_mode).is_dir() {
            return Err("descriptor-bound Git cwd is not a directory".to_string());
        }

        let proc_directory = rustix::fs::open(
            PROC_SELF_FD,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| format!("Linux descriptor cwd is unavailable: {error}"))?;
        let proc_filesystem = rustix::fs::fstatfs(&proc_directory).map_err(|error| {
            format!("failed to inspect Linux descriptor cwd namespace: {error}")
        })?;
        if proc_filesystem.f_type != rustix::fs::PROC_SUPER_MAGIC {
            return Err("Linux descriptor cwd namespace is not procfs".to_string());
        }

        let component = raw.to_string();
        let link = rustix::fs::statat(
            &proc_directory,
            component.as_str(),
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|error| format!("failed to inspect Linux descriptor cwd link: {error}"))?;
        if !rustix::fs::FileType::from_raw_mode(link.st_mode).is_symlink() {
            return Err("Linux descriptor cwd entry is not a procfs magic link".to_string());
        }
        let resolved = rustix::fs::openat(
            &proc_directory,
            component.as_str(),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| format!("failed to resolve Linux descriptor cwd: {error}"))?;
        let observed = rustix::fs::fstat(&resolved)
            .map_err(|error| format!("failed to verify Linux descriptor cwd: {error}"))?;
        if !rustix::fs::FileType::from_raw_mode(observed.st_mode).is_dir()
            || observed.st_dev != expected.st_dev
            || observed.st_ino != expected.st_ino
        {
            return Err("Linux descriptor cwd resolved to a different directory".to_string());
        }

        Ok(Self {
            descriptor,
            proc_path: Path::new(PROC_SELF_FD).join(component),
        })
    }

    fn path(&self) -> &OsStr {
        // Holding this field is the authority: it keeps the numeric procfs
        // entry live until the child-side chdir action has consumed it.
        let _keep_alive = &self.descriptor;
        self.proc_path.as_os_str()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    use std::time::{Duration, Instant};

    use sha1::{Digest as _, Sha1};

    use super::*;

    const TEST_TIMEOUT: Duration = Duration::from_secs(10);

    fn open_directory(path: &Path) -> Result<File, String> {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| error.to_string())
    }

    fn wait_with_output(mut child: Child) -> Result<std::process::Output, String> {
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| "descriptor-bound Git stdout was unavailable".to_string())?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| "descriptor-bound Git stderr was unavailable".to_string())?;
        let stdout_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).map(|_| bytes)
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).map(|_| bytes)
        });
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < TEST_TIMEOUT => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
                        let _ =
                            rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
                    }
                    let _ = child.wait();
                    return Err("descriptor-bound Git test timed out".to_string());
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        let stdout = stdout_reader
            .join()
            .map_err(|_| "stdout reader panicked".to_string())?
            .map_err(|error| error.to_string())?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| "stderr reader panicked".to_string())?
            .map_err(|error| error.to_string())?;
        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    }

    fn git_blob_oid(bytes: &[u8]) -> String {
        let mut hasher = Sha1::new();
        hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    #[test]
    fn descriptor_cwd_is_non_stdio_cloexec_and_rejects_regular_file() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Ok(());
        }
        let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
        let directory = open_directory(sandbox.path())?;
        let cwd = DescriptorCwd::pin(&directory)?;
        assert!(cwd.descriptor.as_raw_fd() >= FIRST_NON_STDIO_FD);
        assert!(rustix::io::fcntl_getfd(&cwd.descriptor)
            .map_err(|error| error.to_string())?
            .contains(rustix::io::FdFlags::CLOEXEC));

        let regular_path = sandbox.path().join("regular");
        std::fs::write(&regular_path, b"not a directory\n").map_err(|error| error.to_string())?;
        let regular = File::open(&regular_path).map_err(|error| error.to_string())?;
        assert!(DescriptorCwd::pin(&regular).is_err_and(|error| error.contains("not a directory")));
        Ok(())
    }

    #[test]
    fn renamed_and_replaced_path_still_launches_in_opened_directory() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Ok(());
        }
        let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
        let original = sandbox.path().join("target");
        let moved = sandbox.path().join("moved");
        std::fs::create_dir(&original).map_err(|error| error.to_string())?;
        let expected = b"descriptor-bound original\n";
        std::fs::write(original.join("marker"), expected).map_err(|error| error.to_string())?;
        let directory = open_directory(&original)?;

        let authority = GitLaunchAuthority::admit(&directory)?;
        let mut command = authority.command();
        command
            .command_mut()
            .args(["hash-object", "--no-filters", "--", "marker"])
            .env("LC_ALL", "C");
        let output = wait_with_output(authority.spawn_with_hook(
            &directory,
            command,
            Stdio::null(),
            || {
                std::fs::rename(&original, &moved).map_err(|error| error.to_string())?;
                std::fs::create_dir(&original).map_err(|error| error.to_string())?;
                std::fs::write(original.join("marker"), b"replacement\n")
                    .map_err(|error| error.to_string())
            },
        )?)?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout)
                .map_err(|error| error.to_string())?
                .trim(),
            git_blob_oid(expected)
        );
        Ok(())
    }

    #[test]
    fn stdin_capture_and_process_group_are_independent_from_cwd_fd() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Ok(());
        }
        let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
        let directory = open_directory(sandbox.path())?;
        let payload = b"payload delivered on stdin\n";
        let (input, mut writer) = std::io::pipe().map_err(|error| error.to_string())?;

        let authority = GitLaunchAuthority::admit(&directory)?;
        let mut command = authority.command();
        command
            .command_mut()
            .args(["hash-object", "--stdin"])
            .env("LC_ALL", "C");
        let child = authority.spawn(&directory, command, Stdio::from(input))?;
        let pid = rustix::process::Pid::from_raw(child.id() as i32)
            .ok_or_else(|| "descriptor-bound Git child PID was invalid".to_string())?;
        let process_group = rustix::process::getpgid(Some(pid))
            .map_err(|error| format!("failed to inspect Git process group: {error}"))?;
        assert_eq!(process_group, pid);
        writer
            .write_all(payload)
            .map_err(|error| error.to_string())?;
        drop(writer);
        let output = wait_with_output(child)?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout)
                .map_err(|error| error.to_string())?
                .trim(),
            git_blob_oid(payload)
        );
        Ok(())
    }
}
