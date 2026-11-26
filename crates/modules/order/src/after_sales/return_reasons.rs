use super::*;

/// Identifier of a configured return reason.
///
/// Return reasons only exist inside the order module, so the identifier lives
/// here instead of `fastcart_kernel`; it still follows the kernel identifier
/// shape (`"{prefix}_{uuid}"` with a UUID v7 suffix).
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, utoipa::ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct ReturnReasonId(String);

impl ReturnReasonId {
    /// Creates a new identifier with a UUID v7 suffix.
    #[must_use]
    pub fn new() -> Self {
        Self(format!("rreason_{}", uuid::Uuid::now_v7().simple()))
    }

    /// Wraps an existing identifier string without validation.
    #[must_use]
    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ReturnReasonId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ReturnReasonId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<String> for ReturnReasonId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ReturnReasonId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// A configurable reason for creating a return, claim or exchange.
///
/// After-sales records snapshot the [`Self::label`] into their own free-text
/// `reason` column when they are created, so deleting a reason never rewrites
/// or breaks the records that referenced it.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReturnReason {
    /// Reason identifier.
    pub id: ReturnReasonId,
    /// Stable machine-readable value, unique per store.
    pub value: String,
    /// Human-readable label snapshotted onto new after-sales records.
    pub label: String,
    /// Optional description.
    pub description: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating a return reason.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateReturnReasonInput {
    /// Stable machine-readable value, unique per store.
    pub value: String,
    /// Human-readable label snapshotted onto new after-sales records.
    pub label: String,
    /// Optional description.
    pub description: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
}

/// Input for partially updating a return reason.
///
/// Omitted fields keep their stored value. `description` distinguishes an
/// omitted field (unchanged) from an explicit `null` (cleared).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateReturnReasonInput {
    /// Replacement machine-readable value.
    pub value: Option<String>,
    /// Replacement label.
    pub label: Option<String>,
    /// Replacement description; `Some(None)` clears it.
    pub description: Option<Option<String>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}
