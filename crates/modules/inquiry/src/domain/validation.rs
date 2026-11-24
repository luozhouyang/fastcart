//! Submission and form-field validation.

use super::*;

/// Validates a field list, including uniqueness and contact coverage.
pub fn validate_fields(fields: &[InquiryFormField]) -> Result<(), AppError> {
    if fields.is_empty() {
        return Err(AppError::validation(
            "a form must declare at least one field",
        ));
    }
    if fields.len() > MAX_FORM_FIELDS {
        return Err(AppError::validation(format!(
            "a form must not exceed {MAX_FORM_FIELDS} fields"
        )));
    }
    let mut keys = HashSet::new();
    let mut has_contact = false;
    for field in fields {
        field.validate()?;
        if !keys.insert(field.key.as_str()) {
            return Err(AppError::validation(format!(
                "duplicate field key: {}",
                field.key
            )));
        }
        if field.is_contact_field() {
            has_contact = true;
        }
    }
    if !has_contact {
        return Err(AppError::validation(
            "a form must include at least one of email, phone or message",
        ));
    }
    Ok(())
}

/// Validates and normalizes a submission against a published revision.
#[allow(clippy::too_many_arguments)]
pub fn validate_submission(
    revision: &InquiryFormRevision,
    name: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    subject: Option<&str>,
    message: Option<&str>,
    data: &Value,
    consent: bool,
) -> Result<NormalizedSubmission, AppError> {
    revision.validate()?;

    for (key, provided) in [
        ("name", name.is_some()),
        ("email", email.is_some()),
        ("phone", phone.is_some()),
        ("subject", subject.is_some()),
        ("message", message.is_some()),
    ] {
        if provided && !revision.fields.iter().any(|field| field.key == key) {
            return Err(AppError::validation(format!(
                "field {key} is not part of this form"
            )));
        }
    }

    let data_object = match data {
        Value::Null => Map::new(),
        Value::Object(object) => object.clone(),
        _ => {
            return Err(AppError::validation(
                "data must be a JSON object keyed by field key",
            ));
        }
    };
    if serde_json::to_string(&Value::Object(data_object.clone()))
        .map(|serialized| serialized.len())
        .unwrap_or(0)
        > MAX_DATA_BYTES
    {
        return Err(AppError::validation(format!(
            "submitted data must not exceed {MAX_DATA_BYTES} bytes"
        )));
    }

    let field_keys: HashSet<&str> = revision
        .fields
        .iter()
        .map(|field| field.key.as_str())
        .collect();
    for key in data_object.keys() {
        if !field_keys.contains(key.as_str()) {
            return Err(AppError::validation(format!("unknown field: {key}")));
        }
        if RESERVED_FIELD_KEYS.contains(&key.as_str()) {
            return Err(AppError::validation(format!(
                "reserved field {key} must be supplied as a core attribute"
            )));
        }
    }

    let mut normalized = NormalizedSubmission {
        name: normalize_optional(name, MAX_NAME_LENGTH, "name")?,
        email: normalize_email_value(email)?,
        phone: normalize_optional(phone, 40, "phone")?,
        subject: normalize_optional(subject, MAX_SUBJECT_LENGTH, "subject")?,
        message: normalize_optional(message, MAX_MESSAGE_LENGTH, "message")?,
        data: Value::Object(Map::new()),
    };
    let mut custom = Map::new();
    let mut consent_required = revision.settings.consent_required;

    for field in &revision.fields {
        let (value, is_core) = match field.key.as_str() {
            "name" => (
                normalized
                    .name
                    .as_ref()
                    .map(|value| Value::String(value.clone())),
                true,
            ),
            "email" => (
                normalized
                    .email
                    .as_ref()
                    .map(|value| Value::String(value.clone())),
                true,
            ),
            "phone" => (
                normalized
                    .phone
                    .as_ref()
                    .map(|value| Value::String(value.clone())),
                true,
            ),
            "subject" => (
                normalized
                    .subject
                    .as_ref()
                    .map(|value| Value::String(value.clone())),
                true,
            ),
            "message" => (
                normalized
                    .message
                    .as_ref()
                    .map(|value| Value::String(value.clone())),
                true,
            ),
            key => (data_object.get(key).cloned(), false),
        };

        if field.kind == FieldKind::File {
            continue;
        }
        if field.kind == FieldKind::Consent {
            consent_required = field.required || consent_required;
            continue;
        }

        let empty = value.as_ref().is_none_or(is_empty_value);
        if empty {
            if field.required {
                return Err(AppError::validation(format!(
                    "field {} is required",
                    field.key
                )));
            }
            continue;
        }
        let value = value.expect("non-empty values are present");
        validate_field_value(field, &value)?;
        if !is_core {
            custom.insert(field.key.clone(), value);
        }
    }

    if consent_required && !consent {
        return Err(AppError::validation("consent is required"));
    }
    if normalized.email.is_none() && normalized.phone.is_none() && normalized.message.is_none() {
        return Err(AppError::validation(
            "at least one of email, phone or message is required",
        ));
    }

    normalized.data = Value::Object(custom);
    Ok(normalized)
}

fn validate_field_value(field: &InquiryFormField, value: &Value) -> Result<(), AppError> {
    match field.kind {
        FieldKind::Text | FieldKind::Textarea | FieldKind::Hidden => {
            let Some(text) = value.as_str() else {
                return Err(type_error(field, "a string"));
            };
            validate_text_bounds(field, text)?;
        }
        FieldKind::Email => {
            let Some(text) = value.as_str() else {
                return Err(type_error(field, "a string"));
            };
            validate_email(text, &field.key)?;
        }
        FieldKind::Phone => {
            let Some(text) = value.as_str() else {
                return Err(type_error(field, "a string"));
            };
            let trimmed = text.trim();
            if trimmed.len() < 3
                || trimmed.len() > 40
                || trimmed.chars().any(|character| character.is_control())
            {
                return Err(type_error(field, "a valid phone number"));
            }
        }
        FieldKind::Number => {
            let number = value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
                .ok_or_else(|| type_error(field, "a number"))?;
            if let Some(min) = field.validation.min
                && number < min
            {
                return Err(AppError::validation(format!(
                    "field {} must be at least {min}",
                    field.key
                )));
            }
            if let Some(max) = field.validation.max
                && number > max
            {
                return Err(AppError::validation(format!(
                    "field {} must not exceed {max}",
                    field.key
                )));
            }
        }
        FieldKind::Select | FieldKind::Radio => {
            let Some(text) = value.as_str() else {
                return Err(type_error(field, "a string option value"));
            };
            if !field.options.iter().any(|option| option.value == text) {
                return Err(AppError::validation(format!(
                    "field {} has an unknown option value",
                    field.key
                )));
            }
        }
        FieldKind::Checkbox => {
            let Some(values) = value.as_array() else {
                return Err(type_error(field, "an array of option values"));
            };
            for value in values {
                let Some(text) = value.as_str() else {
                    return Err(type_error(field, "an array of option values"));
                };
                if !field.options.iter().any(|option| option.value == text) {
                    return Err(AppError::validation(format!(
                        "field {} has an unknown option value",
                        field.key
                    )));
                }
            }
        }
        FieldKind::Date => {
            let Some(text) = value.as_str() else {
                return Err(type_error(field, "a YYYY-MM-DD date"));
            };
            if !valid_date(text) {
                return Err(type_error(field, "a YYYY-MM-DD date"));
            }
        }
        FieldKind::Consent | FieldKind::File => {}
    }
    Ok(())
}

fn validate_text_bounds(field: &InquiryFormField, text: &str) -> Result<(), AppError> {
    let length = text.chars().count();
    if let Some(min) = field.validation.min_length
        && length < usize::try_from(min).unwrap_or(usize::MAX)
    {
        return Err(AppError::validation(format!(
            "field {} must be at least {min} characters",
            field.key
        )));
    }
    if let Some(max) = field.validation.max_length
        && length > usize::try_from(max).unwrap_or(usize::MAX)
    {
        return Err(AppError::validation(format!(
            "field {} must not exceed {max} characters",
            field.key
        )));
    }
    Ok(())
}

pub(super) fn validate_file_config(
    file: &FileFieldConfig,
    field_key: &str,
) -> Result<(), AppError> {
    if file.max_files == 0 || file.max_files > u32::try_from(MAX_FILES_PER_INQUIRY).unwrap_or(10) {
        return Err(AppError::validation(format!(
            "file field {field_key} max_files must be between 1 and {MAX_FILES_PER_INQUIRY}"
        )));
    }
    if file.max_bytes == 0 || file.max_bytes > MAX_FILE_BYTES {
        return Err(AppError::validation(format!(
            "file field {field_key} max_bytes must be between 1 and {MAX_FILE_BYTES}"
        )));
    }
    if file.accept.is_empty() {
        return Err(AppError::validation(format!(
            "file field {field_key} must accept at least one content type"
        )));
    }
    for content_type in &file.accept {
        if !ALLOWED_FILE_TYPES.contains(&content_type.as_str()) {
            return Err(AppError::validation(format!(
                "file field {field_key} declares unsupported content type: {content_type}"
            )));
        }
    }
    Ok(())
}

fn normalize_optional(
    value: Option<&str>,
    max: usize,
    name: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    validate_length(value, name, max)?;
    Ok(Some(value.to_owned()))
}

fn normalize_email_value(value: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let normalized = normalize_email(value);
    validate_email(&normalized, "email")?;
    Ok(Some(normalized))
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(object) => object.is_empty(),
        _ => false,
    }
}

fn type_error(field: &InquiryFormField, expected: &str) -> AppError {
    AppError::validation(format!(
        "field {} must be {expected}, got an incompatible value",
        field.key
    ))
}

pub(super) fn valid_field_key(key: &str) -> bool {
    let mut characters = key.chars();
    matches!(characters.next(), Some(character) if character.is_ascii_lowercase())
        && key.len() <= 64
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

pub(super) fn validate_redirect_url(url: &str) -> Result<(), AppError> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_PAGE_URL_LENGTH {
        return Err(AppError::validation(
            "redirect_url must be a non-empty URL up to 2048 bytes",
        ));
    }
    if url
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(AppError::validation(
            "redirect_url must not contain whitespace or control characters",
        ));
    }
    if !(url.starts_with("https://") || url.starts_with("http://") || url.starts_with('/')) {
        return Err(AppError::validation(
            "redirect_url must be an absolute http(s) URL or a root-relative path",
        ));
    }
    Ok(())
}
