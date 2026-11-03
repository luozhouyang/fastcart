//! Catalog domain types and inputs.
//!
//! The catalog owns products, variants, options, media and the organizational
//! entities (categories, collections and tags) used to merchandise them. Money is
//! not modelled here; prices live in the pricing module.

use serde::{Deserialize, Serialize};

use fastcart_kernel::Metadata;

/// Publication state of a product or variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductStatus {
    /// Created but not visible in storefronts.
    #[default]
    Draft,
    /// Published and purchasable.
    Active,
    /// Retired: no longer purchasable and cannot be published again.
    Archived,
}

impl ProductStatus {
    /// Returns the database representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    /// Parses a stored status, falling back to `draft` for unknown values.
    #[must_use]
    pub fn from_db(value: &str) -> Self {
        match value {
            "active" => Self::Active,
            "archived" => Self::Archived,
            _ => Self::Draft,
        }
    }

    /// Returns true when the product may be published.
    #[must_use]
    pub fn can_publish(self) -> bool {
        matches!(self, Self::Draft | Self::Active)
    }
}

/// Physical dimensions in the store's configured unit system.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dimensions {
    /// Length.
    pub length: Option<f64>,
    /// Height.
    pub height: Option<f64>,
    /// Width.
    pub width: Option<f64>,
}

/// A sellable catalog product.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Product {
    /// Product identifier.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Optional subtitle.
    pub subtitle: Option<String>,
    /// Optional long description.
    pub description: Option<String>,
    /// URL-safe unique handle.
    pub handle: String,
    /// Publication state.
    pub status: ProductStatus,
    /// Primary thumbnail URL.
    pub thumbnail: Option<String>,
    /// Optional shipping weight.
    pub weight: Option<f64>,
    /// Optional physical dimensions.
    pub dimensions: Dimensions,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
    /// Variants belonging to the product.
    pub variants: Vec<ProductVariant>,
    /// Configurable options and their values.
    pub options: Vec<ProductOption>,
    /// Product media, ordered by rank.
    pub images: Vec<ProductImage>,
    /// Linked categories.
    pub categories: Vec<Category>,
    /// Linked collections.
    pub collections: Vec<Collection>,
    /// Linked tags.
    pub tags: Vec<Tag>,
}

/// A concrete purchasable variant of a product.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductVariant {
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
    pub status: ProductStatus,
    /// Optional shipping weight.
    pub weight: Option<f64>,
    /// Optional physical dimensions.
    pub dimensions: Dimensions,
    /// Whether inventory is tracked for this variant.
    pub manage_inventory: bool,
    /// Whether orders may exceed available stock.
    pub allow_backorder: bool,
    /// Optional tax category reference.
    pub tax_category_id: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Option values selected for this variant.
    pub option_value_ids: Vec<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
    /// Soft-deletion timestamp, present when retained for history.
    pub deleted_at: Option<i64>,
}

/// A product option, such as size or colour.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductOption {
    /// Option identifier.
    pub id: String,
    /// Owning product identifier.
    pub product_id: String,
    /// Display title.
    pub title: String,
    /// Sort rank.
    pub rank: i64,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Values belonging to the option.
    pub values: Vec<ProductOptionValue>,
}

/// A single value of a product option.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductOptionValue {
    /// Option value identifier.
    pub id: String,
    /// Owning option identifier.
    pub option_id: String,
    /// Display value.
    pub value: String,
    /// Sort rank.
    pub rank: i64,
    /// Free-form metadata.
    pub metadata: Metadata,
}

/// A product image.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductImage {
    /// Image identifier.
    pub id: String,
    /// Owning product identifier.
    pub product_id: String,
    /// Optional variant this image is specific to.
    pub variant_id: Option<String>,
    /// Image URL.
    pub url: String,
    /// Optional alternative text.
    pub alt: Option<String>,
    /// Sort rank.
    pub rank: i64,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
}

/// A node in the category hierarchy.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Category {
    /// Category identifier.
    pub id: String,
    /// Optional parent category identifier.
    pub parent_id: Option<String>,
    /// Display title.
    pub title: String,
    /// URL-safe unique handle.
    pub handle: String,
    /// Optional description.
    pub description: Option<String>,
    /// Whether the category is visible.
    pub is_active: bool,
    /// Whether the category is internal-only.
    pub is_internal: bool,
    /// Sort rank.
    pub rank: i64,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// A curated collection of products.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    /// Collection identifier.
    pub id: String,
    /// Display title.
    pub title: String,
    /// URL-safe unique handle.
    pub handle: String,
    /// Optional description.
    pub description: Option<String>,
    /// Whether the collection is published.
    pub is_published: bool,
    /// Sort rank.
    pub rank: i64,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// A free-form product tag.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    /// Tag identifier.
    pub id: String,
    /// Unique tag value.
    pub value: String,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
}

/// A selected option value used to build a variant during product creation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantOptionInput {
    /// Title of the option, matched case-insensitively.
    pub option_title: String,
    /// Value of the option, matched case-insensitively.
    pub value: String,
}

/// Input for creating a product option.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateOptionInput {
    /// Option title.
    pub title: String,
    /// Values belonging to the option.
    pub values: Vec<String>,
    /// Optional sort rank. Defaults to input order.
    pub rank: Option<i64>,
    /// Free-form metadata.
    pub metadata: Metadata,
}

/// Input for creating a product variant.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateVariantInput {
    /// Variant title.
    pub title: String,
    /// Optional unique SKU.
    pub sku: Option<String>,
    /// Optional barcode.
    pub barcode: Option<String>,
    /// Publication state; defaults to `active`.
    pub status: Option<ProductStatus>,
    /// Optional shipping weight.
    pub weight: Option<f64>,
    /// Optional physical dimensions.
    pub dimensions: Option<Dimensions>,
    /// Whether inventory is tracked; defaults to `true`.
    pub manage_inventory: Option<bool>,
    /// Whether backorders are allowed; defaults to `false`.
    pub allow_backorder: Option<bool>,
    /// Optional tax category reference.
    pub tax_category_id: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Existing option value identifiers to link.
    pub option_value_ids: Vec<String>,
    /// Option values resolved by option title and value.
    pub option_values: Vec<VariantOptionInput>,
}

/// Input for partially updating a product variant.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateVariantInput {
    /// Existing variant identifier when used by [`UpdateProductInput`]. Ignored
    /// by direct variant updates.
    pub id: Option<String>,
    /// New title. `None` leaves it unchanged.
    pub title: Option<String>,
    /// New SKU. `Some("")` clears it; `None` leaves it unchanged.
    pub sku: Option<String>,
    /// New barcode. `Some("")` clears it; `None` leaves it unchanged.
    pub barcode: Option<String>,
    /// New status. `None` leaves it unchanged.
    pub status: Option<ProductStatus>,
    /// New weight. `None` leaves it unchanged.
    pub weight: Option<f64>,
    /// Replacement dimensions. `None` leaves them unchanged.
    pub dimensions: Option<Dimensions>,
    /// New inventory-management flag.
    pub manage_inventory: Option<bool>,
    /// New backorder flag.
    pub allow_backorder: Option<bool>,
    /// New tax category. `Some("")` clears it; `None` leaves it unchanged.
    pub tax_category_id: Option<String>,
    /// Replacement metadata. `None` leaves it unchanged.
    pub metadata: Option<Metadata>,
    /// Replacement option-value links. `None` leaves them unchanged.
    pub option_value_ids: Option<Vec<String>>,
}

/// Input for creating a product.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateProductInput {
    /// Required product title.
    pub title: String,
    /// Optional handle; a slug is derived from the title when omitted.
    pub handle: Option<String>,
    /// Optional subtitle.
    pub subtitle: Option<String>,
    /// Optional description.
    pub description: Option<String>,
    /// Optional thumbnail URL.
    pub thumbnail: Option<String>,
    /// Optional shipping weight.
    pub weight: Option<f64>,
    /// Optional physical dimensions.
    pub dimensions: Option<Dimensions>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Variants to create with the product.
    pub variants: Vec<CreateVariantInput>,
    /// Options and values to create with the product.
    pub options: Vec<CreateOptionInput>,
    /// Category identifiers to link.
    pub category_ids: Vec<String>,
    /// Collection identifiers to link.
    pub collection_ids: Vec<String>,
    /// Tag values to attach, creating tags as needed.
    pub tag_values: Vec<String>,
    /// Image URLs to attach, ordered as supplied.
    pub images: Vec<String>,
}

/// Input for partially updating a product.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateProductInput {
    /// New title. `None` leaves it unchanged.
    pub title: Option<String>,
    /// New handle. `None` leaves it unchanged.
    pub handle: Option<String>,
    /// New subtitle. `Some("")` clears it; `None` leaves it unchanged.
    pub subtitle: Option<String>,
    /// New description. `Some("")` clears it; `None` leaves it unchanged.
    pub description: Option<String>,
    /// New thumbnail. `Some("")` clears it; `None` leaves it unchanged.
    pub thumbnail: Option<String>,
    /// New status. Archived products cannot be moved back to active.
    pub status: Option<ProductStatus>,
    /// New weight. `None` leaves it unchanged.
    pub weight: Option<f64>,
    /// Replacement dimensions. `None` leaves them unchanged.
    pub dimensions: Option<Dimensions>,
    /// Replacement metadata. `None` leaves it unchanged.
    pub metadata: Option<Metadata>,
    /// Replaces the full variant set when supplied. Entries with an `id` update
    /// that variant; entries without one create a variant; omitted variants are
    /// deleted (soft-deleted when referenced by carts or orders).
    pub variants: Option<Vec<UpdateVariantInput>>,
    /// Replaces the full option set when supplied.
    pub options: Option<Vec<CreateOptionInput>>,
    /// Replaces category links when supplied.
    pub category_ids: Option<Vec<String>>,
    /// Replaces collection links when supplied.
    pub collection_ids: Option<Vec<String>>,
    /// Replaces tag links when supplied.
    pub tag_values: Option<Vec<String>>,
    /// Replaces product images when supplied.
    pub images: Option<Vec<String>>,
}

/// Input for creating a category.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateCategoryInput {
    /// Required title.
    pub title: String,
    /// Optional handle; a slug is derived from the title when omitted.
    pub handle: Option<String>,
    /// Optional parent category.
    pub parent_id: Option<String>,
    /// Optional description.
    pub description: Option<String>,
    /// Whether the category is visible; defaults to `true`.
    pub is_active: Option<bool>,
    /// Whether the category is internal-only; defaults to `false`.
    pub is_internal: Option<bool>,
    /// Optional sort rank.
    pub rank: Option<i64>,
    /// Free-form metadata.
    pub metadata: Metadata,
}

/// Input for partially updating a category.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateCategoryInput {
    /// New title. `None` leaves it unchanged.
    pub title: Option<String>,
    /// New handle. `None` leaves it unchanged.
    pub handle: Option<String>,
    /// New parent. `Some("")` makes the category a root; `None` leaves it
    /// unchanged.
    pub parent_id: Option<String>,
    /// New description. `Some("")` clears it; `None` leaves it unchanged.
    pub description: Option<String>,
    /// New active flag.
    pub is_active: Option<bool>,
    /// New internal flag.
    pub is_internal: Option<bool>,
    /// New sort rank.
    pub rank: Option<i64>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Input for creating a collection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateCollectionInput {
    /// Required title.
    pub title: String,
    /// Optional handle; a slug is derived from the title when omitted.
    pub handle: Option<String>,
    /// Optional description.
    pub description: Option<String>,
    /// Whether the collection is published; defaults to `false`.
    pub is_published: Option<bool>,
    /// Optional sort rank.
    pub rank: Option<i64>,
    /// Free-form metadata.
    pub metadata: Metadata,
}

/// Input for partially updating a collection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateCollectionInput {
    /// New title.
    pub title: Option<String>,
    /// New handle.
    pub handle: Option<String>,
    /// New description. `Some("")` clears it.
    pub description: Option<String>,
    /// New publication flag.
    pub is_published: Option<bool>,
    /// New sort rank.
    pub rank: Option<i64>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Input for adding a product image.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateImageInput {
    /// Image URL.
    pub url: String,
    /// Optional alternative text.
    pub alt: Option<String>,
    /// Optional owning variant.
    pub variant_id: Option<String>,
    /// Optional sort rank.
    pub rank: Option<i64>,
    /// Free-form metadata.
    pub metadata: Metadata,
}

/// Filtering options for administrative product listing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductFilter {
    /// Restrict to a publication state.
    pub status: Option<ProductStatus>,
    /// Restrict to products linked to a category.
    pub category_id: Option<String>,
    /// Restrict to products linked to a collection.
    pub collection_id: Option<String>,
    /// Restrict to products carrying a tag value.
    pub tag_value: Option<String>,
    /// Case-insensitive text matched against title, description, handle and SKU.
    pub query: Option<String>,
    /// Include soft-deleted products.
    pub include_deleted: bool,
}
