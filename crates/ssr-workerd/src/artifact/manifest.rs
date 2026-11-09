//! Versioned FastCart SSR artifact manifest.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::ArtifactError;

/// Manifest schema version understood by this build.
pub const SUPPORTED_SCHEMA_VERSION: &str = "1";

/// Runtime capabilities the gateway can provide today.
pub const SUPPORTED_CAPABILITIES: [&str; 6] = [
    "fetch",
    "streams",
    "web-crypto",
    "timers",
    "url",
    "nodejs-compat",
];

/// One FastCart SSR artifact manifest (`fastcart-ssr.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    /// Manifest schema version.
    pub schema_version: String,
    /// Stable artifact identity.
    pub artifact_id: String,
    /// Framework metadata.
    pub framework: FrameworkMetadata,
    /// Runtime compatibility metadata.
    pub runtime: RuntimeMetadata,
    /// Worker entrypoint.
    pub entry: EntryMetadata,
    /// Client asset location.
    pub assets: AssetMetadata,
    /// Rendering strategy.
    pub rendering: RenderingMetadata,
    /// Build identity.
    pub build: BuildMetadata,
}

/// Framework that produced the artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkMetadata {
    /// Framework name, such as `tanstack-start`.
    pub name: String,
    /// Framework version range or exact version.
    pub version: String,
}

/// Workerd compatibility settings recorded by the packager.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMetadata {
    /// Execution engine. Currently `workerd`.
    pub engine: String,
    /// Workerd compatibility date, `YYYY-MM-DD`.
    pub compatibility_date: String,
    /// Workerd compatibility flags.
    #[serde(default)]
    pub compatibility_flags: Vec<String>,
    /// Capabilities the Worker requires from FastCart.
    #[serde(default)]
    pub required_capabilities: Vec<String>,
}

/// Worker entrypoint inside the artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryMetadata {
    /// Entry type. Currently `worker`.
    #[serde(rename = "type")]
    pub entry_type: String,
    /// Bundle-relative path to the Worker module.
    pub path: String,
    /// Named export implementing the Fetch handler.
    #[serde(default = "default_export")]
    pub export: String,
}

/// Client asset location inside the artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetMetadata {
    /// Bundle-relative client asset directory.
    pub directory: String,
    /// Public URL prefix served by Rust.
    pub base_path: String,
}

/// Rendering strategy declared by the artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderingMetadata {
    /// `static`, `ssr` or `hybrid`.
    pub mode: String,
    /// Whether the artifact may stream a response body.
    pub streaming: bool,
}

/// Build identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildMetadata {
    /// Build identifier.
    pub build_id: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

fn default_export() -> String {
    "default".to_owned()
}

impl ArtifactManifest {
    /// Validates every field and path before any file is copied.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(invalid(format!(
                "unsupported schema_version: {}",
                self.schema_version
            )));
        }
        if !valid_identifier(&self.artifact_id, 128) {
            return Err(invalid("artifact_id must be 1-128 safe characters"));
        }
        if !valid_label(&self.framework.name) {
            return Err(invalid("framework.name is invalid"));
        }
        if !valid_label(&self.framework.version) {
            return Err(invalid("framework.version is invalid"));
        }
        if self.runtime.engine != "workerd" {
            return Err(invalid(format!(
                "unsupported runtime engine: {}",
                self.runtime.engine
            )));
        }
        if !valid_date(&self.runtime.compatibility_date) {
            return Err(invalid("runtime.compatibility_date must be YYYY-MM-DD"));
        }
        let mut flags = HashSet::new();
        for flag in &self.runtime.compatibility_flags {
            if !valid_flag(flag) {
                return Err(invalid(format!("invalid compatibility flag: {flag}")));
            }
            if !flags.insert(flag) {
                return Err(invalid(format!("duplicate compatibility flag: {flag}")));
            }
        }
        let mut capabilities = HashSet::new();
        for capability in &self.runtime.required_capabilities {
            if !SUPPORTED_CAPABILITIES.contains(&capability.as_str()) {
                return Err(invalid(format!(
                    "unsupported required capability: {capability}"
                )));
            }
            if !capabilities.insert(capability) {
                return Err(invalid(format!(
                    "duplicate required capability: {capability}"
                )));
            }
        }
        if self.entry.entry_type != "worker" {
            return Err(invalid(format!(
                "unsupported entry type: {}",
                self.entry.entry_type
            )));
        }
        validate_relative_path(&self.entry.path)?;
        if !valid_export(&self.entry.export) {
            return Err(invalid("entry.export is not a valid JavaScript identifier"));
        }
        validate_relative_path(&self.assets.directory)?;
        if !valid_base_path(&self.assets.base_path) {
            return Err(invalid("assets.base_path must be an absolute URL path"));
        }
        if !matches!(self.rendering.mode.as_str(), "static" | "ssr" | "hybrid") {
            return Err(invalid(format!(
                "unsupported rendering mode: {}",
                self.rendering.mode
            )));
        }
        if !valid_label(&self.build.build_id) {
            return Err(invalid("build.build_id is invalid"));
        }
        if !valid_timestamp(&self.build.created_at) {
            return Err(invalid("build.created_at must be an RFC 3339 timestamp"));
        }
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> ArtifactError {
    ArtifactError::InvalidManifest(message.into())
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'\'' | b'"' | b'\\'))
}

fn valid_flag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_export(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_' || first == b'$')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$')
}

fn valid_base_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains("..")
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'\\' || byte == b'?')
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..].iter().all(u8::is_ascii_digit)
}

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(date) = value.get(..10) else {
        return false;
    };
    bytes.len() >= 20
        && valid_date(date)
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && value.ends_with('Z')
}

/// Validates a bundle-relative path and rejects traversal.
pub fn validate_relative_path(path: &str) -> Result<(), ArtifactError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
        || std::path::Path::new(path).is_absolute()
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return Err(ArtifactError::UnsafePath(path.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ArtifactManifest {
        ArtifactManifest {
            schema_version: "1".to_owned(),
            artifact_id: "artifact_01".to_owned(),
            framework: FrameworkMetadata {
                name: "tanstack-start".to_owned(),
                version: "1.169.5".to_owned(),
            },
            runtime: RuntimeMetadata {
                engine: "workerd".to_owned(),
                compatibility_date: "2026-11-01".to_owned(),
                compatibility_flags: vec![],
                required_capabilities: vec!["fetch".to_owned(), "streams".to_owned()],
            },
            entry: EntryMetadata {
                entry_type: "worker".to_owned(),
                path: "worker/index.js".to_owned(),
                export: "default".to_owned(),
            },
            assets: AssetMetadata {
                directory: "client".to_owned(),
                base_path: "/_fastcart/assets/".to_owned(),
            },
            rendering: RenderingMetadata {
                mode: "hybrid".to_owned(),
                streaming: true,
            },
            build: BuildMetadata {
                build_id: "build_123".to_owned(),
                created_at: "2026-11-01T10:00:00Z".to_owned(),
            },
        }
    }

    #[test]
    fn a_valid_manifest_passes() {
        manifest().validate().unwrap();
    }

    #[test]
    fn traversal_and_absolute_paths_are_rejected() {
        for path in [
            "../worker.js",
            "/worker.js",
            "worker/../x.js",
            "worker\\x.js",
        ] {
            let mut value = manifest();
            value.entry.path = path.to_owned();
            assert!(value.validate().is_err(), "{path}");
        }
    }

    #[test]
    fn unsupported_runtime_and_capabilities_are_rejected() {
        let mut value = manifest();
        value.runtime.engine = "node".to_owned();
        assert!(value.validate().is_err());

        let mut value = manifest();
        value
            .runtime
            .required_capabilities
            .push("durable-objects".to_owned());
        assert!(value.validate().is_err());
    }

    #[test]
    fn duplicate_flags_are_rejected() {
        let mut value = manifest();
        value.runtime.compatibility_flags = vec!["nodejs_compat".to_owned(); 2];
        assert!(value.validate().is_err());
    }
}
