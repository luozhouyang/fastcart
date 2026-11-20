//! Runtime bundle served to the workerd loader.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::manifest::PluginManifest;

/// Module graph handed to the workerd loader.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeBundle {
    /// Artifact digest.
    pub digest: String,
    /// workerd compatibility date.
    pub compatibility_date: String,
    /// Entry module name.
    pub main_module: String,
    /// Module sources keyed by bundle-relative path.
    pub modules: BTreeMap<String, String>,
    /// Compatibility flags.
    pub compatibility_flags: Vec<String>,
    /// Optional runtime limits; unset until P2.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<serde_json::Value>,
}

impl RuntimeBundle {
    /// Builds a bundle from a manifest and its module sources.
    #[must_use]
    pub fn new(
        digest: impl Into<String>,
        manifest: &PluginManifest,
        modules: BTreeMap<String, String>,
    ) -> Self {
        Self {
            digest: digest.into(),
            compatibility_date: manifest.runtime.compatibility_date.clone(),
            main_module: manifest.entry.path.clone(),
            modules,
            compatibility_flags: manifest.runtime.compatibility_flags.clone(),
            limits: None,
        }
    }
}
