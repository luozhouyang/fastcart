//! Provider-independent transactional email delivery.
//!
//! The crate validates messages, enqueues `send_email` jobs on the persistent
//! queue and dispatches through a replaceable [`EmailProvider`]. The default
//! registry uses a logging provider so local development never performs
//! external I/O.

mod message;
mod provider;
mod service;

pub use message::{
    EmailMessage, EmailReceipt, MAX_BODY_BYTES, MAX_SUBJECT_CHARS, SEND_EMAIL_JOB_TYPE,
    SendEmailJob,
};
pub use provider::{
    EmailError, EmailProvider, EmailProviderRegistry, LogEmailProvider, MemoryEmailProvider,
};
pub use service::EmailService;
