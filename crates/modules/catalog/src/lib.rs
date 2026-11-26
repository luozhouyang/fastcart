//! The `catalog` commerce module.
//!
//! Owns products, variants, options, media and the categories, collections and
//! tags used to merchandise them. The crate is self-contained: cross-module
//! orchestration belongs in `fastcart-workflows`.

pub mod domain;
pub mod service;

#[cfg(test)]
mod tests;

pub use domain::{
    Category, Collection, CreateCategoryInput, CreateCollectionInput, CreateImageInput,
    CreateOptionInput, CreateProductInput, CreateProductTypeInput, CreateVariantInput, Dimensions,
    Product, ProductFilter, ProductImage, ProductOption, ProductOptionValue, ProductSort,
    ProductStatus, ProductType, ProductVariant, PublishedRouteEntry, Tag, UpdateCategoryInput,
    UpdateCollectionInput, UpdateProductInput, UpdateProductTypeInput, UpdateVariantInput,
    VariantOptionInput,
};
pub use service::{
    CatalogService, MAX_OPTION_VALUE_LENGTH, MAX_PRODUCT_TYPE_VALUE_LENGTH, MAX_SKU_LENGTH,
    MAX_TAG_VALUE_LENGTH, MAX_TITLE_LENGTH, MAX_URL_LENGTH,
};
