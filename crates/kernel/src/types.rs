//! Shared value types used across platform and commerce modules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, KernelError};
use crate::ids::{EventId, StoreId};

/// Wall-clock timestamp in Unix seconds.
pub type Timestamp = i64;

/// Returns the current Unix timestamp in seconds.
#[must_use]
pub fn now_timestamp() -> Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

/// A validated two-letter ISO 3166-1 country code.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct CountryCode(String);

impl CountryCode {
    /// Creates a validated uppercase country code.
    pub fn new(code: impl Into<String>) -> Result<Self, KernelError> {
        let code = code.into().trim().to_ascii_uppercase();
        if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(KernelError::Validation(format!(
                "invalid country code: {code}"
            )));
        }
        Ok(Self(code))
    }

    /// Returns the code as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CountryCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CountryCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A locale identifier such as `en-US` or `fr_FR`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Locale(String);

impl Locale {
    /// Creates a validated locale identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, KernelError> {
        let raw = value.into().trim().replace('_', "-");
        let normalized = raw
            .split('-')
            .enumerate()
            .map(|(index, part)| {
                if index == 0 {
                    part.to_ascii_lowercase()
                } else if part.len() == 2 {
                    part.to_ascii_uppercase()
                } else {
                    part.to_ascii_lowercase()
                }
            })
            .collect::<Vec<_>>()
            .join("-");
        let valid = normalized.len() >= 2
            && normalized.len() <= 35
            && normalized
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
            && normalized
                .split('-')
                .all(|part| !part.is_empty() && part.len() <= 8);
        if !valid {
            return Err(KernelError::Validation(format!(
                "invalid locale: {normalized}"
            )));
        }
        Ok(Self(normalized))
    }

    /// Returns the locale as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Locale {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Locale {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Free-form extension metadata attached to catalog and commerce records.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Metadata(BTreeMap<String, Value>);

impl Metadata {
    /// Creates empty metadata.
    #[must_use]
    pub fn empty() -> Self {
        Self(BTreeMap::new())
    }

    /// Inserts a metadata entry.
    pub fn insert(&mut self, key: impl Into<String>, value: Value) -> &mut Self {
        self.0.insert(key.into(), value);
        self
    }

    /// Returns a metadata entry.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Returns true when no entries are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Serializes the metadata for storage.
    #[must_use]
    pub fn to_json_string(&self) -> String {
        serde_json::to_string(&self.0).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Parses metadata from a stored JSON string.
    ///
    /// Invalid or empty input yields empty metadata rather than an error so a
    /// single malformed record cannot break list endpoints.
    #[must_use]
    pub fn from_json_str(value: &str) -> Self {
        serde_json::from_str::<BTreeMap<String, Value>>(value)
            .map(Self)
            .unwrap_or_default()
    }
}

/// Offset/limit pagination input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pagination {
    /// Maximum number of records to return.
    pub limit: i64,
    /// Number of records to skip.
    pub offset: i64,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            limit: 20,
            offset: 0,
        }
    }
}

impl Pagination {
    /// Largest accepted page size.
    pub const MAX_LIMIT: i64 = 100;

    /// Validates and normalizes pagination input.
    pub fn new(limit: Option<i64>, offset: Option<i64>) -> Result<Self, AppError> {
        let limit = limit.unwrap_or(Self::default().limit);
        let offset = offset.unwrap_or(0);
        if !(1..=Self::MAX_LIMIT).contains(&limit) {
            return Err(AppError::Validation(format!(
                "limit must be between 1 and {}",
                Self::MAX_LIMIT
            )));
        }
        if offset < 0 {
            return Err(AppError::Validation(
                "offset must not be negative".to_owned(),
            ));
        }
        Ok(Self { limit, offset })
    }
}

/// A page of results with pagination metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageResult<T> {
    /// Records in this page.
    pub data: Vec<T>,
    /// Total number of matching records.
    pub count: i64,
    /// Effective pagination.
    pub pagination: Pagination,
}

impl<T> PageResult<T> {
    /// Creates a page result.
    #[must_use]
    pub fn new(data: Vec<T>, count: i64, pagination: Pagination) -> Self {
        Self {
            data,
            count,
            pagination,
        }
    }

    /// Maps the records while preserving page metadata.
    pub fn map<U>(self, mut mapper: impl FnMut(T) -> U) -> PageResult<U> {
        PageResult {
            data: self.data.into_iter().map(&mut mapper).collect(),
            count: self.count,
            pagination: self.pagination,
        }
    }
}

/// A stable, versioned domain event envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainEventEnvelope {
    /// Unique event identifier.
    pub id: EventId,
    /// Stable event type, for example `product.created`.
    pub event_type: String,
    /// Event schema version.
    pub version: u32,
    /// Store that produced the event, if any.
    pub store_id: Option<StoreId>,
    /// Timestamp in Unix seconds.
    pub occurred_at: Timestamp,
    /// Actor that caused the event.
    pub actor_id: Option<String>,
    /// Event payload.
    pub data: Value,
    /// Optional key used by consumers to deduplicate delivery.
    pub idempotency_key: Option<String>,
}

impl DomainEventEnvelope {
    /// Creates a version-1 event for a store.
    #[must_use]
    pub fn new(event_type: impl Into<String>, store_id: Option<StoreId>, data: Value) -> Self {
        Self {
            id: EventId::new(),
            event_type: event_type.into(),
            version: 1,
            store_id,
            occurred_at: now_timestamp(),
            actor_id: None,
            data,
            idempotency_key: None,
        }
    }

    /// Sets the actor that caused the event.
    #[must_use]
    pub fn with_actor(mut self, actor_id: Option<String>) -> Self {
        self.actor_id = actor_id;
        self
    }

    /// Sets the deduplication key.
    #[must_use]
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{CountryCode, Locale, Metadata, PageResult, Pagination};
    use crate::error::AppError;

    #[test]
    fn country_codes_are_validated() {
        assert_eq!(CountryCode::new("us").unwrap().as_str(), "US");
        assert!(CountryCode::new("USA").is_err());
    }

    #[test]
    fn locales_are_normalized() {
        assert_eq!(Locale::new("en_us").unwrap().as_str(), "en-US");
        assert!(Locale::new("-").is_err());
    }

    #[test]
    fn metadata_round_trips_json() {
        let mut metadata = Metadata::empty();
        metadata.insert("foo", serde_json::json!("bar"));
        assert_eq!(
            Metadata::from_json_str(&metadata.to_json_string()),
            metadata
        );
        assert!(Metadata::from_json_str("not-json").is_empty());
    }

    #[test]
    fn pagination_validates_bounds() {
        assert!(Pagination::new(Some(0), None).is_err());
        assert!(Pagination::new(Some(101), None).is_err());
        assert!(Pagination::new(None, Some(-1)).is_err());
        let error = Pagination::new(Some(101), None).unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[test]
    fn page_results_preserve_metadata_when_mapped() {
        let page = PageResult::new(vec![1, 2], 10, Pagination::default());
        let mapped = page.map(|value| value * 10);
        assert_eq!(mapped.data, vec![10, 20]);
        assert_eq!(mapped.count, 10);
    }
}
