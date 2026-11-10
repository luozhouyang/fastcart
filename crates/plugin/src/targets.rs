//! Admin extension targets enabled in this host version.

use crate::error::PluginError;

/// Targets available in v1.
pub const ENABLED_TARGETS: &[&str] = &[
    "admin.nav.section",
    "admin.page.overview.after-header",
    "admin.page.orders.details.side",
    "admin.page.products.details.side",
    "admin.page.customers.details.side",
    "admin.page.settings.tabs",
];

/// Known targets that are not enabled yet.
pub const RESERVED_TARGETS: &[&str] = &[
    "admin.global.banner",
    "admin.page.orders.list.after-table",
    "admin.page.orders.details.after-summary",
    "admin.page.orders.details.actions",
    "admin.page.products.list.after-table",
    "admin.page.products.details.tabs",
    "admin.page.customers.list.after-table",
    "admin.page.inventory.after-table",
    "admin.page.promotions.list.after-table",
    "admin.page.settings.after-sections",
    "admin.page.themes.details.side",
    "admin.command-palette.items",
];

/// Validates a declared target.
///
/// # Errors
///
/// Returns [`PluginError::ReservedTarget`] for a known but disabled target and
/// [`PluginError::UnknownTarget`] for an unrecognized name.
pub fn validate_target(target: &str) -> Result<(), PluginError> {
    if ENABLED_TARGETS.contains(&target) {
        return Ok(());
    }
    if RESERVED_TARGETS.contains(&target) {
        return Err(PluginError::ReservedTarget(target.to_owned()));
    }
    Err(PluginError::UnknownTarget(target.to_owned()))
}

/// Permission required to render one target.
#[must_use]
pub fn required_permission(target: &str) -> Option<&'static str> {
    match target {
        "admin.nav.section" | "admin.page.overview.after-header" => Some("plugins.read"),
        "admin.page.orders.details.side" => Some("orders.read"),
        "admin.page.products.details.side" => Some("products.read"),
        "admin.page.customers.details.side" => Some("customers.read"),
        "admin.page.settings.tabs" => Some("settings.read"),
        _ => None,
    }
}

/// Returns true when the target renders a full-page panel rather than a nav entry.
#[must_use]
pub fn is_page_target(target: &str) -> bool {
    target.starts_with("admin.page.")
}
