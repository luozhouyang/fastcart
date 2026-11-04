//! The `asset` commerce module.
//!
//! Owns media asset metadata for the admin media library. Binary storage is
//! handled by the API layer through [`fastcart_infrastructure::Storage`].

mod domain;
mod purge;
mod service;

#[cfg(test)]
mod tests;

pub use domain::{
    Asset, AssetFilter, CreateAssetInput, MAX_ALT_LENGTH, MAX_CONTENT_TYPE_LENGTH,
    MAX_FILENAME_LENGTH, MAX_SIZE_BYTES, MAX_STORAGE_KEY_LENGTH, public_url,
};
pub use purge::{
    AssetBlobPurgeRecord, fetch_pending_asset_purge_for_key, fetch_pending_asset_purges,
    insert_asset_blob_purge, mark_asset_purge_failed, mark_asset_purge_processed,
};
pub use service::AssetService;
