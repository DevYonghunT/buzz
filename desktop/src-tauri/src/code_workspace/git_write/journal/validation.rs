use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::super::protocol::{CodeGitCommitIdentity, CodeGitMutationReceipt};
use super::{
    GitJournalFileIdentity, GitJournalPathIdentity, MAX_ARTIFACT_NAME_BYTES, MAX_GIT_PATH_BYTES,
    MAX_IDENTIFIER_BYTES, MAX_IDENTITY_EMAIL_BYTES, MAX_IDENTITY_NAME_BYTES, MAX_PATH_BYTES,
};

impl GitJournalPathIdentity {
    pub(super) fn validate(&self, label: &str) -> Result<(), String> {
        validate_absolute_path(label, &self.exact_path)?;
        if self.device == 0 || self.inode == 0 || self.link_count == 0 {
            return Err(format!("{label} has an invalid filesystem identity"));
        }
        if self.mode == 0 {
            return Err(format!("{label} has an invalid filesystem mode"));
        }
        Ok(())
    }

    pub(super) fn validate_directory(&self, label: &str) -> Result<(), String> {
        self.validate(label)?;
        validate_directory_mode(label, self.mode)
    }
}

impl GitJournalFileIdentity {
    pub(super) fn validate_executable(&self, label: &str) -> Result<(), String> {
        validate_absolute_path(label, &self.exact_path)?;
        if self.device == 0
            || self.inode == 0
            || self.link_count == 0
            || self.size == 0
            || self.size > 128 * 1024 * 1024
        {
            return Err(format!("{label} has an invalid filesystem identity"));
        }
        validate_executable_mode(label, self.mode)?;
        validate_sha256(&format!("{label} digest"), &self.sha256)
    }

    pub(super) fn validate_singly_linked_regular_file(
        &self,
        label: &str,
        max_size: u64,
    ) -> Result<(), String> {
        validate_absolute_path(label, &self.exact_path)?;
        if self.device == 0
            || self.inode == 0
            || self.link_count != 1
            || self.size == 0
            || self.size > max_size
        {
            return Err(format!("{label} has an invalid filesystem identity"));
        }
        validate_regular_file_mode(label, self.mode)?;
        validate_sha256(&format!("{label} digest"), &self.sha256)
    }
}

#[cfg(unix)]
pub(super) fn validate_directory_mode(label: &str, mode: u32) -> Result<(), String> {
    if mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFDIR) {
        return Err(format!("{label} is not a directory identity"));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn validate_directory_mode(label: &str, mode: u32) -> Result<(), String> {
    if mode == 0 {
        return Err(format!("{label} has an invalid directory mode"));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn validate_regular_file_mode(label: &str, mode: u32) -> Result<(), String> {
    if mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFREG) {
        return Err(format!("{label} is not a regular-file identity"));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn validate_regular_file_mode(label: &str, mode: u32) -> Result<(), String> {
    if mode == 0 {
        return Err(format!("{label} has an invalid regular-file mode"));
    }
    Ok(())
}

pub(super) fn validate_executable_mode(label: &str, mode: u32) -> Result<(), String> {
    validate_regular_file_mode(label, mode)?;
    if mode & 0o111 == 0 {
        return Err(format!("{label} has no executable permission bit"));
    }
    Ok(())
}

pub(super) fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    validate_bounded_text(label, value, MAX_IDENTIFIER_BYTES)?;
    if value.chars().any(|character| character.is_control()) {
        return Err(format!("{label} contains a control character"));
    }
    Ok(())
}

pub(super) fn validate_bounded_text(
    label: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), String> {
    if value.is_empty() || value.len() > max_bytes || value != value.trim() || value.contains('\0')
    {
        return Err(format!(
            "{label} is empty, non-canonical, or exceeds {max_bytes} bytes"
        ));
    }
    Ok(())
}

pub(super) fn validate_absolute_path(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_PATH_BYTES || value.contains('\0') {
        return Err(format!("{label} is empty or exceeds the path bound"));
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(format!("{label} is not a canonical absolute path"));
    }
    let normalized = normalized_path(path)?;
    if normalized.to_str() != Some(value) {
        return Err(format!("{label} is not a canonical absolute path"));
    }
    Ok(())
}

pub(super) fn validate_relative_git_path(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_GIT_PATH_BYTES
        || value.contains('\0')
        || Path::new(value).is_absolute()
        || value.chars().any(|character| character.is_control())
    {
        return Err("Git journal selected path is not a canonical relative path".to_string());
    }
    let normalized = normalized_path(Path::new(value))?;
    if normalized.to_str() != Some(value)
        || normalized
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Git journal selected path is not a canonical relative path".to_string());
    }
    Ok(())
}

pub(super) fn normalized_path(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir | Component::ParentDir => {
                return Err("Git journal path contains a traversal component".to_string());
            }
        }
    }
    Ok(normalized)
}

pub(super) fn validate_component(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_ARTIFACT_NAME_BYTES
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', '\0'])
        || value.chars().any(|character| character.is_control())
    {
        return Err(format!("{label} is not a canonical path component"));
    }
    Ok(())
}

pub(super) fn validate_sha256(label: &str, value: &str) -> Result<(), String> {
    validate_nonzero_hex_64(label, value)
}

pub(super) fn validate_hex_64(label: &str, value: &str) -> Result<(), String> {
    validate_nonzero_hex_64(label, value)
}

fn validate_nonzero_hex_64(label: &str, value: &str) -> Result<(), String> {
    validate_hex_with_length(label, value, 64)?;
    if value.bytes().all(|byte| byte == b'0') {
        return Err(format!("{label} cannot be all-zero hex"));
    }
    Ok(())
}

pub(super) fn validate_hex_with_length(
    label: &str,
    value: &str,
    length: usize,
) -> Result<(), String> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase hex"));
    }
    Ok(())
}

pub(super) fn validate_object_id(label: &str, value: &str, allow_zero: bool) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64) {
        return Err(format!("{label} has an unsupported object-id length"));
    }
    validate_object_id_with_length(label, value, value.len(), allow_zero)
}

pub(super) fn validate_object_id_with_length(
    label: &str,
    value: &str,
    length: usize,
    allow_zero: bool,
) -> Result<(), String> {
    validate_hex_with_length(label, value, length)?;
    if !allow_zero && value.bytes().all(|byte| byte == b'0') {
        return Err(format!("{label} cannot be the zero object id"));
    }
    Ok(())
}

pub(super) fn validate_commit_identity(identity: &CodeGitCommitIdentity) -> Result<(), String> {
    validate_bounded_text(
        "Git journal identity name",
        &identity.name,
        MAX_IDENTITY_NAME_BYTES,
    )?;
    validate_bounded_text(
        "Git journal identity email",
        &identity.email,
        MAX_IDENTITY_EMAIL_BYTES,
    )?;
    if identity
        .name
        .chars()
        .any(|value| value.is_control() || matches!(value, '<' | '>'))
        || identity
            .email
            .chars()
            .any(|value| value.is_control() || value.is_whitespace() || matches!(value, '<' | '>'))
        || identity.email.matches('@').count() != 1
        || identity.email.starts_with('@')
        || identity.email.ends_with('@')
    {
        return Err("Git journal commit identity is not canonical".to_string());
    }
    Ok(())
}

pub(super) fn receipt_digest(receipt: &CodeGitMutationReceipt) -> Result<String, String> {
    serde_json::to_vec(receipt)
        .map(|bytes| digest_bytes(&bytes))
        .map_err(|error| format!("failed to digest Git receipt: {error}"))
}

pub(super) fn digest_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
