//! Tax provider resolution.

use super::*;

impl CartWorkflow {
    /// Resolves the provider that serves one recalculation.
    ///
    /// [`TAX_PROVIDER_SETTING_KEY`] names the provider the store selected. An
    /// absent key keeps the registry's active provider — the built-in
    /// region/rate provider for a workflow built by [`Self::new`] — so a store
    /// that never chose one is taxed exactly as before. A present value must
    /// name a registered provider: silently falling back to the built-in rates
    /// would let a store believe its external engine is pricing the carts when
    /// it is not, so an unknown identifier fails the pass instead. The provider
    /// is returned rather than activated on the registry: the selection belongs
    /// to the pass that read the setting, not to the process.
    pub(crate) async fn tax_provider(&self) -> Result<Arc<dyn TaxProvider>, AppError> {
        let providers = self.tax.providers();
        let Some(configured) = self.store.get_setting(TAX_PROVIDER_SETTING_KEY).await? else {
            return providers.active();
        };
        if let Some(provider) = providers.get(&configured) {
            return Ok(provider);
        }
        let registered: Vec<String> = providers
            .list()
            .iter()
            .map(|provider| provider.id().to_owned())
            .collect();
        Err(AppError::conflict(format!(
            "{TAX_PROVIDER_SETTING_KEY} is set to \"{configured}\", which is not a registered tax \
             provider; registered providers: {}",
            if registered.is_empty() {
                "none".to_owned()
            } else {
                registered.join(", ")
            }
        )))
    }
}
