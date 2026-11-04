//! Platform API: first-run setup, authentication, stores, domains and members.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::{delete, get, post};
use fastcart_http::{AdminActor, AdminAuthState, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_kernel::{ActorContext, AppError, StoreId, UserId};
use fastcart_platform::{CreateStoreInput, CreateUserInput, NewAudit, UpdateStoreInput};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::auth::{login, logout, me, setup};
use super::{authorize_store, require_owner};
use crate::dto::{
    ApiTokenDto, CreateApiTokenRequest, CreateStoreRequest, CreateUserRequest,
    CreatedApiTokenResponse, DeleteStoreRequest, DomainDto, DomainRequest, MembershipDto,
    MembershipRequest, PageResponse, PrimaryDomainRequest, ProvisioningResponse, RoleDto, StoreDto,
    UpdateStoreRequest, UserDto,
};
use crate::state::AppContext;

/// Builds the platform router with authentication applied to protected routes.
pub fn router(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    let public = Router::new()
        .route("/auth/login", post(login))
        .route("/auth/setup", post(setup));

    let authenticated = Router::new()
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/roles", get(list_roles))
        .route("/users", post(create_user))
        .route("/api-tokens", get(list_api_tokens).post(create_api_token))
        .route("/api-tokens/{token_id}", delete(revoke_api_token))
        .route("/stores", get(list_stores).post(create_store))
        .route("/stores/{store_id}", get(get_store).patch(update_store))
        .route("/stores/{store_id}/activate", post(activate_store))
        .route("/stores/{store_id}/suspend", post(suspend_store))
        .route("/stores/{store_id}/resume", post(resume_store))
        .route("/stores/{store_id}/provision", post(provision_store))
        .route("/stores/{store_id}/delete", post(delete_store))
        .route(
            "/stores/{store_id}/domains",
            get(list_domains).post(add_domain),
        )
        .route(
            "/stores/{store_id}/domains/primary",
            post(set_primary_domain),
        )
        .route(
            "/stores/{store_id}/domains/{hostname}",
            delete(remove_domain),
        )
        .route(
            "/stores/{store_id}/memberships",
            get(list_memberships).post(upsert_membership),
        )
        .route(
            "/stores/{store_id}/memberships/{user_id}",
            delete(remove_membership),
        )
        .route_layer(middleware::from_fn_with_state(
            AdminAuthState {
                auth: context.auth.clone(),
                require_store: false,
            },
            fastcart_http::admin_auth_middleware,
        ));

    public.merge(authenticated)
}

/// Builds the documented platform routes.
///
/// Shared authentication handlers are documented by [`super::auth`] and are
/// not repeated here.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_roles))
        .routes(routes!(create_user))
        .routes(routes!(list_api_tokens, create_api_token))
        .routes(routes!(revoke_api_token))
        .routes(routes!(list_stores, create_store))
        .routes(routes!(get_store, update_store))
        .routes(routes!(activate_store))
        .routes(routes!(suspend_store))
        .routes(routes!(resume_store))
        .routes(routes!(provision_store))
        .routes(routes!(delete_store))
        .routes(routes!(list_domains, add_domain))
        .routes(routes!(set_primary_domain))
        .routes(routes!(remove_domain))
        .routes(routes!(list_memberships, upsert_membership))
        .routes(routes!(remove_membership))
}

#[utoipa::path(
    get,
    path = "/stores",
    tag = "Platform",
    summary = "List stores visible to the actor",
    params(PaginationQuery),
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = PageResponse<StoreDto>))
)]
async fn list_stores(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<StoreDto>>> {
    let user_id = actor.require_user().map_err(ApiError::new)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = context
        .platform
        .list_stores_for_user(user_id, pagination)
        .await?;
    Ok(Json(PageResponse::from_page(
        page.map(|store| StoreDto::from(&store)),
    )))
}

#[utoipa::path(
    post,
    path = "/stores",
    tag = "Platform",
    summary = "Create and provision a store",
    request_body = CreateStoreRequest,
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 201, body = StoreDto))
)]
async fn create_store(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    ApiJson(request): ApiJson<CreateStoreRequest>,
) -> ApiResult<(StatusCode, Json<StoreDto>)> {
    let user_id = actor.require_user().map_err(ApiError::new)?.clone();
    let mut input = CreateStoreInput::new(request.name, request.slug, request.primary_domain);
    input.locale = request.locale;
    input.currency_code = request.currency_code;
    input.timezone = request.timezone;
    let store = context.platform.create_store(input).await?;

    if let Err(error) = context
        .provisioning
        .provision(&store.id, Some(&user_id))
        .await
    {
        tracing::warn!(store_id = %store.id, error = %error, "store provisioning failed");
    }
    context.resolver.invalidate_all().await;
    let store = context
        .platform
        .get_store(&store.id)
        .await?
        .ok_or_else(|| ApiError::new(AppError::not_found("store", &store.id)))?;
    audit(
        &context,
        &actor,
        Some(&store.id),
        "store.created",
        Some("store"),
        Some(store.id.as_str()),
    )
    .await;
    Ok((StatusCode::CREATED, Json(StoreDto::from(&store))))
}

#[utoipa::path(
    get,
    path = "/stores/{store_id}",
    tag = "Platform",
    summary = "Get a store",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = StoreDto))
)]
async fn get_store(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
) -> ApiResult<Json<StoreDto>> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, None).await?;
    let store = context
        .platform
        .get_store(&store_id)
        .await?
        .ok_or_else(|| ApiError::new(AppError::not_found("store", &store_id)))?;
    Ok(Json(StoreDto::from(&store)))
}

#[utoipa::path(
    patch,
    path = "/stores/{store_id}",
    tag = "Platform",
    summary = "Update a store",
    request_body = UpdateStoreRequest,
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = StoreDto))
)]
async fn update_store(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
    ApiJson(request): ApiJson<UpdateStoreRequest>,
) -> ApiResult<Json<StoreDto>> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, Some("settings.write")).await?;
    let store = context
        .platform
        .update_store(
            &store_id,
            UpdateStoreInput {
                name: request.name,
                locale: request.locale,
                currency_code: request.currency_code,
                timezone: request.timezone,
                metadata: None,
            },
        )
        .await?;
    audit(
        &context,
        &actor,
        Some(&store_id),
        "store.updated",
        Some("store"),
        Some(store_id.as_str()),
    )
    .await;
    Ok(Json(StoreDto::from(&store)))
}

#[utoipa::path(
    post,
    path = "/stores/{store_id}/activate",
    tag = "Platform",
    summary = "Activate a store",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = StoreDto))
)]
async fn activate_store(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
) -> ApiResult<Json<StoreDto>> {
    lifecycle(&context, &actor, &store_id, "activate").await
}

#[utoipa::path(
    post,
    path = "/stores/{store_id}/suspend",
    tag = "Platform",
    summary = "Suspend a store",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = StoreDto))
)]
async fn suspend_store(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
) -> ApiResult<Json<StoreDto>> {
    lifecycle(&context, &actor, &store_id, "suspend").await
}

#[utoipa::path(
    post,
    path = "/stores/{store_id}/resume",
    tag = "Platform",
    summary = "Resume a suspended store",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = StoreDto))
)]
async fn resume_store(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
) -> ApiResult<Json<StoreDto>> {
    lifecycle(&context, &actor, &store_id, "resume").await
}

async fn lifecycle(
    context: &Arc<AppContext>,
    actor: &ActorContext,
    store_id: &str,
    action: &str,
) -> ApiResult<Json<StoreDto>> {
    let store_id = parse_store_id(store_id)?;
    authorize_store(&context.platform, actor, &store_id, Some("settings.write")).await?;
    let store = match action {
        "activate" | "resume" => context.provisioning.resume(&store_id).await?,
        "suspend" => context.provisioning.suspend(&store_id).await?,
        _ => {
            return Err(ApiError::new(AppError::Internal(
                "unknown lifecycle action".into(),
            )));
        }
    };
    context.resolver.invalidate_all().await;
    audit(
        context,
        actor,
        Some(&store_id),
        &format!("store.{action}"),
        Some("store"),
        Some(store_id.as_str()),
    )
    .await;
    Ok(Json(StoreDto::from(&store)))
}

#[utoipa::path(
    post,
    path = "/stores/{store_id}/provision",
    tag = "Platform",
    summary = "Provision store infrastructure",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ProvisioningResponse))
)]
async fn provision_store(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
) -> ApiResult<Json<ProvisioningResponse>> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, Some("settings.write")).await?;
    let owner = actor.require_user().map_err(ApiError::new)?.clone();
    let report = context
        .provisioning
        .provision(&store_id, Some(&owner))
        .await?;
    context.resolver.invalidate_all().await;
    Ok(Json(ProvisioningResponse {
        store: StoreDto::from(&report.store),
        steps: report.steps,
        already_active: report.already_active,
    }))
}

#[utoipa::path(
    post,
    path = "/stores/{store_id}/delete",
    tag = "Platform",
    summary = "Delete a store",
    request_body = DeleteStoreRequest,
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = StoreDto))
)]
async fn delete_store(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
    ApiJson(request): ApiJson<DeleteStoreRequest>,
) -> ApiResult<Json<StoreDto>> {
    let store_id = parse_store_id(&store_id)?;
    require_owner(&context.platform, &actor, &store_id).await?;
    if request.purge_data && !request.backup {
        return Err(ApiError::validation(
            "purge_data requires backup to be enabled",
        ));
    }
    context.provisioning.start_deletion(&store_id).await?;
    let store = context
        .provisioning
        .complete_deletion(
            &store_id,
            fastcart_platform::DeleteStoreOptions {
                backup_dir: request.backup.then(|| context.config.backup_dir.clone()),
                purge_data: request.purge_data,
            },
        )
        .await?;
    context.resolver.invalidate_all().await;
    audit(
        &context,
        &actor,
        Some(&store_id),
        "store.deleted",
        Some("store"),
        Some(store_id.as_str()),
    )
    .await;
    Ok(Json(StoreDto::from(&store)))
}

#[utoipa::path(
    get,
    path = "/stores/{store_id}/domains",
    tag = "Platform",
    summary = "List store domains",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<DomainDto>))
)]
async fn list_domains(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
) -> ApiResult<Json<Vec<DomainDto>>> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, None).await?;
    let domains = context.platform.list_domains(&store_id).await?;
    Ok(Json(domains.iter().map(DomainDto::from).collect()))
}

#[utoipa::path(
    post,
    path = "/stores/{store_id}/domains",
    tag = "Platform",
    summary = "Register a store domain",
    request_body = DomainRequest,
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 201, body = DomainDto))
)]
async fn add_domain(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
    ApiJson(request): ApiJson<DomainRequest>,
) -> ApiResult<(StatusCode, Json<DomainDto>)> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, Some("settings.write")).await?;
    let domain = context
        .platform
        .add_domain(&store_id, &request.hostname, request.is_primary)
        .await?;
    context.resolver.invalidate_all().await;
    Ok((StatusCode::CREATED, Json(DomainDto::from(&domain))))
}

#[utoipa::path(
    delete,
    path = "/stores/{store_id}/domains/{hostname}",
    tag = "Platform",
    summary = "Remove a store domain",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn remove_domain(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path((store_id, hostname)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, Some("settings.write")).await?;
    context.platform.remove_domain(&store_id, &hostname).await?;
    context.resolver.invalidate_hostname(&hostname).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/stores/{store_id}/domains/primary",
    tag = "Platform",
    summary = "Set the primary store domain",
    request_body = PrimaryDomainRequest,
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = DomainDto))
)]
async fn set_primary_domain(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
    ApiJson(request): ApiJson<PrimaryDomainRequest>,
) -> ApiResult<Json<DomainDto>> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, Some("settings.write")).await?;
    let domain = context
        .platform
        .set_primary_domain(&store_id, &request.hostname)
        .await?;
    context.resolver.invalidate_all().await;
    Ok(Json(DomainDto::from(&domain)))
}

#[utoipa::path(
    get,
    path = "/stores/{store_id}/memberships",
    tag = "Platform",
    summary = "List store memberships",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<MembershipDto>))
)]
async fn list_memberships(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
) -> ApiResult<Json<Vec<MembershipDto>>> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, Some("users.read")).await?;
    let memberships = context.platform.list_store_memberships(&store_id).await?;
    Ok(Json(memberships.iter().map(MembershipDto::from).collect()))
}

#[utoipa::path(
    post,
    path = "/stores/{store_id}/memberships",
    tag = "Platform",
    summary = "Create or update a store membership",
    request_body = MembershipRequest,
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 201, body = MembershipDto))
)]
async fn upsert_membership(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(store_id): Path<String>,
    ApiJson(request): ApiJson<MembershipRequest>,
) -> ApiResult<(StatusCode, Json<MembershipDto>)> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, Some("users.write")).await?;
    if request.roles.is_empty() {
        return Err(ApiError::validation("at least one role is required"));
    }
    let user_id = UserId::from_string(request.user_id);
    if context.platform.get_user(&user_id).await?.is_none() {
        return Err(ApiError::new(AppError::not_found("user", &user_id)));
    }
    let roles: Vec<&str> = request.roles.iter().map(String::as_str).collect();
    let membership = context
        .platform
        .upsert_membership(&user_id, &store_id, &roles)
        .await?;
    Ok((StatusCode::CREATED, Json(MembershipDto::from(&membership))))
}

#[utoipa::path(
    delete,
    path = "/stores/{store_id}/memberships/{user_id}",
    tag = "Platform",
    summary = "Remove a store membership",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn remove_membership(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path((store_id, user_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let store_id = parse_store_id(&store_id)?;
    authorize_store(&context.platform, &actor, &store_id, Some("users.write")).await?;
    let user_id = UserId::from_string(user_id);
    if let Some(membership) = context.platform.get_membership(&user_id, &store_id).await?
        && membership.roles.iter().any(|role| role == "owner")
    {
        let memberships = context.platform.list_store_memberships(&store_id).await?;
        let other_owners = memberships
            .iter()
            .filter(|membership| {
                membership.user_id != user_id && membership.roles.iter().any(|role| role == "owner")
            })
            .count();
        if other_owners == 0 {
            return Err(ApiError::new(AppError::Conflict(
                "cannot remove the last owner of a store".to_owned(),
            )));
        }
    }
    context
        .platform
        .revoke_membership(&user_id, &store_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/roles",
    tag = "Platform",
    summary = "List store roles",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<RoleDto>))
)]
async fn list_roles(
    State(context): State<Arc<AppContext>>,
    AdminActor(_actor): AdminActor,
) -> ApiResult<Json<Vec<RoleDto>>> {
    let roles = context.platform.list_roles().await?;
    Ok(Json(
        roles
            .into_iter()
            .map(|(name, description)| RoleDto { name, description })
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/users",
    tag = "Platform",
    summary = "Create a platform user",
    request_body = CreateUserRequest,
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 201, body = UserDto))
)]
async fn create_user(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    ApiJson(request): ApiJson<CreateUserRequest>,
) -> ApiResult<(StatusCode, Json<UserDto>)> {
    let store_id = match request.store_id.as_deref() {
        Some(store_id) => {
            let store_id = parse_store_id(store_id)?;
            authorize_store(&context.platform, &actor, &store_id, Some("users.write")).await?;
            if request.roles.is_empty() {
                return Err(ApiError::validation(
                    "roles are required when creating a user for a store",
                ));
            }
            store_id
        }
        None => {
            return Err(ApiError::validation(
                "store_id is required when creating a user",
            ));
        }
    };
    let mut input = CreateUserInput::new(&request.email);
    input.first_name = request.first_name;
    input.last_name = request.last_name;
    let user = context.auth.register(input, &request.password).await?;
    let roles: Vec<&str> = request.roles.iter().map(String::as_str).collect();
    context
        .platform
        .upsert_membership(&user.id, &store_id, &roles)
        .await?;
    Ok((StatusCode::CREATED, Json(UserDto::from(&user))))
}

fn parse_store_id(value: &str) -> ApiResult<StoreId> {
    if value.is_empty() || value.len() > 128 {
        return Err(ApiError::validation("invalid store id"));
    }
    Ok(StoreId::from_string(value))
}

async fn audit(
    context: &AppContext,
    actor: &ActorContext,
    store_id: Option<&StoreId>,
    action: &str,
    resource_type: Option<&str>,
    resource_id: Option<&str>,
) {
    let entry = NewAudit {
        actor_id: actor.user_id.as_ref().map(ToString::to_string),
        store_id: store_id.map(ToString::to_string),
        action: action.to_owned(),
        resource_type: resource_type.map(str::to_owned),
        resource_id: resource_id.map(str::to_owned),
        data: serde_json::Value::Null,
        ip_address: None,
    };
    if let Err(error) = context.platform.record_audit(entry).await {
        tracing::warn!(error = %error, "failed to write audit log");
    }
}

#[utoipa::path(
    post,
    path = "/api-tokens",
    tag = "Platform",
    summary = "Create an API token",
    request_body = CreateApiTokenRequest,
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 201, body = CreatedApiTokenResponse))
)]
async fn create_api_token(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    ApiJson(request): ApiJson<CreateApiTokenRequest>,
) -> ApiResult<(StatusCode, Json<CreatedApiTokenResponse>)> {
    let user_id = actor.require_user().map_err(ApiError::new)?.clone();
    let store_id = request
        .store_id
        .as_deref()
        .map(parse_store_id)
        .transpose()?;
    if let Some(store_id) = store_id.as_ref() {
        authorize_store(&context.platform, &actor, store_id, Some("users.write")).await?;
    }
    let created = context
        .auth
        .create_api_token(
            &user_id,
            store_id.as_ref(),
            &request.name,
            &request.scopes,
            request.ttl_seconds,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedApiTokenResponse {
            token: created.token,
            record: ApiTokenDto::from(&created.record),
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/api-tokens",
    tag = "Platform",
    summary = "List API tokens",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<ApiTokenDto>))
)]
async fn list_api_tokens(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
) -> ApiResult<Json<Vec<ApiTokenDto>>> {
    let user_id = actor.require_user().map_err(ApiError::new)?;
    let tokens = context.auth.list_api_tokens(user_id).await?;
    Ok(Json(tokens.iter().map(ApiTokenDto::from).collect()))
}

#[utoipa::path(
    delete,
    path = "/api-tokens/{token_id}",
    tag = "Platform",
    summary = "Revoke an API token",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn revoke_api_token(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    Path(token_id): Path<String>,
) -> ApiResult<StatusCode> {
    let user_id = actor.require_user().map_err(ApiError::new)?;
    context.auth.revoke_api_token(user_id, &token_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
