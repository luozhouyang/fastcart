use super::*;

/// Query identifying one translation row.
#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct TranslationQuery {
    /// Resource type: product, category, collection, page or navigation.
    resource_type: String,
    /// Translated resource identifier.
    resource_id: String,
    /// Translation locale; when omitted, GET lists every locale of the resource.
    #[serde(default)]
    locale: Option<String>,
}

#[utoipa::path(
    get,
    path = "/content/translations",
    tag = "Admin",
    summary = "List the translations of a content resource",
    security(("adminSession" = []), ("apiToken" = [])),
    params(TranslationQuery),
    responses((status = 200, body = Vec<Translation>))
)]
pub(super) async fn list_translations(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<TranslationQuery>,
) -> ApiResult<Json<Vec<Translation>>> {
    authorize(&actor, "settings.read")?;
    let service = ContentService::new(db);
    let translations = match query.locale.as_deref() {
        Some(locale) => vec![
            service
                .get_translation(&query.resource_type, &query.resource_id, locale)
                .await?,
        ],
        None => {
            service
                .list_translations(&query.resource_type, &query.resource_id)
                .await?
        }
    };
    Ok(Json(translations))
}

#[utoipa::path(
    put,
    path = "/content/translations",
    tag = "Admin",
    summary = "Create or replace one translation",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpsertTranslationRequest,
    params(TranslationQuery),
    responses((status = 200, body = Translation))
)]
pub(super) async fn upsert_translation(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<TranslationQuery>,
    ApiJson(request): ApiJson<UpsertTranslationRequest>,
) -> ApiResult<Json<Translation>> {
    authorize(&actor, "settings.write")?;
    let locale = query
        .locale
        .ok_or_else(|| ApiError::validation("locale is required when writing a translation"))?;
    let translation = ContentService::new(db)
        .with_audit_actor(actor_id(&actor))
        .upsert_translation(UpsertTranslationInput {
            resource_type: query.resource_type,
            resource_id: query.resource_id,
            locale,
            fields: request.fields,
        })
        .await?;
    Ok(Json(translation))
}

#[utoipa::path(
    delete,
    path = "/content/translations",
    tag = "Admin",
    summary = "Delete one translation",
    security(("adminSession" = []), ("apiToken" = [])),
    params(TranslationQuery),
    responses((status = 204))
)]
pub(super) async fn delete_translation(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<TranslationQuery>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "settings.write")?;
    let locale = query
        .locale
        .ok_or_else(|| ApiError::validation("locale is required when deleting a translation"))?;
    ContentService::new(db)
        .with_audit_actor(actor_id(&actor))
        .delete_translation(&query.resource_type, &query.resource_id, &locale)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
