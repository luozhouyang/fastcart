//! Shared authentication handlers for platform and admin namespaces.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::http::header::{HeaderValue, SET_COOKIE};
use axum::response::{IntoResponse, Response};
use fastcart_http::{
    ADMIN_CSRF_COOKIE, ADMIN_SESSION_COOKIE, AdminActor, ApiError, ApiJson, ApiResult,
    SessionToken, csrf_cookie, session_cookie, session_cookie_clear,
};
use fastcart_kernel::{AppError, normalize_email, now_timestamp};
use fastcart_platform::{CreateUserInput, CreatedSession, User};

use crate::dto::{LoginRequest, LoginResponse, MeResponse, SetupRequest, UserDto};
use crate::state::AppContext;

/// Authenticates a platform or store administrator.
pub async fn login(
    State(context): State<Arc<AppContext>>,
    ApiJson(request): ApiJson<LoginRequest>,
) -> ApiResult<Response> {
    let email = normalize_email(&request.email);
    let limiter_key = format!("login:{email}");
    if !context.login_limiter.check(&limiter_key) {
        return Err(ApiError::new(AppError::RateLimited));
    }
    let session = context.auth.login(&email, &request.password).await?;
    context.login_limiter.reset(&limiter_key);
    session_response(&context, session)
}

/// Creates the first platform user and logs them in.
///
/// This endpoint is only available before the first user exists.
pub async fn setup(
    State(context): State<Arc<AppContext>>,
    ApiJson(request): ApiJson<SetupRequest>,
) -> ApiResult<Response> {
    if context.platform.count_users().await? > 0 {
        return Err(ApiError::new(AppError::Conflict(
            "platform is already initialized".to_owned(),
        )));
    }
    let mut input = CreateUserInput::new(&request.email);
    if let Some(first_name) = request.first_name.clone() {
        input.first_name = Some(first_name);
    }
    if let Some(last_name) = request.last_name.clone() {
        input.last_name = Some(last_name);
    }
    let user = context.auth.register(input, &request.password).await?;
    let session = context.auth.login(&user.email, &request.password).await?;
    session_response(&context, session)
}

/// Revokes the current session and clears cookies.
pub async fn logout(
    State(context): State<Arc<AppContext>>,
    Extension(SessionToken(token)): Extension<SessionToken>,
) -> ApiResult<Response> {
    context.auth.logout(&token).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    append_cookie(
        &mut response,
        session_cookie_clear(ADMIN_SESSION_COOKIE, context.config.cookie_secure),
    )?;
    append_cookie(
        &mut response,
        session_cookie_clear(ADMIN_CSRF_COOKIE, context.config.cookie_secure),
    )?;
    Ok(response)
}

/// Returns the authenticated actor.
pub async fn me(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
) -> ApiResult<Json<MeResponse>> {
    let user_id = actor
        .user_id
        .as_ref()
        .ok_or_else(|| ApiError::new(AppError::Unauthorized))?;
    let user = context
        .platform
        .get_user(user_id)
        .await?
        .ok_or_else(|| ApiError::new(AppError::not_found("user", user_id)))?;
    Ok(Json(MeResponse {
        user: UserDto::from(&user),
        store_id: actor.store_id.as_ref().map(ToString::to_string),
        permissions: actor.permissions.into_iter().collect(),
    }))
}

fn session_response(context: &AppContext, session: CreatedSession) -> ApiResult<Response> {
    let max_age = session.expires_at.saturating_sub(now_timestamp());
    let body = Json(LoginResponse {
        user: UserDto::from(&session.user),
        csrf_token: session.csrf_token.clone(),
        expires_at: session.expires_at,
    });
    let mut response = body.into_response();
    append_cookie(
        &mut response,
        session_cookie(
            ADMIN_SESSION_COOKIE,
            &session.token,
            max_age,
            context.config.cookie_secure,
        ),
    )?;
    append_cookie(
        &mut response,
        csrf_cookie(
            ADMIN_CSRF_COOKIE,
            &session.csrf_token,
            context.config.cookie_secure,
        ),
    )?;
    Ok(response)
}

fn append_cookie(response: &mut Response, cookie: String) -> ApiResult<()> {
    let value = HeaderValue::from_str(&cookie)
        .map_err(|_| ApiError::new(AppError::Internal("invalid cookie".to_owned())))?;
    response.headers_mut().append(SET_COOKIE, value);
    Ok(())
}

/// Maps a user into the public DTO.
#[must_use]
pub fn user_dto(user: &User) -> UserDto {
    UserDto::from(user)
}
