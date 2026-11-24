//! Endpoint authoring, secret rotation and listing.

use super::security::{
    generate_secret, normalize_events, sanitize_audit_url, validate_secret, validate_url,
};
use super::*;

impl WebhookService {
    /// Registers an endpoint and returns its signing secret exactly once.
    pub async fn create_endpoint(
        &self,
        input: CreateWebhookEndpointInput,
    ) -> Result<CreatedWebhookEndpoint, AppError> {
        let audit_url = sanitize_audit_url(&input.url);
        let audit_events = input.events.clone();
        let url = validate_url(&input.url, self.allow_private_networks)?;
        let events = normalize_events(input.events)?;
        let secret = match input.secret {
            Some(secret) => {
                validate_secret(&secret)?;
                secret
            }
            None => generate_secret(),
        };
        let metadata = input.metadata.unwrap_or_default();
        validate_metadata(&metadata)?;
        let stored_secret = self.encrypt_secret(&secret)?;

        let id = WebhookId::new();
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "INSERT INTO webhook_endpoints
             (id, url, secret, is_active, events, metadata, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(&url)
        .bind(&stored_secret)
        .bind(serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_owned()))
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "webhook.created",
            "webhook_endpoint",
            id.as_str(),
            serde_json::json!({"url": audit_url, "events": audit_events}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        Ok(CreatedWebhookEndpoint {
            endpoint: self.get_endpoint(&id).await?,
            secret,
        })
    }
}

impl WebhookService {
    /// Lists endpoints in creation order. Secrets are never returned.
    pub async fn list_endpoints(&self) -> Result<Vec<WebhookEndpoint>, AppError> {
        let rows = sqlx::query_as::<_, EndpointRow>(&format!(
            "SELECT {ENDPOINT_COLUMNS} FROM webhook_endpoints ORDER BY created_at ASC, id ASC"
        ))
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(EndpointRow::into_endpoint).collect()
    }
}

impl WebhookService {
    /// Loads one endpoint.
    pub async fn get_endpoint(&self, id: &WebhookId) -> Result<WebhookEndpoint, AppError> {
        self.fetch_endpoint_row(id).await?.into_endpoint()
    }
}

impl WebhookService {
    /// Updates URL, subscriptions or activation.
    pub async fn update_endpoint(
        &self,
        id: &WebhookId,
        input: UpdateWebhookEndpointInput,
    ) -> Result<WebhookEndpoint, AppError> {
        let current = self.fetch_endpoint_row(id).await?;
        let url = match input.url {
            Some(url) => validate_url(&url, self.allow_private_networks)?,
            None => current.url.clone(),
        };
        let current_events = current.events()?;
        let events = match input.events {
            Some(events) => normalize_events(events)?,
            None => current_events.clone(),
        };
        let is_active = input
            .is_active
            .map_or(current.is_active != 0, |active| active);
        let unchanged =
            url == current.url && events == current_events && is_active == (current.is_active != 0);
        if unchanged {
            // A byte-identical PATCH is a true no-op: no write, no updated_at
            // bump and no audit row.
            return self.get_endpoint(id).await;
        }

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query(
            "UPDATE webhook_endpoints
             SET url = ?, events = ?, is_active = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&url)
        .bind(serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_owned()))
        .bind(i64::from(is_active))
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            // Roll back and report the concurrent delete before writing audit.
            return Err(AppError::not_found("webhook endpoint", id));
        }
        self.append_audit(
            &mut tx,
            "webhook.updated",
            "webhook_endpoint",
            id.as_str(),
            serde_json::json!({
                "url": sanitize_audit_url(&url),
                "events": events,
                "is_active": is_active,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_endpoint(id).await
    }
}

impl WebhookService {
    /// Deletes an endpoint and its delivery history.
    pub async fn delete_endpoint(&self, id: &WebhookId) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query("DELETE FROM webhook_endpoints WHERE id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("webhook endpoint", id));
        }
        self.append_audit(
            &mut tx,
            "webhook.deleted",
            "webhook_endpoint",
            id.as_str(),
            serde_json::Value::Null,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}

impl WebhookService {
    /// Rotates the signing secret and returns it exactly once.
    ///
    /// The previous secret is retained so deliveries can carry both
    /// signatures during the rotation grace window.
    pub async fn rotate_secret(&self, id: &WebhookId) -> Result<String, AppError> {
        let secret = generate_secret();
        let stored_secret = self.encrypt_secret(&secret)?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query(
            "UPDATE webhook_endpoints
             SET secret_previous = secret, secret = ?, secret_rotated_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&stored_secret)
        .bind(now)
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("webhook endpoint", id));
        }
        self.append_audit(
            &mut tx,
            "webhook.secret_rotated",
            "webhook_endpoint",
            id.as_str(),
            serde_json::Value::Null,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(secret)
    }
}
