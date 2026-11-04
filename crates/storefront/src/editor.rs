//! Theme editor manifest: the structural contract shared by the admin editor,
//! page validation and the AI tool layer.
//!
//! The manifest describes data only. It never contains render code, and the
//! platform uses it to validate page bodies before they are stored.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::StorefrontError;
use crate::theme::SUPPORTED_EDITOR_SCHEMA_VERSION;

/// Maximum nesting depth accepted in an editor field definition.
const MAX_FIELD_DEPTH: usize = 8;
/// Maximum number of components a theme may expose.
pub const MAX_EDITOR_COMPONENTS: usize = 500;
/// Maximum number of fields on one component.
pub const MAX_COMPONENT_FIELDS: usize = 100;

/// Data source identifiers fixed by the platform vocabulary.
pub const DATA_SOURCE_KINDS: [&str; 8] = [
    "collection",
    "product",
    "product_list",
    "page",
    "navigation",
    "asset",
    "search",
    "inquiry_form",
];

/// Field types accepted by the editor contract.
pub const EDITOR_FIELD_TYPES: [&str; 10] = [
    "text",
    "textarea",
    "number",
    "boolean",
    "select",
    "object",
    "array",
    "data_source",
    "asset",
    "custom",
];

/// A theme editor manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeEditorManifest {
    /// Contract version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Editor palette categories.
    #[serde(default)]
    pub categories: BTreeMap<String, EditorCategory>,
    /// Components available to page authors.
    #[serde(default)]
    pub components: BTreeMap<String, EditorComponent>,
    /// Fields available on the page root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<EditorRoot>,
}

/// One editor palette category.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorCategory {
    /// Display title.
    pub title: String,
    /// Component identifiers assigned to this category.
    #[serde(default)]
    pub components: Vec<String>,
}

/// Root-level page fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorRoot {
    /// Root field definitions.
    #[serde(default)]
    pub fields: BTreeMap<String, EditorField>,
}

/// One page component exposed by the theme.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorComponent {
    /// Display title.
    pub title: String,
    /// Palette category, when the component is user-insertable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Named field definitions keyed by prop name.
    #[serde(default)]
    pub fields: BTreeMap<String, EditorField>,
    /// Default prop values.
    #[serde(default, rename = "defaultProps")]
    pub default_props: Value,
    /// Named nested regions.
    #[serde(default)]
    pub slots: Vec<EditorSlot>,
}

/// A nested region declared by a component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorSlot {
    /// Slot name used in page data.
    pub name: String,
    /// Display title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Component types allowed in the slot; `None` allows every component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_components: Option<Vec<String>>,
}

/// One field definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorField {
    /// Field type.
    #[serde(rename = "type")]
    pub field_type: String,
    /// Data source kind for `data_source` and `asset` fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Whether the prop must be supplied (or present in `defaultProps`).
    #[serde(default)]
    pub required: bool,
    /// Minimum value for numeric fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Maximum value for numeric fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Selectable values for `select` fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<EditorFieldOption>>,
    /// Default value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Nested field definitions for `object` fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<BTreeMap<String, EditorField>>,
    /// Item definition for `array` fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<EditorField>>,
}

/// One select option.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorFieldOption {
    /// Submitted value.
    pub value: String,
    /// Display label.
    pub label: String,
}

impl ThemeEditorManifest {
    /// Parses and validates an editor manifest.
    pub fn parse(value: &Value) -> Result<Self, StorefrontError> {
        let manifest: Self = serde_json::from_value(value.clone()).map_err(|error| {
            StorefrontError::Invalid(format!("invalid editor manifest: {error}"))
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates categories, component references, fields and slots.
    pub fn validate(&self) -> Result<(), StorefrontError> {
        if self.schema_version != SUPPORTED_EDITOR_SCHEMA_VERSION {
            return Err(StorefrontError::Invalid(format!(
                "unsupported editor manifest schema version: {}",
                self.schema_version
            )));
        }
        if self.components.len() > MAX_EDITOR_COMPONENTS {
            return Err(StorefrontError::Invalid(format!(
                "a theme may expose at most {MAX_EDITOR_COMPONENTS} components"
            )));
        }
        if self.components.is_empty() {
            return Err(StorefrontError::Invalid(
                "editor manifest must expose at least one component".to_owned(),
            ));
        }

        for (name, category) in &self.categories {
            if !valid_name(name) {
                return Err(StorefrontError::Invalid(format!(
                    "invalid editor category name: {name}"
                )));
            }
            if category.title.trim().is_empty() || category.title.chars().count() > 120 {
                return Err(StorefrontError::Invalid(format!(
                    "category {name} title must be between 1 and 120 characters"
                )));
            }
            let mut listed = HashSet::new();
            for component in &category.components {
                if !self.components.contains_key(component) {
                    return Err(StorefrontError::Invalid(format!(
                        "category {name} references unknown component: {component}"
                    )));
                }
                if !listed.insert(component.as_str()) {
                    return Err(StorefrontError::Invalid(format!(
                        "category {name} lists component {component} more than once"
                    )));
                }
            }
        }

        for (name, component) in &self.components {
            if !valid_name(name) {
                return Err(StorefrontError::Invalid(format!(
                    "invalid component name: {name}"
                )));
            }
            component.validate(name, self)?;
        }

        if let Some(root) = self.root.as_ref() {
            validate_fields(&root.fields, "root")?;
        }
        Ok(())
    }

    /// Returns the field definitions for one component.
    #[must_use]
    pub fn component_fields(&self, component: &str) -> Option<&BTreeMap<String, EditorField>> {
        self.components.get(component).map(|entry| &entry.fields)
    }
}

impl EditorComponent {
    fn validate(&self, name: &str, manifest: &ThemeEditorManifest) -> Result<(), StorefrontError> {
        if self.title.trim().is_empty() || self.title.chars().count() > 120 {
            return Err(StorefrontError::Invalid(
                "component title must be between 1 and 120 characters".to_owned(),
            ));
        }
        if self.fields.len() > MAX_COMPONENT_FIELDS {
            return Err(StorefrontError::Invalid(format!(
                "a component may declare at most {MAX_COMPONENT_FIELDS} fields"
            )));
        }
        if !self.default_props.is_object() && !self.default_props.is_null() {
            return Err(StorefrontError::Invalid(
                "component defaultProps must be a JSON object".to_owned(),
            ));
        }
        if let Some(defaults) = self.default_props.as_object() {
            for (name, value) in defaults {
                let Some(field) = self.fields.get(name) else {
                    return Err(StorefrontError::Invalid(format!(
                        "component defaultProps declares unknown field: {name}"
                    )));
                };
                field.validate_default(name, value)?;
            }
        }
        if let Some(category) = self.category.as_deref() {
            let Some(entry) = manifest.categories.get(category) else {
                return Err(StorefrontError::Invalid(format!(
                    "component references unknown category: {category}"
                )));
            };
            if !entry.components.iter().any(|listed| listed == name) {
                return Err(StorefrontError::Invalid(format!(
                    "category {category} does not list component {name}"
                )));
            }
        }
        validate_fields(&self.fields, "component")?;

        let mut slots = HashSet::new();
        for slot in &self.slots {
            if !valid_name(&slot.name) {
                return Err(StorefrontError::Invalid(format!(
                    "invalid slot name: {}",
                    slot.name
                )));
            }
            if !slots.insert(slot.name.as_str()) {
                return Err(StorefrontError::Invalid(format!(
                    "duplicate slot: {}",
                    slot.name
                )));
            }
            if let Some(allowed) = slot.allowed_components.as_ref() {
                for component in allowed {
                    if !manifest.components.contains_key(component) {
                        return Err(StorefrontError::Invalid(format!(
                            "slot {} references unknown component: {component}",
                            slot.name
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

fn validate_fields(
    fields: &BTreeMap<String, EditorField>,
    owner: &str,
) -> Result<(), StorefrontError> {
    for (name, field) in fields {
        if !valid_name(name) {
            return Err(StorefrontError::Invalid(format!(
                "invalid {owner} field name: {name}"
            )));
        }
        field.validate(1)?;
    }
    Ok(())
}

impl EditorField {
    fn validate(&self, depth: usize) -> Result<(), StorefrontError> {
        if depth > MAX_FIELD_DEPTH {
            return Err(StorefrontError::Invalid(
                "editor fields exceed the maximum nesting depth".to_owned(),
            ));
        }
        if let Some(default) = self.default.as_ref() {
            self.validate_default("default", default)?;
        }
        if !EDITOR_FIELD_TYPES.contains(&self.field_type.as_str()) {
            return Err(StorefrontError::Invalid(format!(
                "unsupported editor field type: {}",
                self.field_type
            )));
        }
        match self.field_type.as_str() {
            "data_source" => {
                let source = self.source.as_deref().ok_or_else(|| {
                    StorefrontError::Invalid("data_source fields must declare a source".to_owned())
                })?;
                if !DATA_SOURCE_KINDS.contains(&source) {
                    return Err(StorefrontError::Invalid(format!(
                        "unsupported data source: {source}"
                    )));
                }
            }
            "asset" => {
                // `asset` implies the asset source; an explicit source is
                // still validated so the manifest cannot smuggle another kind.
                if let Some(source) = self.source.as_deref()
                    && !DATA_SOURCE_KINDS.contains(&source)
                {
                    return Err(StorefrontError::Invalid(format!(
                        "unsupported data source: {source}"
                    )));
                }
            }
            "number" => {
                if let (Some(minimum), Some(maximum)) = (self.min, self.max)
                    && minimum > maximum
                {
                    return Err(StorefrontError::Invalid(
                        "field min must not exceed max".to_owned(),
                    ));
                }
            }
            "select" => {
                let options = self.options.as_ref().ok_or_else(|| {
                    StorefrontError::Invalid("select fields must declare options".to_owned())
                })?;
                if options.is_empty() {
                    return Err(StorefrontError::Invalid(
                        "select fields must declare at least one option".to_owned(),
                    ));
                }
                let mut values = HashSet::new();
                for option in options {
                    if option.value.trim().is_empty() || option.label.trim().is_empty() {
                        return Err(StorefrontError::Invalid(
                            "select option values and labels must not be empty".to_owned(),
                        ));
                    }
                    if !values.insert(option.value.as_str()) {
                        return Err(StorefrontError::Invalid(format!(
                            "duplicate select option: {}",
                            option.value
                        )));
                    }
                }
            }
            "object" => {
                if let Some(fields) = self.fields.as_ref() {
                    for (name, field) in fields {
                        if !valid_name(name) {
                            return Err(StorefrontError::Invalid(format!(
                                "invalid object field name: {name}"
                            )));
                        }
                        field.validate(depth + 1)?;
                    }
                }
            }
            "array" => {
                if let Some(items) = self.items.as_ref() {
                    items.validate(depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_default(&self, name: &str, value: &Value) -> Result<(), StorefrontError> {
        if value.is_null() {
            return Err(StorefrontError::Invalid(format!(
                "default value for {name} must not be null"
            )));
        }
        match self.field_type.as_str() {
            "text" | "textarea" => {
                if !value.is_string() {
                    return Err(StorefrontError::Invalid(format!(
                        "default value for {name} must be a string"
                    )));
                }
            }
            "number" => {
                let Some(number) = value.as_f64() else {
                    return Err(StorefrontError::Invalid(format!(
                        "default value for {name} must be a number"
                    )));
                };
                if self.min.is_some_and(|minimum| number < minimum)
                    || self.max.is_some_and(|maximum| number > maximum)
                {
                    return Err(StorefrontError::Invalid(format!(
                        "default value for {name} is outside the declared range"
                    )));
                }
            }
            "boolean" => {
                if !value.is_boolean() {
                    return Err(StorefrontError::Invalid(format!(
                        "default value for {name} must be a boolean"
                    )));
                }
            }
            "select" => {
                let Some(selected) = value.as_str() else {
                    return Err(StorefrontError::Invalid(format!(
                        "default value for {name} must be a string"
                    )));
                };
                let valid = self
                    .options
                    .as_ref()
                    .is_some_and(|options| options.iter().any(|option| option.value == selected));
                if !valid {
                    return Err(StorefrontError::Invalid(format!(
                        "default value for {name} is not a declared option"
                    )));
                }
            }
            "object" | "data_source" | "asset" => {
                if !value.is_object() {
                    return Err(StorefrontError::Invalid(format!(
                        "default value for {name} must be an object"
                    )));
                }
            }
            "array" if !value.is_array() => {
                return Err(StorefrontError::Invalid(format!(
                    "default value for {name} must be an array"
                )));
            }
            _ => {}
        }
        Ok(())
    }
}

const fn default_schema_version() -> u32 {
    SUPPORTED_EDITOR_SCHEMA_VERSION
}

/// Returns true for identifiers accepted in the editor contract.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && !name.starts_with('-')
        && !name.ends_with('-')
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ThemeEditorManifest, valid_name};

    fn manifest() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "categories": {
                "hero": { "title": "Hero", "components": ["Hero"] },
                "product": { "title": "Product", "components": ["ProductGrid"] }
            },
            "components": {
                "Hero": {
                    "title": "Hero",
                    "category": "hero",
                    "fields": {
                        "title": { "type": "text", "required": true }
                    },
                    "defaultProps": { "title": "Welcome" },
                    "slots": []
                },
                "ProductGrid": {
                    "title": "Product Grid",
                    "category": "product",
                    "fields": {
                        "collection": {
                            "type": "data_source",
                            "source": "collection",
                            "required": true
                        },
                        "limit": { "type": "number", "min": 1, "max": 24, "default": 4 }
                    },
                    "defaultProps": { "limit": 4 },
                    "slots": []
                }
            },
            "root": { "fields": { "title": { "type": "text" } } }
        })
    }

    #[test]
    fn parses_a_valid_manifest() {
        let parsed = ThemeEditorManifest::parse(&manifest()).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert!(parsed.components.contains_key("ProductGrid"));
        assert!(
            parsed
                .component_fields("ProductGrid")
                .unwrap()
                .contains_key("collection")
        );
    }

    #[test]
    fn rejects_unknown_components_categories_and_fields() {
        let mut value = manifest();
        value["categories"]["hero"]["components"] = json!(["Missing"]);
        assert!(ThemeEditorManifest::parse(&value).is_err());

        let mut value = manifest();
        value["components"]["Hero"]["category"] = json!("missing");
        assert!(ThemeEditorManifest::parse(&value).is_err());

        let mut value = manifest();
        value["components"]["Hero"]["fields"]["title"]["type"] = json!("unsupported");
        assert!(ThemeEditorManifest::parse(&value).is_err());

        let mut value = manifest();
        value["components"]["Hero"]["fields"]["collection"] = json!({ "type": "data_source" });
        assert!(ThemeEditorManifest::parse(&value).is_err());

        let mut value = manifest();
        value["components"]["ProductGrid"]["defaultProps"]["limit"] = json!("4");
        assert!(ThemeEditorManifest::parse(&value).is_err());
    }

    #[test]
    fn validates_identifiers() {
        assert!(valid_name("ProductGrid"));
        assert!(valid_name("product-grid"));
        assert!(!valid_name(""));
        assert!(!valid_name("space name"));
        assert!(!valid_name("-leading"));
    }
}
