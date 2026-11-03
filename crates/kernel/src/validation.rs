//! Reusable validation primitives shared by application services.

use crate::error::AppError;
use crate::types::Metadata;

/// Maximum accepted length for a handle or slug.
pub const MAX_HANDLE_LENGTH: usize = 120;

/// Maximum accepted metadata payload size in bytes.
pub const MAX_METADATA_BYTES: usize = 16 * 1024;

/// Validates a URL handle or slug.
///
/// Handles contain lowercase ASCII letters, digits and single hyphens, and must
/// start and end with an alphanumeric character.
pub fn validate_handle(value: &str, field: &str) -> Result<(), AppError> {
    let valid = !value.is_empty()
        && value.len() <= MAX_HANDLE_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--");
    if valid {
        Ok(())
    } else {
        Err(AppError::validation(format!(
            "{field} must be lowercase alphanumeric words separated by single hyphens"
        )))
    }
}

/// Converts arbitrary text into a valid handle.
#[must_use]
pub fn slugify(value: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    let mut last_was_separator = true;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('-');
            last_was_separator = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Returns true when the value looks like an email address.
///
/// This is a structural check only; deliverability is verified by the
/// verification workflow, never by the format check.
#[must_use]
pub fn is_valid_email(value: &str) -> bool {
    if value.len() > 320 || value.trim() != value {
        return false;
    }
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    if local.is_empty() || local.len() > 64 || domain.contains('@') {
        return false;
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return false;
    }
    value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(
                character,
                '!' | '#'
                    | '$'
                    | '%'
                    | '&'
                    | '\''
                    | '*'
                    | '+'
                    | '-'
                    | '/'
                    | '='
                    | '?'
                    | '^'
                    | '_'
                    | '`'
                    | '{'
                    | '|'
                    | '}'
                    | '~'
                    | '.'
                    | '@'
            )
    })
}

/// Validates an email address.
pub fn validate_email(value: &str, field: &str) -> Result<(), AppError> {
    if is_valid_email(value) {
        Ok(())
    } else {
        Err(AppError::validation(format!(
            "{field} is not a valid email address"
        )))
    }
}

/// Normalizes an email address for storage and lookup.
#[must_use]
pub fn normalize_email(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Validates a non-negative line item quantity.
pub fn validate_quantity(quantity: i64, allow_zero: bool) -> Result<(), AppError> {
    let minimum = if allow_zero { 0 } else { 1 };
    if quantity < minimum || quantity > 1_000_000 {
        return Err(AppError::validation(format!(
            "quantity must be between {minimum} and 1000000"
        )));
    }
    Ok(())
}

/// Validates that a metadata payload does not exceed the platform limit.
pub fn validate_metadata(metadata: &Metadata) -> Result<(), AppError> {
    if metadata.to_json_string().len() > MAX_METADATA_BYTES {
        return Err(AppError::validation(format!(
            "metadata must not exceed {MAX_METADATA_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Validates that a monetary amount is not negative.
pub fn validate_non_negative_amount(amount: i64, field: &str) -> Result<(), AppError> {
    if amount < 0 {
        return Err(AppError::validation(format!(
            "{field} must not be negative"
        )));
    }
    Ok(())
}

/// Validates a trimmed, non-empty string with a maximum length.
pub fn validate_length(value: &str, field: &str, maximum: usize) -> Result<(), AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation(format!("{field} must not be empty")));
    }
    if trimmed.chars().count() > maximum {
        return Err(AppError::validation(format!(
            "{field} must not exceed {maximum} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_valid_email, normalize_email, slugify, validate_handle, validate_quantity};

    #[test]
    fn handles_reject_invalid_patterns() {
        assert!(validate_handle("summer-sale", "handle").is_ok());
        assert!(validate_handle("Summer-Sale", "handle").is_err());
        assert!(validate_handle("-summer", "handle").is_err());
        assert!(validate_handle("summer--sale", "handle").is_err());
    }

    #[test]
    fn slugify_produces_valid_handles() {
        assert_eq!(slugify("  Summer Sale 2026! "), "summer-sale-2026");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn emails_are_checked_structurally() {
        assert!(is_valid_email("owner@example.com"));
        assert!(!is_valid_email("owner@example"));
        assert!(!is_valid_email("owner example.com"));
        assert!(!is_valid_email(" owner@example.com"));
        assert_eq!(normalize_email(" Owner@Example.COM "), "owner@example.com");
    }

    #[test]
    fn quantities_are_bounded() {
        assert!(validate_quantity(1, false).is_ok());
        assert!(validate_quantity(0, false).is_err());
        assert!(validate_quantity(0, true).is_ok());
        assert!(validate_quantity(1_000_001, true).is_err());
    }
}
