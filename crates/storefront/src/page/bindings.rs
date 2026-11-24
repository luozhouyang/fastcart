//! Data binding extraction.

use super::validation::positive_integer;
use super::*;

pub(super) fn collect_block_bindings(
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
        "inquiry_form" => Ok(DataBinding::InquiryForm {
            handle: object
                .get("handle")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }),
        other => Err(StorefrontError::Invalid(format!(
            "unsupported data source: {other}"
        ))),
    }
}
