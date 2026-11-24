//! Plugin manifest model and validation.

use std::collections::{BTreeMap, BTreeSet};

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::{Scope, parse_scope};
use crate::error::PluginError;
use crate::targets::{is_page_target, validate_target};

/// Manifest schema version understood by this build.
pub const SCHEMA_VERSION: &str = "1";
/// Host plugin API version.
pub const HOST_API_VERSION: u32 = 2;
/// Oldest accepted workerd compatibility date.
pub const MIN_COMPATIBILITY_DATE: &str = "2025-01-01";
/// Newest accepted workerd compatibility date.
pub const MAX_COMPATIBILITY_DATE: &str = "2026-12-31";
/// Compatibility flags accepted in v1.
pub const ALLOWED_COMPATIBILITY_FLAGS: &[&str] = &["nodejs_compat"];
/// Maximum number of allowlisted egress hosts.
pub const MAX_EGRESS_HOSTS: usize = 10;
/// Maximum number of declared jobs.
pub const MAX_JOBS: usize = 10;
/// Maximum number of admin extensions.
pub const MAX_ADMIN_EXTENSIONS: usize = 10;
/// Maximum number of storefront extensions.
pub const MAX_STOREFRONT_EXTENSIONS: usize = 5;
/// Maximum number of subscribed event types.
pub const MAX_EVENTS: usize = 20;

/// A validated plugin manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    /// Manifest schema version.
    pub schema_version: String,
    /// Stable plugin identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Semantic version.
    pub version: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional author metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    /// Host plugin API version.
    pub api_version: u32,
    /// Runtime requirements.
    pub runtime: RuntimeSpec,
    /// Worker entry.
    pub entry: EntrySpec,
    /// Requested capabilities.
    #[serde(default)]
    pub capabilities: Capabilities,
    /// UI extensions.
    #[serde(default)]
    pub extensions: Extensions,
    /// Settings schema and optional custom UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<SettingsSpec>,
    /// Declared admin RPC methods.
    #[serde(default)]
    pub rpc: Vec<String>,
    /// Declared jobs.
    #[serde(default)]
    pub jobs: Vec<JobSpec>,
    /// Reserved for publisher signatures; ignored in v1.
    #[serde(default)]
    pub signatures: Vec<Value>,
    /// Build metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildInfo>,
}

/// Manifest author metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    /// Author name.
    pub name: String,
    /// Optional URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Runtime requirements.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSpec {
    /// Execution engine.
    pub engine: String,
    /// workerd compatibility date (`YYYY-MM-DD`).
    pub compatibility_date: String,
    /// Compatibility flags.
    #[serde(default)]
    pub compatibility_flags: Vec<String>,
}

/// Worker entry point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntrySpec {
    /// Entry type; only `worker` is supported.
    #[serde(rename = "type")]
    pub kind: String,
    /// Bundle-relative module path.
    pub path: String,
    /// Exported handler name; defaults to the default export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<String>,
}

/// Requested capabilities grouped by area.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    /// Business data scopes.
    #[serde(default)]
    pub data: Vec<String>,
    /// Storage scopes.
    #[serde(default)]
    pub storage: Vec<String>,
    /// Subscribed event types.
    #[serde(default)]
    pub events: Vec<String>,
    /// Allowlisted egress hosts.
    #[serde(default)]
    pub network: Vec<String>,
    /// Hosts for which the Authorization header may be forwarded.
    #[serde(default)]
    pub network_auth: Vec<String>,
    /// Admin capability names.
    #[serde(default)]
    pub admin: Vec<String>,
    /// Storefront scopes.
    #[serde(default)]
    pub storefront: Vec<String>,
}

/// UI extension declarations.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extensions {
    /// Admin targets.
    #[serde(default)]
    pub admin: Vec<AdminExtension>,
    /// Storefront embeds.
    #[serde(default)]
    pub storefront: Vec<StorefrontExtension>,
}

/// One admin UI extension.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminExtension {
    /// Target name.
    pub target: String,
    /// Merchant-facing title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Navigation label (nav target only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Icon name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Navigation route (nav target only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// HTML entry served in the iframe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// Preferred initial height in CSS pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Sort order within a slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
}

/// One storefront embed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorefrontExtension {
    /// Extension kind; only `embed` in v1.
    pub kind: String,
    /// Injection placement.
    pub placement: String,
    /// Boot module path.
    pub entry: String,
    /// Optional stylesheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub css: Option<String>,
    /// Page matchers; defaults to every page.
    #[serde(default = "default_pages")]
    pub pages: Vec<String>,
    /// Paths excluded from injection.
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    /// Approximate rendered height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_hint: Option<u32>,
}

fn default_pages() -> Vec<String> {
    vec!["*".to_owned()]
}

/// Settings schema and optional custom UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsSpec {
    /// JSON Schema for the settings object.
    pub schema: Value,
    /// Optional custom settings page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,
}

/// One declared job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobSpec {
    /// Job name.
    pub name: String,
    /// Optional five-field cron schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
}

/// Build metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildInfo {
    /// Build identifier.
    pub build_id: String,
    /// Creation timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

mod validation;

impl PluginManifest {
    /// Parses a manifest from JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidManifest`] when the JSON shape is invalid.
    pub fn parse(bytes: &[u8]) -> Result<Self, PluginError> {
        serde_json::from_slice(bytes)
            .map_err(|error| PluginError::InvalidManifest(error.to_string()))
    }
}

impl PluginManifest {
    /// Effective capabilities granted by this manifest.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::UnsupportedScope`] when a declared scope is invalid.
    pub fn scopes(&self) -> Result<Vec<Scope>, PluginError> {
        let mut scopes = BTreeSet::new();
        scopes.insert(Scope::Context);
        for value in &self.capabilities.data {
            scopes.insert(parse_scope("data", value)?);
        }
        for value in &self.capabilities.storage {
            scopes.insert(parse_scope("storage", value)?);
        }
        for value in &self.capabilities.storefront {
            scopes.insert(parse_scope("storefront", value)?);
        }
        if !self.capabilities.events.is_empty() {
            scopes.insert(Scope::EventsSubscribe);
        }
        if !self.capabilities.network.is_empty() {
            scopes.insert(Scope::NetworkEgress);
        }
        if !self.jobs.is_empty() {
            scopes.insert(Scope::JobsEnqueue);
        }
        if !self.extensions.admin.is_empty() {
            scopes.insert(Scope::AdminUi);
        }
        if !self.extensions.storefront.is_empty() {
            scopes.insert(Scope::StorefrontUi);
        }
        Ok(scopes.into_iter().collect())
    }
}

impl PluginManifest {
    /// Validates references to files that must exist in the archive.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidManifest`] for a missing reference and
    /// [`PluginError::ArchiveTooLarge`] when one module exceeds the cap.
    pub fn validate_archive(
        &self,
        files: &BTreeMap<String, u64>,
        max_module_bytes: u64,
    ) -> Result<(), PluginError> {
        require_file(files, "plugin.json")?;
        require_file(files, &self.entry.path)?;
        for extension in &self.extensions.admin {
            if let Some(entry) = &extension.entry {
                require_file(files, entry)?;
            }
        }
        for extension in &self.extensions.storefront {
            require_file(files, &extension.entry)?;
            if let Some(css) = &extension.css {
                require_file(files, css)?;
            }
        }
        if let Some(settings) = &self.settings
            && let Some(ui) = &settings.ui
        {
            require_file(files, ui)?;
        }
        for (path, size) in files {
            if (path.ends_with(".js") || path.ends_with(".mjs")) && *size > max_module_bytes {
                return Err(PluginError::ArchiveTooLarge(format!(
                    "module {path} exceeds {max_module_bytes} bytes"
                )));
            }
        }
        Ok(())
    }
}

fn require_file(files: &BTreeMap<String, u64>, path: &str) -> Result<(), PluginError> {
    if files.contains_key(path) {
        Ok(())
    } else {
        Err(PluginError::InvalidManifest(format!(
            "manifest references a missing file: {path}"
        )))
    }
}

/// Validates a settings object against the manifest schema.
///
/// # Errors
///
/// Returns [`PluginError::InvalidSettings`] with the first violation.
pub fn validate_settings(manifest: &PluginManifest, settings: &Value) -> Result<(), PluginError> {
    let Some(spec) = &manifest.settings else {
        if settings.as_object().is_some_and(serde_json::Map::is_empty) {
            return Ok(());
        }
        return Err(PluginError::InvalidSettings(
            "this plugin does not declare settings".to_owned(),
        ));
    };
    let validator = jsonschema::validator_for(&spec.schema)
        .map_err(|error| PluginError::InvalidSettings(error.to_string()))?;
    if validator.is_valid(settings) {
        return Ok(());
    }
    let message = validator
        .iter_errors(settings)
        .next()
        .map(|error| error.to_string())
        .unwrap_or_else(|| "settings do not match the schema".to_owned());
    Err(PluginError::InvalidSettings(message))
}

/// Validates a bundle-relative path with an allowed extension.
///
/// # Errors
///
/// Returns [`PluginError::UnsafePath`] or [`PluginError::InvalidManifest`].
pub fn validate_relative_path(path: &str, extensions: &[&str]) -> Result<(), PluginError> {
    if path.is_empty() || path.len() > 256 {
        return Err(PluginError::InvalidManifest(format!(
            "invalid path length: {path}"
        )));
    }
    if path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return Err(PluginError::UnsafePath(path.to_owned()));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(PluginError::UnsafePath(path.to_owned()));
        }
        if !segment
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._".contains(character))
        {
            return Err(PluginError::UnsafePath(path.to_owned()));
        }
    }
    let extension = path.rsplit('.').next().unwrap_or_default();
    if !extensions.contains(&extension) {
        return Err(PluginError::InvalidManifest(format!(
            "unsupported file extension: {path}"
        )));
    }
    Ok(())
}

/// Returns true for a valid plugin, job or dependency identifier.
#[must_use]
pub fn valid_rpc_method(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b'_')
        })
        && !value.starts_with('-')
        && !value.starts_with('.')
        && !value.starts_with('_')
        && !value.ends_with('-')
        && !value.ends_with('.')
        && !value.ends_with('_')
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

fn valid_js_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) if first.is_ascii_alphabetic() || matches!(first, '_' | '$') => {}
        _ => return false,
    }
    characters.all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
}

fn valid_event_name(value: &str) -> bool {
    let segments: Vec<&str> = value.split('.').collect();
    if segments.len() < 2 || segments.len() > 3 {
        return false;
    }
    segments.iter().all(|segment| {
        let mut characters = segment.chars();
        match characters.next() {
            Some(first) if first.is_ascii_lowercase() => {}
            _ => return false,
        }
        characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
    })
}

fn valid_hostname(value: &str) -> bool {
    if value.is_empty() || value.len() > 253 || value != value.to_ascii_lowercase() {
        return false;
    }
    let value = value.strip_prefix("*.").unwrap_or(value);
    if value.is_empty() || value.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    value.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    })
}

fn valid_cron(value: &str) -> bool {
    let fields: Vec<&str> = value.split_whitespace().collect();
    fields.len() == 5
        && fields.iter().all(|field| {
            !field.is_empty()
                && field
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'*' | b'/' | b',' | b'-'))
        })
}
