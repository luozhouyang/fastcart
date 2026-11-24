use super::*;

#[utoipa::path(
    post,
    path = "/content/pages/{id}/preview-token",
    tag = "Admin",
    summary = "Issue a draft preview token for a page",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = PreviewTokenResponse))
)]
pub(super) async fn create_preview_token(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<(
    [(axum::http::HeaderName, axum::http::HeaderValue); 1],
    Json<PreviewTokenResponse>,
)> {
    authorize(&actor, "themes.read")?;
    let Some(secret) = context
        .config
        .preview_secret
        .as_deref()
        .filter(|value| valid_secret(value))
    else {
        return Err(ApiError::new(AppError::NotImplemented(
            "page preview is not configured".to_owned(),
        )));
    };
    let page_id = PageId::from_string(id);
    // Drafts are allowed; archived and deleted pages are rejected because the
    // preview surface refuses to render them.
    let page = ContentService::new(db.clone()).get_page(&page_id).await?;
    if page.status == PageStatus::Archived {
        return Err(ApiError::not_found("page"));
    }
    let issued = issue_preview_token(
        secret,
        db.store_id().as_str(),
        page_id.as_str(),
        now_timestamp(),
    )?;
    // Never record the token itself: it is a bearer credential valid for the
    // whole TTL, so the audit entry only records that one was issued.
    record_store_audit(
        &context,
        &actor,
        "page.preview_token.issued",
        "page",
        page_id.as_str(),
        serde_json::json!({ "expires_at": issued.expires_at, "jti": issued.jti }),
    )
    .await;
    // A response containing a bearer token must not be retained by browsers,
    // proxies or shared caches.
    let headers = [(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    )];
    Ok((
        headers,
        Json(PreviewTokenResponse {
            token: issued.token,
            expires_at: issued.expires_at,
            jti: issued.jti,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/content/pages/{id}/preview-token/revoke",
    tag = "Admin",
    summary = "Revoke a draft preview token",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = RevokePreviewTokenRequest,
    responses((status = 204))
)]
pub(super) async fn revoke_preview_token(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<RevokePreviewTokenRequest>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    let page_id = PageId::from_string(id);
    let jti = input.jti.trim();
    if jti.is_empty() || jti.len() > 128 {
        return Err(ApiError::validation(
            "jti must be between 1 and 128 characters",
        ));
    }
    // Only let an administrator of this store revoke against a page they can
    // load; the jti itself is opaque and carries no page authority.
    ContentService::new(db.clone()).get_page(&page_id).await?;
    let now = now_timestamp();
    let store_id = db.store_id().as_str().to_owned();
    let asset_authorization_key = crate::routes::preview::asset_authorization_key(&store_id, jti);
    let mut tx = db.begin().await.map_err(AppError::from)?;
    sqlx::query("DELETE FROM preview_token_revocations WHERE expires_at <= ?")
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(fastcart_infrastructure::database_error)?;
    sqlx::query(
        "INSERT OR REPLACE INTO preview_token_revocations (jti, page_id, expires_at) \
         VALUES (?, ?, ?)",
    )
    .bind(jti)
    .bind(page_id.as_str())
    .bind(now.saturating_add(crate::routes::preview::PREVIEW_TOKEN_TTL_SECONDS))
    .execute(&mut *tx)
    .await
    .map_err(fastcart_infrastructure::database_error)?;
    if let Some(user_id) = actor.user_id.as_ref() {
        fastcart_infrastructure::AuditActor::new(user_id.to_string(), store_id)
            .append(
                &mut *tx,
                "page.preview_token.revoked",
                "page",
                page_id.as_str(),
                serde_json::json!({ "jti": jti }),
            )
            .await
            .map_err(fastcart_infrastructure::database_error)?;
    }
    tx.commit()
        .await
        .map_err(fastcart_infrastructure::database_error)?;
    context
        .preview_asset_authorizations
        .invalidate(&asset_authorization_key)
        .await;
    Ok(StatusCode::NO_CONTENT)
}
