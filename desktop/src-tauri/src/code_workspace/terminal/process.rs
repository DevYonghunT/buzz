//! OS-specific PTY process ownership and descendant cleanup.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(any(windows, test))]
use std::ffi::OsStr;
#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};

use portable_pty::{
    native_pty_system, Child as PtyChild, CommandBuilder, ExitStatus, MasterPty, PtySize,
};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PROCESS_TERM_TIMEOUT: Duration = Duration::from_millis(750);
#[cfg(windows)]
const CONPTY_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) type SpawnedPty = (SessionProcess, Box<dyn Read + Send>, Box<dyn Write + Send>);

pub(super) struct SessionProcess {
    master: Option<Box<dyn MasterPty + Send>>,
    child: Box<dyn PtyChild + Send + Sync>,
    leader_pid: u32,
    #[cfg(windows)]
    job: Option<crate::managed_agents::JobHandle>,
    #[cfg(windows)]
    master_close_tx: SyncSender<MasterCloseRequest>,
    #[cfg(windows)]
    pending_natural_close: Option<PendingNaturalClose>,
    discard_output: Arc<AtomicBool>,
    finished: bool,
}

#[cfg(windows)]
struct MasterCloseRequest {
    master: Box<dyn MasterPty + Send>,
    done: Option<SyncSender<()>>,
}

#[cfg(windows)]
struct PendingNaturalClose {
    status: ExitStatus,
    done: Receiver<()>,
    deadline: Instant,
}

struct SpawnMasterGuard {
    master: Option<Box<dyn MasterPty + Send>>,
    #[cfg(windows)]
    close_tx: SyncSender<MasterCloseRequest>,
}

impl SpawnMasterGuard {
    #[cfg(not(windows))]
    fn new(master: Box<dyn MasterPty + Send>) -> Self {
        Self {
            master: Some(master),
        }
    }

    #[cfg(windows)]
    fn new(master: Box<dyn MasterPty + Send>, close_tx: SyncSender<MasterCloseRequest>) -> Self {
        Self {
            master: Some(master),
            close_tx,
        }
    }

    fn master(&self) -> Result<&(dyn MasterPty + Send), String> {
        self.master
            .as_deref()
            .ok_or_else(|| "SchoolX Code PTY rollback owner is unavailable".to_string())
    }

    fn into_master(mut self) -> Result<Box<dyn MasterPty + Send>, String> {
        self.master
            .take()
            .ok_or_else(|| "SchoolX Code PTY rollback owner is unavailable".to_string())
    }
}

impl Drop for SpawnMasterGuard {
    fn drop(&mut self) {
        let Some(master) = self.master.take() else {
            return;
        };
        #[cfg(windows)]
        {
            let request = MasterCloseRequest { master, done: None };
            if let Err(error) = self.close_tx.send(request) {
                // Never run an un-cancellable ClosePseudoConsole on the spawn
                // caller during rollback. The ConPTY has no registered owner.
                std::mem::forget(error.0.master);
            }
        }
        #[cfg(not(windows))]
        drop(master);
    }
}

pub(super) fn spawn_pty(execution_root: &Path, cols: u16, rows: u16) -> Result<SpawnedPty, String> {
    #[cfg(not(windows))]
    let mut command = CommandBuilder::new_default_prog();
    #[cfg(windows)]
    let mut command = windows_shell_command()?;
    command.cwd(execution_root.as_os_str());
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for key in crate::managed_agents::RESERVED_ENV_KEYS {
        command.env_remove(key);
    }

    #[cfg(windows)]
    let master_close_tx = spawn_master_closer()?;

    let pair = native_pty_system()
        .openpty(pty_size(cols, rows))
        .map_err(|error| format!("failed to open SchoolX Code PTY: {error}"))?;
    #[cfg(not(windows))]
    let master = SpawnMasterGuard::new(pair.master);
    #[cfg(windows)]
    let master = SpawnMasterGuard::new(pair.master, master_close_tx.clone());
    // Declared after the guard so rollback always drops the shared ConPTY
    // slave reference before handing the final master reference to the helper.
    let slave = pair.slave;
    let reader = master
        .master()?
        .try_clone_reader()
        .map_err(|error| format!("failed to clone SchoolX Code PTY reader: {error}"))?;
    let writer = master
        .master()?
        .take_writer()
        .map_err(|error| format!("failed to take SchoolX Code PTY writer: {error}"))?;

    let mut child = slave
        .spawn_command(command)
        .map_err(|error| format!("failed to spawn SchoolX Code terminal shell: {error}"))?;
    drop(slave);
    let leader_pid = child.process_id().ok_or_else(|| {
        let _ = child.kill();
        let _ = child.wait();
        "SchoolX Code terminal shell did not expose a process id".to_string()
    })?;

    #[cfg(unix)]
    if let Err(error) = validate_session_leader(leader_pid) {
        if let Ok(pid) = unix_pid(leader_pid) {
            signal_process_groups(&[pid], rustix::process::Signal::KILL);
        }
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    #[cfg(windows)]
    let job = match crate::managed_agents::create_kill_on_close_job_for_child(leader_pid) {
        Some(job) => Some(job),
        None => {
            let tree_cleanup_error = crate::managed_agents::taskkill_tree(leader_pid).err();
            if tree_cleanup_error.is_some() {
                let _ = child.kill();
            }
            let reap_error = child.wait().err();
            let mut error =
                "failed to assign SchoolX Code terminal shell to a kill-on-close Job Object"
                    .to_string();
            if let Some(cleanup_error) = tree_cleanup_error {
                error.push_str(&format!("; tree cleanup also failed: {cleanup_error}"));
            }
            if let Some(reap_error) = reap_error {
                error.push_str(&format!("; leader reap also failed: {reap_error}"));
            }
            return Err(error);
        }
    };
    let discard_output = Arc::new(AtomicBool::new(false));
    let master = master.into_master()?;

    Ok((
        SessionProcess {
            master: Some(master),
            child,
            leader_pid,
            #[cfg(windows)]
            job,
            #[cfg(windows)]
            master_close_tx,
            #[cfg(windows)]
            pending_natural_close: None,
            discard_output,
            finished: false,
        },
        reader,
        writer,
    ))
}

#[cfg(windows)]
fn spawn_master_closer() -> Result<SyncSender<MasterCloseRequest>, String> {
    let (close_tx, close_rx) = mpsc::sync_channel::<MasterCloseRequest>(1);
    thread::Builder::new()
        .name("code-terminal-conpty-closer".to_string())
        .spawn(move || {
            if let Ok(request) = close_rx.recv() {
                let MasterCloseRequest { master, done } = request;
                drop(master);
                if let Some(done) = done {
                    let _ = done.send(());
                }
            }
        })
        .map_err(|error| format!("failed to start SchoolX Code ConPTY closer: {error}"))?;
    Ok(close_tx)
}

impl SessionProcess {
    pub(super) fn output_discard_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.discard_output)
    }

    pub(super) fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .as_ref()
            .ok_or_else(|| "SchoolX Code terminal PTY is closing".to_string())?
            .resize(pty_size(cols, rows))
            .map_err(|error| format!("failed to resize SchoolX Code terminal: {error}"))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn poll_exit(&mut self) -> Result<Option<ExitStatus>, String> {
        let pid = unix_pid(self.leader_pid)?;
        let observed = rustix::process::waitid(
            rustix::process::WaitId::Pid(pid),
            rustix::process::WaitIdOptions::EXITED
                | rustix::process::WaitIdOptions::NOHANG
                | rustix::process::WaitIdOptions::NOWAIT,
        )
        .map_err(|error| format!("failed to inspect SchoolX Code terminal shell: {error}"))?;
        if observed.is_none() {
            return Ok(None);
        }
        let groups = self.process_groups();
        let (session_members, scan_error) =
            session_scan_parts(session_processes(self.leader_pid, false));
        signal_process_groups(&groups, rustix::process::Signal::KILL);
        signal_processes(&session_members, rustix::process::Signal::KILL);
        self.close_master_after_natural_exit();
        let status = self
            .child
            .wait()
            .map_err(|error| format!("failed to reap SchoolX Code terminal shell: {error}"))?;
        self.finished = true;
        if let Some(error) = scan_error {
            return Err(format!(
                "{error}; terminal leader was reaped with process-group fallback"
            ));
        }
        Ok(Some(status))
    }

    #[cfg(all(not(any(target_os = "linux", target_os = "macos")), not(windows)))]
    pub(super) fn poll_exit(&mut self) -> Result<Option<ExitStatus>, String> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect SchoolX Code terminal shell: {error}"))?;
        if let Some(status) = status {
            #[cfg(unix)]
            signal_process_groups(&self.process_groups(), rustix::process::Signal::KILL);
            self.close_master_after_natural_exit();
            self.finished = true;
            return Ok(Some(status));
        }
        Ok(None)
    }

    #[cfg(windows)]
    pub(super) fn poll_exit(&mut self) -> Result<Option<ExitStatus>, String> {
        if let Some(pending) = self.pending_natural_close.take() {
            return match pending.done.try_recv() {
                Ok(()) => {
                    self.finished = true;
                    Ok(Some(pending.status))
                }
                Err(TryRecvError::Empty) => {
                    if Instant::now() >= pending.deadline {
                        self.discard_output.store(true, Ordering::Release);
                        self.finished = true;
                        return Err("SchoolX Code terminal ConPTY close timed out in its helper"
                            .to_string());
                    }
                    self.pending_natural_close = Some(pending);
                    Ok(None)
                }
                Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    Err("SchoolX Code terminal ConPTY closer stopped unexpectedly".to_string())
                }
            };
        }

        let status = self
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect SchoolX Code terminal shell: {error}"))?;
        let Some(status) = status else {
            return Ok(None);
        };
        let Some(master) = self.master.take() else {
            self.finished = true;
            return Ok(Some(status));
        };

        // On Windows versions where ClosePseudoConsole waits for its output
        // pipe to drain, dropping the master on this actor would deadlock once
        // the bounded reader queue fills. Kill the Job first, then let a
        // dedicated closer block while this actor keeps consuming output.
        self.job.take();
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let request = MasterCloseRequest {
            master,
            done: Some(done_tx),
        };
        if let Err(error) = self.master_close_tx.send(request) {
            self.discard_output.store(true, Ordering::Release);
            // ClosePseudoConsole itself cannot be cancelled and has blocked
            // indefinitely on older Windows cursor-inheritance paths. The Job
            // is already closed, so leak this one native handle instead of
            // deadlocking the terminal actor or app shutdown.
            std::mem::forget(error.0.master);
            self.finished = true;
            return Err("SchoolX Code terminal ConPTY closer is unavailable".to_string());
        }
        self.pending_natural_close = Some(PendingNaturalClose {
            status,
            done: done_rx,
            deadline: Instant::now() + CONPTY_CLOSE_TIMEOUT,
        });
        Ok(None)
    }

    #[cfg(unix)]
    pub(super) fn terminate(&mut self) -> Result<ExitStatus, String> {
        let mut inspection_error = None;
        match self.poll_exit() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) if self.finished => return Err(error),
            Err(error) => inspection_error = Some(error),
        }
        let groups = self.process_groups();
        // Interactive shells put foreground and background jobs in their own
        // process groups. Every such job remains in the PTY leader's POSIX
        // session even after reparenting, so signal the complete session.
        let (mut session_members, mut scan_error) =
            session_scan_parts(session_processes(self.leader_pid, true));
        if let Some(error) = inspection_error {
            scan_error = Some(match scan_error {
                Some(scan_error) => format!("{error}; {scan_error}"),
                None => error,
            });
        }
        signal_process_groups(&groups, rustix::process::Signal::HUP);
        signal_process_groups(&groups, rustix::process::Signal::TERM);
        signal_processes(&session_members, rustix::process::Signal::HUP);
        signal_processes(&session_members, rustix::process::Signal::TERM);
        self.close_native_handles_for_termination();

        let deadline = Instant::now() + PROCESS_TERM_TIMEOUT;
        loop {
            match self.poll_exit() {
                Ok(Some(status)) => {
                    // `poll_exit` can only rediscover the leader after the master
                    // closes. Kill the originally captured foreground group too.
                    signal_process_groups(&groups, rustix::process::Signal::KILL);
                    signal_processes(&session_members, rustix::process::Signal::KILL);
                    if let Some(error) = scan_error.take() {
                        return Err(format!(
                            "{error}; terminal leader was reaped with process-group fallback"
                        ));
                    }
                    return Ok(status);
                }
                Ok(None) => {}
                Err(error) if self.finished => {
                    signal_process_groups(&groups, rustix::process::Signal::KILL);
                    signal_processes(&session_members, rustix::process::Signal::KILL);
                    return Err(match scan_error.take() {
                        Some(scan_error) => format!("{scan_error}; {error}"),
                        None => error,
                    });
                }
                Err(error) => {
                    signal_process_groups(&groups, rustix::process::Signal::KILL);
                    signal_processes(&session_members, rustix::process::Signal::KILL);
                    scan_error = Some(match scan_error.take() {
                        Some(scan_error) => format!("{scan_error}; {error}"),
                        None => error,
                    });
                    break;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }

        let (latest_members, latest_scan_error) =
            session_scan_parts(session_processes(self.leader_pid, false));
        for member in latest_members {
            if !session_members.contains(&member) {
                session_members.push(member);
            }
        }
        if let Some(error) = latest_scan_error {
            scan_error = Some(match scan_error {
                Some(previous) => format!("{previous}; {error}"),
                None => error,
            });
        }
        signal_process_groups(&groups, rustix::process::Signal::KILL);
        signal_processes(&session_members, rustix::process::Signal::KILL);
        let _ = self.child.kill();
        let status = self
            .child
            .wait()
            .map_err(|error| format!("failed to reap SchoolX Code terminal shell: {error}"))?;
        self.finished = true;
        if let Some(error) = scan_error {
            return Err(format!(
                "{error}; terminal leader was reaped with process-group fallback"
            ));
        }
        Ok(status)
    }

    #[cfg(windows)]
    pub(super) fn terminate(&mut self) -> Result<ExitStatus, String> {
        if let Some(status) = self.detach_pending_natural_close() {
            return Ok(status);
        }
        if let Some(status) = self.poll_exit()? {
            return Ok(status);
        }
        if let Some(status) = self.detach_pending_natural_close() {
            return Ok(status);
        }
        self.close_native_handles_for_termination();
        let deadline = Instant::now() + PROCESS_TERM_TIMEOUT;
        loop {
            if let Some(status) = self.poll_exit()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }
        self.child
            .kill()
            .map_err(|error| format!("failed to kill SchoolX Code terminal shell: {error}"))?;
        let status = self
            .child
            .wait()
            .map_err(|error| format!("failed to reap SchoolX Code terminal shell: {error}"))?;
        self.finished = true;
        Ok(status)
    }

    #[cfg(not(any(unix, windows)))]
    pub(super) fn terminate(&mut self) -> Result<ExitStatus, String> {
        if let Some(status) = self.poll_exit()? {
            return Ok(status);
        }
        self.close_native_handles_for_termination();
        self.child
            .kill()
            .map_err(|error| format!("failed to kill SchoolX Code terminal shell: {error}"))?;
        let status = self
            .child
            .wait()
            .map_err(|error| format!("failed to reap SchoolX Code terminal shell: {error}"))?;
        self.finished = true;
        Ok(status)
    }

    #[cfg(unix)]
    fn process_groups(&self) -> Vec<rustix::process::Pid> {
        let mut groups = Vec::with_capacity(2);
        if let Ok(leader) = unix_pid(self.leader_pid) {
            groups.push(leader);
        }
        if let Some(foreground) = self
            .master
            .as_ref()
            .and_then(|master| master.process_group_leader())
            .and_then(rustix::process::Pid::from_raw)
        {
            if !groups.contains(&foreground) {
                groups.push(foreground);
            }
        }
        groups
    }

    #[cfg(not(windows))]
    fn close_master_after_natural_exit(&mut self) {
        self.master.take();
    }

    fn close_native_handles_for_termination(&mut self) {
        self.discard_output.store(true, Ordering::Release);
        #[cfg(windows)]
        {
            // KILL_ON_JOB_CLOSE terminates the complete ConPTY process tree.
            self.job.take();
            if let Some(master) = self.master.take() {
                let request = MasterCloseRequest { master, done: None };
                if let Err(error) = self.master_close_tx.send(request) {
                    // The process tree is already dead. Avoid a synchronous
                    // ClosePseudoConsole call on this lifecycle-critical actor.
                    std::mem::forget(error.0.master);
                }
            }
        }
        #[cfg(not(windows))]
        self.master.take();
    }

    #[cfg(windows)]
    fn detach_pending_natural_close(&mut self) -> Option<ExitStatus> {
        let pending = self.pending_natural_close.take()?;
        self.discard_output.store(true, Ordering::Release);
        self.finished = true;
        Some(pending.status)
    }
}

impl Drop for SessionProcess {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.terminate();
        }
    }
}

#[cfg(unix)]
fn unix_pid(raw_pid: u32) -> Result<rustix::process::Pid, String> {
    let raw_pid = i32::try_from(raw_pid)
        .map_err(|_| "SchoolX Code terminal returned an invalid process id".to_string())?;
    rustix::process::Pid::from_raw(raw_pid)
        .ok_or_else(|| "SchoolX Code terminal returned an invalid process id".to_string())
}

#[cfg(unix)]
fn signal_process_groups(groups: &[rustix::process::Pid], signal: rustix::process::Signal) {
    for group in groups {
        match rustix::process::kill_process_group(*group, signal) {
            Ok(()) => {}
            Err(error) if error == rustix::io::Errno::SRCH => {}
            Err(_) => {}
        }
    }
}

#[cfg(unix)]
fn validate_session_leader(raw_pid: u32) -> Result<(), String> {
    let leader = unix_pid(raw_pid)?;
    let session = rustix::process::getsid(Some(leader))
        .map_err(|error| format!("failed to inspect SchoolX Code terminal session: {error}"))?;
    if session != leader {
        return Err("SchoolX Code terminal shell is not its POSIX session leader".to_string());
    }
    if !session_processes(raw_pid, true)?.contains(&leader) {
        return Err("SchoolX Code terminal session is not visible to native cleanup".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn session_processes(
    session_pid: u32,
    require_leader: bool,
) -> Result<Vec<rustix::process::Pid>, String> {
    let session = unix_pid(session_pid)?;
    let ps = if Path::new("/bin/ps").is_file() {
        "/bin/ps"
    } else {
        "/usr/bin/ps"
    };
    let output = std::process::Command::new(ps)
        .args(["-axo", "pid="])
        .output()
        .map_err(|error| format!("failed to list SchoolX Code terminal session: {error}"))?;
    if !output.status.success() {
        return Err("failed to list SchoolX Code terminal session".to_string());
    }

    let candidates = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let raw_pid = line.split_whitespace().next()?.parse::<i32>().ok()?;
            rustix::process::Pid::from_raw(raw_pid)
        })
        .collect::<Vec<_>>();
    select_session_members(session, candidates, require_leader, |pid| {
        rustix::process::getsid(Some(pid)).ok()
    })
}

#[cfg(unix)]
fn select_session_members<I, F>(
    session: rustix::process::Pid,
    candidates: I,
    require_leader: bool,
    mut session_of: F,
) -> Result<Vec<rustix::process::Pid>, String>
where
    I: IntoIterator<Item = rustix::process::Pid>,
    F: FnMut(rustix::process::Pid) -> Option<rustix::process::Pid>,
{
    let mut inspected = 0_usize;
    let members = candidates
        .into_iter()
        .filter(|pid| match session_of(*pid) {
            Some(candidate_session) => {
                inspected = inspected.saturating_add(1);
                candidate_session == session
            }
            None => false,
        })
        .collect::<Vec<_>>();
    if inspected == 0 {
        return Err(
            "SchoolX Code terminal session enumeration could not inspect any pid".to_string(),
        );
    }
    if require_leader && !members.contains(&session) {
        return Err("SchoolX Code terminal session enumeration lost its leader".to_string());
    }
    Ok(members)
}

#[cfg(unix)]
fn session_scan_parts(
    result: Result<Vec<rustix::process::Pid>, String>,
) -> (Vec<rustix::process::Pid>, Option<String>) {
    match result {
        Ok(members) => (members, None),
        Err(error) => (Vec::new(), Some(error)),
    }
}

#[cfg(unix)]
fn signal_processes(processes: &[rustix::process::Pid], signal: rustix::process::Signal) {
    for process in processes {
        match rustix::process::kill_process(*process, signal) {
            Ok(()) => {}
            Err(error) if error == rustix::io::Errno::SRCH => {}
            Err(_) => {}
        }
    }
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[cfg(windows)]
fn windows_shell_command() -> Result<CommandBuilder, String> {
    // portable-pty does not expose CREATE_SUSPENDED, so Job Object assignment
    // cannot be atomic with ConPTY spawn. Keep the unavoidable window minimal
    // and suppress cmd.exe AutoRun hooks, an avoidable pre-assignment child
    // path. A non-cmd ComSpec is rejected because `/D` is cmd-specific.
    let shell = std::env::var_os("ComSpec").unwrap_or_else(|| OsString::from("cmd.exe"));
    if !is_windows_cmd_shell(&shell) {
        return Err("SchoolX Code terminal ComSpec is not cmd.exe".to_string());
    }
    let mut command = CommandBuilder::new(shell);
    command.arg("/D");
    Ok(command)
}

#[cfg(any(windows, test))]
fn is_windows_cmd_shell(shell: &OsStr) -> bool {
    Path::new(shell)
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("cmd.exe"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    struct DropProbe(Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl MasterPty for DropProbe {
        fn resize(&self, _size: PtySize) -> Result<(), anyhow::Error> {
            Ok(())
        }

        fn get_size(&self) -> Result<PtySize, anyhow::Error> {
            Ok(PtySize::default())
        }

        fn try_clone_reader(&self) -> Result<Box<dyn Read + Send>, anyhow::Error> {
            Err(anyhow::anyhow!("unused test reader"))
        }

        fn take_writer(&self) -> Result<Box<dyn Write + Send>, anyhow::Error> {
            Err(anyhow::anyhow!("unused test writer"))
        }

        fn process_group_leader(&self) -> Option<libc::pid_t> {
            None
        }

        fn as_raw_fd(&self) -> Option<std::os::fd::RawFd> {
            None
        }

        fn tty_name(&self) -> Option<std::path::PathBuf> {
            None
        }
    }

    #[test]
    fn spawn_master_guard_owns_rollback_until_disarmed() -> Result<(), String> {
        let rollback_drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let _guard = SpawnMasterGuard::new(Box::new(DropProbe(Arc::clone(&rollback_drops))));
        }
        assert_eq!(rollback_drops.load(Ordering::SeqCst), 1);

        let success_drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let guard = SpawnMasterGuard::new(Box::new(DropProbe(Arc::clone(&success_drops))));
        let master = guard.into_master()?;
        assert_eq!(success_drops.load(Ordering::SeqCst), 0);
        drop(master);
        assert_eq!(success_drops.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[test]
    fn session_member_selection_surfaces_lost_cleanup_visibility() {
        let leader = rustix::process::getpid();
        let result = select_session_members(leader, [leader], true, |_| None);
        assert!(result.is_err());

        let members = select_session_members(leader, [leader], true, |_| Some(leader));
        assert_eq!(members, Ok(vec![leader]));
    }

    #[test]
    fn cleanup_selection_retains_members_after_leader_disappears() {
        let leader = rustix::process::getpid();
        let Some(member) = rustix::process::Pid::from_raw(leader.as_raw_nonzero().get() + 1) else {
            panic!("test member pid must be nonzero");
        };
        let members = select_session_members(leader, [member], false, |_| Some(leader));
        assert_eq!(members, Ok(vec![member]));
    }

    #[test]
    fn windows_shell_guard_only_accepts_cmd_exe() {
        assert!(is_windows_cmd_shell(OsStr::new("cmd.exe")));
        assert!(is_windows_cmd_shell(OsStr::new(
            "/Windows/System32/CMD.EXE"
        )));
        assert!(!is_windows_cmd_shell(OsStr::new("powershell.exe")));
    }
}
