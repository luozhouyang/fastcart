//! Plugin manifest, artifact and service errors.

use fastcart_kernel::AppError;

/// Errors raised by the plugin registry, artifact store and service.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// The manifest is structurally invalid or violates a rule.
    #[error("invalid plugin manifest: {0}")]
    InvalidManifest(String),
    /// The manifest targets a host API this build does not provide.
    #[error("unsupported plugin API version: {0}")]
    UnsupportedApi(u32),
    /// The manifest declares a target the host does not know.
    #[error("unknown admin target: {0}")]
    UnknownTarget(String),
    /// The manifest declares a known but disabled target.
    #[error("target is not enabled in this host version: {0}")]
    ReservedTarget(String),
    /// The manifest declares a capability the host does not grant.
    #[error("unsupported capability: {0}")]
    UnsupportedScope(String),
    /// The archive is malformed.
    #[error("invalid plugin archive: {0}")]
    InvalidArchive(String),
    /// The archive exceeds an entry, file or size limit.
    #[error("plugin archive exceeds a limit: {0}")]
    ArchiveTooLarge(String),
    /// The archive contains an unsafe path or entry type.
    #[error("unsafe plugin archive entry: {0}")]
    UnsafePath(String),
    /// A plugin with the same id is already installed.
    #[error("plugin already installed: {0}")]
    Duplicate(String),
    /// The plugin is not installed.
    #[error("plugin not found: {0}")]
    NotFound(String),
    /// The plugin is enabled and cannot be removed.
    #[error("plugin is enabled: {0}")]
    Enabled(String),
    /// The plugin is disabled and cannot be served.
    #[error("plugin is disabled: {0}")]
    Disabled(String),
    /// There is no previous artifact to roll back to.
    #[error("plugin has no previous digest")]
    NoPreviousDigest,
    /// Settings do not match the manifest schema.
    #[error("settings do not match the plugin schema: {0}")]
    InvalidSettings(String),
    /// A secret name or value violates a rule.
    #[error("invalid plugin secret: {0}")]
    InvalidSecret(String),
    /// A stored secret is absent.
    #[error("plugin secret not found: {0}")]
    SecretNotFound(String),
    /// Secret encryption or decryption failed.
    #[error("plugin secret error: {0}")]
    Crypto(String),
    /// The stored artifact does not match its digest.
    #[error("artifact digest mismatch: {0}")]
    DigestMismatch(String),
    /// An upgrade adds capabilities without explicit acceptance.
    #[error("the new version requests additional capabilities: {0}")]
    ScopesNotAccepted(String),
    /// Filesystem or artifact storage failure.
    #[error("artifact storage error: {0}")]
    Storage(String),
    /// Store database failure.
    #[error("database error: {0}")]
    Database(String),
    /// Plugin host process or forwarding failure.
    #[error("plugin runtime error: {0}")]
    Runtime(String),
}

impl PluginError {
    /// Stable machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidManifest(_) | Self::UnsupportedApi(_) => "invalid_manifest",
            Self::UnknownTarget(_) => "unknown_target",
            Self::ReservedTarget(_) => "reserved_target",
            Self::UnsupportedScope(_) => "unsupported_scope",
            Self::InvalidArchive(_) => "invalid_archive",
            Self::ArchiveTooLarge(_) => "archive_too_large",
            Self::UnsafePath(_) => "path_traversal",
            Self::Duplicate(_) => "duplicate_plugin",
            Self::NotFound(_) => "not_found",
            Self::Enabled(_) => "plugin_enabled",
            Self::Disabled(_) => "plugin_disabled",
            Self::NoPreviousDigest => "no_previous_digest",
            Self::InvalidSettings(_) => "settings_invalid",
            Self::InvalidSecret(_) => "secret_invalid",
            Self::SecretNotFound(_) => "secret_not_found",
            Self::Crypto(_) => "secret_error",
            Self::DigestMismatch(_) => "digest_mismatch",
            Self::ScopesNotAccepted(_) => "scopes_not_accepted",
            Self::Storage(_) => "storage",
            Self::Database(_) => "database",
            Self::Runtime(_) => "plugin_runtime",
        }
    }
}

impl From<sqlx::Error> for PluginError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<fastcart_infrastructure::InfrastructureError> for PluginError {
    fn from(error: fastcart_infrastructure::InfrastructureError) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<PluginError> for AppError {
    fn from(error: PluginError) -> Self {
        match error {
            PluginError::InvalidManifest(message)
            | PluginError::InvalidArchive(message)
            | PluginError::ArchiveTooLarge(message)
            | PluginError::UnsafePath(message)
            | PluginError::InvalidSettings(message)
            | PluginError::InvalidSecret(message) => AppError::Validation(message),
            PluginError::UnsupportedApi(version) => {
                AppError::Validation(format!("unsupported plugin API version: {version}"))
            }
            PluginError::UnknownTarget(target) | PluginError::ReservedTarget(target) => {
                AppError::Validation(format!("unsupported admin target: {target}"))
            }
            PluginError::UnsupportedScope(scope) => {
                AppError::Validation(format!("unsupported capability: {scope}"))
            }
            PluginError::Duplicate(id) => {
                AppError::Conflict(format!("plugin already installed: {id}"))
            }
            PluginError::NotFound(id) => AppError::not_found("plugin", id),
            PluginError::Enabled(id) => AppError::Conflict(format!("plugin is enabled: {id}")),
            PluginError::Disabled(id) => AppError::Conflict(format!("plugin is disabled: {id}")),
            PluginError::NoPreviousDigest => {
                AppError::Conflict("plugin has no previous digest".to_owned())
            }
            PluginError::DigestMismatch(digest) => {
                AppError::Conflict(format!("artifact digest mismatch: {digest}"))
            }
            PluginError::ScopesNotAccepted(scopes) => AppError::Conflict(format!(
                "additional capabilities require acceptance: {scopes}"
            )),
            PluginError::SecretNotFound(name) => AppError::not_found("plugin secret", name),
            PluginError::Storage(message) | PluginError::Runtime(message) => {
                AppError::internal(message)
            }
            PluginError::Crypto(message) => AppError::internal(message),
            PluginError::Database(message) => AppError::database(message),
        }
    }
}
