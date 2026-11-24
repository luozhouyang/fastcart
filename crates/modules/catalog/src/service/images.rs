//! Product images.

use std::collections::HashSet;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, ImageId, now_timestamp, validate_length, validate_metadata};

use crate::domain::{CreateImageInput, ProductImage};

use super::shared::*;
/// Full CRUD over the catalog aggregate.
use super::*;
impl CatalogService {
    /// Adds an image to a product.
    pub async fn add_image(
        &self,
        product_id: &str,
        input: CreateImageInput,
    ) -> Result<ProductImage, AppError> {
        let url = input.url.trim();
        validate_length(url, "image url", MAX_URL_LENGTH)?;
        validate_metadata(&input.metadata)?;
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        ensure_product_exists_tx(&mut tx, product_id).await?;
        if let Some(variant_id) = input.variant_id.as_deref() {
            ensure_variant_belongs_tx(&mut tx, product_id, variant_id).await?;
        }
        let rank = match input.rank {
            Some(rank) => rank,
            None => sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(rank), -1) + 1 FROM product_images WHERE product_id = ?",
            )
            .bind(product_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?,
        };
        let id = ImageId::new().as_str().to_owned();
        sqlx::query(
            "INSERT INTO product_images \
             (id, product_id, variant_id, url, alt, rank, metadata, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(product_id)
        .bind(&input.variant_id)
        .bind(url)
        .bind(normalize_optional(input.alt))
        .bind(rank)
        .bind(input.metadata.to_json_string())
        .bind(now_timestamp())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        self.get_image(&id).await
    }

    /// Loads a product image.
    pub async fn get_image(&self, id: &str) -> Result<ProductImage, AppError> {
        let row = sqlx::query("SELECT * FROM product_images WHERE id = ?")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("image", id))?;
        image_from_row(&row)
    }

    /// Lists a product's images ordered by rank.
    pub async fn list_images(&self, product_id: &str) -> Result<Vec<ProductImage>, AppError> {
        let rows = sqlx::query(
            "SELECT * FROM product_images WHERE product_id = ? \
             ORDER BY rank ASC, created_at ASC, id ASC",
        )
        .bind(product_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let mut images = Vec::with_capacity(rows.len());
        for row in &rows {
            images.push(image_from_row(row)?);
        }
        Ok(images)
    }

    /// Removes a product image.
    pub async fn remove_image(&self, id: &str) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM product_images WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("image", id));
        }
        Ok(())
    }

    /// Reorders a product's images according to the supplied identifier order.
    pub async fn reorder_images(
        &self,
        product_id: &str,
        image_ids: &[String],
    ) -> Result<Vec<ProductImage>, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let existing_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM product_images WHERE product_id = ?")
                .bind(product_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(database_error)?;
        let existing_ids: HashSet<&str> = existing_ids.iter().map(String::as_str).collect();
        let mut seen = HashSet::new();
        for image_id in image_ids {
            if !seen.insert(image_id.as_str()) {
                return Err(AppError::validation(format!(
                    "duplicate image id in reorder request: {image_id}"
                )));
            }
            if !existing_ids.contains(image_id.as_str()) {
                return Err(AppError::not_found("image", image_id));
            }
        }
        for (rank, image_id) in image_ids.iter().enumerate() {
            let rank = i64::try_from(rank).map_err(|_| AppError::validation("rank overflow"))?;
            let updated =
                sqlx::query("UPDATE product_images SET rank = ? WHERE id = ? AND product_id = ?")
                    .bind(rank)
                    .bind(image_id)
                    .bind(product_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
            if updated.rows_affected() == 0 {
                return Err(AppError::not_found("image", image_id));
            }
        }
        tx.commit().await.map_err(database_error)?;
        self.list_images(product_id).await
    }
}
