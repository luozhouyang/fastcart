use super::*;

/// A purchasable product variant.
///
/// This mirrors the catalog domain `ProductVariant` on the wire; the API DTO
/// exists so the documented contract is derived from a type owned by this
/// crate rather than the catalog module.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Variant)]
pub struct VariantDto {
    /// Variant identifier.
    pub id: String,
    /// Owning product identifier.
    pub product_id: String,
    /// Display title.
    pub title: String,
    /// Optional unique stock-keeping unit.
    pub sku: Option<String>,
    /// Optional barcode.
    pub barcode: Option<String>,
    /// Publication state.
    #[schema(required = false)]
    pub status: ProductStatus,
    /// Optional shipping weight.
    pub weight: Option<f64>,
    /// Optional physical dimensions.
    pub dimensions: Option<Dimensions>,
    /// Whether inventory is tracked for this variant.
    #[schema(required = false)]
    pub manage_inventory: bool,
    /// Whether orders may exceed available stock.
    #[schema(required = false)]
    pub allow_backorder: bool,
    /// Optional tax category reference.
    pub tax_category_id: Option<String>,
    /// Free-form metadata.
    #[schema(required = false)]
    pub metadata: Metadata,
    /// Option values selected for this variant.
    #[schema(required = false)]
    pub option_value_ids: Vec<String>,
    /// Creation timestamp in Unix seconds.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    #[schema(required = false)]
    pub updated_at: i64,
    /// Soft-deletion timestamp, present when retained for history.
    pub deleted_at: Option<i64>,
}

impl From<ProductVariant> for VariantDto {
    fn from(variant: ProductVariant) -> Self {
        Self {
            id: variant.id,
            product_id: variant.product_id,
            title: variant.title,
            sku: variant.sku,
            barcode: variant.barcode,
            status: variant.status,
            weight: variant.weight,
            dimensions: Some(variant.dimensions),
            manage_inventory: variant.manage_inventory,
            allow_backorder: variant.allow_backorder,
            tax_category_id: variant.tax_category_id,
            metadata: variant.metadata,
            option_value_ids: variant.option_value_ids,
            created_at: variant.created_at,
            updated_at: variant.updated_at,
            deleted_at: variant.deleted_at,
        }
    }
}

/// A single storefront product.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ProductResponse {
    /// The requested product.
    pub product: StoreProduct,
}

/// A page of storefront products.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ProductListResponse {
    /// Products in this page.
    pub products: Vec<StoreProduct>,
    /// Total number of matching products.
    pub count: i64,
    /// Number of products skipped.
    pub offset: i64,
    /// Maximum number of products requested.
    pub limit: i64,
}

impl ProductListResponse {
    /// Builds a product envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<StoreProduct>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            products: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A page of storefront search results.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ProductSearchResponse {
    /// Matching products.
    pub products: Vec<Product>,
    /// Total number of matching products.
    pub count: i64,
    /// Number of products skipped.
    pub offset: i64,
    /// Maximum number of products requested.
    pub limit: i64,
}

impl ProductSearchResponse {
    /// Builds a search envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Product>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            products: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A page of storefront product categories.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CategoryListResponse {
    /// Categories in this page.
    pub product_categories: Vec<Category>,
    /// Total number of matching categories.
    pub count: i64,
    /// Number of categories skipped.
    pub offset: i64,
    /// Maximum number of categories requested.
    pub limit: i64,
}

impl CategoryListResponse {
    /// Builds a category envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Category>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            product_categories: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A page of storefront product collections.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CollectionListResponse {
    /// Collections in this page.
    pub collections: Vec<Collection>,
    /// Total number of matching collections.
    pub count: i64,
    /// Number of collections skipped.
    pub offset: i64,
    /// Maximum number of collections requested.
    pub limit: i64,
}

impl CollectionListResponse {
    /// Builds a collection envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Collection>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            collections: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}
