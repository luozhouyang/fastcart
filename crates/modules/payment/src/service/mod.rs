//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::AppError;

use crate::provider::{PaymentProvider, ProviderRegistry};

mod capture;
mod collections;
mod gift_cards;
mod reads;
mod refunds;
mod sessions;
mod store;
mod webhooks;

pub struct PaymentService {
    db: StoreDatabase,
    providers: ProviderRegistry,
}

impl PaymentService {
    /// Creates a service with an empty provider registry.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self {
            db,
            providers: ProviderRegistry::new(),
        }
    }

    /// Creates a service with a pre-populated provider registry.
    #[must_use]
    pub fn with_registry(db: StoreDatabase, providers: ProviderRegistry) -> Self {
        Self { db, providers }
    }

    /// Returns the provider registry.
    #[must_use]
    pub fn providers(&self) -> &ProviderRegistry {
        &self.providers
    }

    /// Registers a provider, failing when the identifier is already in use.
    pub fn register_provider(&self, provider: Arc<dyn PaymentProvider>) -> Result<(), AppError> {
        self.providers.register(provider)
    }

    /// Registers or replaces a provider.
    pub fn replace_provider(
        &self,
        provider: Arc<dyn PaymentProvider>,
    ) -> Option<Arc<dyn PaymentProvider>> {
        self.providers.replace(provider)
    }
}

#[cfg(test)]
mod tests;
