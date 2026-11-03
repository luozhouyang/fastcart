//! Typed request extractors and pagination helpers.

use axum::Json;
use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::http::request::Parts;
use fastcart_kernel::{ActorContext, Pagination, StoreContext};
use serde::de::DeserializeOwned;

use crate::error::{ApiError, ApiResult};

/// Extracts the resolved store context.
#[derive(Clone, Debug)]
pub struct StoreCtx(pub StoreContext);

impl<S> FromRequestParts<S> for StoreCtx
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<StoreContext>()
            .cloned()
            .map(StoreCtx)
            .ok_or_else(|| {
                ApiError::new(fastcart_kernel::AppError::Internal(
                    "store context is missing from the request".to_owned(),
                ))
            })
    }
}

/// Extracts the authenticated admin actor.
#[derive(Clone, Debug)]
pub struct AdminActor(pub ActorContext);

impl<S> FromRequestParts<S> for AdminActor
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<ActorContext>()
            .cloned()
            .map(AdminActor)
            .ok_or_else(|| ApiError::new(fastcart_kernel::AppError::Unauthorized))
    }
}

/// Extracts the authenticated storefront customer.
#[derive(Clone, Debug)]
pub struct CustomerActor(pub ActorContext);

impl<S> FromRequestParts<S> for CustomerActor
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let actor = parts
            .extensions
            .get::<ActorContext>()
            .cloned()
            .ok_or_else(|| ApiError::new(fastcart_kernel::AppError::Unauthorized))?;
        actor.require_customer().map_err(ApiError::new)?;
        Ok(CustomerActor(actor))
    }
}

/// Offset/limit query parameters shared by list endpoints.
#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
pub struct PaginationQuery {
    /// Page size, 1-100.
    pub limit: Option<i64>,
    /// Records to skip.
    pub offset: Option<i64>,
}

impl PaginationQuery {
    /// Validates the query into pagination values.
    pub fn pagination(&self) -> ApiResult<Pagination> {
        Pagination::new(self.limit, self.offset).map_err(ApiError::new)
    }
}

/// JSON body extractor that renders rejection errors in the platform envelope.
#[derive(Clone, Debug)]
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(|rejection| ApiError::validation(rejection.body_text()))?;
        Ok(Self(value))
    }
}

/// Query extractor with envelope errors.
#[derive(Clone, Debug)]
pub struct ApiQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::validation(rejection.body_text()))?;
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    use super::PaginationQuery;

    #[test]
    fn pagination_query_validates_bounds() {
        let query = PaginationQuery {
            limit: Some(20),
            offset: Some(5),
        };
        let pagination = query.pagination().unwrap();
        assert_eq!(pagination.limit, 20);
        assert_eq!(pagination.offset, 5);

        let invalid = PaginationQuery {
            limit: Some(500),
            offset: None,
        };
        assert!(invalid.pagination().is_err());
    }
}
