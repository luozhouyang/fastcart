//! Fulfillment provider abstraction and in-process registry.
//!
//! Providers create and cancel provider-side fulfillments. Shipping options
//! reference a registered provider by identifier; the registry lets the API and
//! workflows resolve that implementation without knowing its concrete type.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use fastcart_kernel::{AppError, CurrencyCode, Metadata};
use serde_json::Value;
use uuid::Uuid;

use crate::domain::ShippingOption;

/// A line item included in a fulfillment request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FulfillmentItemRequest {
    /// Variant being fulfilled.
    pub variant_id: String,
    /// Quantity to fulfill.
    pub quantity: i64,
}

/// Provider-independent fulfillment request.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateFulfillmentRequest {
    /// Order being fulfilled.
    pub order_id: String,
    /// Items and quantities.
    pub items: Vec<FulfillmentItemRequest>,
    /// Tracking numbers supplied by the operator.
    pub tracking_numbers: Vec<String>,
    /// Provider-specific configuration from the shipping option.
    pub provider_data: Value,
    /// Extension metadata.
    pub metadata: Metadata,
}

/// Provider-side result of creating a fulfillment.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FulfillmentReceipt {
    /// Registered provider identifier.
    pub provider_id: String,
    /// Provider-side fulfillment identifier.
    pub provider_fulfillment_id: String,
    /// Tracking numbers known to the provider.
    pub tracking_numbers: Vec<String>,
    /// Provider-specific response data.
    pub data: Value,
}

/// Context used by providers that calculate shipping prices dynamically.
#[derive(Clone, Debug, PartialEq)]
pub struct FulfillmentPriceContext {
    /// Destination country.
    pub country_code: String,
    /// Destination province, when known.
    pub province_code: Option<String>,
    /// Cart subtotal in minor units.
    pub subtotal: i64,
    /// Cart currency.
    pub currency: CurrencyCode,
    /// Number of items in the shipment.
    pub item_count: i64,
}

/// A fulfillment backend such as manual shipping or a third-party carrier.
#[async_trait]
pub trait FulfillmentProvider: Send + Sync {
    /// Stable identifier referenced by shipping options.
    fn id(&self) -> &str;

    /// Whether the provider prices options dynamically.
    fn can_calculate_price(&self) -> bool {
        false
    }

    /// Calculates a shipping price, returning `None` when the provider cannot
    /// price the option for this context.
    async fn calculate_price(
        &self,
        _option: &ShippingOption,
        _context: &FulfillmentPriceContext,
    ) -> Result<Option<i64>, AppError> {
        Ok(None)
    }

    /// Creates a provider-side fulfillment.
    async fn create_fulfillment(
        &self,
        request: CreateFulfillmentRequest,
    ) -> Result<FulfillmentReceipt, AppError>;

    /// Cancels a provider-side fulfillment.
    async fn cancel_fulfillment(&self, provider_fulfillment_id: &str) -> Result<(), AppError>;
}

/// Built-in provider that records fulfillments without contacting a carrier.
#[derive(Clone, Debug)]
pub struct ManualFulfillmentProvider {
    id: String,
}

impl ManualFulfillmentProvider {
    /// Creates a manual provider with the given identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl Default for ManualFulfillmentProvider {
    fn default() -> Self {
        Self::new("manual")
    }
}

#[async_trait]
impl FulfillmentProvider for ManualFulfillmentProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn create_fulfillment(
        &self,
        request: CreateFulfillmentRequest,
    ) -> Result<FulfillmentReceipt, AppError> {
        if request.items.is_empty() {
            return Err(AppError::validation(
                "a fulfillment must contain at least one item",
            ));
        }
        if request.items.iter().any(|item| item.quantity <= 0) {
            return Err(AppError::validation(
                "fulfillment item quantities must be positive",
            ));
        }
        Ok(FulfillmentReceipt {
            provider_id: self.id.clone(),
            provider_fulfillment_id: format!("ful_{}", Uuid::now_v7().simple()),
            tracking_numbers: request.tracking_numbers,
            data: Value::Object(serde_json::Map::new()),
        })
    }

    async fn cancel_fulfillment(&self, _provider_fulfillment_id: &str) -> Result<(), AppError> {
        Ok(())
    }
}

/// Thread-safe registry of fulfillment providers.
#[derive(Clone, Default)]
pub struct FulfillmentProviderRegistry {
    providers: Arc<RwLock<HashMap<String, Arc<dyn FulfillmentProvider>>>>,
}

impl FulfillmentProviderRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a registry with the built-in manual provider registered.
    #[must_use]
    pub fn with_defaults() -> Self {
        let registry = Self::new();
        registry
            .register(Arc::new(ManualFulfillmentProvider::default()))
            .expect("the manual provider is not present in a fresh registry");
        registry
    }

    /// Registers a provider, failing when its identifier is already present.
    pub fn register(&self, provider: Arc<dyn FulfillmentProvider>) -> Result<(), AppError> {
        let id = provider.id().to_owned();
        if id.trim().is_empty() {
            return Err(AppError::validation(
                "fulfillment provider id must not be empty",
            ));
        }
        let mut providers = self
            .providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if providers.contains_key(&id) {
            return Err(AppError::conflict(format!(
                "fulfillment provider already registered: {id}"
            )));
        }
        providers.insert(id, provider);
        Ok(())
    }

    /// Looks up a provider by identifier.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<dyn FulfillmentProvider>> {
        self.providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Returns all registered providers ordered by identifier.
    #[must_use]
    pub fn list(&self) -> Vec<Arc<dyn FulfillmentProvider>> {
        let mut providers: Vec<_> = self
            .providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        providers.sort_by(|left, right| left.id().cmp(right.id()));
        providers
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fastcart_kernel::Metadata;

    use super::{
        CreateFulfillmentRequest, FulfillmentItemRequest, FulfillmentProvider,
        FulfillmentProviderRegistry, ManualFulfillmentProvider,
    };

    #[tokio::test]
    async fn manual_provider_creates_and_cancels_fulfillments() {
        let provider = ManualFulfillmentProvider::default();
        let receipt = provider
            .create_fulfillment(CreateFulfillmentRequest {
                order_id: "order_1".to_owned(),
                items: vec![FulfillmentItemRequest {
                    variant_id: "variant_1".to_owned(),
                    quantity: 2,
                }],
                tracking_numbers: vec!["TRACK".to_owned()],
                provider_data: serde_json::json!({}),
                metadata: Metadata::empty(),
            })
            .await
            .unwrap();
        assert_eq!(receipt.provider_id, "manual");
        assert_eq!(receipt.tracking_numbers, vec!["TRACK".to_owned()]);
        provider
            .cancel_fulfillment(&receipt.provider_fulfillment_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn registry_rejects_duplicate_provider_ids() {
        let registry = FulfillmentProviderRegistry::with_defaults();
        let duplicate = registry.register(Arc::new(ManualFulfillmentProvider::default()));
        assert!(duplicate.is_err());
        assert!(registry.get("manual").is_some());
        assert_eq!(registry.list().len(), 1);
        assert!(!registry.get("manual").unwrap().can_calculate_price());
    }
}
