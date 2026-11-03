//! Structured page schema validation.

use std::collections::HashSet;

use serde_json::Value;

use crate::StorefrontError;

/// Maximum number of sections in one page.
pub const MAX_PAGE_SECTIONS: usize = 200;
/// Current page schema version.
pub const CURRENT_PAGE_SCHEMA_VERSION: u32 = 1;

/// A versioned page definition stored as data, never as executable code.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageSchema {
    /// Schema version.
    pub version: u32,
    /// Ordered page sections.
    pub sections: Vec<PageSection>,
}

/// One section in a page.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageSection {
    /// Stable section instance identifier.
    pub id: String,
    /// Registered section type, resolved through a trusted component registry.
    #[serde(rename = "type")]
    pub section_type: String,
    /// Section settings validated against the section schema.
    pub settings: Value,
}

impl PageSchema {
    /// Validates the schema version, section identifiers, types and settings.
    pub fn validate(&self) -> Result<(), StorefrontError> {
        if self.version != CURRENT_PAGE_SCHEMA_VERSION {
            return Err(StorefrontError::Invalid(format!(
                "unsupported page schema version: {}",
                self.version
            )));
        }
        if self.sections.len() > MAX_PAGE_SECTIONS {
            return Err(StorefrontError::Invalid(format!(
                "a page may contain at most {MAX_PAGE_SECTIONS} sections"
            )));
        }
        let mut ids = HashSet::new();
        for section in &self.sections {
            if section.id.trim().is_empty() || section.id.len() > 120 {
                return Err(StorefrontError::Invalid(
                    "section id must be between 1 and 120 characters".to_owned(),
                ));
            }
            if !ids.insert(section.id.as_str()) {
                return Err(StorefrontError::Invalid(format!(
                    "duplicate section id: {}",
                    section.id
                )));
            }
            if section.section_type.trim().is_empty() || section.section_type.len() > 120 {
                return Err(StorefrontError::Invalid(
                    "section type must be between 1 and 120 characters".to_owned(),
                ));
            }
            if !section.settings.is_object() && !section.settings.is_null() {
                return Err(StorefrontError::Invalid(format!(
                    "section {} settings must be a JSON object",
                    section.id
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{PageSchema, PageSection};

    fn page() -> PageSchema {
        PageSchema {
            version: 1,
            sections: vec![PageSection {
                id: "hero".to_owned(),
                section_type: "hero".to_owned(),
                settings: json!({"title": "Welcome"}),
            }],
        }
    }

    #[test]
    fn valid_pages_pass() {
        page().validate().unwrap();
    }

    #[test]
    fn duplicate_section_ids_and_bad_versions_are_rejected() {
        let mut schema = page();
        schema.sections.push(schema.sections[0].clone());
        assert!(schema.validate().is_err());

        let mut schema = page();
        schema.version = 99;
        assert!(schema.validate().is_err());
    }

    #[test]
    fn section_types_and_settings_are_validated() {
        let mut schema = page();
        schema.sections[0].section_type = " ".to_owned();
        assert!(schema.validate().is_err());

        let mut schema = page();
        schema.sections[0].settings = json!("not an object");
        assert!(schema.validate().is_err());

        let mut schema = page();
        schema.sections[0].settings = serde_json::Value::Null;
        assert!(schema.validate().is_ok());
    }
}
