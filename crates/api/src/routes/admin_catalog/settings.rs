use super::*;

/// Batch settings update body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct SettingsRequest {
    values: BTreeMap<String, String>,
}

/// Single setting update body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct SettingRequest {
    value: String,
}

#[utoipa::path(
    get,
    path = "/settings",
    tag = "Admin",
    summary = "Get all store settings",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = BTreeMap<String, String>))
)]
pub(super) async fn get_settings(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<BTreeMap<String, String>>> {
    authorize(&actor, "settings.write")?;
    let settings = StoreService::new(db).get_all_settings().await?;
    Ok(Json(settings))
}

#[utoipa::path(
    put,
    path = "/settings",
    tag = "Admin",
    summary = "Update multiple store settings",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = SettingsRequest,
    responses((status = 200, body = BTreeMap<String, String>))
)]
pub(super) async fn update_settings(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<SettingsRequest>,
) -> ApiResult<Json<BTreeMap<String, String>>> {
    authorize(&actor, "settings.write")?;
    let service = StoreService::new(db);
    let values = request.values.into_iter().collect::<Vec<_>>();
    service.set_settings(&values).await?;
    let settings = service.get_all_settings().await?;
    Ok(Json(settings))
}

#[utoipa::path(
    get,
    path = "/settings/{key}",
    tag = "Admin",
    summary = "Get a store setting",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = serde_json::Value))
)]
pub(super) async fn get_setting(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(key): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&actor, "settings.write")?;
    let value = StoreService::new(db)
        .get_setting(&key)
        .await?
        .ok_or_else(|| ApiError::new(AppError::not_found("setting", &key)))?;
    Ok(Json(serde_json::json!({ "key": key, "value": value })))
}

#[utoipa::path(
    put,
    path = "/settings/{key}",
    tag = "Admin",
    summary = "Set a store setting",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = SettingRequest,
    responses((status = 200, body = serde_json::Value))
)]
pub(super) async fn set_setting(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(key): Path<String>,
    ApiJson(request): ApiJson<SettingRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&actor, "settings.write")?;
    let service = StoreService::new(db);
    service.set_setting(&key, &request.value).await?;
    Ok(Json(
        serde_json::json!({ "key": key, "value": request.value }),
    ))
}

#[utoipa::path(
    delete,
    path = "/settings/{key}",
    tag = "Admin",
    summary = "Delete a store setting",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
pub(super) async fn delete_setting(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(key): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "settings.write")?;
    if !StoreService::new(db).delete_setting(&key).await? {
        return Err(ApiError::new(AppError::not_found("setting", &key)));
    }
    Ok(StatusCode::NO_CONTENT)
}
