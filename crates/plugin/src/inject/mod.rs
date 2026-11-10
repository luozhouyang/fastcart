//! Storefront extension injection.
//!
//! The scanner is byte-oriented. It replaces explicit theme markers first and
//! falls back to `</head>` / `</body>` when a marker is absent. A sentinel makes
//! a second pass a no-op.

mod marker;
mod stream;

pub use marker::{InjectionConfig, InjectionOutcome, inject, is_full_document};
pub use stream::StreamInjector;
