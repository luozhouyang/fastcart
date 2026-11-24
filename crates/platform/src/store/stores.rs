//! Store records and lifecycle.

use super::*;

impl PlatformStore {
    /// Creates a store and its primary domain in one transaction.
    pub async fn create_store(&self, input: CreateStoreInput) -> PlatformResult<Store> {
        input.validate()?;
        let slug = input.slug.clone();
        let hostname = normalize_hostname(&input.primary_domain);
        let locale = Locale::new(input.locale.clone().unwrap_or_else(|| "en-US".to_owned()))?;
        let currency = CurrencyCode::new(
            input
                .currency_code
                .clone()
                .unwrap_or_else(|| "USD".to_owned()),
        )?;
        let timezone = input.timezone.clone().unwrap_or_else(|| "UTC".to_owned());

        let id = StoreId::new();
        let now = now_timestamp();
        let database_key = id.as_str().to_owned();
        let mut transaction = self.pool().begin().await?;

        let existing_slug: Option<String> =
            sqlx::query_scalar("SELECT id FROM stores WHERE slug = ?")
                .bind(&slug)
                .fetch_optional(&mut *transaction)
                .await?;
        if existing_slug.is_some() {
            return Err(PlatformError::SlugConflict(slug));
        }
        let existing_domain: Option<String> =
            sqlx::query_scalar("SELECT store_id FROM store_domains WHERE hostname = ?")
                .bind(&hostname)
                .fetch_optional(&mut *transaction)
                .await?;
        if existing_domain.is_some() {
            return Err(PlatformError::DomainConflict(hostname));
        }

        let insert = sqlx::query(
            "INSERT INTO stores
             (id, name, slug, status, database_key, primary_domain, locale, currency_code,
              timezone, metadata, created_at, updated_at)
             VALUES (?, ?, ?, 'provisioning', ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.name.trim())
        .bind(&slug)
        .bind(&database_key)
        .bind(&hostname)
        .bind(locale.as_str())
        .bind(currency.as_str())
        .bind(&timezone)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = insert {
            return Err(if unique_violation(&error) {
                PlatformError::SlugConflict(slug)
            } else {
                error.into()
            });
        }

        let domain_id = format!("dom_{}", uuid::Uuid::now_v7().simple());
        sqlx::query(
            "INSERT INTO store_domains (id, store_id, hostname, is_primary, verified_at, created_at)
             VALUES (?, ?, ?, 1, NULL, ?)",
        )
        .bind(domain_id)
        .bind(id.as_str())
        .bind(&hostname)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        self.get_store(&id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))
    }
}

impl PlatformStore {
    /// Fetches a store by identifier.
    pub async fn get_store(&self, id: &StoreId) -> PlatformResult<Option<Store>> {
        let row = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {STORE_COLUMNS} FROM stores WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(StoreRow::into_store))
    }
}

impl PlatformStore {
    /// Fetches a store by slug.
    pub async fn get_store_by_slug(&self, slug: &str) -> PlatformResult<Option<Store>> {
        let row = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {STORE_COLUMNS} FROM stores WHERE slug = ?"
        ))
        .bind(slug)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(StoreRow::into_store))
    }
}

impl PlatformStore {
    /// Resolves a store by hostname.
    pub async fn get_store_by_domain(&self, hostname: &str) -> PlatformResult<Option<Store>> {
        let hostname = normalize_hostname(hostname);
        let row = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {} FROM stores s
             JOIN store_domains d ON d.store_id = s.id
             WHERE d.hostname = ?",
            STORE_COLUMNS
                .split(',')
                .map(|column| format!("s.{}", column.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .bind(hostname)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(StoreRow::into_store))
    }
}

impl PlatformStore {
    /// Lists stores with offset pagination.
    pub async fn list_stores(&self, pagination: Pagination) -> PlatformResult<PageResult<Store>> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores")
            .fetch_one(self.pool())
            .await?;
        let rows = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {STORE_COLUMNS} FROM stores ORDER BY created_at ASC LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.pool())
        .await?;
        Ok(PageResult::new(
            rows.into_iter().map(StoreRow::into_store).collect(),
            total,
            pagination,
        ))
    }
}

impl PlatformStore {
    /// Lists stores a user is an active member of.
    pub async fn list_stores_for_user(
        &self,
        user_id: &fastcart_kernel::UserId,
        pagination: Pagination,
    ) -> PlatformResult<PageResult<Store>> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM store_memberships WHERE user_id = ? AND status = 'active'",
        )
        .bind(user_id.as_str())
        .fetch_one(self.pool())
        .await?;
        let columns = STORE_COLUMNS
            .split(',')
            .map(|column| format!("s.{}", column.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = sqlx::query_as::<_, StoreRow>(&format!(
            "SELECT {columns} FROM stores s
             JOIN store_memberships m ON m.store_id = s.id
             WHERE m.user_id = ? AND m.status = 'active'
             ORDER BY s.created_at ASC LIMIT ? OFFSET ?"
        ))
        .bind(user_id.as_str())
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.pool())
        .await?;
        Ok(PageResult::new(
            rows.into_iter().map(StoreRow::into_store).collect(),
            total,
            pagination,
        ))
    }
}

impl PlatformStore {
    /// Updates mutable store attributes.
    pub async fn update_store(
        &self,
        id: &StoreId,
        input: UpdateStoreInput,
    ) -> PlatformResult<Store> {
        let current = self
            .get_store(id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))?;
        if let Some(name) = input.name.as_deref()
            && (name.trim().is_empty() || name.chars().count() > 120)
        {
            return Err(PlatformError::InvalidInput(
                "store name must be between 1 and 120 characters".to_owned(),
            ));
        }
        if let Some(locale) = input.locale.as_deref() {
            Locale::new(locale)?;
        }
        if let Some(currency) = input.currency_code.as_deref() {
            CurrencyCode::new(currency)?;
        }
        let name = input.name.unwrap_or(current.name);
        let locale = input.locale.unwrap_or(current.locale);
        let currency_code = input.currency_code.unwrap_or(current.currency_code);
        let timezone = input.timezone.unwrap_or(current.timezone);
        let metadata = input.metadata.unwrap_or(current.metadata);
        sqlx::query(
            "UPDATE stores SET name = ?, locale = ?, currency_code = ?, timezone = ?,
             metadata = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(locale)
        .bind(currency_code)
        .bind(timezone)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.pool())
        .await?;
        self.get_store(id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))
    }
}

impl PlatformStore {
    /// Applies a validated lifecycle transition.
    pub async fn update_status(&self, id: &StoreId, status: StoreStatus) -> PlatformResult<Store> {
        let current = self
            .get_store(id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))?;
        if !current.status.can_transition_to(status) {
            return Err(PlatformError::Conflict(format!(
                "cannot transition store {} from {} to {}",
                id, current.status, status
            )));
        }
        sqlx::query("UPDATE stores SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(now_timestamp())
            .bind(id.as_str())
            .execute(self.pool())
            .await?;
        self.get_store(id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(id.to_string()))
    }
}
