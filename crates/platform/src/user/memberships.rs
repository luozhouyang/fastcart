//! Store memberships and role checks.

use super::*;

/// Counts active owners in a store other than `membership_id` through `tx`.
async fn store_has_other_active_owner(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    store_id: &StoreId,
    membership_id: &str,
) -> PlatformResult<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT m.id) FROM store_memberships m
         JOIN membership_roles mr ON mr.membership_id = m.id
         JOIN roles r ON r.id = mr.role_id
         WHERE m.store_id = ? AND m.id != ? AND m.status = 'active' AND r.name = 'owner'",
    )
    .bind(store_id.as_str())
    .bind(membership_id)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(count > 0)
}

impl PlatformStore {
    /// Creates or updates a user's membership and replaces assigned roles.
    pub async fn upsert_membership(
        &self,
        user_id: &UserId,
        store_id: &StoreId,
        roles: &[&str],
    ) -> PlatformResult<Membership> {
        let now = now_timestamp();
        // Duplicate role names must not violate the membership_roles primary key
        // and must not produce duplicate audit entries.
        let roles: Vec<&str> = roles
            .iter()
            .copied()
            .collect::<BTreeSet<&str>>()
            .into_iter()
            .collect();
        let mut transaction = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM store_memberships WHERE user_id = ? AND store_id = ?",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;

        let membership_id = match existing {
            Some(id) => id,
            None => {
                let id = format!("mem_{}", uuid::Uuid::now_v7().simple());
                let insert = sqlx::query(
                    "INSERT INTO store_memberships
                     (id, user_id, store_id, status, created_at, updated_at)
                     VALUES (?, ?, ?, 'active', ?, ?)",
                )
                .bind(&id)
                .bind(user_id.as_str())
                .bind(store_id.as_str())
                .bind(now)
                .bind(now)
                .execute(&mut *transaction)
                .await;
                if let Err(error) = insert {
                    return Err(if unique_violation(&error) {
                        PlatformError::Conflict(
                            "membership already exists for this user and store".to_owned(),
                        )
                    } else {
                        error.into()
                    });
                }
                id
            }
        };

        // Resolve every role before mutating so unknown roles abort the
        // transaction without partial role changes.
        let mut role_ids = Vec::with_capacity(roles.len());
        for role in &roles {
            let role_id: Option<String> = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
                .bind(*role)
                .fetch_optional(&mut *transaction)
                .await?;
            let Some(role_id) = role_id else {
                return Err(PlatformError::InvalidInput(format!("unknown role: {role}")));
            };
            role_ids.push(role_id);
        }

        // Removing the owner role from the last active owner would leave the
        // store administrable by nobody. The count runs in the same immediate
        // transaction as the replacement below, so it cannot race.
        if !roles.contains(&"owner") {
            let was_active_owner: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM store_memberships m
                    JOIN membership_roles mr ON mr.membership_id = m.id
                    JOIN roles r ON r.id = mr.role_id
                    WHERE m.id = ? AND m.status = 'active' AND r.name = 'owner'
                )",
            )
            .bind(&membership_id)
            .fetch_one(&mut *transaction)
            .await?;
            if was_active_owner
                && !store_has_other_active_owner(&mut transaction, store_id, &membership_id).await?
            {
                return Err(PlatformError::Conflict(
                    "cannot remove the last owner of a store".to_owned(),
                ));
            }
        }

        sqlx::query("UPDATE store_memberships SET status = 'active', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(&membership_id)
            .execute(&mut *transaction)
            .await?;

        sqlx::query("DELETE FROM membership_roles WHERE membership_id = ?")
            .bind(&membership_id)
            .execute(&mut *transaction)
            .await?;
        for role_id in &role_ids {
            sqlx::query("INSERT INTO membership_roles (membership_id, role_id) VALUES (?, ?)")
                .bind(&membership_id)
                .bind(role_id)
                .execute(&mut *transaction)
                .await?;
        }
        let audit_id = format!("audit_{}", uuid::Uuid::now_v7().simple());
        Self::append_audit_tx(
            &mut transaction,
            &audit_id,
            &NewAudit {
                actor_id: self.audit_actor.clone(),
                store_id: Some(store_id.to_string()),
                action: "membership.upserted".to_owned(),
                resource_type: Some("membership".to_owned()),
                resource_id: Some(membership_id.clone()),
                data: serde_json::json!({"roles": roles, "status": "active"}),
                ip_address: None,
            },
        )
        .await?;
        transaction.commit().await?;
        self.get_membership(user_id, store_id)
            .await?
            .ok_or_else(|| PlatformError::NotFound("membership".to_owned()))
    }
}

impl PlatformStore {
    /// Returns a membership with its roles.
    pub async fn get_membership(
        &self,
        user_id: &UserId,
        store_id: &StoreId,
    ) -> PlatformResult<Option<Membership>> {
        let row = sqlx::query(
            "SELECT id, status, created_at, updated_at FROM store_memberships
             WHERE user_id = ? AND store_id = ?",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_optional(self.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let membership_id: String = row.try_get("id")?;
        let roles: Vec<String> = sqlx::query_scalar(
            "SELECT r.name FROM roles r
             JOIN membership_roles mr ON mr.role_id = r.id
             WHERE mr.membership_id = ? ORDER BY r.name ASC",
        )
        .bind(&membership_id)
        .fetch_all(self.pool())
        .await?;
        Ok(Some(Membership {
            id: membership_id,
            user_id: user_id.clone(),
            store_id: store_id.clone(),
            status: MembershipStatus::from_db(&row.try_get::<String, _>("status")?),
            roles,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        }))
    }
}

impl PlatformStore {
    /// Lists memberships for a store.
    pub async fn list_store_memberships(
        &self,
        store_id: &StoreId,
    ) -> PlatformResult<Vec<Membership>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT user_id FROM store_memberships WHERE store_id = ? ORDER BY created_at ASC",
        )
        .bind(store_id.as_str())
        .fetch_all(self.pool())
        .await?;
        let mut memberships = Vec::with_capacity(ids.len());
        for user_id in ids {
            if let Some(membership) = self
                .get_membership(&UserId::from_string(user_id), store_id)
                .await?
            {
                memberships.push(membership);
            }
        }
        Ok(memberships)
    }
}

impl PlatformStore {
    /// Revokes a membership.
    pub async fn revoke_membership(
        &self,
        user_id: &UserId,
        store_id: &StoreId,
    ) -> PlatformResult<()> {
        let mut transaction = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query(
            "SELECT id, status FROM store_memberships WHERE user_id = ? AND store_id = ?",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Err(PlatformError::NotFound("membership".to_owned()));
        };
        let membership_id: String = row.try_get("id")?;
        let status: String = row.try_get("status")?;

        // The owner check and the revocation share one immediate transaction,
        // so no concurrent revocation can remove the last two owners.
        if status == "active" {
            let is_owner: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM membership_roles mr
                    JOIN roles r ON r.id = mr.role_id
                    WHERE mr.membership_id = ? AND r.name = 'owner'
                )",
            )
            .bind(&membership_id)
            .fetch_one(&mut *transaction)
            .await?;
            if is_owner
                && !store_has_other_active_owner(&mut transaction, store_id, &membership_id).await?
            {
                return Err(PlatformError::Conflict(
                    "cannot remove the last owner of a store".to_owned(),
                ));
            }
        }

        sqlx::query("UPDATE store_memberships SET status = 'revoked', updated_at = ? WHERE id = ?")
            .bind(now_timestamp())
            .bind(&membership_id)
            .execute(&mut *transaction)
            .await?;
        let audit_id = format!("audit_{}", uuid::Uuid::now_v7().simple());
        Self::append_audit_tx(
            &mut transaction,
            &audit_id,
            &NewAudit {
                actor_id: self.audit_actor.clone(),
                store_id: Some(store_id.to_string()),
                action: "membership.removed".to_owned(),
                resource_type: Some("membership".to_owned()),
                resource_id: Some(membership_id),
                data: serde_json::json!({"status": "revoked"}),
                ip_address: None,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

impl PlatformStore {
    /// Resolves the effective permission set for a user within a store.
    pub async fn permissions_for(
        &self,
        user_id: &UserId,
        store_id: &StoreId,
    ) -> PlatformResult<BTreeSet<String>> {
        let permissions: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT p.name FROM permissions p
             JOIN role_permissions rp ON rp.permission_id = p.id
             JOIN membership_roles mr ON mr.role_id = rp.role_id
             JOIN store_memberships m ON m.id = mr.membership_id
             WHERE m.user_id = ? AND m.store_id = ? AND m.status = 'active'",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_all(self.pool())
        .await?;
        Ok(permissions.into_iter().collect())
    }
}

impl PlatformStore {
    /// Returns true when an active membership exists.
    pub async fn is_member(&self, user_id: &UserId, store_id: &StoreId) -> PlatformResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM store_memberships
             WHERE user_id = ? AND store_id = ? AND status = 'active'",
        )
        .bind(user_id.as_str())
        .bind(store_id.as_str())
        .fetch_one(self.pool())
        .await?;
        Ok(count > 0)
    }
}

impl PlatformStore {
    /// Returns true when the user holds any of `roles` in an active store.
    ///
    /// Platform-level actions that are not tied to a single store use this to
    /// require existing store ownership instead of accepting any authenticated
    /// user.
    pub async fn has_any_store_role(
        &self,
        user_id: &UserId,
        roles: &[&str],
    ) -> PlatformResult<bool> {
        if roles.is_empty() {
            return Ok(false);
        }
        let placeholders = vec!["?"; roles.len()].join(", ");
        let sql = format!(
            "SELECT COUNT(*) FROM store_memberships m
             JOIN membership_roles mr ON mr.membership_id = m.id
             JOIN roles r ON r.id = mr.role_id
             WHERE m.user_id = ? AND m.status = 'active' AND r.name IN ({placeholders})"
        );
        let mut query = sqlx::query_scalar::<_, i64>(&sql).bind(user_id.as_str());
        for role in roles {
            query = query.bind(*role);
        }
        let count: i64 = query.fetch_one(self.pool()).await?;
        Ok(count > 0)
    }
}

impl PlatformStore {
    /// Returns the number of registered stores.
    pub async fn store_count(&self) -> PlatformResult<i64> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores")
            .fetch_one(self.pool())
            .await?;
        Ok(count)
    }
}

impl PlatformStore {
    /// Lists available system roles.
    pub async fn list_roles(&self) -> PlatformResult<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT name, COALESCE(description, '') AS description FROM roles ORDER BY name",
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get("name")?, row.try_get("description")?)))
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(Into::into)
    }
}

impl PlatformStore {
    /// Lists permission names.
    pub async fn list_permissions(&self) -> PlatformResult<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT name FROM permissions ORDER BY name ASC")
                .fetch_all(self.pool())
                .await?,
        )
    }
}
