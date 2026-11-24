//! Channel authoring, lookup and test sends.

use super::*;

impl NotificationChannelService {
    /// Lists active and inactive non-deleted channels in creation order.
    pub async fn list_channels(
        &self,
        db: &StoreDatabase,
    ) -> Result<Vec<NotificationChannel>, AppError> {
        let rows = sqlx::query_as::<_, ChannelRow>(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM notification_channels
             WHERE deleted_at IS NULL
             ORDER BY created_at ASC, id ASC"
        ))
        .fetch_all(db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(ChannelRow::into_channel).collect()
    }
}

impl NotificationChannelService {
    /// Loads one non-deleted channel.
    pub async fn get_channel(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
    ) -> Result<NotificationChannel, AppError> {
        self.fetch_channel_row(db, id).await?.into_channel()
    }
}

impl NotificationChannelService {
    /// Registers a new channel.
    pub async fn create_channel(
        &self,
        db: &StoreDatabase,
        input: CreateChannelInput,
    ) -> Result<NotificationChannel, AppError> {
        let name = normalize_name(&input.name)?;
        let endpoint_url = self.validate_url(&input.endpoint_url)?;
        let signing_secret = normalize_secret(input.signing_secret)?;
        let events = normalize_events(&input.events)?;
        validate_config(&input.config)?;
        let config = input.config.clone();

        let id = NotificationChannelId::new();
        let now = now_timestamp();
        let mut tx = db.begin().await?;
        sqlx::query(
            "INSERT INTO notification_channels
             (id, kind, name, endpoint_url, signing_secret, events, config, is_active,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.kind.as_str())
        .bind(&name)
        .bind(&endpoint_url)
        .bind(signing_secret.as_deref())
        .bind(serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_owned()))
        .bind(config.to_string())
        .bind(i64::from(input.is_active))
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            db,
            &mut tx,
            "notification_channel.created",
            "notification_channel",
            id.as_str(),
            json!({
                "kind": input.kind.as_str(),
                "events": events,
                "endpoint_url_masked": mask_endpoint_url(&endpoint_url),
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_channel(db, &id).await
    }
}

impl NotificationChannelService {
    /// Updates a channel. Omitted fields keep their current value.
    pub async fn update_channel(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
        input: UpdateChannelInput,
    ) -> Result<NotificationChannel, AppError> {
        let current = self.fetch_channel_row(db, id).await?;

        let name = match input.name {
            Some(name) => normalize_name(&name)?,
            None => current.name.clone(),
        };
        let endpoint_url = match input.endpoint_url {
            Some(url) => self.validate_url(&url)?,
            None => current.endpoint_url.clone(),
        };
        let signing_secret = match input.signing_secret {
            Some(secret) => normalize_secret(secret)?,
            None => current.signing_secret.clone(),
        };
        let events = match input.events {
            Some(events) => normalize_events(&events)?,
            None => current.events()?,
        };
        let config = match input.config {
            Some(config) => {
                validate_config(&config)?;
                config
            }
            None => current.config()?,
        };
        let is_active = input.is_active.unwrap_or(current.is_active != 0);
        let changed = name != current.name
            || endpoint_url != current.endpoint_url
            || signing_secret != current.signing_secret
            || events != current.events()?
            || config != current.config()?
            || is_active != (current.is_active != 0);

        let mut tx = db.begin().await?;
        sqlx::query(
            "UPDATE notification_channels
             SET name = ?, endpoint_url = ?, signing_secret = ?, events = ?,
                 config = ?, is_active = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(&name)
        .bind(&endpoint_url)
        .bind(signing_secret.as_deref())
        .bind(serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_owned()))
        .bind(config.to_string())
        .bind(i64::from(is_active))
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if changed {
            self.append_audit(
                db,
                &mut tx,
                "notification_channel.updated",
                "notification_channel",
                id.as_str(),
                json!({"events": events, "is_active": is_active}),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;

        self.get_channel(db, id).await
    }
}

impl NotificationChannelService {
    /// Soft-deletes a channel. Delivery history is retained.
    pub async fn delete_channel(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
    ) -> Result<(), AppError> {
        let now = now_timestamp();
        let mut tx = db.begin().await?;
        let result = sqlx::query(
            "UPDATE notification_channels
             SET deleted_at = ?, updated_at = ?
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("notification channel", id));
        }
        self.append_audit(
            db,
            &mut tx,
            "notification_channel.deleted",
            "notification_channel",
            id.as_str(),
            json!({}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}

impl NotificationChannelService {
    /// Sends a test message without creating a delivery row.
    pub async fn test_channel(
        &self,
        db: &StoreDatabase,
        id: &NotificationChannelId,
        context: &RenderContext,
    ) -> Result<ChannelTestResult, AppError> {
        let _ = context;
        let channel = self.fetch_channel_row(db, id).await?;
        let kind = ChannelKind::parse(&channel.kind)?;
        if kind != ChannelKind::Feishu {
            return Err(AppError::validation(format!(
                "{} channels cannot be tested",
                kind.as_str()
            )));
        }

        let message = json!({
            "msg_type": "text",
            "content": {"text": "Fastcart 通知测试"}
        });
        let result = feishu::send(
            &self.client,
            &channel.endpoint_url,
            channel.signing_secret.as_deref(),
            &message,
        )
        .await;
        let now = now_timestamp();

        match &result.result {
            Ok(()) => {
                let mut tx = db.begin().await?;
                sqlx::query(
                    "UPDATE notification_channels
                     SET last_success_at = ?, last_error = NULL, updated_at = ?
                     WHERE id = ?",
                )
                .bind(now)
                .bind(now)
                .bind(id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                self.append_audit(
                    db,
                    &mut tx,
                    "notification_channel.tested",
                    "notification_channel",
                    id.as_str(),
                    json!({"ok": true, "status": result.status}),
                )
                .await?;
                tx.commit().await.map_err(database_error)?;
                Ok(ChannelTestResult {
                    ok: true,
                    status: result.status,
                    error: None,
                })
            }
            Err(error) => {
                let mut tx = db.begin().await?;
                sqlx::query(
                    "UPDATE notification_channels SET last_error = ?, updated_at = ?
                     WHERE id = ?",
                )
                .bind(error.to_string())
                .bind(now)
                .bind(id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                self.append_audit(
                    db,
                    &mut tx,
                    "notification_channel.tested",
                    "notification_channel",
                    id.as_str(),
                    json!({"ok": false, "status": result.status}),
                )
                .await?;
                tx.commit().await.map_err(database_error)?;
                Ok(ChannelTestResult {
                    ok: false,
                    status: result.status,
                    error: Some(error.to_string()),
                })
            }
        }
    }
}

fn normalize_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::validation(format!(
            "notification channel name must be 1 to {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(name.to_owned())
}

fn normalize_secret(secret: Option<String>) -> Result<Option<String>, AppError> {
    let Some(secret) = secret else {
        return Ok(None);
    };
    if secret.is_empty() {
        return Ok(None);
    }
    if secret.chars().count() > 512 || secret.chars().any(char::is_control) {
        return Err(AppError::validation(
            "notification channel signing secret must be at most 512 printable characters",
        ));
    }
    Ok(Some(secret))
}

fn normalize_events(events: &[String]) -> Result<Vec<String>, AppError> {
    if events.is_empty() {
        return Err(AppError::validation(
            "at least one notification event subscription is required",
        ));
    }
    if events.len() > MAX_EVENTS {
        return Err(AppError::validation(format!(
            "a notification channel supports at most {MAX_EVENTS} event subscriptions"
        )));
    }
    let supported: BTreeSet<&'static str> = supported_channel_events()
        .iter()
        .map(|(event_type, _, _, _)| *event_type)
        .collect();
    let mut normalized = Vec::with_capacity(events.len());
    for event in events {
        let event = event.trim();
        if !supported.contains(event) {
            return Err(AppError::validation(format!(
                "unsupported notification event: {event}"
            )));
        }
        if !normalized.iter().any(|existing: &String| existing == event) {
            normalized.push(event.to_owned());
        }
    }
    Ok(normalized)
}

fn validate_config(config: &Value) -> Result<(), AppError> {
    if !config.is_object() {
        return Err(AppError::validation(
            "notification channel config must be a JSON object",
        ));
    }
    Ok(())
}
