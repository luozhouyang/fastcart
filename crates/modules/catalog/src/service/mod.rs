//! SQLite-backed catalog service.

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
use fastcart_kernel::AppError;
use sqlx::Transaction;
use sqlx::sqlite::Sqlite;

/// Maximum product, variant, option or category title length.
pub const MAX_TITLE_LENGTH: usize = 200;
/// Maximum option value length.
pub const MAX_OPTION_VALUE_LENGTH: usize = 200;
/// Maximum SKU length.
pub const MAX_SKU_LENGTH: usize = 120;
/// Maximum tag value length.
pub const MAX_TAG_VALUE_LENGTH: usize = 100;
/// Maximum image URL length.
pub const MAX_URL_LENGTH: usize = 2048;

/// Maximum product type value length.
pub use product_types::MAX_PRODUCT_TYPE_VALUE_LENGTH;

/// Maximum number of bound parameters used in one `IN (...)` statement.
/// SQLite's default limit is 999; staying under it leaves headroom for the
/// other parameters a query may carry.
const SQLITE_PARAMETER_LIMIT: usize = 900;

const PRODUCT_COLUMNS: &str = "p.id, p.title, p.subtitle, p.description, p.handle, p.status, \
     p.thumbnail, p.weight, p.length, p.height, p.width, p.metadata, p.product_type_id, \
     p.created_at, p.updated_at";

/// Full CRUD over the catalog aggregate.
mod categories;
mod collections;
mod images;
mod product_types;
mod products;
mod shared;
mod tags;
mod variants;

#[derive(Clone, Debug)]
pub struct CatalogService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

impl CatalogService {
    /// Creates a service over an already-migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }

    /// Records catalog mutations in the store audit outbox.
    ///
    /// When an actor is set, audited mutations append their outbox row in the
    /// same transaction as the change, so a rollback leaves no audit behind.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: serde_json::Value,
    ) -> Result<(), AppError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, resource_type, resource_id, data)
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }
}
