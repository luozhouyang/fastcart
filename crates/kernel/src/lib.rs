//! Shared foundational types for the Fastcart commerce platform.
//!
//! The kernel stays small: identifiers, primitive value objects, request context
//! types, validation primitives and the shared application error model. It must
//! not depend on Axum, SQLx or any infrastructure crate.

pub mod context;
pub mod error;
pub mod ids;
pub mod money;
pub mod types;
pub mod validation;

pub use context::{ActorContext, AuthenticationMethod, RequestContext, StoreContext};
pub use error::{AppError, KernelError};
pub use ids::{
    ApiTokenId, CartId, CategoryId, CheckoutId, ClaimId, CollectionId, CouponId, CustomerId,
    CustomerSessionId, EventId, ExchangeId, FulfillmentId, ImageId, InquiryFileId, InquiryFormId,
    InquiryId, InquiryRevisionId, InventoryItemId, InventoryLocationId, JobId, LineItemId,
    NavigationId, NotificationChannelId, NotificationDeliveryId, OptionId, OptionValueId,
    OrderEditId, OrderId, PageId, PaymentAttemptId, PaymentId, PaymentSessionId, PriceId,
    PriceListId, PriceSetId, ProductId, PromotionId, RefundId, ReservationId, ReturnId, SessionId,
    ShippingOptionId, ShippingProfileId, StoreId, TagId, TaxCategoryId, TaxRateId, TaxRegionId,
    ThemeActivationId, ThemeId, UserId, VariantId, WebhookDeliveryId, WebhookId,
};
pub use money::{CurrencyCode, Money, Rounding};
pub use types::{
    CountryCode, DomainEventEnvelope, Locale, Metadata, PageResult, Pagination, Timestamp,
    now_timestamp,
};
pub use validation::{
    is_valid_email, normalize_email, slugify, validate_email, validate_handle, validate_length,
    validate_metadata, validate_non_negative_amount, validate_quantity,
};
