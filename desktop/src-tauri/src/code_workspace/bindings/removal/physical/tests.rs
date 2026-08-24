use std::collections::BTreeMap;
use std::fs;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileTypeExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::*;
use crate::code_workspace::bindings::{
    CodeExecutionMode, CodeThreadBindingLookupInput, CodeThreadBindingScope,
    CodeThreadBindingStore, CodeThreadLifecycleStatus,
};
use crate::code_workspace::worktrees::{
    prepare_execution_root_with_merge_target, CodeWorktreeDescriptor, CodeWorktreePrepareInput,
};

const PREPARATION_ID: &str = "11111111-1111-4111-8111-111111111111";
const THREAD_ID: &str = "thread-physical-removal";
const CRASH_REQUEST_ENV: &str = "SCHOOLX_CODE_REMOVAL_CRASH_REQUEST_V1";
const CRASH_REQUEST_VERSION: u32 = 1;
const MAX_CRASH_REQUEST_BYTES: usize = 64 * 1024;
const INJECTED_CRASH_EXIT_CODE: i32 = 86;

mod claim;
mod cleanup;
mod contracts;
mod crash_support;
mod linux_mount;
mod recovery;
mod support;

use crash_support::*;
use support::*;

/// Child-process entry that terminates at one durable physical-removal boundary.
#[test]
#[ignore]
fn physical_removal_crash_subprocess_entry() {
    if let Err(error) = run_crash_child_from_env() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

/// Child-process entry used by the typed removal Git helper during unit tests.
#[test]
#[ignore]
fn removal_git_helper_subprocess_entry() {
    if let Err(error) = unix::execute_helper() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
