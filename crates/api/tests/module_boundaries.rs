//! Guards the module data-ownership boundary.
//!
//! Modules are separate crates but share one SQLite schema. This test scans
//! their SQL for writes and fails when a module writes a table owned by another
//! module, unless the pair is an explicitly reviewed exception. It is a
//! compile-time-adjacent CI guard, not a runtime check.

use std::fs;
use std::path::{Path, PathBuf};

/// Tables that infrastructure or orchestrators may write from anywhere.
const SHARED_TABLES: &[&str] = &[
    "audit_outbox",
    "idempotency_keys",
    "jobs",
    "outbox",
    "workflow_executions",
    "workflow_steps",
];

/// Reviewed cross-module writes that still need a refactor.
///
/// Keep this list small and add the reason; new entries should not be accepted
/// without an owner review.
const ALLOWED: &[(&str, &str, &str)] = &[];

/// Reviewed cross-module reads.
///
/// Reads are less dangerous than writes, but they still couple a module to
/// another module's schema. Prefer the owning service; when a read must stay
/// local (reporting joins, referential guards), record it here.
const READ_ALLOWED: &[(&str, &str, &str)] = &[
    (
        "catalog",
        "sales_channels",
        "catalog validates a sales channel id against the store table",
    ),
    (
        "catalog",
        "cart_items",
        "catalog reports best sellers from cart rows",
    ),
    (
        "catalog",
        "order_items",
        "catalog reports best sellers from order rows",
    ),
    (
        "checkout",
        "carts",
        "checkout snapshots the cart before creating an order",
    ),
    ("checkout", "cart_items", "checkout snapshots cart lines"),
    (
        "checkout",
        "payment_sessions",
        "checkout reads payment session state",
    ),
    (
        "pricing",
        "customer_groups",
        "pricing resolves customer group membership",
    ),
    (
        "pricing",
        "sales_channels",
        "pricing validates a sales channel id",
    ),
    (
        "tax",
        "product_variants",
        "tax category deletion checks variant references",
    ),
    (
        "pricing",
        "product_variant_price_set",
        "pricing checks the catalog-owned variant link inside its own transaction; writes go through CatalogService",
    ),
    (
        "order",
        "refunds",
        "order reconciliation reads provider refunds to repair the order ledger",
    ),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/api lives under the repository root")
        .to_path_buf()
}

fn modules() -> Vec<(String, PathBuf)> {
    let dir = repo_root().join("crates/modules");
    let mut modules: Vec<_> = fs::read_dir(&dir)
        .expect("modules directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path().join("src"),
            )
        })
        .collect();
    modules.sort();
    modules
}

fn source_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .expect("module src directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "rs"))
        .filter(|path| path.file_name().is_some_and(|name| name != "tests.rs"))
        .collect();
    files.sort();
    files
}

/// Returns `(table, 1-based line)` for each INSERT/UPDATE/DELETE target.
fn write_targets(source: &str) -> Vec<(String, usize)> {
    let mut targets = Vec::new();
    let mut in_test_module = false;
    for (index, line) in source.lines().enumerate() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            in_test_module = true;
        }
        if in_test_module {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        for keyword in [
            "INSERT INTO ",
            "INSERT OR IGNORE INTO ",
            "INSERT OR REPLACE INTO ",
            "UPDATE ",
            "DELETE FROM ",
        ] {
            let mut offset = 0;
            while let Some(found) = upper[offset..].find(keyword) {
                let start = offset + found + keyword.len();
                let table: String = line[start..]
                    .chars()
                    .skip_while(|character| character.is_whitespace())
                    .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .collect();
                if !table.is_empty() {
                    targets.push((table, index + 1));
                }
                offset = start;
            }
        }
    }
    targets
}

/// Returns `(table, 1-based line)` for each table read by SELECT ... FROM/JOIN.
fn read_targets(source: &str) -> Vec<(String, usize)> {
    let mut targets = Vec::new();
    let mut in_test_module = false;
    for (index, line) in source.lines().enumerate() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            in_test_module = true;
        }
        if in_test_module {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if upper.trim_start().starts_with("DELETE FROM ") {
            continue;
        }
        for keyword in ["FROM", "JOIN"] {
            let mut offset = 0;
            while let Some(found) = upper[offset..].find(keyword) {
                let after = offset + found + keyword.len();
                let before_ok = if keyword == "FROM" {
                    offset + found == 0
                        || !upper.as_bytes()[offset + found - 1].is_ascii_alphanumeric()
                } else {
                    true
                };
                if !before_ok {
                    offset = after;
                    continue;
                }
                let rest = line[after..].trim_start();
                if rest.starts_with('(') {
                    offset = after;
                    continue;
                }
                let table: String = rest
                    .chars()
                    .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .collect();
                if !table.is_empty() {
                    targets.push((table.to_ascii_lowercase(), index + 1));
                }
                offset = after;
            }
        }
    }
    targets
}

/// Maps a table to the module that owns it. Unknown tables are ignored so this
/// guard only governs tables that have a declared owner.
fn owner_for(table: &str) -> Option<&'static str> {
    if SHARED_TABLES.contains(&table) {
        return None;
    }
    let owners: &[(&str, &str)] = &[
        ("carts", "cart"),
        ("cart_", "cart"),
        ("checkout", "checkout"),
        ("orders", "order"),
        ("order_", "order"),
        ("after_sales", "order"),
        ("products", "catalog"),
        ("product_", "catalog"),
        ("categories", "catalog"),
        ("collections", "catalog"),
        ("tags", "catalog"),
        ("prices", "pricing"),
        ("price_", "pricing"),
        ("inventory", "inventory"),
        ("customers", "customer"),
        ("customer_", "customer"),
        ("promotions", "promotion"),
        ("promotion_", "promotion"),
        ("coupons", "promotion"),
        ("campaigns", "promotion"),
        ("campaign_", "promotion"),
        ("payments", "payment"),
        ("payment_", "payment"),
        ("refunds", "payment"),
        ("shipping", "shipping"),
        ("fulfillment_", "shipping"),
        ("tax_", "tax"),
        ("themes", "theme"),
        ("theme_", "theme"),
        ("pages", "content"),
        ("page_", "content"),
        ("navigations", "content"),
        ("navigation_", "content"),
        ("redirects", "content"),
        ("inquiries", "inquiry"),
        ("inquiry_", "inquiry"),
        ("assets", "asset"),
        ("notification", "notifications"),
        ("webhook_", "webhook"),
        ("workflow_", "workflows"),
        ("settings", "store"),
        ("sales_channel", "store"),
        ("regions", "store"),
        ("publishable", "store"),
        ("preview_token_revocations", "theme"),
    ];
    owners
        .iter()
        .find(|(prefix, _)| table.starts_with(prefix))
        .map(|(_, owner)| *owner)
}

fn allowed(module: &str, table: &str) -> bool {
    ALLOWED.iter().any(|(allowed_module, allowed_table, _)| {
        *allowed_module == module && *allowed_table == table
    })
}

fn read_allowed(module: &str, table: &str) -> bool {
    READ_ALLOWED
        .iter()
        .any(|(allowed_module, allowed_table, _)| {
            *allowed_module == module && *allowed_table == table
        })
}

#[test]
fn modules_only_write_their_own_tables() {
    let mut violations = Vec::new();
    for (module, src) in modules() {
        for file in source_files(&src) {
            let source = fs::read_to_string(&file).expect("read module source");
            for (table, line) in write_targets(&source) {
                let Some(owner) = owner_for(&table) else {
                    continue;
                };
                if owner != module && !allowed(&module, &table) {
                    violations.push(format!(
                        "{}:{} writes `{}`, owned by the `{}` module",
                        file.display(),
                        line,
                        table,
                        owner
                    ));
                }
            }
        }
    }
    violations.sort();
    assert!(
        violations.is_empty(),
        "cross-module writes must go through the owning module's service or be added to ALLOWED with a reason:\n{}",
        violations.join("\n")
    );
}

#[test]
fn modules_only_read_their_own_tables() {
    let mut violations = Vec::new();
    for (module, src) in modules() {
        for file in source_files(&src) {
            let source = fs::read_to_string(&file).expect("read module source");
            for (table, line) in read_targets(&source) {
                let Some(owner) = owner_for(&table) else {
                    continue;
                };
                if owner != module && !read_allowed(&module, &table) {
                    violations.push(format!(
                        "{}:{} reads `{}`, owned by the `{}` module",
                        file.display(),
                        line,
                        table,
                        owner
                    ));
                }
            }
        }
    }
    violations.sort();
    assert!(
        violations.is_empty(),
        "cross-module reads must go through the owning module's service or be added to READ_ALLOWED with a reason:\n{}",
        violations.join("\n")
    );
}

#[test]
fn read_exceptions_still_exist() {
    let mut seen = Vec::new();
    for (module, src) in modules() {
        for file in source_files(&src) {
            let source = fs::read_to_string(&file).expect("read module source");
            for (table, _) in read_targets(&source) {
                if read_allowed(&module, &table) {
                    seen.push((module.clone(), table));
                }
            }
        }
    }
    for (module, table, reason) in READ_ALLOWED {
        assert!(
            seen.iter()
                .any(|(seen_module, seen_table)| seen_module == module && seen_table == table),
            "stale module-boundary read exception ({module} -> {table}: {reason}); remove it"
        );
    }
}

#[test]
fn allowed_exceptions_still_exist() {
    let mut seen = Vec::new();
    for (module, src) in modules() {
        for file in source_files(&src) {
            let source = fs::read_to_string(&file).expect("read module source");
            for (table, _) in write_targets(&source) {
                if allowed(&module, &table) {
                    seen.push((module.clone(), table));
                }
            }
        }
    }
    for (module, table, reason) in ALLOWED {
        assert!(
            seen.iter()
                .any(|(seen_module, seen_table)| seen_module == module && seen_table == table),
            "stale module-boundary exception ({module} -> {table}: {reason}); remove it"
        );
    }
}
