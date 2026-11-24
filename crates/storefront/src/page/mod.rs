//! Structured page body contract.
//!
//! Page JSON is trusted only as data. The platform validates it before storing
//! and again before handing it to a storefront runtime. Bindings are reference
//! objects; live commerce data never becomes part of the stored body.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::StorefrontError;
use crate::data::{BindingRequest, DataBinding};
use crate::editor::{EditorComponent, EditorField, ThemeEditorManifest, valid_name};
use crate::theme::SUPPORTED_EDITOR_SCHEMA_VERSION;

/// Current page body schema version.
pub const CURRENT_PAGE_SCHEMA_VERSION: u32 = 1;
/// Maximum number of blocks in one page, including nested slots.
pub const MAX_PAGE_BLOCKS: usize = 500;
/// Maximum nesting depth for blocks and slots.
pub const MAX_PAGE_DEPTH: usize = 8;
/// Maximum serialized page body size.
pub const MAX_PAGE_BODY_BYTES: usize = 1024 * 1024;
/// Maximum length of a text prop.
pub const MAX_TEXT_LENGTH: usize = 10_000;
/// Maximum length of a textarea prop.
pub const MAX_TEXTAREA_LENGTH: usize = 100_000;
/// Maximum number of products a data binding may request.
pub const MAX_BINDING_LIMIT: u32 = 100;
/// Ordering values accepted by product list bindings.
pub const PRODUCT_SORT_VALUES: [&str; 4] = [
    "title_asc",
    "title_desc",
    "created_at_asc",
    "created_at_desc",
];

/// Data bag key reserved for request context.
///
/// The server injects store, theme, cart and customer context under this key,
/// so a page block may not reuse it as its block identifier.
pub const RESERVED_CONTEXT_KEY: &str = "context";

/// A versioned page definition compatible with the editor data model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageBody {
    /// Schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Theme the body was authored against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<PageThemeRef>,
    /// Ordered page blocks.
    #[serde(default)]
    pub content: Vec<PageBlock>,
    /// Root property values.
    #[serde(default)]
    pub root: PageRoot,
    /// Named top-level regions such as header or footer.
    #[serde(default)]
    pub zones: BTreeMap<String, Vec<PageBlock>>,
}

/// Theme metadata captured with the page body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageThemeRef {
    /// Theme manifest identifier.
    pub id: String,
    /// Theme version.
    pub version: String,
    /// Editor schema version used when the body was written.
    #[serde(default = "default_editor_schema_version")]
    pub schema_version: u32,
}

/// Root property values.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRoot {
    /// Root props; must be a JSON object.
    #[serde(default)]
    pub props: Value,
}

/// One block in a page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageBlock {
    /// Registered component type.
    #[serde(rename = "type")]
    pub block_type: String,
    /// Component props. The `id` prop is the stable block identifier.
    #[serde(default)]
    pub props: Value,
    /// Nested slot content keyed by declared slot name.
    #[serde(default)]
    pub slots: BTreeMap<String, Vec<PageBlock>>,
}

impl Default for PageBody {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_PAGE_SCHEMA_VERSION,
            theme: None,
            content: Vec::new(),
            root: PageRoot::default(),
            zones: BTreeMap::new(),
        }
    }
}

impl PageBody {
    /// Parses a page body. Null and an empty object are treated as an empty
    /// page so newly created pages remain valid before the first edit.
    pub fn parse(value: &Value) -> Result<Self, StorefrontError> {
        if value.is_null() || value.as_object().is_some_and(Map::is_empty) {
            return Ok(Self::default());
        }
        let serialized = serde_json::to_string(value)
            .map_err(|error| StorefrontError::Invalid(format!("invalid page body: {error}")))?;
        if serialized.len() > MAX_PAGE_BODY_BYTES {
            return Err(StorefrontError::Invalid(format!(
                "page body must not exceed {MAX_PAGE_BODY_BYTES} bytes"
            )));
        }
        let body: Self = serde_json::from_value(value.clone())
            .map_err(|error| StorefrontError::Invalid(format!("invalid page body: {error}")))?;
        body.validate()?;
        Ok(body)
    }

    /// Validates the body without a theme editor manifest.
    pub fn validate(&self) -> Result<(), StorefrontError> {
        if self.schema_version != CURRENT_PAGE_SCHEMA_VERSION {
            return Err(StorefrontError::Invalid(format!(
                "unsupported page schema version: {}",
                self.schema_version
            )));
        }
        if let Some(theme) = self.theme.as_ref() {
            if !valid_name(&theme.id) {
                return Err(StorefrontError::Invalid(format!(
                    "invalid page theme id: {}",
                    theme.id
                )));
            }
            if theme.version.trim().is_empty() || theme.version.len() > 120 {
                return Err(StorefrontError::Invalid(
                    "page theme version must be between 1 and 120 characters".to_owned(),
                ));
            }
            if theme.schema_version != SUPPORTED_EDITOR_SCHEMA_VERSION {
                return Err(StorefrontError::Invalid(format!(
                    "unsupported page theme editor schema version: {}",
                    theme.schema_version
                )));
            }
        }
        if !self.root.props.is_object() && !self.root.props.is_null() {
            return Err(StorefrontError::Invalid(
                "page root props must be a JSON object".to_owned(),
            ));
        }
        let mut ids = HashSet::new();
        let mut count = 0_usize;
        validate_blocks(&self.content, 1, &mut ids, &mut count)?;
        for (zone, blocks) in &self.zones {
            if !valid_name(zone) {
                return Err(StorefrontError::Invalid(format!(
                    "invalid page zone name: {zone}"
                )));
            }
            validate_blocks(blocks, 1, &mut ids, &mut count)?;
        }
        Ok(())
    }

    /// Validates block types, props and slots against a theme editor manifest.
    pub fn validate_with_editor(
        &self,
        editor: &ThemeEditorManifest,
    ) -> Result<(), StorefrontError> {
        self.validate()?;
        if let Some(root) = editor.root.as_ref() {
            validate_known_props(&self.root.props, &root.fields, "root", &Value::Null)?;
            validate_props(&self.root.props, &root.fields, "root", &Value::Null)?;
        }
        let mut ids = HashSet::new();
        let mut count = 0_usize;
        validate_blocks_with_editor(&self.content, editor, 1, &mut ids, &mut count)?;
        for (zone, blocks) in &self.zones {
            if !editor.zones.is_empty() {
                let definition = editor.zones.get(zone).ok_or_else(|| {
                    StorefrontError::Invalid(format!("unknown page zone: {zone}"))
                })?;
                if let Some(allowed) = definition.allowed_components.as_ref() {
                    for block in blocks {
                        if !allowed
                            .iter()
                            .any(|component| component == &block.block_type)
                        {
                            return Err(StorefrontError::Invalid(format!(
                                "component {} is not allowed in zone {zone}",
                                block.block_type
                            )));
                        }
                    }
                }
            }
            validate_blocks_with_editor(blocks, editor, 1, &mut ids, &mut count)?;
        }
        Ok(())
    }

    /// Extracts data bindings declared by component fields.
    ///
    /// The body must already validate against `editor`; callers normally use
    /// [`Self::parse`] followed by [`Self::validate_with_editor`].
    pub fn collect_bindings(
        &self,
        editor: &ThemeEditorManifest,
    ) -> Result<Vec<BindingRequest>, StorefrontError> {
        self.validate_with_editor(editor)?;
        self.collect_bindings_after_validation(editor)
    }

    /// Extracts data bindings from a body that already validates against
    /// `editor`.
    ///
    /// Unlike [`Self::collect_bindings`], this performs only the binding walk
    /// and does not validate the body again. Storefront renderers use it after
    /// validating one page revision so validation cost is paid once.
    pub fn collect_bindings_after_validation(
        &self,
        editor: &ThemeEditorManifest,
    ) -> Result<Vec<BindingRequest>, StorefrontError> {
        let mut bindings = Vec::new();
        collect_block_bindings(&self.content, editor, &mut bindings)?;
        for blocks in self.zones.values() {
            collect_block_bindings(blocks, editor, &mut bindings)?;
        }
        Ok(bindings)
    }
}

mod bindings;
mod validation;

use bindings::collect_block_bindings;
#[cfg(test)]
pub(crate) use validation::validate_binding_value;
use validation::{
    validate_blocks, validate_blocks_with_editor, validate_known_props, validate_props,
};

const fn default_schema_version() -> u32 {
    CURRENT_PAGE_SCHEMA_VERSION
}

const fn default_editor_schema_version() -> u32 {
    SUPPORTED_EDITOR_SCHEMA_VERSION
}

#[cfg(test)]
mod tests;
