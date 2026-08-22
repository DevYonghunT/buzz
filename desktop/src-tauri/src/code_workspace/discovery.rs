use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const VERSION_OUTPUT_CAP: u64 = 64 * 1024;
const SUPPORTED_CODEX_VERSION_PREFIXES: [&str; 2] = ["0.145.", "0.149."];
const SUPPORTED_CODEX_VERSION_REQUIREMENT: &str =
    "codex-cli 0.145.<numeric patch> or codex-cli 0.149.<numeric patch>";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRuntimeProbe {
    pub available: bool,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub error: Option<String>,
}

impl CodeRuntimeProbe {
    fn unavailable(error: impl Into<String>) -> Self {
        Self {
            available: false,
            executable: None,
            version: None,
            error: Some(error.into()),
        }
    }

    /// Clone this probe for Tauri/status egress without altering spawn authority.
    pub(crate) fn redacted_for_egress(&self) -> Self {
        self.redacted_for_egress_with(super::protocol::redact_protocol_text)
    }

    fn redacted_for_egress_with(&self, redact: impl Fn(&str) -> String) -> Self {
        Self {
            available: self.available,
            executable: self.executable.as_deref().map(&redact),
            version: self.version.as_deref().map(&redact),
            error: self.error.as_deref().map(redact),
        }
    }
}

pub(crate) fn probe_codex(explicit: Option<&Path>) -> CodeRuntimeProbe {
    let executable = match resolve_codex(explicit) {
        Ok(path) => path,
        Err(error) => return CodeRuntimeProbe::unavailable(error),
    };
    match probe_version(&executable) {
        Ok(version) => CodeRuntimeProbe {
            available: true,
            executable: Some(executable.display().to_string()),
            version: Some(version),
            error: None,
        },
        Err(error) => CodeRuntimeProbe {
            available: false,
            executable: Some(executable.display().to_string()),
            version: None,
            error: Some(error),
        },
    }
}

/// Reject Codex app-server versions whose wire contract has not been fixed by
/// the checked-in SchoolX compatibility fixtures.
///
/// Availability and compatibility deliberately remain separate: callers keep
/// the complete probe result for diagnostics even when runtime startup is
/// denied.
pub(crate) fn ensure_supported_codex_version(probe: &CodeRuntimeProbe) -> Result<(), String> {
    let version = probe.version.as_deref().ok_or_else(|| {
        "Codex probe returned no version for compatibility validation".to_string()
    })?;
    if is_supported_codex_version(version) {
        Ok(())
    } else {
        Err(format!(
            "unsupported Codex CLI version; SchoolX Code currently requires {SUPPORTED_CODEX_VERSION_REQUIREMENT} without a prerelease or build suffix"
        ))
    }
}

fn is_supported_codex_version(value: &str) -> bool {
    let mut fields = value.split_ascii_whitespace();
    if fields.next() != Some("codex-cli") {
        return false;
    }
    let Some(version) = fields.next() else {
        return false;
    };
    if fields.next().is_some() {
        return false;
    }

    let Some(prefix) = SUPPORTED_CODEX_VERSION_PREFIXES
        .iter()
        .find(|prefix| version.starts_with(**prefix))
    else {
        return false;
    };
    let patch = &version[prefix.len()..];
    if patch.is_empty()
        || !patch.bytes().all(|byte| byte.is_ascii_digit())
        || (patch.len() > 1 && patch.starts_with('0'))
    {
        return false;
    }
    true
}

fn resolve_codex(explicit: Option<&Path>) -> Result<PathBuf, String> {
    let candidate = match explicit {
        Some(path) => {
            if !path.is_absolute() {
                return Err("configured Codex executable must be an absolute path".to_string());
            }
            path.to_path_buf()
        }
        None => crate::managed_agents::resolve_command("codex")
            .ok_or_else(|| "Codex CLI was not found on this Mac".to_string())?,
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("Codex executable is not accessible: {error}"))?;
    if !canonical.is_file() {
        return Err("Codex executable is not a regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = canonical
            .metadata()
            .map_err(|error| format!("failed to inspect Codex executable: {error}"))?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err("Codex executable is not marked executable".to_string());
        }
    }
    Ok(canonical)
}

fn probe_version(executable: &Path) -> Result<String, String> {
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::util::configure_no_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to run Codex CLI: {error}"))?;
    let deadline = Instant::now() + VERSION_PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Codex version probe timed out".to_string());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to wait for Codex version probe: {error}"));
            }
        }
    };

    let stdout = read_capped(child.stdout.take(), VERSION_OUTPUT_CAP);
    let stderr = read_capped(child.stderr.take(), VERSION_OUTPUT_CAP);
    if !status.success() {
        let detail = first_nonempty_line(&stderr)
            .or_else(|| first_nonempty_line(&stdout))
            .unwrap_or("unknown error");
        return Err(format!("Codex version probe failed: {detail}"));
    }
    first_nonempty_line(&stdout)
        .or_else(|| first_nonempty_line(&stderr))
        .map(str::to_string)
        .ok_or_else(|| "Codex version probe returned no version".to_string())
}

fn read_capped<R: Read>(reader: Option<R>, cap: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(reader) = reader {
        let _ = reader.take(cap).read_to_end(&mut bytes);
    }
    bytes
}

fn first_nonempty_line(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes)
        .ok()?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn fake_codex(script: &str) -> Result<(tempfile::TempDir, PathBuf), String> {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let path = directory.path().join("codex");
        std::fs::write(&path, script).map_err(|error| error.to_string())?;
        let mut permissions = std::fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).map_err(|error| error.to_string())?;
        Ok((directory, path))
    }

    #[test]
    fn explicit_paths_must_be_absolute() {
        let result = resolve_codex(Some(Path::new("codex")));
        assert!(result.is_err());
    }

    #[test]
    fn picks_first_nonempty_version_line() {
        assert_eq!(
            first_nonempty_line(b"\n codex-cli 0.145.0 \nwarning\n"),
            Some("codex-cli 0.145.0")
        );
    }

    #[cfg(unix)]
    #[test]
    fn redacts_child_supplied_probe_error_and_version_text() -> Result<(), String> {
        let failure_canary = "sk-probe-failure-canary";
        let (_failure_directory, failure_executable) = fake_codex(
            "#!/bin/sh\nprintf '%s\\n' 'version failed: sk-probe-failure-canary' >&2\nexit 1\n",
        )?;
        let raw_failed = probe_codex(Some(&failure_executable));
        assert!(raw_failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains(failure_canary)));
        let failed = raw_failed.redacted_for_egress();
        let error = failed
            .error
            .as_deref()
            .ok_or_else(|| "failed probe returned no diagnostic".to_string())?;
        assert!(!failed.available);
        assert!(!error.contains(failure_canary));
        assert!(error.contains("[REDACTED]"));

        let version_canary = "sk-probe-version-canary";
        let (_version_directory, version_executable) = fake_codex(
            "#!/bin/sh\nprintf '%s\\n' 'codex-cli 0.145.0 sk-probe-version-canary'\nexit 0\n",
        )?;
        let raw_succeeded = probe_codex(Some(&version_executable));
        assert!(raw_succeeded
            .version
            .as_deref()
            .is_some_and(|version| version.contains(version_canary)));
        assert!(ensure_supported_codex_version(&raw_succeeded).is_err());
        let succeeded = raw_succeeded.redacted_for_egress();
        let version = succeeded
            .version
            .as_deref()
            .ok_or_else(|| "successful probe returned no version".to_string())?;
        assert!(succeeded.available);
        assert!(!version.contains(version_canary));
        assert!(version.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn egress_redaction_does_not_change_raw_version_compatibility() -> Result<(), String> {
        let raw = CodeRuntimeProbe {
            available: true,
            executable: Some("/canonical/schoolx-executable-canary/codex".to_string()),
            version: Some("codex-cli 0.145.0".to_string()),
            error: None,
        };
        ensure_supported_codex_version(&raw)?;

        let redacted = raw.redacted_for_egress_with(|text| {
            super::super::protocol::redact_protocol_text_with_sensitive_values(
                text,
                &["0.145.0", "schoolx-executable-canary"],
            )
        });
        assert_eq!(raw.version.as_deref(), Some("codex-cli 0.145.0"));
        assert_eq!(redacted.version.as_deref(), Some("codex-cli [REDACTED]"));
        assert_eq!(
            raw.executable.as_deref(),
            Some("/canonical/schoolx-executable-canary/codex")
        );
        assert!(redacted
            .executable
            .as_deref()
            .is_some_and(
                |executable| !executable.contains("schoolx-executable-canary")
                    && executable.contains("[REDACTED]")
            ));
        Ok(())
    }

    #[test]
    fn compatibility_gate_accepts_only_audited_codex_minor_patch_versions() {
        for version in [
            "codex-cli 0.145.0",
            "codex-cli 0.145.9",
            "codex-cli 0.149.0",
            "codex-cli 0.149.12",
        ] {
            assert!(
                is_supported_codex_version(version),
                "expected {version} to pass"
            );
        }

        for version in [
            "codex-cli 0.144.99",
            "codex-cli 0.146.0",
            "codex-cli 0.147.0",
            "codex-cli 0.148.0",
            "codex-cli 0.150.0",
            "codex-cli 1.145.0",
            "codex-cli 0.145",
            "codex-cli 0.145.x",
            "codex-cli 0.145.00",
            "codex-cli 0.145.0-test",
            "codex-cli 0.145.12+fixture.1",
            "codex-cli 0.145.12-test.2+fixture.1",
            "codex-cli 0.145.0-",
            "codex-cli 0.145.12+",
            "codex-cli 0.145.0-01",
            "codex-cli 0.145.0-test..two",
            "codex-cli 0.145.0-test+one+two",
            "codex-cli 0.145.0 trailing",
            "codex-cli 0.149",
            "codex-cli 0.149.x",
            "codex-cli 0.149.00",
            "codex-cli 0.149.0-test",
            "codex-cli 0.149.12+fixture.1",
            "codex-cli 0.149.0 trailing",
            "codex 0.145.0",
            "codex-cli v0.145.0",
        ] {
            assert!(
                !is_supported_codex_version(version),
                "expected {version} to fail"
            );
        }
    }
}
