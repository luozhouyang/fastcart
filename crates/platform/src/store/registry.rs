//! `StoreRegistry` implementation.

use super::*;

#[async_trait::async_trait]
impl StoreRegistry for PlatformStore {
    async fn create_store(&self, input: CreateStoreInput) -> PlatformResult<Store> {
        PlatformStore::create_store(self, input).await
    }

    async fn get_store(&self, id: &StoreId) -> PlatformResult<Option<Store>> {
        PlatformStore::get_store(self, id).await
    }

    async fn get_store_by_domain(&self, hostname: &str) -> PlatformResult<Option<Store>> {
        PlatformStore::get_store_by_domain(self, hostname).await
    }

    async fn update_store(&self, id: &StoreId, input: UpdateStoreInput) -> PlatformResult<Store> {
        PlatformStore::update_store(self, id, input).await
    }

    async fn update_status(&self, id: &StoreId, status: StoreStatus) -> PlatformResult<Store> {
        PlatformStore::update_status(self, id, status).await
    }
}
