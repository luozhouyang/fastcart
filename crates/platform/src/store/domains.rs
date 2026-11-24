//! Store domain management.

use super::*;

impl PlatformStore {
    /// Registers a hostname for a store.
    pub async fn add_domain(
        &self,
        store_id: &StoreId,
        hostname: &str,
        is_primary: bool,
    ) -> PlatformResult<StoreDomain> {
        let hostname = normalize_hostname(hostname);
        validate_hostname(&hostname).map_err(PlatformError::InvalidInput)?;
        if self.get_store(store_id).await?.is_none() {
            return Err(PlatformError::StoreNotFound(store_id.to_string()));
        }
        let now = now_timestamp();
        let id = format!("dom_{}", uuid::Uuid::now_v7().simple());
        let mut transaction = self.pool().begin().await?;
        if is_primary {
            sqlx::query("UPDATE store_domains SET is_primary = 0 WHERE store_id = ?")
                .bind(store_id.as_str())
                .execute(&mut *transaction)
                .await?;
        }
        let insert = sqlx::query(
            "INSERT INTO store_domains (id, store_id, hostname, is_primary, verified_at, created_at)
             VALUES (?, ?, ?, ?, NULL, ?)",
        )
        .bind(&id)
        .bind(store_id.as_str())
        .bind(&hostname)
        .bind(i64::from(is_primary))
        .bind(now)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = insert {
            return Err(if unique_violation(&error) {
                PlatformError::DomainConflict(hostname)
            } else {
                error.into()
            });
        }
        if is_primary {
            sqlx::query("UPDATE stores SET primary_domain = ?, updated_at = ? WHERE id = ?")
                .bind(&hostname)
                .bind(now)
                .bind(store_id.as_str())
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(StoreDomain {
            id,
            store_id: store_id.clone(),
            hostname,
            is_primary,
            verified_at: None,
            created_at: now,
        })
    }
}

impl PlatformStore {
    /// Removes a domain. The primary domain cannot be removed directly.
    pub async fn remove_domain(&self, store_id: &StoreId, hostname: &str) -> PlatformResult<()> {
        let hostname = normalize_hostname(hostname);
        let is_primary: Option<i64> = sqlx::query_scalar(
            "SELECT is_primary FROM store_domains WHERE store_id = ? AND hostname = ?",
        )
        .bind(store_id.as_str())
        .bind(&hostname)
        .fetch_optional(self.pool())
        .await?;
        match is_primary {
            None => Err(PlatformError::NotFound(format!(
                "domain not found: {hostname}"
            ))),
            Some(1) => Err(PlatformError::Conflict(
                "cannot remove the primary domain; assign another primary domain first".to_owned(),
            )),
            Some(_) => {
                sqlx::query("DELETE FROM store_domains WHERE store_id = ? AND hostname = ?")
                    .bind(store_id.as_str())
                    .bind(&hostname)
                    .execute(self.pool())
                    .await?;
                Ok(())
            }
        }
    }
}

impl PlatformStore {
    /// Makes an existing domain primary.
    pub async fn set_primary_domain(
        &self,
        store_id: &StoreId,
        hostname: &str,
    ) -> PlatformResult<StoreDomain> {
        let hostname = normalize_hostname(hostname);
        let mut transaction = self.pool().begin().await?;
        let exists: Option<String> =
            sqlx::query_scalar("SELECT id FROM store_domains WHERE store_id = ? AND hostname = ?")
                .bind(store_id.as_str())
                .bind(&hostname)
                .fetch_optional(&mut *transaction)
                .await?;
        let Some(id) = exists else {
            return Err(PlatformError::NotFound(format!(
                "domain not found: {hostname}"
            )));
        };
        sqlx::query("UPDATE store_domains SET is_primary = 0 WHERE store_id = ?")
            .bind(store_id.as_str())
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE store_domains SET is_primary = 1 WHERE id = ?")
            .bind(&id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE stores SET primary_domain = ?, updated_at = ? WHERE id = ?")
            .bind(&hostname)
            .bind(now_timestamp())
            .bind(store_id.as_str())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        let domain = sqlx::query("SELECT created_at, verified_at FROM store_domains WHERE id = ?")
            .bind(&id)
            .fetch_one(self.pool())
            .await?;
        Ok(StoreDomain {
            id,
            store_id: store_id.clone(),
            hostname,
            is_primary: true,
            verified_at: domain.try_get("verified_at")?,
            created_at: domain.try_get("created_at")?,
        })
    }
}

impl PlatformStore {
    /// Lists domains registered to a store.
    pub async fn list_domains(&self, store_id: &StoreId) -> PlatformResult<Vec<StoreDomain>> {
        let rows = sqlx::query(
            "SELECT id, store_id, hostname, is_primary, verified_at, created_at
             FROM store_domains WHERE store_id = ? ORDER BY is_primary DESC, created_at ASC",
        )
        .bind(store_id.as_str())
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(StoreDomain {
                    id: row.try_get("id")?,
                    store_id: StoreId::from_string(row.try_get::<String, _>("store_id")?),
                    hostname: row.try_get("hostname")?,
                    is_primary: row.try_get::<i64, _>("is_primary")? != 0,
                    verified_at: row.try_get("verified_at")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(Into::into)
    }
}

impl PlatformStore {
    /// Marks a domain verified.
    pub async fn verify_domain(&self, store_id: &StoreId, hostname: &str) -> PlatformResult<()> {
        let hostname = normalize_hostname(hostname);
        let result = sqlx::query(
            "UPDATE store_domains SET verified_at = ? WHERE store_id = ? AND hostname = ?",
        )
        .bind(now_timestamp())
        .bind(store_id.as_str())
        .bind(&hostname)
        .execute(self.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Err(PlatformError::NotFound(format!(
                "domain not found: {hostname}"
            )));
        }
        Ok(())
    }
}
