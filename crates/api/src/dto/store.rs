use super::*;

/// A page of storefront regions.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct RegionListResponse {
    /// Regions in this page.
    pub regions: Vec<Region>,
    /// Total number of matching regions.
    pub count: i64,
    /// Number of regions skipped.
    pub offset: i64,
    /// Maximum number of regions requested.
    pub limit: i64,
}

impl RegionListResponse {
    /// Builds a region envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Region>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            regions: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A single storefront region.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct RegionResponse {
    /// The requested region.
    pub region: Region,
}

/// Per-variant availability for a storefront product.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct VariantAvailabilityResponse {
    /// Availability records.
    pub variants: Vec<VariantAvailability>,
}

/// Shipping options available for a destination.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ShippingOptionsResponse {
    /// Matching shipping options.
    pub shipping_options: Vec<ResolvedShippingOption>,
}
