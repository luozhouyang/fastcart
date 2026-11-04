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
        for blocks in self.zones.values() {
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
        let mut bindings = Vec::new();
        collect_block_bindings(&self.content, editor, &mut bindings)?;
        for blocks in self.zones.values() {
            collect_block_bindings(blocks, editor, &mut bindings)?;
        }
        Ok(bindings)
    }
}

fn validate_blocks(
    blocks: &[PageBlock],
    depth: usize,
    ids: &mut HashSet<String>,
    count: &mut usize,
) -> Result<(), StorefrontError> {
    if depth > MAX_PAGE_DEPTH {
        return Err(StorefrontError::Invalid(
            "page blocks exceed the maximum nesting depth".to_owned(),
        ));
    }
    for block in blocks {
        *count += 1;
        if *count > MAX_PAGE_BLOCKS {
            return Err(StorefrontError::Invalid(format!(
                "a page may contain at most {MAX_PAGE_BLOCKS} blocks"
            )));
        }
        if !valid_name(&block.block_type) {
            return Err(StorefrontError::Invalid(format!(
                "invalid block type: {}",
                block.block_type
            )));
        }
        let Some(id) = block.props.get("id").and_then(Value::as_str) else {
            return Err(StorefrontError::Invalid(format!(
                "block {} must have a string props.id",
                block.block_type
            )));
        };
        if !valid_name(id) {
            return Err(StorefrontError::Invalid(format!("invalid block id: {id}")));
        }
        if !ids.insert(id.to_owned()) {
            return Err(StorefrontError::Invalid(format!(
                "duplicate block id: {id}"
            )));
        }
        if !block.props.is_object() {
            return Err(StorefrontError::Invalid(format!(
                "block {id} props must be a JSON object"
            )));
        }
        for (slot, children) in &block.slots {
            if !valid_name(slot) {
                return Err(StorefrontError::Invalid(format!(
                    "invalid slot name on block {id}: {slot}"
                )));
            }
            validate_blocks(children, depth + 1, ids, count)?;
        }
    }
    Ok(())
}

fn validate_blocks_with_editor(
    blocks: &[PageBlock],
    editor: &ThemeEditorManifest,
    depth: usize,
    ids: &mut HashSet<String>,
    count: &mut usize,
) -> Result<(), StorefrontError> {
    if depth > MAX_PAGE_DEPTH {
        return Err(StorefrontError::Invalid(
            "page blocks exceed the maximum nesting depth".to_owned(),
        ));
    }
    for block in blocks {
        *count += 1;
        if *count > MAX_PAGE_BLOCKS {
            return Err(StorefrontError::Invalid(format!(
                "a page may contain at most {MAX_PAGE_BLOCKS} blocks"
            )));
        }
        let Some(component) = editor.components.get(&block.block_type) else {
            return Err(StorefrontError::Invalid(format!(
                "unknown block type: {}",
                block.block_type
            )));
        };
        let id = block
            .props
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StorefrontError::Invalid(format!(
                    "block {} must have a string props.id",
                    block.block_type
                ))
            })?;
        if !ids.insert(id.to_owned()) {
            return Err(StorefrontError::Invalid(format!(
                "duplicate block id: {id}"
            )));
        }
        validate_known_props(
            &block.props,
            &component.fields,
            id,
            &component.default_props,
        )?;
        validate_props(
            &block.props,
            &component.fields,
            id,
            &component.default_props,
        )?;
        validate_slots(block, component, editor, depth, ids, count)?;
    }
    Ok(())
}

fn validate_slots(
    block: &PageBlock,
    component: &EditorComponent,
    editor: &ThemeEditorManifest,
    depth: usize,
    ids: &mut HashSet<String>,
    count: &mut usize,
) -> Result<(), StorefrontError> {
    for (slot_name, children) in &block.slots {
        let Some(slot) = component.slots.iter().find(|slot| slot.name == *slot_name) else {
            return Err(StorefrontError::Invalid(format!(
                "block {} contains undeclared slot: {slot_name}",
                block.block_type
            )));
        };
        if let Some(allowed) = slot.allowed_components.as_ref() {
            for child in children {
                if !allowed.iter().any(|name| name == &child.block_type) {
                    return Err(StorefrontError::Invalid(format!(
                        "block type {} is not allowed in slot {slot_name}",
                        child.block_type
                    )));
                }
            }
        }
        validate_blocks_with_editor(children, editor, depth + 1, ids, count)?;
    }
    Ok(())
}

fn validate_known_props(
    props: &Value,
    fields: &BTreeMap<String, EditorField>,
    owner: &str,
    defaults: &Value,
) -> Result<(), StorefrontError> {
    let Some(values) = props.as_object() else {
        return Ok(());
    };
    for name in values.keys() {
        if name == "id" || name.starts_with('_') || fields.contains_key(name) {
            continue;
        }
        if defaults.get(name).is_some() {
            continue;
        }
        return Err(StorefrontError::Invalid(format!(
            "{owner} declares unknown prop: {name}"
        )));
    }
    Ok(())
}

fn validate_props(
    props: &Value,
    fields: &BTreeMap<String, EditorField>,
    owner: &str,
    defaults: &Value,
) -> Result<(), StorefrontError> {
    let Some(values) = props.as_object() else {
        return Ok(());
    };
    for (name, field) in fields {
        match values.get(name) {
            Some(value) => validate_field_value(field, name, value)?,
            None => {
                let has_default = field.default.is_some() || defaults.get(name).is_some();
                if field.required && !has_default {
                    return Err(StorefrontError::Invalid(format!(
                        "{owner} is missing required prop: {name}"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn validate_field_value(
    field: &EditorField,
    name: &str,
    value: &Value,
) -> Result<(), StorefrontError> {
    if value.is_null() {
        if field.required {
            return Err(StorefrontError::Invalid(format!(
                "required prop {name} must not be null"
            )));
        }
        return Ok(());
    }
    match field.field_type.as_str() {
        "text" | "textarea" => {
            let Some(text) = value.as_str() else {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} must be a string"
                )));
            };
            let limit = if field.field_type == "text" {
                MAX_TEXT_LENGTH
            } else {
                MAX_TEXTAREA_LENGTH
            };
            if text.len() > limit {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} must not exceed {limit} bytes"
                )));
            }
        }
        "number" => {
            let Some(number) = value.as_f64() else {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} must be a number"
                )));
            };
            if field.min.is_some_and(|minimum| number < minimum) {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} is below its minimum"
                )));
            }
            if field.max.is_some_and(|maximum| number > maximum) {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} exceeds its maximum"
                )));
            }
        }
        "boolean" => {
            if !value.is_boolean() {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} must be a boolean"
                )));
            }
        }
        "select" => {
            let Some(selected) = value.as_str() else {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} must be a string"
                )));
            };
            let valid = field
                .options
                .as_ref()
                .is_some_and(|options| options.iter().any(|option| option.value == selected));
            if !valid {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} is not one of the declared options"
                )));
            }
        }
        "object" => {
            if !value.is_object() {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} must be an object"
                )));
            }
            if let Some(fields) = field.fields.as_ref() {
                validate_known_props(value, fields, name, &Value::Null)?;
                validate_props(value, fields, name, &Value::Null)?;
            }
        }
        "array" => {
            let Some(items) = value.as_array() else {
                return Err(StorefrontError::Invalid(format!(
                    "prop {name} must be an array"
                )));
            };
            if let Some(item) = field.items.as_ref() {
                for (index, value) in items.iter().enumerate() {
                    validate_field_value(item, &format!("{name}[{index}]"), value)?;
                }
            }
        }
        "data_source" | "asset" => {
            let source = field
                .source
                .as_deref()
                .unwrap_or(if field.field_type == "asset" {
                    "asset"
                } else {
                    ""
                });
            validate_binding_value(source, name, value)?;
        }
        "custom" => {}
        other => {
            return Err(StorefrontError::Invalid(format!(
                "unsupported field type: {other}"
            )));
        }
    }
    Ok(())
}

fn validate_binding_value(source: &str, name: &str, value: &Value) -> Result<(), StorefrontError> {
    let Some(object) = value.as_object() else {
        return Err(StorefrontError::Invalid(format!(
            "binding prop {name} must be an object"
        )));
    };
    match source {
        "collection" | "product" | "page" | "navigation" | "asset" => {
            for key in object.keys() {
                if !["id", "handle", "label"].contains(&key.as_str()) {
                    return Err(StorefrontError::Invalid(format!(
                        "binding prop {name} declares unknown field: {key}"
                    )));
                }
            }
            let id = object.get("id").and_then(Value::as_str).unwrap_or("");
            if !valid_name(id) {
                return Err(StorefrontError::Invalid(format!(
                    "binding prop {name} must contain a valid id"
                )));
            }
            for key in ["handle", "label"] {
                if let Some(value) = object.get(key)
                    && !value.is_string()
                {
                    return Err(StorefrontError::Invalid(format!(
                        "binding prop {name}.{key} must be a string"
                    )));
                }
            }
        }
        "product_list" => {
            for key in object.keys() {
                if !["collection_id", "tag", "limit"].contains(&key.as_str()) {
                    return Err(StorefrontError::Invalid(format!(
                        "binding prop {name} declares unknown field: {key}"
                    )));
                }
            }
            for key in ["collection_id", "tag"] {
                if let Some(value) = object.get(key)
                    && !value.is_string()
                {
                    return Err(StorefrontError::Invalid(format!(
                        "binding prop {name}.{key} must be a string"
                    )));
                }
            }
            validate_binding_limit(object.get("limit"))?;
        }
        "search" => {
            for key in object.keys() {
                if !["query", "limit"].contains(&key.as_str()) {
                    return Err(StorefrontError::Invalid(format!(
                        "binding prop {name} declares unknown field: {key}"
                    )));
                }
            }
            let query = object.get("query").and_then(Value::as_str).unwrap_or("");
            if query.trim().is_empty() || query.len() > 200 {
                return Err(StorefrontError::Invalid(format!(
                    "binding prop {name} must contain a search query of 1-200 characters"
                )));
            }
            validate_binding_limit(object.get("limit"))?;
        }
        other => {
            return Err(StorefrontError::Invalid(format!(
                "unsupported data source: {other}"
            )));
        }
    }
    Ok(())
}

fn validate_binding_limit(limit: Option<&Value>) -> Result<(), StorefrontError> {
    let Some(limit) = limit else {
        return Ok(());
    };
    let limit = positive_integer(limit).ok_or_else(|| {
        StorefrontError::Invalid("binding limit must be a positive integer".to_owned())
    })?;
    if limit == 0 || limit > u64::from(MAX_BINDING_LIMIT) {
        return Err(StorefrontError::Invalid(format!(
            "binding limit must be between 1 and {MAX_BINDING_LIMIT}"
        )));
    }
    Ok(())
}

/// Reads a non-negative integral JSON number as `u64`.
///
/// Editors may serialize whole numbers as `4.0`; those are accepted only when
/// the fractional part is zero so the integer contract stays unambiguous.
fn positive_integer(value: &Value) -> Option<u64> {
    if let Some(value) = value.as_u64() {
        return Some(value);
    }
    let number = value.as_f64()?;
    if number.is_finite() && number >= 0.0 && number.fract() == 0.0 && number <= u64::MAX as f64 {
        Some(number as u64)
    } else {
        None
    }
}

fn collect_block_bindings(
    blocks: &[PageBlock],
    editor: &ThemeEditorManifest,
    out: &mut Vec<BindingRequest>,
) -> Result<(), StorefrontError> {
    for block in blocks {
        let Some(component) = editor.components.get(&block.block_type) else {
            continue;
        };
        let Some(props) = block.props.as_object() else {
            continue;
        };
        let Some(block_id) = props.get("id").and_then(Value::as_str) else {
            continue;
        };
        for (field_name, field) in &component.fields {
            if field.field_type != "data_source" && field.field_type != "asset" {
                continue;
            }
            let Some(value) = props.get(field_name) else {
                continue;
            };
            if value.is_null() {
                continue;
            }
            let source = match field.source.as_deref() {
                Some(source) => source,
                None if field.field_type == "asset" => "asset",
                None => continue,
            };
            let binding = binding_from_value(source, value, props)?;
            out.push(BindingRequest {
                block_id: block_id.to_owned(),
                field: field_name.clone(),
                binding,
            });
        }
        for children in block.slots.values() {
            collect_block_bindings(children, editor, out)?;
        }
    }
    Ok(())
}

fn binding_from_value(
    source: &str,
    value: &Value,
    props: &Map<String, Value>,
) -> Result<DataBinding, StorefrontError> {
    let object = value
        .as_object()
        .ok_or_else(|| StorefrontError::Invalid("binding value must be an object".to_owned()))?;
    let id = || {
        object
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default()
    };
    let limit = props
        .get("limit")
        .and_then(positive_integer)
        .or_else(|| object.get("limit").and_then(positive_integer))
        .and_then(|value| u32::try_from(value).ok());
    match source {
        "collection" => Ok(DataBinding::Collection { id: id(), limit }),
        "product" => Ok(DataBinding::Product { id: id() }),
        "page" => Ok(DataBinding::Page { id: id() }),
        "navigation" => Ok(DataBinding::Navigation { id: id() }),
        "asset" => Ok(DataBinding::Asset { id: id() }),
        "product_list" => Ok(DataBinding::ProductList {
            collection_id: object
                .get("collection_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            tag: object.get("tag").and_then(Value::as_str).map(str::to_owned),
            sort: object
                .get("sort")
                .and_then(Value::as_str)
                .map(str::to_owned),
            limit,
        }),
        "search" => Ok(DataBinding::Search {
            query: object
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            limit,
        }),
        other => Err(StorefrontError::Invalid(format!(
            "unsupported data source: {other}"
        ))),
    }
}

const fn default_schema_version() -> u32 {
    CURRENT_PAGE_SCHEMA_VERSION
}

const fn default_editor_schema_version() -> u32 {
    SUPPORTED_EDITOR_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{MAX_PAGE_BLOCKS, PageBody, validate_binding_value};
    use crate::editor::ThemeEditorManifest;

    fn editor() -> ThemeEditorManifest {
        ThemeEditorManifest::parse(&json!({
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
                        "title": { "type": "text", "required": true },
                        "subtitle": { "type": "text" }
                    },
                    "defaultProps": { "title": "Welcome" },
                    "slots": [{ "name": "content" }]
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
            }
        }))
        .unwrap()
    }

    fn body() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "content": [
                { "type": "Hero", "props": { "id": "hero-1", "title": "Welcome" } },
                {
                    "type": "ProductGrid",
                    "props": {
                        "id": "grid-1",
                        "collection": { "id": "col_1", "handle": "featured", "label": "Featured" },
                        "limit": 8
                    }
                }
            ],
            "root": { "props": { "title": "Home" } },
            "zones": {}
        })
    }

    #[test]
    fn parses_empty_and_valid_bodies() {
        assert!(PageBody::parse(&json!(null)).unwrap().content.is_empty());
        assert!(PageBody::parse(&json!({})).unwrap().content.is_empty());
        PageBody::parse(&body()).unwrap();
    }

    #[test]
    fn rejects_unknown_blocks_and_props() {
        let manifest = editor();
        let mut value = body();
        value["content"][0]["type"] = json!("Missing");
        let parsed = PageBody::parse(&value).unwrap();
        assert!(parsed.validate_with_editor(&manifest).is_err());

        let mut value = body();
        value["content"][0]["props"]["unknown"] = json!("x");
        let parsed = PageBody::parse(&value).unwrap();
        assert!(parsed.validate_with_editor(&manifest).is_err());

        let mut value = body();
        value["content"][0]["props"]["title"] = json!(42);
        let parsed = PageBody::parse(&value).unwrap();
        assert!(parsed.validate_with_editor(&manifest).is_err());
    }

    #[test]
    fn duplicate_block_ids_are_rejected() {
        let mut value = body();
        value["content"][1]["props"]["id"] = json!("hero-1");
        assert!(PageBody::parse(&value).is_err());
    }

    #[test]
    fn block_count_is_bounded() {
        let mut value = body();
        let content = (0..=MAX_PAGE_BLOCKS)
            .map(|index| {
                json!({
                    "type": "Hero",
                    "props": { "id": format!("hero-{index}"), "title": "x" }
                })
            })
            .collect::<Vec<_>>();
        value["content"] = serde_json::Value::Array(content);
        assert!(PageBody::parse(&value).is_err());
    }

    #[test]
    fn collects_typed_bindings() {
        let manifest = editor();
        let parsed = PageBody::parse(&body()).unwrap();
        let bindings = parsed.collect_bindings(&manifest).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].block_id, "grid-1");
        assert_eq!(bindings[0].field, "collection");
        assert!(matches!(
            bindings[0].binding,
            crate::data::DataBinding::Collection { limit: Some(8), .. }
        ));
    }

    #[test]
    fn product_list_and_search_binding_shapes_are_strict() {
        assert!(
            validate_binding_value(
                "product_list",
                "products",
                &json!({"collection_id": "col_1"})
            )
            .is_ok()
        );
        assert!(
            validate_binding_value("product_list", "products", &json!({"collection_id": 123}))
                .is_err()
        );
        assert!(
            validate_binding_value("product_list", "products", &json!({"id": "col_1"})).is_err()
        );
        assert!(
            validate_binding_value("product_list", "products", &json!({"sort": "price"})).is_err()
        );
        assert!(validate_binding_value("search", "products", &json!({"query": "tee"})).is_ok());
        assert!(
            validate_binding_value(
                "search",
                "products",
                &json!({"query": "tee", "sort": "asc"})
            )
            .is_err()
        );
    }
}
