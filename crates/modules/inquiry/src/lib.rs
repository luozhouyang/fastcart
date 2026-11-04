//! The `inquiry` commerce module.
//!
//! Owns store-scoped inquiry forms, their immutable published revisions,
//! submissions and attachment upload metadata. Cross-module orchestration
//! belongs in `fastcart-workflows`; HTTP and object storage live in the API
//! layer.

mod domain;
mod service;

#[cfg(test)]
mod tests;

pub use domain::{
    ALLOWED_FILE_TYPES, ATTACHMENT_TTL_SECONDS, CreateInquiryFormInput, CreateInquiryInput,
    FieldKind, FieldOption, FieldValidation, FileFieldConfig, Inquiry, InquiryFile,
    InquiryFileStatus, InquiryForm, InquiryFormField, InquiryFormRevision, InquiryFormSettings,
    InquiryFormStatus, InquiryListFilter, InquiryStatus, MAX_DATA_BYTES, MAX_FIELD_OPTIONS,
    MAX_FILE_BYTES, MAX_FILES_PER_INQUIRY, MAX_FORM_FIELDS, MAX_MESSAGE_LENGTH, MAX_NAME_LENGTH,
    MAX_SUBJECT_LENGTH, NormalizedSubmission, PublishedInquiryForm, RESERVED_FIELD_KEYS,
    SaveInquiryDraftInput, UpdateInquiryFormInput, UpdateInquiryStatusInput, default_handle,
    validate_fields, validate_submission,
};
pub use service::{InquiryFileService, InquiryFormService, InquiryService};
