//! Inquiry form, revision, submission and attachment domain types.
//!
//! Forms are stored as immutable revisions. Publications point at a revision
//! and submissions keep the revision identifier they were accepted against, so
//! historical data always renders with the field definitions that were live at
//! submission time.

use std::collections::{BTreeMap, HashSet};

use fastcart_kernel::{
    AppError, InquiryFileId, InquiryFormId, InquiryId, InquiryRevisionId, Metadata, Timestamp,
    normalize_email, slugify, validate_email, validate_length,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Maximum number of fields on one form.
pub const MAX_FORM_FIELDS: usize = 100;
/// Maximum number of options on one choice field.
pub const MAX_FIELD_OPTIONS: usize = 100;
/// Maximum number of attachments on one submission.
pub const MAX_FILES_PER_INQUIRY: usize = 10;
/// Hard cap for one uploaded file.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Maximum serialized size of one submission's custom data.
pub const MAX_DATA_BYTES: usize = 64 * 1024;
/// Maximum message length in characters.
pub const MAX_MESSAGE_LENGTH: usize = 20_000;
/// Maximum name length in characters.
pub const MAX_NAME_LENGTH: usize = 200;
/// Maximum subject length in characters.
pub const MAX_SUBJECT_LENGTH: usize = 255;
/// Maximum page URL length in bytes.
pub const MAX_PAGE_URL_LENGTH: usize = 2_048;
/// Maximum source identifier length.
pub const MAX_SOURCE_LENGTH: usize = 64;
/// Maximum locale length.
pub const MAX_LOCALE_LENGTH: usize = 35;
/// Maximum user-agent length.
pub const MAX_USER_AGENT_LENGTH: usize = 512;
/// Maximum idempotency key length.
pub const MAX_IDEMPOTENCY_KEY_LENGTH: usize = 200;
/// Default lifetime of a pending attachment upload.
pub const ATTACHMENT_TTL_SECONDS: i64 = 3_600;
/// Content types accepted for inquiry attachments.
pub const ALLOWED_FILE_TYPES: [&str; 4] =
    ["image/jpeg", "image/png", "image/webp", "application/pdf"];

/// Field keys that map to dedicated submission columns.
pub const RESERVED_FIELD_KEYS: [&str; 5] = ["name", "email", "phone", "subject", "message"];

/// Publication state of a form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InquiryFormStatus {
    /// Created but never published.
    Draft,
    /// At least one revision is published.
    Published,
    /// Hidden from storefronts.
    Archived,
}

impl InquiryFormStatus {
    /// Returns the storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Archived => "archived",
        }
    }

    /// Parses a stored status.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "published" => Some(Self::Published),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// Supported form field kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// Single-line text.
    Text,
    /// Multi-line text.
    Textarea,
    /// Validated email address.
    Email,
    /// Free-form phone number.
    Phone,
    /// Numeric value.
    Number,
    /// Single choice from a select control.
    Select,
    /// Single choice from radio buttons.
    Radio,
    /// Multiple choices.
    Checkbox,
    /// Calendar date in `YYYY-MM-DD` form.
    Date,
    /// File attachment.
    File,
    /// Hidden value supplied by the theme.
    Hidden,
    /// Explicit consent checkbox.
    Consent,
}

impl FieldKind {
    /// Returns the storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Textarea => "textarea",
            Self::Email => "email",
            Self::Phone => "phone",
            Self::Number => "number",
            Self::Select => "select",
            Self::Radio => "radio",
            Self::Checkbox => "checkbox",
            Self::Date => "date",
            Self::File => "file",
            Self::Hidden => "hidden",
            Self::Consent => "consent",
        }
    }

    /// Parses a stored field kind.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "textarea" => Some(Self::Textarea),
            "email" => Some(Self::Email),
            "phone" => Some(Self::Phone),
            "number" => Some(Self::Number),
            "select" => Some(Self::Select),
            "radio" => Some(Self::Radio),
            "checkbox" => Some(Self::Checkbox),
            "date" => Some(Self::Date),
            "file" => Some(Self::File),
            "hidden" => Some(Self::Hidden),
            "consent" => Some(Self::Consent),
            _ => None,
        }
    }
}

/// One selectable option.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldOption {
    /// Submitted value.
    pub value: String,
    /// Display label.
    pub label: String,
}

/// Text and number validation bounds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldValidation {
    /// Minimum text length in characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    /// Maximum text length in characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// Minimum numeric value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Maximum numeric value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

/// Attachment constraints declared by a `file` field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FileFieldConfig {
    /// Maximum number of files.
    pub max_files: u32,
    /// Maximum size of one file in bytes.
    pub max_bytes: u64,
    /// Accepted content types.
    pub accept: Vec<String>,
}

/// One field in a form revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InquiryFormField {
    /// Stable field key.
    pub key: String,
    /// Field kind.
    pub kind: FieldKind,
    /// Display label.
    pub label: String,
    /// Whether a value is required.
    #[serde(default)]
    pub required: bool,
    /// Optional placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Optional help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Optional default value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Choices for select, radio and checkbox fields.
    #[serde(default)]
    pub options: Vec<FieldOption>,
    /// Text and number bounds.
    #[serde(default)]
    pub validation: FieldValidation,
    /// Attachment constraints for `file` fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileFieldConfig>,
}

impl InquiryFormField {
    /// Validates the field definition in isolation.
    pub fn validate(&self) -> Result<(), AppError> {
        if !valid_field_key(&self.key) {
            return Err(AppError::validation(format!(
                "invalid field key: {}",
                self.key
            )));
        }
        if self.label.trim().is_empty() || self.label.chars().count() > 120 {
            return Err(AppError::validation(format!(
                "field {} label must be between 1 and 120 characters",
                self.key
            )));
        }
        for text in [self.placeholder.as_deref(), self.help.as_deref()]
            .into_iter()
            .flatten()
        {
            if text.chars().count() > 500 {
                return Err(AppError::validation(format!(
                    "field {} text must not exceed 500 characters",
                    self.key
                )));
            }
        }

        let choice = matches!(
            self.kind,
            FieldKind::Select | FieldKind::Radio | FieldKind::Checkbox
        );
        if choice {
            if self.options.is_empty() {
                return Err(AppError::validation(format!(
                    "field {} must declare at least one option",
                    self.key
                )));
            }
            if self.options.len() > MAX_FIELD_OPTIONS {
                return Err(AppError::validation(format!(
                    "field {} must not exceed {MAX_FIELD_OPTIONS} options",
                    self.key
                )));
            }
            let mut values = HashSet::new();
            for option in &self.options {
                let value = option.value.trim();
                if value.is_empty() || value.chars().count() > 120 {
                    return Err(AppError::validation(format!(
                        "field {} option values must be between 1 and 120 characters",
                        self.key
                    )));
                }
                if option.label.trim().is_empty() || option.label.chars().count() > 200 {
                    return Err(AppError::validation(format!(
                        "field {} option labels must be between 1 and 200 characters",
                        self.key
                    )));
                }
                if !values.insert(value) {
                    return Err(AppError::validation(format!(
                        "field {} declares duplicate option value: {value}",
                        self.key
                    )));
                }
            }
        } else if !self.options.is_empty() {
            return Err(AppError::validation(format!(
                "field {} may not declare options",
                self.key
            )));
        }

        if self.kind == FieldKind::File {
            let Some(file) = self.file.as_ref() else {
                return Err(AppError::validation(format!(
                    "file field {} must declare file constraints",
                    self.key
                )));
            };
            validate_file_config(file, &self.key)?;
        } else if self.file.is_some() {
            return Err(AppError::validation(format!(
                "field {} may not declare file constraints",
                self.key
            )));
        }

        if let Some(min) = self.validation.min_length
            && let Some(max) = self.validation.max_length
            && min > max
        {
            return Err(AppError::validation(format!(
                "field {} min_length must not exceed max_length",
                self.key
            )));
        }
        if let Some(min) = self.validation.min
            && let Some(max) = self.validation.max
            && min > max
        {
            return Err(AppError::validation(format!(
                "field {} min must not exceed max",
                self.key
            )));
        }

        match self.key.as_str() {
            "email" if self.kind != FieldKind::Email => {
                return Err(AppError::validation(
                    "the email field must use the email kind".to_owned(),
                ));
            }
            "phone" if self.kind != FieldKind::Phone => {
                return Err(AppError::validation(
                    "the phone field must use the phone kind".to_owned(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    /// Returns true when the field contributes a contact channel.
    #[must_use]
    pub fn is_contact_field(&self) -> bool {
        matches!(self.key.as_str(), "email" | "phone" | "message")
    }
}

/// Form-level presentation and notification settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InquiryFormSettings {
    /// Submit button label.
    #[serde(default = "default_submit_label")]
    pub submit_label: String,
    /// Message shown after a successful submission.
    #[serde(default = "default_success_message")]
    pub success_message: String,
    /// Optional post-submit redirect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<String>,
    /// Optional consent copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent_text: Option<String>,
    /// Whether consent is mandatory.
    #[serde(default)]
    pub consent_required: bool,
    /// Whether the form accepts attachments.
    #[serde(default = "default_true")]
    pub allow_attachments: bool,
    /// Custom field keys included in notification cards.
    #[serde(default)]
    pub notification_fields: Vec<String>,
}

impl Default for InquiryFormSettings {
    fn default() -> Self {
        Self {
            submit_label: default_submit_label(),
            success_message: default_success_message(),
            redirect_url: None,
            consent_text: None,
            consent_required: false,
            allow_attachments: true,
            notification_fields: Vec::new(),
        }
    }
}

impl InquiryFormSettings {
    /// Validates settings against the field list they apply to.
    pub fn validate(&self, fields: &[InquiryFormField]) -> Result<(), AppError> {
        if self.submit_label.trim().is_empty() || self.submit_label.chars().count() > 120 {
            return Err(AppError::validation(
                "submit_label must be between 1 and 120 characters",
            ));
        }
        if self.success_message.trim().is_empty() || self.success_message.chars().count() > 1_000 {
            return Err(AppError::validation(
                "success_message must be between 1 and 1000 characters",
            ));
        }
        if let Some(redirect) = self.redirect_url.as_deref() {
            validate_redirect_url(redirect)?;
        }
        if let Some(text) = self.consent_text.as_deref()
            && text.chars().count() > 2_000
        {
            return Err(AppError::validation(
                "consent_text must not exceed 2000 characters",
            ));
        }
        if self.consent_required
            && self
                .consent_text
                .as_deref()
                .is_none_or(|text| text.trim().is_empty())
        {
            return Err(AppError::validation(
                "consent_text is required when consent_required is enabled",
            ));
        }
        if self.notification_fields.len() > 6 {
            return Err(AppError::validation(
                "notification_fields must not exceed 6 entries",
            ));
        }
        for key in &self.notification_fields {
            let Some(field) = fields.iter().find(|field| field.key == *key) else {
                return Err(AppError::validation(format!(
                    "notification_fields references unknown field: {key}"
                )));
            };
            if field.kind == FieldKind::File {
                return Err(AppError::validation(format!(
                    "notification_fields may not reference file field: {key}"
                )));
            }
        }
        Ok(())
    }
}

/// A form identity with its publication state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryForm {
    /// Form identifier.
    pub id: InquiryFormId,
    /// Display name.
    pub name: String,
    /// Stable storefront handle.
    pub handle: String,
    /// Optional description.
    pub description: Option<String>,
    /// Publication state.
    pub status: InquiryFormStatus,
    /// Highest revision version, including the current draft.
    pub current_version: i64,
    /// Currently published revision version.
    pub published_version: Option<i64>,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Last update timestamp.
    pub updated_at: Timestamp,
}

/// One immutable-or-draft form revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryFormRevision {
    /// Revision identifier.
    pub id: InquiryRevisionId,
    /// Owning form.
    pub form_id: InquiryFormId,
    /// Monotonic version within the form.
    pub version: i64,
    /// Field definitions in display order.
    pub fields: Vec<InquiryFormField>,
    /// Form settings.
    pub settings: InquiryFormSettings,
    /// Platform user that saved the revision.
    pub created_by: Option<String>,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Publication timestamp; `None` means this is the mutably edited draft.
    pub published_at: Option<Timestamp>,
}

impl InquiryFormRevision {
    /// Validates the field list and settings together.
    pub fn validate(&self) -> Result<(), AppError> {
        validate_fields(&self.fields)?;
        self.settings.validate(&self.fields)
    }
}

/// A published form and the revision storefronts should render.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PublishedInquiryForm {
    /// Form identity.
    pub form: InquiryForm,
    /// Published revision.
    pub revision: InquiryFormRevision,
}

/// Submission lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InquiryStatus {
    /// Unread.
    New,
    /// Read by an administrator.
    Read,
    /// Replied outside the platform.
    Replied,
    /// No further action needed.
    Closed,
    /// Rejected by spam controls.
    Spam,
}

impl InquiryStatus {
    /// Returns the storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Read => "read",
            Self::Replied => "replied",
            Self::Closed => "closed",
            Self::Spam => "spam",
        }
    }

    /// Parses a stored status.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "new" => Some(Self::New),
            "read" => Some(Self::Read),
            "replied" => Some(Self::Replied),
            "closed" => Some(Self::Closed),
            "spam" => Some(Self::Spam),
            _ => None,
        }
    }
}

/// One accepted inquiry submission.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Inquiry {
    /// Submission identifier.
    pub id: InquiryId,
    /// Outbox event identifier.
    pub event_id: String,
    /// Form that accepted the submission.
    pub form_id: InquiryFormId,
    /// Revision the submission was validated against.
    pub form_revision_id: InquiryRevisionId,
    /// Revision version.
    pub form_version: i64,
    /// Lifecycle status.
    pub status: InquiryStatus,
    /// Contact name.
    pub name: Option<String>,
    /// Contact email.
    pub email: Option<String>,
    /// Contact phone.
    pub phone: Option<String>,
    /// Subject.
    pub subject: Option<String>,
    /// Free-form message.
    pub message: Option<String>,
    /// Custom field values keyed by field key.
    pub data: Value,
    /// Source identifier.
    pub source: String,
    /// Page the form was submitted from.
    pub page_url: Option<String>,
    /// Locale.
    pub locale: Option<String>,
    /// Idempotency key supplied by the client.
    pub idempotency_key: Option<String>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Last update timestamp.
    pub updated_at: Timestamp,
}

/// Attachment upload lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InquiryFileStatus {
    /// Uploaded, not yet attached to a submission.
    Pending,
    /// Attached to a submission.
    Claimed,
    /// Expired before submission.
    Expired,
}

impl InquiryFileStatus {
    /// Returns the storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Expired => "expired",
        }
    }

    /// Parses a stored status.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "claimed" => Some(Self::Claimed),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

/// Metadata for one private attachment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryFile {
    /// File identifier.
    pub id: InquiryFileId,
    /// Form the upload belongs to.
    pub form_id: InquiryFormId,
    /// Submission that claimed the upload.
    pub inquiry_id: Option<InquiryId>,
    /// Field key that owns the file.
    pub field_key: String,
    /// Private storage key.
    #[serde(skip_serializing)]
    pub storage_key: String,
    /// Original client file name, display only.
    pub original_name: Option<String>,
    /// Sniffed content type.
    pub content_type: String,
    /// Size in bytes.
    pub size_bytes: i64,
    /// SHA-256 checksum.
    pub checksum_sha256: String,
    /// Upload status.
    pub status: InquiryFileStatus,
    /// Pending upload expiry.
    pub expires_at: Option<Timestamp>,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Claim timestamp.
    pub claimed_at: Option<Timestamp>,
}

/// Input for creating a form identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateInquiryFormInput {
    /// Display name.
    pub name: String,
    /// Optional handle; defaults to a slug of the name.
    pub handle: Option<String>,
    /// Optional description.
    pub description: Option<String>,
}

/// Input for updating form identity metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateInquiryFormInput {
    /// Replacement name.
    pub name: Option<String>,
    /// Replacement description.
    pub description: Option<String>,
}

/// Input for saving the current draft revision.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveInquiryDraftInput {
    /// Field definitions.
    pub fields: Vec<InquiryFormField>,
    /// Form settings.
    pub settings: InquiryFormSettings,
    /// Platform user saving the draft.
    pub created_by: Option<String>,
}

/// Input for accepting a submission.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateInquiryInput {
    /// Published form handle.
    pub form_handle: String,
    /// Contact name.
    pub name: Option<String>,
    /// Contact email.
    pub email: Option<String>,
    /// Contact phone.
    pub phone: Option<String>,
    /// Subject.
    pub subject: Option<String>,
    /// Message.
    pub message: Option<String>,
    /// Custom field values.
    pub data: Value,
    /// Attachment identifiers keyed by field key.
    pub files: BTreeMap<String, Vec<InquiryFileId>>,
    /// Source identifier.
    pub source: Option<String>,
    /// Page URL.
    pub page_url: Option<String>,
    /// Locale.
    pub locale: Option<String>,
    /// User agent.
    pub user_agent: Option<String>,
    /// Hashed client address.
    pub ip_hash: Option<String>,
    /// Client idempotency key.
    pub idempotency_key: Option<String>,
    /// Consent flag.
    pub consent: bool,
}

/// Filters accepted by the admin inbox.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InquiryListFilter {
    /// Status filter.
    pub status: Option<InquiryStatus>,
    /// Form filter.
    pub form_id: Option<InquiryFormId>,
    /// Case-insensitive text search across name, email, phone and message.
    pub query: Option<String>,
    /// Inclusive lower creation bound.
    pub from: Option<Timestamp>,
    /// Inclusive upper creation bound.
    pub to: Option<Timestamp>,
}

/// Input for changing a submission's status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInquiryStatusInput {
    /// Replacement status.
    pub status: InquiryStatus,
}

/// Validated and normalized submission values.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedSubmission {
    /// Trimmed name.
    pub name: Option<String>,
    /// Lowercase email.
    pub email: Option<String>,
    /// Trimmed phone.
    pub phone: Option<String>,
    /// Trimmed subject.
    pub subject: Option<String>,
    /// Trimmed message.
    pub message: Option<String>,
    /// Custom values keyed by field key.
    pub data: Value,
}

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

fn validate_file_config(file: &FileFieldConfig, field_key: &str) -> Result<(), AppError> {
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

fn valid_field_key(key: &str) -> bool {
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

fn validate_redirect_url(url: &str) -> Result<(), AppError> {
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

/// Builds a default handle from a form name.
#[must_use]
pub fn default_handle(name: &str) -> String {
    slugify(name)
}

fn default_submit_label() -> String {
    "Submit".to_owned()
}

fn default_success_message() -> String {
    "Thanks! We will get back to you shortly.".to_owned()
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(key: &str, kind: FieldKind) -> InquiryFormField {
        InquiryFormField {
            key: key.to_owned(),
            kind,
            label: key.to_owned(),
            required: false,
            placeholder: None,
            help: None,
            default: None,
            options: Vec::new(),
            validation: FieldValidation::default(),
            file: None,
        }
    }

    fn revision(fields: Vec<InquiryFormField>) -> InquiryFormRevision {
        InquiryFormRevision {
            id: InquiryRevisionId::from_string("ifrev_1"),
            form_id: InquiryFormId::from_string("iform_1"),
            version: 1,
            fields,
            settings: InquiryFormSettings::default(),
            created_by: None,
            created_at: 0,
            published_at: None,
        }
    }

    #[test]
    fn validates_keys_and_contact_coverage() {
        let mut name = field("name", FieldKind::Text);
        name.required = true;
        let email = field("email", FieldKind::Email);
        assert!(validate_fields(&[name, email]).is_ok());

        assert!(validate_fields(&[field("Name", FieldKind::Text)]).is_err());
        assert!(validate_fields(&[field("company", FieldKind::Text)]).is_err());
    }

    #[test]
    fn rejects_unknown_and_reserved_custom_data() {
        let email = field("email", FieldKind::Email);
        let revision = revision(vec![email]);
        assert!(
            validate_submission(
                &revision,
                None,
                Some("a@example.com"),
                None,
                None,
                None,
                &serde_json::json!({"unknown": "x"}),
                false,
            )
            .is_err()
        );
        assert!(
            validate_submission(
                &revision,
                None,
                Some("a@example.com"),
                None,
                None,
                None,
                &serde_json::json!({"email": "b@example.com"}),
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn normalizes_email_and_custom_values() {
        let mut email = field("email", FieldKind::Email);
        email.required = true;
        let mut company = field("company", FieldKind::Text);
        company.validation.max_length = Some(20);
        let revision = revision(vec![email, company]);

        let normalized = validate_submission(
            &revision,
            None,
            Some(" Buyer@Example.com "),
            None,
            None,
            None,
            &serde_json::json!({"company": " Acme "}),
            false,
        )
        .unwrap();
        assert_eq!(normalized.email.as_deref(), Some("buyer@example.com"));
        assert_eq!(normalized.data, serde_json::json!({"company": " Acme "}));
    }

    #[test]
    fn validates_choices_and_numbers() {
        let mut plan = field("plan", FieldKind::Select);
        plan.options = vec![FieldOption {
            value: "pro".to_owned(),
            label: "Pro".to_owned(),
        }];
        let mut seats = field("seats", FieldKind::Number);
        seats.validation.min = Some(1.0);
        seats.validation.max = Some(10.0);
        let revision = revision(vec![field("email", FieldKind::Email), plan, seats]);

        assert!(
            validate_submission(
                &revision,
                None,
                Some("a@example.com"),
                None,
                None,
                None,
                &serde_json::json!({"plan": "pro", "seats": 5}),
                false,
            )
            .is_ok()
        );
        assert!(
            validate_submission(
                &revision,
                None,
                Some("a@example.com"),
                None,
                None,
                None,
                &serde_json::json!({"plan": "enterprise", "seats": 5}),
                false,
            )
            .is_err()
        );
        assert!(
            validate_submission(
                &revision,
                None,
                Some("a@example.com"),
                None,
                None,
                None,
                &serde_json::json!({"plan": "pro", "seats": 50}),
                false,
            )
            .is_err()
        );
    }
}
