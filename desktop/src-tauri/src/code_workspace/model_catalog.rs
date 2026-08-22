//! Pinned Codex model catalog normalization and SchoolX's recent selection.
//!
//! Codex remains authoritative for the models a runtime can execute. SchoolX
//! persists only one installation-local UX preference and revalidates it
//! against the current runtime generation before exposing or applying it.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const CODE_STORE_DIRECTORY: &str = "code";
const MODEL_SELECTION_FILE: &str = "model-selection.json";
const MODEL_SELECTION_VERSION: u32 = 1;
const MAX_MODEL_SELECTION_BYTES: u64 = 16 * 1024;
const MODEL_PAGE_LIMIT: u32 = 100;
const MAX_MODEL_PAGES: usize = 64;
const MAX_MODELS: usize = 4_096;
const MAX_ID_BYTES: usize = 512;
const MAX_LABEL_BYTES: usize = 4 * 1024;
const MAX_DESCRIPTION_BYTES: usize = 16 * 1024;
const MAX_EFFORTS_PER_MODEL: usize = 64;
const MAX_CURSOR_BYTES: usize = 4 * 1024;

/// One reasoning-effort value advertised by a pinned Codex model.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeReasoningEffortOption {
    pub reasoning_effort: String,
    pub description: String,
}

/// Strict, UI-safe projection of one visible Codex model.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeModelOption {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub is_default: bool,
    pub default_reasoning_effort: String,
    pub supported_reasoning_efforts: Vec<CodeReasoningEffortOption>,
}

/// Exact model/effort pair selected by the user.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeModelSelection {
    pub model: String,
    pub reasoning_effort: String,
}

impl CodeModelSelection {
    pub(crate) fn validate_shape(&self) -> Result<(), String> {
        validate_token("model", &self.model)?;
        validate_token("reasoning effort", &self.reasoning_effort)
    }
}

/// Visible models and the last still-supported installation preference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeModelsListResult {
    pub runtime_generation: u64,
    pub models: Vec<CodeModelOption>,
    pub recent_selection: Option<CodeModelSelection>,
}

/// Model catalog tied to one exact ready runtime generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodeModelCatalogSnapshot {
    pub(crate) runtime_generation: u64,
    pub(crate) models: Vec<CodeModelOption>,
}

impl CodeModelCatalogSnapshot {
    pub(crate) fn require_model(&self, model: &str) -> Result<(), String> {
        validate_token("model", model)?;
        if self.models.iter().any(|candidate| candidate.model == model) {
            Ok(())
        } else {
            Err("Selected Codex model is not available in the current runtime catalog".to_string())
        }
    }

    pub(crate) fn require_selection(&self, selection: &CodeModelSelection) -> Result<(), String> {
        selection.validate_shape()?;
        let model = self
            .models
            .iter()
            .find(|candidate| candidate.model == selection.model)
            .ok_or_else(|| {
                "Selected Codex model is not available in the current runtime catalog".to_string()
            })?;
        if model
            .supported_reasoning_efforts
            .iter()
            .any(|candidate| candidate.reasoning_effort == selection.reasoning_effort)
        {
            Ok(())
        } else {
            Err(
                "Selected reasoning effort is not supported by the selected Codex model"
                    .to_string(),
            )
        }
    }

    pub(crate) fn reconcile_recent_selection(
        &self,
        selection: Option<CodeModelSelection>,
    ) -> Option<CodeModelSelection> {
        selection.filter(|selection| self.require_selection(selection).is_ok())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireModelListResponse {
    data: Vec<WireModel>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireModel {
    id: String,
    model: String,
    upgrade: Option<String>,
    upgrade_info: Option<WireModelUpgradeInfo>,
    availability_nux: Option<WireModelAvailabilityNux>,
    display_name: String,
    description: String,
    /// Optional display-only specialty added by the audited Codex 0.149 schema.
    #[serde(default)]
    model_specialty: Option<String>,
    hidden: bool,
    supported_reasoning_efforts: Vec<WireReasoningEffortOption>,
    default_reasoning_effort: String,
    #[serde(default)]
    input_modalities: Vec<WireInputModality>,
    #[serde(default)]
    supports_personality: bool,
    /// Closed Codex 0.149 multi-agent capability. It is not execution authority.
    #[serde(default)]
    multi_agent_version: Option<WireMultiAgentVersion>,
    #[serde(default)]
    additional_speed_tiers: Vec<String>,
    #[serde(default)]
    service_tiers: Vec<WireModelServiceTier>,
    default_service_tier: Option<String>,
    is_default: bool,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireModelUpgradeInfo {
    model: String,
    migration_markdown: Option<String>,
    model_link: Option<String>,
    /// Informational Unix timestamp added by the audited Codex 0.149 schema.
    #[serde(default)]
    retirement_at: Option<i64>,
    upgrade_copy: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireMultiAgentVersion {
    Disabled,
    V1,
    V2,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireModelAvailabilityNux {
    message: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireModelServiceTier {
    id: String,
    name: String,
    description: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireInputModality {
    Text,
    Image,
    Audio,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireReasoningEffortOption {
    reasoning_effort: String,
    description: String,
}

pub(crate) fn model_list_params(cursor: Option<&str>) -> Result<Value, String> {
    if let Some(cursor) = cursor {
        validate_cursor(cursor)?;
        Ok(json!({
            "cursor": cursor,
            "includeHidden": false,
            "limit": MODEL_PAGE_LIMIT
        }))
    } else {
        Ok(json!({
            "includeHidden": false,
            "limit": MODEL_PAGE_LIMIT
        }))
    }
}

/// Exhaustively collect one bounded, same-generation visible model catalog.
pub(crate) fn collect_model_catalog(
    runtime_generation: u64,
    mut request: impl FnMut(Value) -> Result<Value, String>,
) -> Result<CodeModelCatalogSnapshot, String> {
    let mut models = Vec::new();
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();

    for _ in 0..MAX_MODEL_PAGES {
        let params = model_list_params(cursor.as_deref())?;
        let response: WireModelListResponse = serde_json::from_value(request(params)?)
            .map_err(|error| format!("invalid Codex model/list response: {error}"))?;
        if response.data.len() > MODEL_PAGE_LIMIT as usize {
            return Err(format!(
                "Codex model/list page exceeds the {MODEL_PAGE_LIMIT}-model limit"
            ));
        }
        if models.len().saturating_add(response.data.len()) > MAX_MODELS {
            return Err(format!(
                "Codex model catalog exceeds the {MAX_MODELS}-model safety limit"
            ));
        }
        for model in response.data {
            models.push(normalize_model(model)?);
        }

        match response.next_cursor {
            Some(next_cursor) => {
                validate_cursor(&next_cursor)?;
                if !seen_cursors.insert(next_cursor.clone()) {
                    return Err("Codex model/list pagination repeated a cursor".to_string());
                }
                cursor = Some(next_cursor);
            }
            None => {
                validate_catalog(&models)?;
                return Ok(CodeModelCatalogSnapshot {
                    runtime_generation,
                    models,
                });
            }
        }
    }

    Err(format!(
        "Codex model/list pagination exceeded the {MAX_MODEL_PAGES}-page safety limit"
    ))
}

fn normalize_model(model: WireModel) -> Result<CodeModelOption, String> {
    validate_token("model id", &model.id)?;
    validate_token("model", &model.model)?;
    validate_label("model display name", &model.display_name)?;
    validate_description("model description", &model.description)?;
    validate_token("default reasoning effort", &model.default_reasoning_effort)?;
    if model.hidden {
        return Err(
            "Codex model/list returned a hidden model despite includeHidden=false".to_string(),
        );
    }
    if model.supported_reasoning_efforts.len() > MAX_EFFORTS_PER_MODEL {
        return Err(format!(
            "Codex model {} exceeds the {MAX_EFFORTS_PER_MODEL}-effort safety limit",
            model.model
        ));
    }

    let mut seen_efforts = HashSet::new();
    let mut supported_reasoning_efforts =
        Vec::with_capacity(model.supported_reasoning_efforts.len());
    for effort in model.supported_reasoning_efforts {
        validate_token("reasoning effort", &effort.reasoning_effort)?;
        validate_description("reasoning effort description", &effort.description)?;
        if !seen_efforts.insert(effort.reasoning_effort.clone()) {
            return Err(format!(
                "Codex model {} contains duplicate reasoning effort {}",
                model.model, effort.reasoning_effort
            ));
        }
        supported_reasoning_efforts.push(CodeReasoningEffortOption {
            reasoning_effort: effort.reasoning_effort,
            description: effort.description,
        });
    }
    if !seen_efforts.contains(&model.default_reasoning_effort) {
        return Err(format!(
            "Codex model {} default reasoning effort is not advertised as supported",
            model.model
        ));
    }

    Ok(CodeModelOption {
        id: model.id,
        model: model.model,
        display_name: model.display_name,
        description: model.description,
        is_default: model.is_default,
        default_reasoning_effort: model.default_reasoning_effort,
        supported_reasoning_efforts,
    })
}

fn validate_catalog(models: &[CodeModelOption]) -> Result<(), String> {
    if models.is_empty() {
        return Err("Codex model catalog cannot be empty".to_string());
    }
    let mut ids = HashSet::new();
    let mut model_names = HashSet::new();
    let mut default_count = 0usize;
    for model in models {
        if !ids.insert(model.id.as_str()) {
            return Err(format!(
                "Codex model catalog contains duplicate id {}",
                model.id
            ));
        }
        if !model_names.insert(model.model.as_str()) {
            return Err(format!(
                "Codex model catalog contains duplicate model {}",
                model.model
            ));
        }
        if model.is_default {
            default_count += 1;
        }
    }
    if default_count > 1 {
        return Err("Codex model catalog contains more than one default model".to_string());
    }
    Ok(())
}

pub(crate) fn turn_selection(
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<Option<CodeModelSelection>, String> {
    match (model, effort) {
        (None, None) => Ok(None),
        (Some(model), Some(reasoning_effort)) => {
            let selection = CodeModelSelection {
                model: model.to_string(),
                reasoning_effort: reasoning_effort.to_string(),
            };
            selection.validate_shape()?;
            Ok(Some(selection))
        }
        _ => Err(
            "Codex turn model and reasoning effort must either both be provided or both be null"
                .to_string(),
        ),
    }
}

fn validate_token(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "Codex {label} must be a trimmed, non-control string between 1 and {MAX_ID_BYTES} bytes"
        ));
    }
    Ok(())
}

fn validate_label(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_LABEL_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "Codex {label} must be a trimmed, non-control string between 1 and {MAX_LABEL_BYTES} bytes"
        ));
    }
    Ok(())
}

fn validate_description(label: &str, value: &str) -> Result<(), String> {
    if value.len() > MAX_DESCRIPTION_BYTES || value.chars().any(|ch| ch == '\0') {
        return Err(format!(
            "Codex {label} exceeds the {MAX_DESCRIPTION_BYTES}-byte safety limit or contains NUL"
        ));
    }
    Ok(())
}

fn validate_cursor(cursor: &str) -> Result<(), String> {
    if cursor.is_empty()
        || cursor.len() > MAX_CURSOR_BYTES
        || cursor.trim() != cursor
        || cursor.chars().any(char::is_control)
    {
        return Err(format!(
            "Codex model/list cursor must be a trimmed, non-control string between 1 and {MAX_CURSOR_BYTES} bytes"
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodeModelSelectionFile {
    version: u32,
    selection: CodeModelSelection,
}

/// Filesystem owner of SchoolX's installation-global recent selection.
#[derive(Clone, Debug)]
pub(crate) struct CodeModelSelectionStore {
    app_data_dir: PathBuf,
    code_dir: PathBuf,
    path: PathBuf,
    read_only: bool,
}

impl CodeModelSelectionStore {
    pub(crate) fn for_app_data(app_data_dir: &Path) -> Result<Self, String> {
        let (app_data_dir, code_dir) = resolve_store_directories(app_data_dir, true)?
            .ok_or_else(|| "SchoolX Code model selection directory was not created".to_string())?;
        let store = Self {
            path: code_dir.join(MODEL_SELECTION_FILE),
            app_data_dir,
            code_dir,
            read_only: false,
        };
        store.validate_paths()?;
        Ok(store)
    }

    pub(crate) fn for_app_data_read_only(app_data_dir: &Path) -> Result<Option<Self>, String> {
        let Some((app_data_dir, code_dir)) = resolve_store_directories(app_data_dir, false)? else {
            return Ok(None);
        };
        let store = Self {
            path: code_dir.join(MODEL_SELECTION_FILE),
            app_data_dir,
            code_dir,
            read_only: true,
        };
        store.validate_paths()?;
        Ok(Some(store))
    }

    pub(crate) fn load(&self) -> Result<Option<CodeModelSelection>, String> {
        self.validate_paths()?;
        let mut file = match open_selection_file(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "failed to open SchoolX Code model selection: {error}"
                ));
            }
        };
        let metadata = file
            .metadata()
            .map_err(|error| format!("failed to inspect SchoolX Code model selection: {error}"))?;
        if !metadata.is_file() {
            return Err("SchoolX Code model selection path is not a regular file".to_string());
        }
        validate_selection_file_permissions(&self.app_data_dir, &metadata)?;
        if metadata.len() > MAX_MODEL_SELECTION_BYTES {
            return Err(format!(
                "SchoolX Code model selection exceeds the {MAX_MODEL_SELECTION_BYTES}-byte limit"
            ));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.read_to_end(&mut bytes)
            .map_err(|error| format!("failed to read SchoolX Code model selection: {error}"))?;
        if bytes.len() as u64 > MAX_MODEL_SELECTION_BYTES {
            return Err(format!(
                "SchoolX Code model selection exceeds the {MAX_MODEL_SELECTION_BYTES}-byte limit"
            ));
        }
        let stored: CodeModelSelectionFile = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid SchoolX Code model selection: {error}"))?;
        if stored.version != MODEL_SELECTION_VERSION {
            return Err(format!(
                "unsupported SchoolX Code model selection version {}",
                stored.version
            ));
        }
        stored.selection.validate_shape()?;
        Ok(Some(stored.selection))
    }

    pub(crate) fn save(&self, selection: &CodeModelSelection) -> Result<(), String> {
        if self.read_only {
            return Err(
                "read-only SchoolX Code model selection store cannot be changed".to_string(),
            );
        }
        selection.validate_shape()?;
        // Refuse to overwrite malformed or unsupported bytes. A valid stale
        // catalog value may be replaced by a newly validated selection.
        let _ = self.load()?;
        self.validate_paths()?;
        let stored = CodeModelSelectionFile {
            version: MODEL_SELECTION_VERSION,
            selection: selection.clone(),
        };
        let mut payload = serde_json::to_vec_pretty(&stored)
            .map_err(|error| format!("failed to encode SchoolX Code model selection: {error}"))?;
        payload.push(b'\n');
        if payload.len() as u64 > MAX_MODEL_SELECTION_BYTES {
            return Err(format!(
                "SchoolX Code model selection exceeds the {MAX_MODEL_SELECTION_BYTES}-byte limit"
            ));
        }

        let mut file = AtomicWriteFile::open(&self.path)
            .map_err(|error| format!("failed to open SchoolX Code model selection: {error}"))?;
        self.validate_paths()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| {
                    format!("failed to secure SchoolX Code model selection: {error}")
                })?;
        }
        file.write_all(&payload)
            .map_err(|error| format!("failed to write SchoolX Code model selection: {error}"))?;
        #[cfg(unix)]
        let directory = {
            use std::os::fd::AsFd as _;
            file.directory()
                .ok_or_else(|| {
                    "SchoolX Code model selection has no pinned parent directory".to_string()
                })?
                .as_fd()
                .try_clone_to_owned()
                .map_err(|error| {
                    format!("failed to pin SchoolX Code model selection directory: {error}")
                })?
        };
        file.commit()
            .map_err(|error| format!("failed to commit SchoolX Code model selection: {error}"))?;
        #[cfg(unix)]
        rustix::fs::fsync(&directory).map_err(|error| {
            format!("failed to sync SchoolX Code model selection directory: {error}")
        })?;
        Ok(())
    }

    fn validate_paths(&self) -> Result<(), String> {
        validate_real_directory(&self.app_data_dir, "app-data")?;
        validate_real_directory(&self.code_dir, "data")?;
        let current_app_data = self.app_data_dir.canonicalize().map_err(|error| {
            format!("failed to resolve SchoolX Code app-data directory: {error}")
        })?;
        let current_code = self
            .code_dir
            .canonicalize()
            .map_err(|error| format!("failed to resolve SchoolX Code data directory: {error}"))?;
        if current_app_data != self.app_data_dir
            || current_code != self.code_dir
            || current_code.parent() != Some(current_app_data.as_path())
            || self.path.parent() != Some(current_code.as_path())
        {
            return Err("SchoolX Code model selection escaped the app-data root".to_string());
        }
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err("SchoolX Code model selection cannot be a symlink".to_string())
            }
            Ok(metadata) if !metadata.is_file() => {
                Err("SchoolX Code model selection path is not a file".to_string())
            }
            Ok(_) => {
                let resolved = self.path.canonicalize().map_err(|error| {
                    format!("failed to resolve SchoolX Code model selection: {error}")
                })?;
                if resolved.parent() != Some(current_code.as_path()) {
                    return Err(
                        "SchoolX Code model selection escaped its data directory".to_string()
                    );
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "failed to inspect SchoolX Code model selection: {error}"
            )),
        }
    }

    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }
}

fn resolve_store_directories(
    app_data_dir: &Path,
    create: bool,
) -> Result<Option<(PathBuf, PathBuf)>, String> {
    if !app_data_dir.is_absolute() {
        return Err("SchoolX Code app-data directory must be absolute".to_string());
    }
    validate_real_directory(app_data_dir, "app-data")?;
    let app_data_dir = app_data_dir
        .canonicalize()
        .map_err(|error| format!("failed to resolve SchoolX Code app-data directory: {error}"))?;
    let expected_code_dir = app_data_dir.join(CODE_STORE_DIRECTORY);
    match fs::symlink_metadata(&expected_code_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("SchoolX Code data directory cannot be a symlink".to_string());
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err("SchoolX Code data path is not a directory".to_string());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_private_directory(&expected_code_dir)?;
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect SchoolX Code data directory: {error}"
            ));
        }
    }
    if create {
        restrict_directory_to_owner(&expected_code_dir)?;
    }
    let code_dir = expected_code_dir
        .canonicalize()
        .map_err(|error| format!("failed to resolve SchoolX Code data directory: {error}"))?;
    if code_dir != expected_code_dir || code_dir.parent() != Some(app_data_dir.as_path()) {
        return Err("SchoolX Code data directory escaped the app-data root".to_string());
    }
    Ok(Some((app_data_dir, code_dir)))
}

fn validate_real_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect SchoolX Code {label} directory {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "SchoolX Code {label} directory cannot be a symlink"
        ));
    }
    if !metadata.is_dir() {
        return Err(format!("SchoolX Code {label} path is not a directory"));
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(format!(
                "failed to create SchoolX Code data directory {}: {error}",
                path.display()
            )),
        }
    }
    #[cfg(not(unix))]
    {
        match fs::create_dir(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(format!(
                "failed to create SchoolX Code data directory {}: {error}",
                path.display()
            )),
        }
    }
}

#[cfg(unix)]
fn restrict_directory_to_owner(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| format!("failed to securely open SchoolX Code data directory: {error}"))?;
    directory
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to secure SchoolX Code data directory: {error}"))
}

#[cfg(not(unix))]
fn restrict_directory_to_owner(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn open_selection_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_selection_file(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new().read(true).open(path)
}

#[cfg(unix)]
fn validate_selection_file_permissions(
    app_data_dir: &Path,
    metadata: &fs::Metadata,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;

    let app_metadata = fs::symlink_metadata(app_data_dir)
        .map_err(|error| format!("failed to inspect SchoolX Code app-data ownership: {error}"))?;
    if metadata.uid() != app_metadata.uid() {
        return Err("SchoolX Code model selection has an unexpected owner".to_string());
    }
    if metadata.mode() & 0o7777 != 0o600 {
        return Err("SchoolX Code model selection is not private".to_string());
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_selection_file_permissions(
    _app_data_dir: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests;
