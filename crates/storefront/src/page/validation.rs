//! Page body validation.

use super::*;

pub(super) fn validate_blocks(
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
        if id == RESERVED_CONTEXT_KEY {
            return Err(StorefrontError::Invalid(format!(
                "block id {id:?} is reserved for request context data"
            )));
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

pub(super) fn validate_blocks_with_editor(
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

pub(super) fn validate_known_props(
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

pub(super) fn validate_props(
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

pub(crate) fn validate_binding_value(
    source: &str,
    name: &str,
    value: &Value,
) -> Result<(), StorefrontError> {
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
                if !["collection_id", "tag", "sort", "limit"].contains(&key.as_str()) {
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
            if let Some(sort) = object.get("sort") {
                let sort = sort.as_str().unwrap_or("");
                if !PRODUCT_SORT_VALUES.contains(&sort) {
                    return Err(StorefrontError::Invalid(format!(
                        "binding prop {name}.sort must be one of: {}",
                        PRODUCT_SORT_VALUES.join(", ")
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
        "inquiry_form" => {
            for key in object.keys() {
                if !["id", "handle", "label"].contains(&key.as_str()) {
                    return Err(StorefrontError::Invalid(format!(
                        "binding prop {name} declares unknown field: {key}"
                    )));
                }
            }
            let handle = object.get("handle").and_then(Value::as_str).unwrap_or("");
            if !valid_handle(handle) {
                return Err(StorefrontError::Invalid(format!(
                    "binding prop {name} must contain a valid handle"
                )));
            }
            for key in ["id", "label"] {
                if let Some(value) = object.get(key)
                    && !value.is_string()
                {
                    return Err(StorefrontError::Invalid(format!(
                        "binding prop {name}.{key} must be a string"
                    )));
                }
            }
        }
        other => {
            return Err(StorefrontError::Invalid(format!(
                "unsupported data source: {other}"
            )));
        }
    }
    Ok(())
}

/// Returns true for storefront handles accepted by inquiry form bindings.
///
/// Form handles are lowercased slugs stored in the database, so this mirrors
/// the kernel handle contract rather than the looser editor identifier check.
fn valid_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= 120
        && handle
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !handle.starts_with('-')
        && !handle.ends_with('-')
        && !handle.contains("--")
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
pub(super) fn positive_integer(value: &Value) -> Option<u64> {
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
