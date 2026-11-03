//! Structured page schema.

use serde_json::Value;

/// A versioned page definition stored as data, never as executable code.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PageSchema {
    /// Schema version.
    pub version: u32,
    /// Ordered page sections.
    pub sections: Vec<PageSection>,
}

/// One section in a page.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PageSection {
    /// Stable section instance identifier.
    pub id: String,
    /// Registered section type, resolved through a trusted component registry.
    #[serde(rename = "type")]
    pub section_type: String,
    /// Section settings validated against the section schema.
    pub settings: Value,
}
