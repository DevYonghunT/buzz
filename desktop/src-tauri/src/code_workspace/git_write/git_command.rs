use std::fs;
use std::path::Path;
#[cfg(all(unix, any(not(target_os = "macos"), test)))]
use std::process::Child;
use std::process::{Command, Stdio};

mod backlinks;
#[cfg(unix)]
mod capture;
mod executable;
mod identity;
mod relative_file;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
use crate::code_workspace::git_launch::GitLaunchAuthority;
#[cfg(target_os = "macos")]
use crate::code_workspace::macos_git_xpc::{self, DescriptorObservation, MacGitProcessSpec};
#[cfg(all(target_os = "macos", not(test)))]
use crate::code_workspace::macos_git_xpc::{MacGitAuthoritySession, MacGitFamily, MacGitInput};

use backlinks::{pin_admin_backlinks, verify_admin_backlinks};
#[cfg(all(unix, any(not(target_os = "macos"), test)))]
use capture::capture_child;
#[cfg(all(target_os = "macos", not(test)))]
use capture::capture_macos_child;
#[cfg(target_os = "linux")]
pub(in crate::code_workspace) use executable::RootTrustedGit;
use executable::{pin_git_executable, verify_git_executable};
#[cfg(all(unix, test))]
use identity::verify_named_directory;
use identity::{
    directory_identity, open_verified_file, verify_directory_identity, verify_regular_file,
};
pub(super) use identity::{
    pin_directory, pin_input_file, verify_named_directory_identity, DirectoryIdentity, FileIdentity,
};
pub(super) use relative_file::FrozenWorktreeFile;

#[cfg(all(unix, test))]
const HELPER_REQUEST_ENV: &str = "SCHOOLX_CODE_GIT_WRITE_REQUEST";
const HELPER_VERSION: u32 = 1;
const MAX_HELPER_REQUEST_BYTES: usize = 256 * 1024;
const LOCKED_GIT_CONFIG: [&str; 28] = [
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "credential.helper=",
    "-c",
    "credential.interactive=false",
    "-c",
    "core.askPass=false",
    "-c",
    "protocol.allow=never",
    "-c",
    "protocol.file.allow=never",
    "-c",
    "protocol.git.allow=never",
    "-c",
    "protocol.http.allow=never",
    "-c",
    "protocol.https.allow=never",
    "-c",
    "protocol.ssh.allow=never",
    "-c",
    "protocol.ext.allow=never",
    "-c",
    "core.fsync=added,reference",
    "-c",
    "core.fsyncMethod=fsync",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelperEnvelope {
    version: u32,
    root: DirectoryIdentity,
    git: FileIdentity,
    authority: Option<RepositoryAuthority>,
    command: GitWriteCommand,
}

/// Exact linked-worktree repository paths used after discovery.  Once this
/// authority is installed, Git never re-derives its admin/common directories
/// from a mutable worktree `.git` file.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryAuthority {
    worktree_git_file: FileIdentity,
    admin_gitdir_file: FileIdentity,
    admin_commondir_file: FileIdentity,
    admin: DirectoryIdentity,
    common: DirectoryIdentity,
    object_database: DirectoryIdentity,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum GitWriteCommand {
    TopLevel,
    GitDir,
    CommonDir,
    HeadCommit,
    SymbolicHead,
    GitPath {
        marker: GitOperationMarker,
    },
    Status,
    DiffNumstat {
        staged: bool,
        path: String,
    },
    DiffPatch {
        staged: bool,
        path: String,
    },
    ConfigList {
        scope: GitConfigScope,
    },
    ConfigValue {
        scope: GitConfigScope,
        key: GitConfigKey,
    },
    CheckAttributes {
        path: String,
    },
    SharedIndexPath,
    RefFormat,
    ListStageEntries {
        index: Option<FileIdentity>,
    },
    HeadEntry {
        path: String,
    },
    HashObject {
        write: bool,
        source: FileIdentity,
    },
    UpdateIndex {
        index: FileIdentity,
        update: IndexUpdate,
    },
    WriteTree {
        index: FileIdentity,
    },
    CommitTree {
        tree: String,
        parent: String,
        identity: GitCommitIdentity,
        timestamp: String,
        message: FileIdentity,
    },
    ObjectType {
        oid: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum GitOperationMarker {
    MergeHead,
    CherryPickHead,
    RevertHead,
    BisectLog,
    RebaseMerge,
    RebaseApply,
    Sequencer,
}

impl GitOperationMarker {
    fn as_str(self) -> &'static str {
        match self {
            Self::MergeHead => "MERGE_HEAD",
            Self::CherryPickHead => "CHERRY_PICK_HEAD",
            Self::RevertHead => "REVERT_HEAD",
            Self::BisectLog => "BISECT_LOG",
            Self::RebaseMerge => "rebase-merge",
            Self::RebaseApply => "rebase-apply",
            Self::Sequencer => "sequencer",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum GitConfigScope {
    Local,
    Worktree,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum GitConfigKey {
    UserName,
    UserEmail,
    CoreAutocrlf,
    ExtensionsWorktreeConfig,
    ExtensionsObjectFormat,
    ExtensionsRefStorage,
}

impl GitConfigKey {
    fn as_str(self) -> &'static str {
        match self {
            Self::UserName => "user.name",
            Self::UserEmail => "user.email",
            Self::CoreAutocrlf => "core.autocrlf",
            Self::ExtensionsWorktreeConfig => "extensions.worktreeConfig",
            Self::ExtensionsObjectFormat => "extensions.objectFormat",
            Self::ExtensionsRefStorage => "extensions.refStorage",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum IndexUpdate {
    Upsert {
        mode: String,
        oid: String,
        path: String,
    },
    Remove {
        path: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct GitCommitIdentity {
    pub(super) name: String,
    pub(super) email: String,
}

pub(super) struct GitCommandOutput {
    pub(super) code: i32,
    pub(super) stdout: Vec<u8>,
}

pub(super) struct PinnedGitWriteRepository {
    root: DirectoryIdentity,
    root_handle: fs::File,
    git: FileIdentity,
    authority: Option<RepositoryAuthority>,
    #[cfg(target_os = "linux")]
    launch: GitLaunchAuthority,
    #[cfg(all(target_os = "macos", not(test)))]
    session: MacGitAuthoritySession,
}

impl PinnedGitWriteRepository {
    #[cfg(unix)]
    pub(super) fn pin(root: &Path) -> Result<Self, String> {
        use std::os::unix::fs::OpenOptionsExt as _;

        if !root.is_absolute() {
            return Err("SchoolX Code Git write root must be absolute".to_string());
        }
        let root_handle = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(root)
            .map_err(|error| format!("failed to pin Git write root: {error}"))?;
        let root_identity = directory_identity(root, &root_handle)?;
        let git = pin_git_executable()?;
        #[cfg(all(target_os = "macos", not(test)))]
        macos_git_xpc::require_capability()?;
        #[cfg(all(target_os = "macos", not(test)))]
        let session = MacGitAuthoritySession::begin()?;
        #[cfg(target_os = "linux")]
        let launch = GitLaunchAuthority::admit_with_git(
            &root_handle,
            RootTrustedGit::from_identity(git.clone())?,
        )?;
        let pinned = Self {
            root: root_identity,
            root_handle,
            git,
            authority: None,
            #[cfg(target_os = "linux")]
            launch,
            #[cfg(all(target_os = "macos", not(test)))]
            session,
        };
        pinned.revalidate()?;
        Ok(pinned)
    }

    #[cfg(not(unix))]
    pub(super) fn pin(_root: &Path) -> Result<Self, String> {
        Err("SchoolX Code Git writes require descriptor-bound Unix support".to_string())
    }

    pub(super) fn root_identity(&self) -> &DirectoryIdentity {
        &self.root
    }

    pub(super) fn git_identity(&self) -> &FileIdentity {
        &self.git
    }

    /// Bind every subsequent Git invocation to exact native-resolved
    /// linked-worktree paths. The discovery runner is intentionally unusable
    /// as mutation authority until this method succeeds.
    pub(super) fn bind_repository_authority(
        mut self,
        worktree_git_file: FileIdentity,
        admin: DirectoryIdentity,
        common: DirectoryIdentity,
        object_database: DirectoryIdentity,
    ) -> Result<Self, String> {
        let (admin_gitdir_file, admin_commondir_file) =
            pin_admin_backlinks(&self.root, &worktree_git_file, &admin, &common)?;
        let authority = RepositoryAuthority {
            worktree_git_file,
            admin_gitdir_file,
            admin_commondir_file,
            admin,
            common,
            object_database,
        };
        validate_repository_authority(&self.root, &authority)?;
        verify_repository_authority(&self.root, &authority)?;
        self.authority = Some(authority);
        self.revalidate()?;
        Ok(self)
    }

    pub(super) fn read_worktree_file(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> Result<Option<FrozenWorktreeFile>, String> {
        self.revalidate()?;
        let result = relative_file::read_relative(&self.root_handle, path, max_bytes)?;
        self.revalidate()?;
        Ok(result)
    }

    #[cfg(unix)]
    pub(super) fn revalidate(&self) -> Result<(), String> {
        verify_directory_identity(&self.root, &self.root_handle)?;
        verify_git_executable(&self.git)?;
        if let Some(authority) = &self.authority {
            verify_repository_authority(&self.root, authority)?;
        }
        Ok(())
    }

    #[cfg(not(unix))]
    pub(super) fn revalidate(&self) -> Result<(), String> {
        Err("SchoolX Code Git writes require descriptor-bound Unix support".to_string())
    }

    #[cfg(unix)]
    pub(super) fn run(&self, command: GitWriteCommand) -> Result<GitCommandOutput, String> {
        self.revalidate()?;
        validate_command(&command)?;
        if self.authority.is_none() && command_requires_repository_authority(&command) {
            return Err(
                "Typed Git mutation requires exact linked-worktree repository authority"
                    .to_string(),
            );
        }
        let accepts_one = matches!(
            &command,
            GitWriteCommand::SymbolicHead | GitWriteCommand::ConfigValue { .. }
        );
        let envelope = HelperEnvelope {
            version: HELPER_VERSION,
            root: self.root.clone(),
            git: self.git.clone(),
            authority: self.authority.clone(),
            command,
        };
        let encoded = serde_json::to_string(&envelope)
            .map_err(|error| format!("failed to encode Git write helper request: {error}"))?;
        if encoded.len() > MAX_HELPER_REQUEST_BYTES {
            return Err(format!(
                "Git write helper request exceeds {MAX_HELPER_REQUEST_BYTES} bytes"
            ));
        }

        #[cfg(target_os = "linux")]
        let child = {
            validate_envelope(&envelope)?;
            let mut command = self.launch.command();
            configure_git_command(command.command_mut(), &envelope)?;
            let input = git_command_input(&envelope)?;
            self.launch.spawn(
                &self.root_handle,
                command,
                input.map(Stdio::from).unwrap_or_else(Stdio::null),
            )?
        };

        #[cfg(all(target_os = "macos", not(test)))]
        let child = {
            validate_envelope(&envelope)?;
            let input = git_command_input(&envelope)?
                .map(MacGitInput::File)
                .unwrap_or(MacGitInput::Null);
            self.session
                .spawn(MacGitFamily::GitWrite, encoded, &self.root_handle, input)?
        };

        #[cfg(all(not(target_os = "linux"), test))]
        let child = { spawn_helper(&self.root_handle, encoded)? };
        #[cfg(all(not(target_os = "linux"), not(target_os = "macos"), not(test)))]
        let child: Child = {
            let _ = encoded;
            return Err("typed Git launch is unsupported on this Unix platform".to_string());
        };
        #[cfg(all(target_os = "macos", not(test)))]
        let output = capture_macos_child(child, accepts_one)?;
        #[cfg(any(not(target_os = "macos"), test))]
        let output = capture_child(child, accepts_one)?;
        self.revalidate()?;
        Ok(output)
    }

    #[cfg(not(unix))]
    pub(super) fn run(&self, _command: GitWriteCommand) -> Result<GitCommandOutput, String> {
        Err("SchoolX Code Git writes require descriptor-bound Unix support".to_string())
    }
}

#[cfg(all(unix, not(target_os = "linux"), test))]
fn spawn_helper(root: &fs::File, encoded: String) -> Result<Child, String> {
    use std::os::unix::process::CommandExt as _;

    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to resolve SchoolX desktop executable: {error}"))?;
    let mut command = Command::new(executable);
    command.args([
        "--exact",
        "code_workspace::git_write::git_command::tests::helper_subprocess_entry",
        "--ignored",
        "--nocapture",
    ]);
    command
        .env_clear()
        .env(HELPER_REQUEST_ENV, encoded)
        .env("LC_ALL", "C")
        .stdin(Stdio::from(root.try_clone().map_err(|error| {
            format!("failed to clone pinned Git root: {error}")
        })?))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::util::configure_no_window(&mut command);
    command.process_group(0);
    command
        .spawn()
        .map_err(|error| format!("failed to start Git write helper: {error}"))
}

#[cfg(all(unix, test))]
fn execute_helper() -> Result<(), String> {
    use std::os::fd::AsFd as _;
    use std::os::unix::process::CommandExt as _;

    let encoded = std::env::var(HELPER_REQUEST_ENV)
        .map_err(|_| "Git write helper request was missing or not UTF-8".to_string())?;
    if encoded.len() > MAX_HELPER_REQUEST_BYTES {
        return Err(format!(
            "Git write helper request exceeds {MAX_HELPER_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: HelperEnvelope = serde_json::from_str(&encoded)
        .map_err(|error| format!("invalid Git write helper request: {error}"))?;
    validate_envelope(&envelope)?;
    let stdin = std::io::stdin();
    let stat = rustix::fs::fstat(stdin.as_fd())
        .map_err(|error| format!("failed to inspect pinned Git write root: {error}"))?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_dir()
        || stat.st_dev as u64 != envelope.root.device
        || stat.st_ino as u64 != envelope.root.inode
    {
        return Err("Git write helper root identity did not match its request".to_string());
    }
    rustix::process::fchdir(stdin.as_fd())
        .map_err(|error| format!("failed to enter pinned Git write root: {error}"))?;
    verify_named_directory(&envelope.root)?;
    verify_git_executable(&envelope.git)?;
    if let Some(authority) = &envelope.authority {
        verify_repository_authority(&envelope.root, authority)?;
    }
    let (mut command, input) = build_git_command(&envelope)?;
    command.stdin(input.map(Stdio::from).unwrap_or_else(Stdio::null));
    let error = command.exec();
    Err(format!("failed to execute typed Git command: {error}"))
}

#[cfg(test)]
fn build_git_command(envelope: &HelperEnvelope) -> Result<(Command, Option<fs::File>), String> {
    let mut command = Command::new(&envelope.git.path);
    configure_git_command(&mut command, envelope)?;
    let input = git_command_input(envelope)?;
    Ok((command, input))
}

fn configure_git_command(command: &mut Command, envelope: &HelperEnvelope) -> Result<(), String> {
    command.env_clear();
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_GRAFT_FILE", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "false")
        .env("SSH_ASKPASS", "false")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_SSH_COMMAND", "false")
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .env("GIT_ALLOW_PROTOCOL", "")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_PAGER", "cat")
        .env("GIT_EDITOR", "false")
        .env("GIT_SEQUENCE_EDITOR", "false")
        .env("LC_ALL", "C");
    if let Some(authority) = &envelope.authority {
        command
            .env("GIT_DIR", &authority.admin.path)
            .env("GIT_COMMON_DIR", &authority.common.path)
            .env("GIT_WORK_TREE", &envelope.root.path);
    }
    command.args(LOCKED_GIT_CONFIG);
    match &envelope.command {
        GitWriteCommand::TopLevel => command.args(["rev-parse", "--show-toplevel"]),
        GitWriteCommand::GitDir => command.args(["rev-parse", "--git-dir"]),
        GitWriteCommand::CommonDir => command.args(["rev-parse", "--git-common-dir"]),
        GitWriteCommand::HeadCommit => command.args(["rev-parse", "--verify", "HEAD^{commit}"]),
        GitWriteCommand::SymbolicHead => command.args(["symbolic-ref", "-q", "HEAD"]),
        GitWriteCommand::GitPath { marker } => {
            command.args(["rev-parse", "--git-path", marker.as_str()])
        }
        GitWriteCommand::Status => {
            command.args(["status", "--porcelain=v1", "-z", "-uall", "--no-renames"])
        }
        GitWriteCommand::DiffNumstat { staged, path } => {
            command.args([
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--numstat",
            ]);
            if *staged {
                command.arg("--cached");
            }
            command.args(["--", path]);
            &mut *command
        }
        GitWriteCommand::DiffPatch { staged, path } => {
            command.args([
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--unified=80",
            ]);
            if *staged {
                command.arg("--cached");
            }
            command.args(["--", path]);
            &mut *command
        }
        GitWriteCommand::ConfigList { scope } => command.args([
            "config",
            config_scope_arg(*scope),
            "--no-includes",
            "--null",
            "--list",
        ]),
        GitWriteCommand::ConfigValue { scope, key } => command.args([
            "config",
            config_scope_arg(*scope),
            "--no-includes",
            "--get-all",
            key.as_str(),
        ]),
        GitWriteCommand::CheckAttributes { path } => {
            command.args(["check-attr", "-z", "--all", "--", path])
        }
        GitWriteCommand::SharedIndexPath => command.args(["rev-parse", "--shared-index-path"]),
        GitWriteCommand::RefFormat => command.args(["rev-parse", "--show-ref-format"]),
        GitWriteCommand::ListStageEntries { index } => {
            if let Some(index) = index {
                verify_regular_file(index, 64 * 1024 * 1024)?;
                command.env("GIT_INDEX_FILE", &index.path);
            }
            command.args(["ls-files", "--stage", "-v", "-z"])
        }
        GitWriteCommand::HeadEntry { path } => command.args(["ls-tree", "-z", "HEAD", "--", path]),
        GitWriteCommand::HashObject { write, source } => {
            verify_regular_file(source, 64 * 1024 * 1024)?;
            command.args(["hash-object", "--no-filters"]);
            if *write {
                command.arg("-w");
            }
            command.arg("--stdin");
            &mut *command
        }
        GitWriteCommand::UpdateIndex { index, update } => {
            verify_regular_file(index, 64 * 1024 * 1024)?;
            command.env("GIT_INDEX_FILE", &index.path);
            match update {
                IndexUpdate::Upsert { mode, oid, path } => command.args([
                    "update-index",
                    "--add",
                    "--info-only",
                    "--cacheinfo",
                    mode,
                    oid,
                    path,
                ]),
                IndexUpdate::Remove { path } => {
                    command.args(["update-index", "--force-remove", "--", path])
                }
            }
        }
        GitWriteCommand::WriteTree { index } => {
            verify_regular_file(index, 64 * 1024 * 1024)?;
            command.env("GIT_INDEX_FILE", &index.path);
            command.arg("write-tree")
        }
        GitWriteCommand::CommitTree {
            tree,
            parent,
            identity,
            timestamp,
            message,
        } => {
            verify_regular_file(message, 64 * 1024)?;
            command
                .env("GIT_AUTHOR_NAME", &identity.name)
                .env("GIT_AUTHOR_EMAIL", &identity.email)
                .env("GIT_COMMITTER_NAME", &identity.name)
                .env("GIT_COMMITTER_EMAIL", &identity.email)
                .env("GIT_AUTHOR_DATE", timestamp)
                .env("GIT_COMMITTER_DATE", timestamp);
            command.args([
                "-c",
                "commit.gpgSign=false",
                "commit-tree",
                tree,
                "-p",
                parent,
            ])
        }
        GitWriteCommand::ObjectType { oid } => command.args(["cat-file", "-t", oid]),
    };
    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    Ok(())
}

fn git_command_input(envelope: &HelperEnvelope) -> Result<Option<fs::File>, String> {
    match &envelope.command {
        GitWriteCommand::HashObject { source, .. } => {
            open_verified_file(source, 64 * 1024 * 1024).map(Some)
        }
        GitWriteCommand::CommitTree { message, .. } => {
            open_verified_file(message, 64 * 1024).map(Some)
        }
        _ => Ok(None),
    }
}

fn config_scope_arg(scope: GitConfigScope) -> &'static str {
    match scope {
        GitConfigScope::Local => "--local",
        GitConfigScope::Worktree => "--worktree",
    }
}

fn validate_envelope(envelope: &HelperEnvelope) -> Result<(), String> {
    if envelope.version != HELPER_VERSION {
        return Err("unsupported Git write helper request version".to_string());
    }
    validate_absolute_path(&envelope.root.path, "Git write root")?;
    validate_absolute_path(&envelope.git.path, "Git executable")?;
    validate_digest(&envelope.git.digest)?;
    if let Some(authority) = &envelope.authority {
        validate_repository_authority(&envelope.root, authority)?;
    } else if command_requires_repository_authority(&envelope.command) {
        return Err(
            "Git write helper mutation omitted linked-worktree repository authority".to_string(),
        );
    }
    validate_command(&envelope.command)
}

/// Decode and revalidate one closed Git-write envelope inside the signed
/// macOS service, then derive its fixed `/usr/bin/git` process specification.
#[cfg(target_os = "macos")]
pub(in crate::code_workspace) fn prepare_macos_git_write(
    payload: &str,
    cwd: DescriptorObservation,
    stdin: DescriptorObservation,
) -> Result<MacGitProcessSpec, String> {
    if payload.len() > MAX_HELPER_REQUEST_BYTES {
        return Err(format!(
            "Git write helper request exceeds {MAX_HELPER_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: HelperEnvelope = serde_json::from_str(payload)
        .map_err(|error| format!("invalid Git write helper request: {error}"))?;
    validate_envelope(&envelope)?;
    macos_git_xpc::validate_directory_observation(
        cwd,
        envelope.root.device,
        envelope.root.inode,
        Some(envelope.root.mode),
        "Git write root",
    )?;
    verify_git_executable(&envelope.git)?;
    if let Some(authority) = &envelope.authority {
        verify_repository_authority(&envelope.root, authority)?;
    }
    match git_command_input_identity(&envelope) {
        Some(identity) => macos_git_xpc::validate_regular_observation(
            stdin,
            identity.device,
            identity.inode,
            identity.mode,
            identity.size,
            "Git write input",
        )?,
        None => macos_git_xpc::validate_null_observation(stdin, "Git write input")?,
    }
    let mut command = Command::new(&envelope.git.path);
    configure_git_command(&mut command, &envelope)?;
    macos_git_xpc::process_spec_from_command(&command)
}

fn git_command_input_identity(envelope: &HelperEnvelope) -> Option<&FileIdentity> {
    match &envelope.command {
        GitWriteCommand::HashObject { source, .. } => Some(source),
        GitWriteCommand::CommitTree { message, .. } => Some(message),
        _ => None,
    }
}

/// Re-pin the Apple-controlled Git shim inside the signed macOS service.
#[cfg(target_os = "macos")]
pub(crate) fn macos_root_trusted_git() -> Result<std::path::PathBuf, String> {
    let identity = pin_git_executable()?;
    verify_git_executable(&identity)?;
    let path = std::path::PathBuf::from(identity.path);
    if path != Path::new("/usr/bin/git") {
        return Err("macOS typed Git authority did not resolve to /usr/bin/git".to_string());
    }
    Ok(path)
}

fn command_requires_repository_authority(command: &GitWriteCommand) -> bool {
    matches!(
        command,
        GitWriteCommand::HashObject { write: true, .. }
            | GitWriteCommand::UpdateIndex { .. }
            | GitWriteCommand::WriteTree { .. }
            | GitWriteCommand::CommitTree { .. }
            | GitWriteCommand::ObjectType { .. }
    )
}

fn validate_repository_authority(
    root: &DirectoryIdentity,
    authority: &RepositoryAuthority,
) -> Result<(), String> {
    for file in [
        &authority.worktree_git_file,
        &authority.admin_gitdir_file,
        &authority.admin_commondir_file,
    ] {
        validate_file_identity(file)?;
    }
    for (label, directory) in [
        ("Git admin directory", &authority.admin),
        ("Git common directory", &authority.common),
        ("Git object database", &authority.object_database),
    ] {
        validate_absolute_path(&directory.path, label)?;
        if directory.device == 0 || directory.inode == 0 || directory.mode & 0o170000 != 0o040000 {
            return Err(format!("{label} evidence is not a directory"));
        }
    }
    let root_path = Path::new(&root.path);
    if Path::new(&authority.worktree_git_file.path) != root_path.join(".git") {
        return Err("linked-worktree .git evidence escaped the pinned root".to_string());
    }
    let common = Path::new(&authority.common.path);
    if Path::new(&authority.object_database.path) != common.join("objects") {
        return Err("Git object database escaped the pinned common directory".to_string());
    }
    let admin = Path::new(&authority.admin.path);
    if admin == common || admin.parent() != Some(common.join("worktrees").as_path()) {
        return Err("Git authority requires a linked-worktree admin directory".to_string());
    }
    if Path::new(&authority.admin_gitdir_file.path) != admin.join("gitdir")
        || Path::new(&authority.admin_commondir_file.path) != admin.join("commondir")
    {
        return Err("Git authority backlink files escaped the pinned admin directory".to_string());
    }
    Ok(())
}

fn verify_repository_authority(
    root: &DirectoryIdentity,
    authority: &RepositoryAuthority,
) -> Result<(), String> {
    verify_regular_file(&authority.worktree_git_file, 32 * 1024)?;
    verify_regular_file(&authority.admin_gitdir_file, 32 * 1024)?;
    verify_regular_file(&authority.admin_commondir_file, 32 * 1024)?;
    verify_named_directory_identity(&authority.admin)?;
    verify_named_directory_identity(&authority.common)?;
    verify_named_directory_identity(&authority.object_database)?;
    verify_admin_backlinks(root, authority)
}

fn validate_command(command: &GitWriteCommand) -> Result<(), String> {
    match command {
        GitWriteCommand::DiffNumstat { path, .. }
        | GitWriteCommand::DiffPatch { path, .. }
        | GitWriteCommand::CheckAttributes { path }
        | GitWriteCommand::HeadEntry { path } => validate_relative_path(path),
        GitWriteCommand::ListStageEntries { index: Some(index) }
        | GitWriteCommand::WriteTree { index } => validate_file_identity(index),
        GitWriteCommand::HashObject { source, .. } => validate_file_identity(source),
        GitWriteCommand::UpdateIndex { index, update } => {
            validate_file_identity(index)?;
            match update {
                IndexUpdate::Upsert { mode, oid, path } => {
                    if !matches!(mode.as_str(), "100644" | "100755") {
                        return Err("unsupported whole-file Git mode".to_string());
                    }
                    validate_oid(oid)?;
                    validate_relative_path(path)
                }
                IndexUpdate::Remove { path } => validate_relative_path(path),
            }
        }
        GitWriteCommand::CommitTree {
            tree,
            parent,
            identity,
            timestamp,
            message,
        } => {
            validate_oid(tree)?;
            validate_oid(parent)?;
            validate_identity(identity)?;
            validate_timestamp(timestamp)?;
            validate_file_identity(message)
        }
        GitWriteCommand::ObjectType { oid } => validate_oid(oid),
        _ => Ok(()),
    }
}

fn validate_identity(identity: &GitCommitIdentity) -> Result<(), String> {
    let valid = identity.name == identity.name.trim()
        && identity.email == identity.email.trim()
        && !identity.name.is_empty()
        && !identity.email.is_empty()
        && identity.name.len() <= 1024
        && identity.email.len() <= 1024
        && !identity
            .name
            .chars()
            .any(|value| value.is_control() || matches!(value, '<' | '>'))
        && !identity
            .email
            .chars()
            .any(|value| value.is_control() || value.is_whitespace() || matches!(value, '<' | '>'))
        && identity.email.matches('@').count() == 1
        && !identity.email.starts_with('@')
        && !identity.email.ends_with('@');
    if valid {
        Ok(())
    } else {
        Err("invalid frozen Git commit identity".to_string())
    }
}

fn validate_timestamp(value: &str) -> Result<(), String> {
    let Some((seconds, offset)) = value.split_once(' ') else {
        return Err("invalid frozen Git timestamp".to_string());
    };
    if seconds.parse::<i64>().is_err() || offset != "+0000" {
        return Err("invalid frozen Git timestamp".to_string());
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 4096
        || value.starts_with('/')
        || value.split('/').any(|part| matches!(part, "" | "." | ".."))
        || value.chars().any(char::is_control)
    {
        return Err("unsafe Git path in typed write request".to_string());
    }
    Ok(())
}

fn validate_oid(value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err("invalid Git object id in typed write request".to_string())
    }
}

fn validate_digest(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err("invalid SHA-256 evidence digest".to_string())
    }
}

fn validate_absolute_path(value: &str, label: &str) -> Result<(), String> {
    if value.len() > 16 * 1024
        || !Path::new(value).is_absolute()
        || value.chars().any(char::is_control)
    {
        Err(format!("{label} is not a safe absolute path"))
    } else {
        Ok(())
    }
}

fn validate_file_identity(identity: &FileIdentity) -> Result<(), String> {
    validate_absolute_path(&identity.path, "Git artifact")?;
    validate_digest(&identity.digest)?;
    if identity.mode & 0o170000 != 0o100000 || identity.link_count == 0 {
        return Err("Git artifact evidence is not a regular file".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
