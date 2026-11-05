/**
 * Shared wire types for the Fastcart REST API.
 *
 * Every type in this file mirrors the JSON contract produced by the Axum
 * presentation layer in `crates/api`. Field names therefore stay in the
 * backend's `snake_case` form; do not rename them to camelCase.
 *
 * Money is always an integer number of minor currency units paired with an
 * uppercase ISO-style currency code. Timestamps are Unix seconds.
 */

// ---------------------------------------------------------------------------
// Primitive value types
// ---------------------------------------------------------------------------

/** A three-letter uppercase ISO-style currency code, for example `USD`. */
export type CurrencyCode = string;

/** A validated uppercase two-letter ISO 3166-1 country code, for example `US`. */
export type CountryCode = string;

/** Unix timestamp in seconds. */
export type Timestamp = number;

/** Arbitrary JSON value accepted by free-form fields. */
export type JsonValue =
  string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };

/** Free-form metadata attached to records; serialized as a JSON object. */
export type Metadata = Record<string, JsonValue>;

/** An amount in minor units together with its currency. */
export interface Money {
  amount_minor: number;
  currency: CurrencyCode;
}

/** Trusted store context resolved from the request host. */
export interface StoreContext {
  storeId: string;
  domain: string;
  locale: string;
  currency: string;
  timezone: string;
}

/** Context attached to an authenticated request. */
export interface RequestContext {
  requestId: string;
  store?: StoreContext;
  actorId?: string;
}

// ---------------------------------------------------------------------------
// Errors and pagination
// ---------------------------------------------------------------------------

export interface ApiErrorBody {
  code: string;
  message: string;
  /** Correlation identifier; the backend renders it as `request_id`. */
  request_id?: string | null;
  /** Compatibility alias accepted by older clients. */
  requestId?: string | null;
}

export interface ApiErrorResponse {
  error: ApiErrorBody;
}

/** Stable error envelope returned by every API namespace. */
export type ErrorEnvelope = ApiErrorResponse;

export interface Pagination {
  limit: number;
  offset: number;
  total: number;
}

export interface PageResult<T> {
  data: T[];
  pagination: Pagination;
}

/** Offset/limit query accepted by list endpoints. */
export interface PaginationQuery {
  limit?: number;
  offset?: number;
}

// ---------------------------------------------------------------------------
// Platform: stores, domains, users, memberships, roles and tokens
// ---------------------------------------------------------------------------

export type StoreStatus = "provisioning" | "active" | "suspended" | "deleting" | "deleted";

export interface Store {
  id: string;
  name: string;
  slug: string;
  status: StoreStatus;
  primary_domain: string | null;
  locale: string;
  currency_code: CurrencyCode;
  timezone: string;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface Domain {
  id: string;
  store_id: string;
  hostname: string;
  is_primary: boolean;
  verified_at: Timestamp | null;
}

export type UserStatus = "active" | "disabled" | "invited";

export interface User {
  id: string;
  email: string;
  first_name: string | null;
  last_name: string | null;
  status: UserStatus;
  email_verified_at: Timestamp | null;
}

export type MembershipStatus = "active" | "invited" | "revoked";

export interface Membership {
  id: string;
  user_id: string;
  store_id: string;
  status: MembershipStatus;
  roles: string[];
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface Role {
  name: string;
  description: string;
}

export interface ApiToken {
  id: string;
  user_id: string;
  store_id: string | null;
  name: string;
  prefix: string;
  scopes: string[];
  expires_at: Timestamp | null;
  revoked_at: Timestamp | null;
  last_used_at: Timestamp | null;
  created_at: Timestamp;
}

export interface CreatedApiToken {
  token: string;
  record: ApiToken;
}

export interface LoginRequest {
  email: string;
  password: string;
}

export interface LoginResponse {
  user: User;
  csrf_token: string;
  expires_at: Timestamp;
}

export interface MeResponse {
  user: User;
  store_id: string | null;
  permissions: string[];
}

export interface SetupRequest {
  email: string;
  password: string;
  first_name?: string | null;
  last_name?: string | null;
}

export interface CreateStoreRequest {
  name: string;
  slug: string;
  primary_domain: string;
  locale?: string;
  currency_code?: CurrencyCode;
  timezone?: string;
}

export interface UpdateStoreRequest {
  name?: string;
  locale?: string;
  currency_code?: CurrencyCode;
  timezone?: string;
}

export interface DeleteStoreRequest {
  backup?: boolean;
  purge_data?: boolean;
}

export interface DomainRequest {
  hostname: string;
  is_primary?: boolean;
}

export interface PrimaryDomainRequest {
  hostname: string;
}

export interface MembershipRequest {
  user_id: string;
  roles: string[];
}

export interface CreateUserRequest {
  email: string;
  password: string;
  first_name?: string | null;
  last_name?: string | null;
  store_id?: string | null;
  roles?: string[];
}

export interface PermissionCheckResponse {
  allowed: boolean;
}

export interface ProvisioningResponse {
  store: Store;
  steps: string[];
  already_active: boolean;
}

export interface CreateApiTokenRequest {
  name: string;
  store_id?: string | null;
  scopes?: string[];
  ttl_seconds?: number | null;
}

// ---------------------------------------------------------------------------
// Catalog: products, variants, options, media, categories and collections
// ---------------------------------------------------------------------------

export type ProductStatus = "draft" | "active" | "archived";

export interface Dimensions {
  length: number | null;
  height: number | null;
  width: number | null;
}

export interface VariantOptionInput {
  option_title: string;
  value: string;
}

export interface ProductOptionValue {
  id: string;
  option_id: string;
  value: string;
  rank: number;
  metadata: Metadata;
}

export interface ProductOption {
  id: string;
  product_id: string;
  title: string;
  rank: number;
  metadata: Metadata;
  values: ProductOptionValue[];
}

export interface Variant {
  id: string;
  product_id: string;
  title: string;
  sku: string | null;
  barcode: string | null;
  status: ProductStatus;
  weight: number | null;
  dimensions: Dimensions;
  manage_inventory: boolean;
  allow_backorder: boolean;
  tax_category_id: string | null;
  metadata: Metadata;
  option_value_ids: string[];
  created_at: Timestamp;
  updated_at: Timestamp;
  deleted_at: Timestamp | null;
}

/** Backend name for {@link Variant}. */
export type ProductVariant = Variant;

export interface ProductImage {
  id: string;
  product_id: string;
  variant_id: string | null;
  url: string;
  alt: string | null;
  rank: number;
  metadata: Metadata;
  created_at: Timestamp;
}

export interface Category {
  id: string;
  parent_id: string | null;
  title: string;
  handle: string;
  description: string | null;
  is_active: boolean;
  is_internal: boolean;
  rank: number;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface Collection {
  id: string;
  title: string;
  handle: string;
  description: string | null;
  is_published: boolean;
  rank: number;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface Tag {
  id: string;
  value: string;
  metadata: Metadata;
  created_at: Timestamp;
}

export interface Product {
  id: string;
  title: string;
  subtitle: string | null;
  description: string | null;
  handle: string;
  status: ProductStatus;
  thumbnail: string | null;
  weight: number | null;
  dimensions: Dimensions;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
  variants: Variant[];
  options: ProductOption[];
  images: ProductImage[];
  categories: Category[];
  collections: Collection[];
  tags: Tag[];
}

export interface CreateOptionRequest {
  title: string;
  values?: string[];
  rank?: number | null;
  metadata?: Metadata;
}

export interface CreateVariantRequest {
  title: string;
  sku?: string | null;
  barcode?: string | null;
  status?: ProductStatus;
  weight?: number | null;
  dimensions?: Dimensions;
  manage_inventory?: boolean;
  allow_backorder?: boolean;
  tax_category_id?: string | null;
  metadata?: Metadata;
  option_value_ids?: string[];
  option_values?: VariantOptionInput[];
}

export interface CreateProductRequest {
  title: string;
  handle?: string | null;
  subtitle?: string | null;
  description?: string | null;
  thumbnail?: string | null;
  weight?: number | null;
  dimensions?: Dimensions;
  metadata?: Metadata;
  variants?: CreateVariantRequest[];
  options?: CreateOptionRequest[];
  category_ids?: string[];
  collection_ids?: string[];
  tag_values?: string[];
  images?: string[];
}

export interface UpdateVariantRequest {
  id?: string | null;
  title?: string;
  sku?: string;
  barcode?: string;
  status?: ProductStatus;
  weight?: number;
  dimensions?: Dimensions;
  manage_inventory?: boolean;
  allow_backorder?: boolean;
  tax_category_id?: string;
  metadata?: Metadata;
  option_value_ids?: string[];
}

export interface UpdateProductRequest {
  title?: string;
  handle?: string;
  subtitle?: string;
  description?: string;
  thumbnail?: string;
  status?: ProductStatus;
  weight?: number;
  dimensions?: Dimensions;
  metadata?: Metadata;
  variants?: UpdateVariantRequest[];
  options?: CreateOptionRequest[];
  category_ids?: string[];
  collection_ids?: string[];
  tag_values?: string[];
  images?: string[];
}

export interface CreateCategoryRequest {
  title: string;
  handle?: string | null;
  parent_id?: string | null;
  description?: string | null;
  is_active?: boolean;
  is_internal?: boolean;
  rank?: number | null;
  metadata?: Metadata;
}

export interface UpdateCategoryRequest {
  title?: string;
  handle?: string;
  parent_id?: string;
  description?: string;
  is_active?: boolean;
  is_internal?: boolean;
  rank?: number;
  metadata?: Metadata;
}

export interface CreateCollectionRequest {
  title: string;
  handle?: string | null;
  description?: string | null;
  is_published?: boolean;
  rank?: number | null;
  metadata?: Metadata;
}

export interface UpdateCollectionRequest {
  title?: string;
  handle?: string;
  description?: string;
  is_published?: boolean;
  rank?: number;
  metadata?: Metadata;
}

export interface CreateImageRequest {
  url: string;
  alt?: string | null;
  variant_id?: string | null;
  rank?: number | null;
  metadata?: Metadata;
}

export interface ImageReorderRequest {
  image_ids: string[];
}

export interface CollectionProductsRequest {
  product_ids: string[];
}

export interface TagRequest {
  value: string;
}

export interface ProductTagRequest {
  tag_id: string;
}

export interface ProductListQuery extends PaginationQuery {
  status?: ProductStatus;
  category_id?: string;
  collection_id?: string;
  tag?: string;
  query?: string;
  include_deleted?: boolean;
}

export interface StoreProductListQuery extends PaginationQuery {
  category_id?: string;
  collection_id?: string;
  tag?: string;
  query?: string;
}

export interface SearchQuery extends PaginationQuery {
  q?: string;
}

export type StoreSearchQuery = SearchQuery;

// ---------------------------------------------------------------------------
// Pricing
// ---------------------------------------------------------------------------

export type PriceListType = "sale" | "override";
export type PriceListStatus = "draft" | "active";

export interface PriceListRule {
  id: string;
  price_list_id: string;
  attribute: string;
  operator: string;
  value: string;
  created_at: Timestamp;
}

export interface PriceList {
  id: string;
  title: string;
  description: string | null;
  price_type: PriceListType;
  status: PriceListStatus;
  starts_at: Timestamp | null;
  ends_at: Timestamp | null;
  metadata: Metadata;
  rules: PriceListRule[];
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface PriceListRuleRequest {
  attribute: string;
  operator?: string | null;
  value: string;
}

export interface CreatePriceListRequest {
  title: string;
  description?: string | null;
  price_type?: PriceListType;
  status?: PriceListStatus;
  starts_at?: Timestamp | null;
  ends_at?: Timestamp | null;
  metadata?: Metadata;
  rules?: PriceListRuleRequest[];
}

export interface UpdatePriceListRequest {
  title?: string;
  description?: string | null;
  price_type?: PriceListType;
  status?: PriceListStatus;
  starts_at?: Timestamp | null;
  ends_at?: Timestamp | null;
  metadata?: Metadata;
  rules?: PriceListRuleRequest[];
}

export interface Price {
  id: string;
  price_list_id: string | null;
  variant_id: string;
  currency_code: CurrencyCode;
  amount: number;
  min_quantity: number;
  max_quantity: number | null;
  customer_group_id: string | null;
  sales_channel_id: string | null;
  starts_at: Timestamp | null;
  ends_at: Timestamp | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface CreatePriceRequest {
  price_list_id?: string | null;
  variant_id: string;
  currency_code: CurrencyCode;
  amount: number;
  min_quantity?: number;
  max_quantity?: number | null;
  customer_group_id?: string | null;
  sales_channel_id?: string | null;
  starts_at?: Timestamp | null;
  ends_at?: Timestamp | null;
  metadata?: Metadata;
}

export interface UpdatePriceRequest {
  price_list_id?: string | null;
  variant_id?: string;
  currency_code?: CurrencyCode;
  amount?: number;
  min_quantity?: number;
  max_quantity?: number | null;
  customer_group_id?: string | null;
  sales_channel_id?: string | null;
  starts_at?: Timestamp | null;
  ends_at?: Timestamp | null;
  metadata?: Metadata;
}

export interface SelectedPrice {
  price_id: string;
  amount: number;
  currency: CurrencyCode;
  price_list_id: string | null;
}

export interface PriceListQuery extends PaginationQuery {
  price_list_id?: string;
  variant_id?: string;
  currency?: CurrencyCode;
}

export interface CalculatePriceQuery {
  variant_id: string;
  currency: CurrencyCode;
  quantity?: number;
  customer_group_id?: string;
  sales_channel_id?: string;
}

export interface CalculatePricesRequest {
  variant_ids: string[];
  currency: CurrencyCode;
  quantity?: number;
  customer_group_id?: string;
  sales_channel_id?: string;
}

// ---------------------------------------------------------------------------
// Cart and checkout
// ---------------------------------------------------------------------------

export type CartStatus = "open" | "completed" | "expired";

/** Address snapshot stored on a cart; it has no identifier of its own. */
export interface CartAddress {
  first_name: string | null;
  last_name: string | null;
  company: string | null;
  address_1: string | null;
  address_2: string | null;
  city: string | null;
  province: string | null;
  postal_code: string | null;
  country_code: CountryCode | null;
  phone: string | null;
  metadata: Metadata;
}

export interface CartAdjustment {
  promotion_id: string | null;
  code: string | null;
  amount: number;
  description: string | null;
}

export interface CartDiscount {
  promotion_id: string | null;
  code: string | null;
  amount: number;
  is_automatic: boolean;
  metadata: Metadata;
}

export interface CartShippingMethod {
  shipping_option_id: string;
  name: string;
  amount: number;
  currency: CurrencyCode;
  is_tax_inclusive: boolean;
  data: JsonValue;
}

export interface CartTaxLine {
  name: string;
  rate_bps: number;
  amount: number;
  is_tax_inclusive: boolean;
  cart_item_id: string | null;
  shipping_method_id: string | null;
}

export interface CartItem {
  id: string;
  cart_id: string;
  product_id: string;
  variant_id: string;
  quantity: number;
  title: string;
  variant_title: string | null;
  sku: string | null;
  thumbnail: string | null;
  unit_price: number;
  currency_code: CurrencyCode;
  is_tax_inclusive: boolean;
  requires_shipping: boolean;
  metadata: Metadata;
  adjustments: CartAdjustment[];
}

export interface CartTotals {
  item_subtotal: number;
  item_discount_total: number;
  shipping_subtotal: number;
  shipping_discount_total: number;
  subtotal: number;
  discount_total: number;
  shipping_total: number;
  tax_total: number;
  total: number;
  currency_code: CurrencyCode;
}

export interface Cart {
  id: string;
  currency_code: CurrencyCode;
  customer_id: string | null;
  email: string | null;
  sales_channel_id: string | null;
  status: CartStatus;
  shipping_address: CartAddress;
  billing_address: CartAddress;
  items: CartItem[];
  discounts: CartDiscount[];
  shipping_methods: CartShippingMethod[];
  item_adjustments: CartAdjustment[];
  shipping_adjustments: CartAdjustment[];
  tax_lines: CartTaxLine[];
  totals: CartTotals;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
  completed_at: Timestamp | null;
  expires_at: Timestamp | null;
}

export interface CreateCartRequest {
  currency_code: CurrencyCode;
  email?: string | null;
  sales_channel_id?: string | null;
  expires_in_seconds?: number | null;
}

export interface AddItemRequest {
  variant_id: string;
  quantity: number;
  country_code?: CountryCode | null;
  province_code?: string | null;
}

export interface UpdateItemRequest {
  quantity: number;
}

export interface SetEmailRequest {
  email: string;
}

export interface SetAddressRequest {
  type: "shipping" | "billing";
  first_name?: string | null;
  last_name?: string | null;
  company?: string | null;
  address_1?: string | null;
  address_2?: string | null;
  city?: string | null;
  province?: string | null;
  postal_code?: string | null;
  country_code?: CountryCode | null;
  phone?: string | null;
}

export interface SetShippingMethodRequest {
  shipping_option_id: string;
  country_code?: CountryCode | null;
  province_code?: string | null;
}

export interface DiscountRequest {
  code: string;
}

export interface DestinationQuery {
  country_code?: CountryCode;
  province_code?: string;
}

export type CheckoutStatus =
  | "created"
  | "validating"
  | "ready_for_payment"
  | "payment_pending"
  | "payment_confirmed"
  | "order_placed"
  | "failed"
  | "canceled";

export interface CheckoutSession {
  id: string;
  cart_id: string;
  customer_id: string | null;
  email: string | null;
  currency_code: CurrencyCode;
  status: CheckoutStatus;
  shipping_address: JsonValue;
  billing_address: JsonValue;
  shipping_method: JsonValue;
  totals: JsonValue;
  promotion_codes: string[];
  payment_session_id: string | null;
  order_id: string | null;
  idempotency_key: string | null;
  expires_at: Timestamp | null;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface CheckoutStateHistory {
  id: string;
  checkout_id: string;
  from_state: CheckoutStatus | null;
  to_state: CheckoutStatus;
  reason: string | null;
  created_at: Timestamp;
}

export interface CreateCheckoutRequest {
  cart_id: string;
  email?: string | null;
  payment_provider?: string;
  idempotency_key?: string | null;
  expires_in_seconds?: number | null;
  country_code?: CountryCode | null;
  province_code?: string | null;
}

export interface CompleteCheckoutRequest {
  country_code?: CountryCode | null;
  province_code?: string | null;
}

/** Medusa v2 completion envelope: the checkout produced an order. */
export interface CheckoutCompleteResponse {
  type: "order";
  order: Order;
}

/** @deprecated Use {@link CheckoutCompleteResponse}. */
export type CompleteCheckoutResponse = CheckoutCompleteResponse;

// ---------------------------------------------------------------------------
// Orders
// ---------------------------------------------------------------------------

export type OrderStatus =
  "pending" | "confirmed" | "processing" | "fulfilled" | "completed" | "canceled";

/** Payment progress as exposed on an order. */
export type OrderPaymentStatus =
  "awaiting" | "authorized" | "captured" | "partially_refunded" | "refunded" | "failed";

export type FulfillmentStatus = "not_fulfilled" | "partially_fulfilled" | "fulfilled" | "returned";

/** Refund progress as exposed on an order. */
export type RefundStatus = "not_refunded" | "partially_refunded" | "refunded";

/** Alias kept for callers that prefer the order-qualified name. */
export type OrderRefundStatus = RefundStatus;

export interface OrderItemAdjustment {
  id: string;
  order_item_id: string;
  promotion_id: string | null;
  code: string | null;
  amount: number;
  description: string | null;
  created_at: Timestamp;
}

export interface OrderItem {
  id: string;
  order_id: string;
  product_id: string | null;
  variant_id: string | null;
  product_title: string;
  variant_title: string | null;
  sku: string | null;
  thumbnail: string | null;
  unit_price: number;
  quantity: number;
  subtotal: number;
  discount_total: number;
  tax_total: number;
  total: number;
  is_tax_inclusive: boolean;
  requires_shipping: boolean;
  metadata: Metadata;
  adjustments: OrderItemAdjustment[];
}

export interface OrderAdjustment {
  id: string;
  order_id: string;
  code: string;
  amount: number;
  description: string | null;
  created_at: Timestamp;
}

export interface OrderStatusHistory {
  id: string;
  order_id: string;
  status: OrderStatus;
  previous_status: OrderStatus | null;
  note: string | null;
  created_at: Timestamp;
}

export interface Fulfillment {
  id: string;
  order_id: string;
  status: string;
  tracking_numbers: string[];
  shipped_at: Timestamp | null;
  delivered_at: Timestamp | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export type OrderFulfillment = Fulfillment;

export interface Refund {
  id: string;
  order_id: string;
  payment_id: string | null;
  amount: number;
  currency_code: CurrencyCode;
  status: string;
  reason: string | null;
  provider_refund_id: string | null;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export type OrderRefund = Refund;

export interface Order {
  id: string;
  display_number: number;
  customer_id: string | null;
  email: string;
  currency_code: CurrencyCode;
  status: OrderStatus;
  payment_status: OrderPaymentStatus;
  fulfillment_status: FulfillmentStatus;
  refund_status: OrderRefundStatus;
  subtotal: number;
  discount_total: number;
  shipping_total: number;
  tax_total: number;
  total: number;
  shipping_address: JsonValue;
  billing_address: JsonValue;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
  canceled_at: Timestamp | null;
  items: OrderItem[];
  adjustments: OrderAdjustment[];
  status_history: OrderStatusHistory[];
  fulfillments: Fulfillment[];
  refunds: Refund[];
}

export interface OrderListQuery extends PaginationQuery {
  status?: OrderStatus;
  payment_status?: OrderPaymentStatus;
  customer_id?: string;
  email?: string;
}

export interface CancelOrderRequest {
  reason?: string | null;
}

export interface FulfillOrderRequest {
  tracking_numbers?: string[];
  shipped_at?: Timestamp | null;
  partial?: boolean;
}

export interface RefundOrderRequest {
  payment_id: string;
  amount: number;
  currency: CurrencyCode;
  reason?: string | null;
}

// ---------------------------------------------------------------------------
// Customers
// ---------------------------------------------------------------------------

export type CustomerStatus = "active" | "disabled";

export interface Customer {
  id: string;
  email: string;
  first_name: string | null;
  last_name: string | null;
  phone: string | null;
  has_account: boolean;
  status: CustomerStatus;
  email_verified_at: Timestamp | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
  deleted_at: Timestamp | null;
}

export interface CustomerAddress {
  id: string;
  customer_id: string;
  is_default_shipping: boolean;
  is_default_billing: boolean;
  first_name: string | null;
  last_name: string | null;
  company: string | null;
  address_1: string | null;
  address_2: string | null;
  city: string | null;
  province: string | null;
  postal_code: string | null;
  country_code: CountryCode | null;
  phone: string | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

/** Address shape returned by the customer endpoints. */
export type Address = CustomerAddress;

export interface CustomerAddressInput {
  is_default_shipping?: boolean;
  is_default_billing?: boolean;
  first_name?: string | null;
  last_name?: string | null;
  company?: string | null;
  address_1?: string | null;
  address_2?: string | null;
  city?: string | null;
  province?: string | null;
  postal_code?: string | null;
  country_code?: CountryCode | null;
  phone?: string | null;
  metadata?: Metadata;
}

export interface CustomerGroup {
  id: string;
  name: string;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface CustomerSessionResponse {
  customer: Customer;
  csrf_token: string;
  expires_at: Timestamp;
}

export interface RegisterCustomerRequest {
  email: string;
  password: string;
  first_name?: string | null;
  last_name?: string | null;
}

export interface CustomerLoginRequest {
  email: string;
  password: string;
}

export interface UpdateCustomerRequest {
  email?: string;
  first_name?: string | null;
  last_name?: string | null;
  phone?: string | null;
  status?: CustomerStatus;
  metadata?: Metadata;
}

export interface UpdateProfileRequest {
  first_name?: string | null;
  last_name?: string | null;
  phone?: string | null;
}

export interface ChangePasswordRequest {
  current_password: string;
  new_password: string;
}

export interface CustomerListQuery extends PaginationQuery {
  query?: string;
}

export interface CreateCustomerGroupRequest {
  name: string;
  metadata?: Metadata;
}

// ---------------------------------------------------------------------------
// Inventory
// ---------------------------------------------------------------------------

export interface InventoryLocation {
  id: string;
  name: string;
  address: Metadata;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface InventoryItem {
  id: string;
  sku: string | null;
  variant_id: string | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface InventoryLevel {
  id: string;
  inventory_item_id: string;
  location_id: string;
  stocked_quantity: number;
  reserved_quantity: number;
  incoming_quantity: number;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export type ReservationStatus = "reserved" | "released" | "committed";

export interface InventoryReservation {
  id: string;
  inventory_item_id: string;
  location_id: string;
  quantity: number;
  status: ReservationStatus;
  line_item_id: string | null;
  checkout_id: string | null;
  order_id: string | null;
  expires_at: Timestamp | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface InventoryMovement {
  id: string;
  inventory_item_id: string;
  location_id: string;
  quantity_delta: number;
  reason: string;
  reference: string | null;
  actor_id: string | null;
  metadata: Metadata;
  created_at: Timestamp;
}

export interface VariantAvailability {
  variant_id: string;
  available: number;
  manage_inventory: boolean;
  allow_backorder: boolean;
}

export interface CreateInventoryLocationRequest {
  name: string;
  address?: Metadata | null;
  metadata?: Metadata | null;
}

export interface UpdateInventoryLocationRequest {
  name?: string;
  address?: Metadata | null;
  metadata?: Metadata | null;
}

export interface CreateInventoryItemRequest {
  sku?: string | null;
  variant_id?: string | null;
  metadata?: Metadata | null;
}

export interface InventoryLevelsQuery {
  item_id?: string;
  location_id?: string;
}

export interface InventoryAdjustmentRequest {
  inventory_item_id: string;
  location_id: string;
  quantity_delta: number;
  reason: string;
  reference?: string | null;
}

export interface InventoryTransferRequest {
  inventory_item_id: string;
  from_location_id: string;
  to_location_id: string;
  quantity: number;
  reason?: string | null;
}

export interface InventoryTransferResponse {
  from: InventoryLevel;
  to: InventoryLevel;
}

export interface ReservationLineRequest {
  variant_id: string;
  quantity: number;
  line_item_id?: string | null;
}

export interface CreateReservationsRequest {
  lines: ReservationLineRequest[];
  checkout_id: string;
  ttl_seconds: number;
}

export interface ReleaseReservationsRequest {
  reservation_ids?: string[];
}

export interface CommitReservationsRequest {
  checkout_id: string;
  order_id: string;
}

export interface AvailabilityResponse {
  variant_id: string;
  available: number;
}

export interface CountResponse {
  count: number;
}

export interface ReleasedResponse {
  released: number;
}

export interface CommittedResponse {
  committed: number;
}

export interface InventoryMovementsQuery {
  item_id: string;
  limit?: number;
}

// ---------------------------------------------------------------------------
// Shipping
// ---------------------------------------------------------------------------

export interface ShippingProfile {
  id: string;
  name: string;
  profile_type: string;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface ShippingOption {
  id: string;
  profile_id: string;
  name: string;
  price_type: string;
  amount_minor: number;
  currency: CurrencyCode;
  is_enabled: boolean;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface ShippingRate {
  id: string;
  shipping_option_id: string;
  country_code: CountryCode | null;
  province_code: string | null;
  amount_minor: number;
  currency: CurrencyCode;
  min_subtotal: number | null;
  max_subtotal: number | null;
  metadata: Metadata;
  created_at: Timestamp;
}

export interface SelectedShippingRate {
  option_id: string;
  rate_id: string | null;
  amount_minor: number;
  currency: CurrencyCode;
  is_fallback: boolean;
}

export interface ResolvedShippingOption {
  option: ShippingOption;
  amount_minor: number;
  currency: CurrencyCode;
  rate_id: string | null;
}

export interface CreateShippingProfileRequest {
  name: string;
  profile_type?: string | null;
  metadata?: Metadata | null;
}

export interface UpdateShippingProfileRequest {
  name?: string;
  profile_type?: string | null;
  metadata?: Metadata | null;
}

export interface CreateShippingOptionRequest {
  profile_id: string;
  name: string;
  amount_minor: number;
  currency: CurrencyCode;
  is_enabled?: boolean;
  metadata?: Metadata | null;
}

export interface UpdateShippingOptionRequest {
  name?: string;
  amount_minor?: number;
  currency?: CurrencyCode;
  is_enabled?: boolean;
  metadata?: Metadata | null;
}

export interface CreateShippingRateRequest {
  country_code?: CountryCode | null;
  province_code?: string | null;
  amount_minor: number;
  currency: CurrencyCode;
  min_subtotal?: number | null;
  max_subtotal?: number | null;
  metadata?: Metadata | null;
}

export interface UpdateShippingRateRequest {
  country_code?: CountryCode | null;
  province_code?: string | null;
  amount_minor?: number;
  currency?: CurrencyCode;
  min_subtotal?: number | null;
  max_subtotal?: number | null;
  metadata?: Metadata | null;
}

export interface ShippingOptionsQuery {
  country_code: CountryCode;
  subtotal?: number;
}

export interface DestinationOptionsQuery {
  country: CountryCode;
  subtotal?: number;
}

export interface SelectRateQuery {
  country: CountryCode;
  province?: string;
  subtotal?: number;
}

export interface ShippingOptionsListQuery {
  profile_id?: string;
}

export interface AddShippingLocationRequest {
  location_id: string;
}

// ---------------------------------------------------------------------------
// Tax
// ---------------------------------------------------------------------------

export interface TaxCategory {
  id: string;
  name: string;
  code: string;
  description: string | null;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface TaxRegion {
  id: string;
  name: string;
  country_code: CountryCode;
  province_code: string | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface TaxRate {
  id: string;
  tax_region_id: string;
  tax_category_id: string | null;
  name: string;
  rate_bps: number;
  is_inclusive: boolean;
  is_default: boolean;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface TaxBreakdown {
  tax_amount: number;
  net_amount: number;
  gross_amount: number;
}

export interface TaxLine {
  country_code: CountryCode;
  province_code: string | null;
  category_id: string | null;
  rate: TaxRate | null;
  region: TaxRegion | null;
  amount_minor: number;
  rate_bps: number;
  is_inclusive: boolean;
  breakdown: TaxBreakdown;
}

export interface CreateTaxCategoryRequest {
  name: string;
  code: string;
  description?: string | null;
}

export interface UpdateTaxCategoryRequest {
  name?: string;
  code?: string;
  description?: string | null;
}

export interface CreateTaxRegionRequest {
  name: string;
  country_code: CountryCode;
  province_code?: string | null;
  metadata?: Metadata | null;
}

export interface UpdateTaxRegionRequest {
  name?: string;
  country_code?: CountryCode;
  province_code?: string | null;
  metadata?: Metadata | null;
}

export interface CreateTaxRateRequest {
  region_id: string;
  category_id?: string | null;
  name: string;
  rate_bps: number;
  is_inclusive?: boolean;
  is_default?: boolean;
  metadata?: Metadata | null;
}

export interface UpdateTaxRateRequest {
  category_id?: string | null;
  name?: string;
  rate_bps?: number;
  is_inclusive?: boolean;
  is_default?: boolean;
  metadata?: Metadata | null;
}

export interface TaxRateListQuery {
  region_id: string;
}

export interface TaxCalculateLineRequest {
  amount_minor: number;
  category_id?: string | null;
}

export interface TaxCalculateRequest {
  country: CountryCode;
  province?: string | null;
  category_id?: string | null;
  amount_minor?: number;
  lines?: TaxCalculateLineRequest[];
}

export interface TaxCalculationResponse {
  lines: TaxLine[];
  totals: TaxBreakdown;
}

// ---------------------------------------------------------------------------
// Promotions and coupons
// ---------------------------------------------------------------------------

export type PromotionType = "percentage" | "fixed_amount" | "free_shipping";
export type PromotionStatus = "draft" | "active" | "expired" | "disabled";

export type PromotionRuleAttribute =
  | "minimum_order_subtotal"
  | "product_id"
  | "variant_id"
  | "category_id"
  | "customer_group_id"
  | "currency"
  | "first_order";

export type PromotionRuleOperator = "eq" | "in" | "gte" | "lte";

export interface PromotionRule {
  id: string;
  promotion_id: string;
  attribute: PromotionRuleAttribute;
  operator: PromotionRuleOperator;
  value: string;
  created_at: Timestamp;
}

export interface Promotion {
  id: string;
  code: string | null;
  title: string;
  promotion_type: PromotionType;
  value: number;
  status: PromotionStatus;
  is_automatic: boolean;
  starts_at: Timestamp | null;
  ends_at: Timestamp | null;
  usage_limit: number | null;
  usage_count: number;
  metadata: Metadata;
  rules: PromotionRule[];
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface Coupon {
  id: string;
  code: string;
  promotion_id: string;
  usage_limit: number | null;
  usage_count: number;
  starts_at: Timestamp | null;
  ends_at: Timestamp | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface PromotionRuleRequest {
  attribute: PromotionRuleAttribute;
  operator?: PromotionRuleOperator | null;
  value: string;
}

export interface CreatePromotionRequest {
  code?: string | null;
  title: string;
  promotion_type: PromotionType;
  value: number;
  status?: PromotionStatus;
  is_automatic?: boolean;
  starts_at?: Timestamp | null;
  ends_at?: Timestamp | null;
  usage_limit?: number | null;
  metadata?: Metadata;
  rules?: PromotionRuleRequest[];
}

export interface UpdatePromotionRequest {
  code?: string | null;
  title?: string;
  promotion_type?: PromotionType;
  value?: number;
  status?: PromotionStatus;
  is_automatic?: boolean;
  starts_at?: Timestamp | null;
  ends_at?: Timestamp | null;
  usage_limit?: number | null;
  metadata?: Metadata;
  rules?: PromotionRuleRequest[];
}

export interface CreateCouponRequest {
  code: string;
  promotion_id: string;
  usage_limit?: number | null;
  starts_at?: Timestamp | null;
  ends_at?: Timestamp | null;
  metadata?: Metadata;
}

export interface UpdateCouponRequest {
  code?: string;
  promotion_id?: string;
  usage_limit?: number | null;
  starts_at?: Timestamp | null;
  ends_at?: Timestamp | null;
  metadata?: Metadata;
}

export interface ValidateCouponRequest {
  code: string;
}

export interface CouponValidationResponse {
  valid: boolean;
  coupon?: Coupon;
}

export interface PromotionUsageResponse {
  usage_count: number;
}

export interface PromotionListQuery extends PaginationQuery {
  status?: PromotionStatus;
  is_automatic?: boolean;
  promotion_type?: PromotionType;
}

export interface CouponListQuery extends PaginationQuery {
  promotion_id?: string;
}

// ---------------------------------------------------------------------------
// Payments
// ---------------------------------------------------------------------------

export type PaymentStatus =
  | "pending"
  | "requires_action"
  | "authorized"
  | "captured"
  | "partially_refunded"
  | "refunded"
  | "failed"
  | "canceled";

export type PaymentRefundStatus = "pending" | "succeeded" | "failed";

export type PaymentWebhookStatus = "received" | "processed" | "failed";

export interface Payment {
  id: string;
  order_id: string | null;
  provider: string;
  provider_payment_id: string | null;
  amount: number;
  currency_code: CurrencyCode;
  status: PaymentStatus;
  captured_at: Timestamp | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface PaymentSession {
  id: string;
  cart_id: string | null;
  checkout_id: string | null;
  order_id: string | null;
  provider: string;
  provider_session_id: string | null;
  amount: number;
  currency_code: CurrencyCode;
  status: PaymentStatus;
  data: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface PaymentAttempt {
  id: string;
  payment_session_id: string;
  amount: number;
  currency_code: CurrencyCode;
  status: PaymentStatus;
  provider_reference: string | null;
  error: string | null;
  created_at: Timestamp;
}

export interface PaymentRefund {
  id: string;
  payment_id: string | null;
  order_id: string | null;
  amount: number;
  currency_code: CurrencyCode;
  status: PaymentRefundStatus;
  provider_refund_id: string | null;
  reason: string | null;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface PaymentWebhookRecord {
  id: string;
  provider: string;
  provider_event_id: string;
  event_type: string;
  payload: string;
  status: PaymentWebhookStatus;
  attempts: number;
  processed_at: Timestamp | null;
  last_error: string | null;
  created_at: Timestamp;
}

export interface PaymentListQuery extends PaginationQuery {
  order_id?: string;
  status?: PaymentStatus;
}

export interface CreatePaymentRefundRequest {
  amount: number;
  reason?: string | null;
  idempotency_key?: string | null;
}

export interface FailPaymentSessionRequest {
  reason?: string | null;
}

// ---------------------------------------------------------------------------
// Webhooks
// ---------------------------------------------------------------------------

export type WebhookDeliveryStatus = "pending" | "delivering" | "delivered" | "failed";

export interface WebhookEndpoint {
  id: string;
  url: string;
  events: string[];
  is_active: boolean;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface CreatedWebhookEndpoint extends WebhookEndpoint {
  secret: string;
}

export interface RotatedWebhookSecret {
  secret: string;
}

export interface WebhookDelivery {
  id: string;
  endpoint_id: string;
  event_id: string;
  event_type: string;
  payload: JsonValue;
  status: WebhookDeliveryStatus;
  attempts: number;
  response_status: number | null;
  response_body: string | null;
  next_retry_at: Timestamp | null;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface CreateWebhookRequest {
  url: string;
  events: string[];
  secret?: string | null;
  metadata?: Metadata | null;
}

export interface UpdateWebhookRequest {
  url?: string;
  events?: string[];
  is_active?: boolean;
}

export interface WebhookDeliveriesQuery {
  limit?: number;
}

// ---------------------------------------------------------------------------
// Themes
// ---------------------------------------------------------------------------

/** Bundle-relative references to a theme editor surface. */
export interface ThemeEditorRef {
  /** ESM entry that exports render and field components. */
  entry: string;
  /** JSON file describing categories, components and fields. */
  manifest: string;
  /** Editor manifest schema version. */
  schema_version?: number;
}

/** One capability required by a theme. */
export interface CapabilityRequirement {
  /** Capability identifier. */
  id: string;
  /** Required capability version. */
  version: number;
  /** Whether activation may proceed when the capability is missing. */
  optional?: boolean;
}

/** Platform contracts and capabilities a theme depends on. */
export interface ThemeRequirements {
  /** Required storefront data API version. */
  data_api?: number;
  /** Optional or required plugin capabilities. */
  capabilities?: CapabilityRequirement[];
}

export interface ThemeManifest {
  id: string;
  name: string;
  version: string;
  /** Rendering runtime expected by the theme, for example `v8` or `node`. */
  runtime: string;
  /** Bundle-relative entry file. */
  entry: string;
  settings_schema?: string | null;
  assets_manifest?: string | null;
  /** Optional editor bundle and manifest references. */
  editor?: ThemeEditorRef | null;
  /** Optional platform contracts and capabilities the theme depends on. */
  requires?: ThemeRequirements | null;
  /**
   * Dynamic routes keyed by template page type, for example
   * `{ "product": "/products/{handle}", "collection": "/collections/{handle}" }`.
   */
  routes?: Record<string, string> | null;
}

export interface Theme {
  id: string;
  manifest: ThemeManifest;
  bundle_path: string;
  settings_schema: JsonValue | null;
  settings: JsonValue;
  /** Raw editor manifest, when the theme exposes an editor. */
  editor_manifest?: JsonValue | null;
  /** Editor manifest schema version. */
  schema_version?: number;
  /** Parsed asset manifest, when the backend exposes one. */
  assets_manifest?: JsonValue | null;
  /** Working settings that have not been published yet. */
  draft_settings?: JsonValue | null;
  /** Optimistic concurrency token for theme settings. */
  settings_revision?: number | null;
  is_active: boolean;
  created_at: Timestamp;
  updated_at: Timestamp;
}

/** The active theme exposed to storefronts. */
export interface StoreTheme {
  id: string;
  manifest: ThemeManifest;
  /** Bundle-relative asset root; omitted by the storefront DTO. */
  bundle_path?: string;
  settings: JsonValue;
}

export interface RegisterThemeRequest {
  manifest: ThemeManifest;
  bundle_path: string;
  settings_schema?: JsonValue | null;
  settings?: JsonValue | null;
  /** Raw editor manifest; required when `manifest.editor` is present. */
  editor_manifest?: JsonValue | null;
  /** Parsed asset manifest, when the bundle publishes one. */
  assets_manifest?: JsonValue | null;
}

export interface UpdateThemeSettingsRequest {
  settings: JsonValue;
  /** Revision the caller last observed, when supported by the backend. */
  base_revision?: number | null;
}

// ---------------------------------------------------------------------------
// Theme editor manifest
// ---------------------------------------------------------------------------

export type ThemeEditorFieldType =
  | "text"
  | "textarea"
  | "number"
  | "boolean"
  | "select"
  | "object"
  | "array"
  | "data_source"
  | "asset"
  | "custom";

export type ThemeDataSourceKind =
  | "collection"
  | "product"
  | "product_list"
  | "page"
  | "navigation"
  | "asset"
  | "search"
  | "inquiry_form";

export interface ThemeEditorFieldOption {
  value: string;
  label: string;
}

export interface ThemeEditorField {
  type: ThemeEditorFieldType | string;
  /** Human-readable label shown by the editor. */
  label?: string | null;
  /** Optional help text shown under the input. */
  description?: string | null;
  /** Optional picker configuration for data_source and asset fields. */
  picker?: JsonValue | null;
  /** Data source kind for `data_source` and `asset` fields. */
  source?: string | null;
  required?: boolean;
  min?: number | null;
  max?: number | null;
  options?: ThemeEditorFieldOption[] | null;
  default?: JsonValue;
  /** Nested field definitions for `object` fields. */
  fields?: Record<string, ThemeEditorField> | null;
  /** Item definition for `array` fields. */
  items?: ThemeEditorField | null;
}

export interface ThemeEditorSlot {
  name: string;
  title?: string | null;
  /** Component types allowed in the slot; absent allows every component. */
  allowed_components?: string[] | null;
}

export interface ThemeEditorComponent {
  title: string;
  /** Optional short description shown in the palette or inspector. */
  description?: string | null;
  /** Optional icon name understood by the admin editor. */
  icon?: string | null;
  category?: string | null;
  fields?: Record<string, ThemeEditorField>;
  defaultProps?: JsonValue;
  slots?: ThemeEditorSlot[];
}

/** One named page zone (header, footer, sidebar) exposed by the theme. */
export interface ThemeEditorZone {
  title: string;
  /** Component types allowed in the zone; absent allows every component. */
  allowed_components?: string[] | null;
}

export interface ThemeEditorCategory {
  title: string;
  components?: string[];
}

export interface ThemeEditorRoot {
  fields?: Record<string, ThemeEditorField>;
}

export interface ThemeEditorManifest {
  schema_version?: number;
  categories?: Record<string, ThemeEditorCategory>;
  components?: Record<string, ThemeEditorComponent>;
  root?: ThemeEditorRoot;
  /** Named page zones available to authors. */
  zones?: Record<string, ThemeEditorZone>;
}

// ---------------------------------------------------------------------------
// Theme settings revisions, capabilities and validation
// ---------------------------------------------------------------------------

export type ThemeSettingsRevisionStatus = "draft" | "published";
/** @deprecated Use {@link ThemeSettingsRevisionStatus}. */
export type ThemeSettingsRevisionKind = ThemeSettingsRevisionStatus;

/** One recorded theme activation. */
export interface ThemeActivation {
  id: string;
  theme_id: string;
  manifest_id?: string | null;
  theme_name?: string | null;
  theme_version?: string | null;
  previous_theme_id?: string | null;
  created_at: Timestamp;
}

export interface ThemeSettingsRevision {
  id: string;
  theme_id: string;
  revision: number;
  settings: JsonValue;
  status: ThemeSettingsRevisionStatus;
  created_by?: string | null;
  created_at: Timestamp;
}

/** Settings state returned by the draft, publish and restore endpoints. */
export interface ThemeSettingsState {
  theme_id: string;
  settings: JsonValue;
  draft_settings?: JsonValue | null;
  settings_revision: number;
}

export interface SaveThemeSettingsDraftRequest {
  settings: JsonValue;
  /** Revision the caller last observed, enabling optimistic concurrency. */
  base_revision?: number | null;
}

export interface PublishThemeSettingsRequest {
  /** Revision the caller last observed, enabling optimistic concurrency. */
  base_revision?: number | null;
}

export interface RestoreThemeSettingsRequest {
  /** Optional optimistic revision, when a backend accepts one. */
  base_revision?: number | null;
}

/** A runtime capability endpoint bound to a theme. */
export interface ThemeCapabilityBinding {
  id: string;
  theme_id: string;
  capability_id: string;
  endpoint: string;
  config: JsonValue;
  is_active: boolean;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface UpsertThemeCapabilityRequest {
  capability_id: string;
  endpoint: string;
  config?: JsonValue;
  is_active?: boolean;
}

export interface DeleteThemeCapabilityRequest {
  capability_id: string;
}

/** One page that failed validation against a theme editor manifest. */
export interface ThemePageValidationIssue {
  page_id: string;
  handle: string;
  error: string;
}

export interface ThemePageValidationResult {
  valid: boolean;
  issues: ThemePageValidationIssue[];
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

export type PageStatus = "draft" | "published" | "archived";
export type PageVersionKind = "draft" | "published";
export type PageRevisionOrigin = "manual" | "ai";

/** Theme metadata captured with a page body. */
export interface PageThemeRef {
  /** Theme manifest identifier. */
  id: string;
  /** Theme version. */
  version: string;
  /** Editor schema version used when the body was written. */
  schema_version?: number;
}

/** Root property values for a page body. */
export interface PageRoot {
  props?: JsonValue;
}

/** One block in a structured page body. */
export interface PageBlock {
  /** Registered component type. */
  type: string;
  /** Component props. The `id` prop is the stable block identifier. */
  props?: JsonValue;
  /** Nested slot content keyed by declared slot name. */
  slots?: Record<string, PageBlock[]>;
}

/** A versioned page definition compatible with the theme editor data model. */
export interface PageBody {
  schema_version?: number;
  /** Theme the body was authored against. */
  theme?: PageThemeRef | null;
  /** Ordered page blocks. */
  content?: PageBlock[];
  root?: PageRoot;
  /** Named top-level regions such as header or footer. */
  zones?: Record<string, PageBlock[]>;
}

/**
 * A stored data reference. The backend stores bindings as tagged JSON
 * objects, for example `{ "source": "collection", "id": "col_123" }`.
 */
export interface DataBinding {
  source: ThemeDataSourceKind;
  id?: string;
  handle?: string;
  query?: string;
  limit?: number;
  collection_id?: string;
  tag?: string;
  sort?: string;
}

export interface Page {
  id: string;
  title: string;
  handle: string;
  status: PageStatus;
  page_type: string;
  /** Optimistic concurrency token; incremented on every write. */
  revision: number;
  body: JsonValue;
  /** Body last published to storefronts, when the page has been published. */
  published_body?: JsonValue | null;
  seo: JsonValue;
  metadata: Metadata;
  published_at: Timestamp | null;
  created_at: Timestamp;
  updated_at: Timestamp;
  deleted_at: Timestamp | null;
}

export interface PageVersion {
  id: string;
  page_id: string;
  version: number;
  body: JsonValue;
  kind: PageVersionKind;
  origin: PageRevisionOrigin;
  created_by?: string | null;
  created_at: Timestamp;
}

export interface PageListQuery extends PaginationQuery {
  status?: PageStatus;
  include_deleted?: boolean;
}

export interface CreatePageRequest {
  title: string;
  handle?: string | null;
  page_type?: string | null;
  body?: JsonValue;
  seo?: JsonValue;
  metadata?: Metadata;
}

/** Partial page update; `base_revision` enables optimistic concurrency. */
export interface UpdatePageRequest {
  base_revision: number;
  origin?: PageRevisionOrigin;
  title?: string;
  handle?: string;
  page_type?: string;
  body?: JsonValue;
  seo?: JsonValue;
  metadata?: Metadata;
}

export interface RestorePageVersionQuery {
  base_revision?: number;
}

export interface PreviewTokenResponse {
  token: string;
  expires_at: Timestamp;
  jti: string;
}

export interface RevokePreviewTokenRequest {
  jti: string;
}

// ---------------------------------------------------------------------------
// Editor data-source pickers
// ---------------------------------------------------------------------------

export interface DataPickerItem {
  id: string;
  handle: string;
  label: string;
}

export interface DataPickerResponse {
  items: DataPickerItem[];
  /** Offset of the next page, or `null` when this is the last page. */
  next: number | null;
}

export interface DataSourceQuery {
  search?: string;
  limit?: number;
  offset?: number;
  collection_id?: string;
  tag?: string;
  published?: boolean;
  sort?: string;
}

// ---------------------------------------------------------------------------
// Media assets
// ---------------------------------------------------------------------------

export interface Asset {
  id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  storage_key: string;
  alt: string | null;
  width: number | null;
  height: number | null;
  created_by: string | null;
  created_at: Timestamp;
  deleted_at: Timestamp | null;
}

export interface AssetListQuery extends PaginationQuery {
  search?: string;
}

export interface UploadAssetFields {
  alt?: string;
  width?: number;
  height?: number;
}

export interface NavigationItem {
  id: string;
  navigation_id: string;
  parent_id: string | null;
  title: string;
  url: string;
  rank: number;
  metadata: Metadata;
}

export interface NavigationTreeItem extends NavigationItem {
  children: NavigationTreeItem[];
}

export interface NavigationTree {
  id: string;
  name: string;
  handle: string;
  metadata: Metadata;
  items: NavigationTreeItem[];
}

// ---------------------------------------------------------------------------
// Store settings
// ---------------------------------------------------------------------------

export interface SalesChannel {
  id: string;
  name: string;
  description: string | null;
  is_disabled: boolean;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface CreateSalesChannelRequest {
  name: string;
  description?: string | null;
  metadata?: Metadata;
}

export interface UpdateSalesChannelRequest {
  name?: string;
  description?: string;
  metadata?: Metadata;
}

export type StoreSettings = Record<string, string>;

export interface SettingsUpdateRequest {
  values: StoreSettings;
}

// ---------------------------------------------------------------------------
// Storefront response envelopes (Medusa v2 compatible)
//
// Public `/api/v1/store` and `/internal/storefront` responses wrap resources and
// lists in named envelopes. Admin responses still use `PageResult<T>`.
// ---------------------------------------------------------------------------

export interface ProductResponse<TProduct = Product> {
  product: TProduct;
}

export interface ProductListResponse<TProduct = Product> {
  products: TProduct[];
  count: number;
  offset: number;
  limit: number;
}

export type ProductSearchResponse<TProduct = Product> = ProductListResponse<TProduct>;

export interface CategoryResponse {
  category: Category;
}

export interface CategoryListResponse {
  product_categories: Category[];
  count: number;
  offset: number;
  limit: number;
}

export interface CollectionResponse {
  collection: Collection;
}

export interface CollectionListResponse {
  collections: Collection[];
  count: number;
  offset: number;
  limit: number;
}

export interface CartResponse {
  cart: Cart;
}

export interface CheckoutResponse {
  checkout: CheckoutSession;
}

export interface CheckoutHistoryResponse {
  history: CheckoutStateHistory[];
}

export interface CustomerResponse {
  customer: Customer;
}

export interface CustomerAddressResponse {
  address: CustomerAddress;
}

export interface CustomerAddressListResponse {
  addresses: CustomerAddress[];
}

export interface CustomerGroupsResponse {
  customer_groups: CustomerGroup[];
}

export interface OrderResponse {
  order: Order;
}

export interface OrderListResponse {
  orders: Order[];
  count: number;
  offset: number;
  limit: number;
}

export interface StorePageResponse {
  page: Page;
}

export interface NavigationResponse {
  navigation: NavigationTree;
}

export interface Region {
  id: string;
  name: string;
  currency_code: CurrencyCode;
  countries: CountryCode[];
  tax_region_id: string | null;
  payment_providers: string[];
  is_default: boolean;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface RegionResponse {
  region: Region;
}

export interface RegionListResponse {
  regions: Region[];
  count: number;
  offset: number;
  limit: number;
}

export interface StoreThemeResponse {
  theme: StoreTheme;
}

export interface VariantAvailabilityResponse {
  variants: VariantAvailability[];
}

export interface ShippingOptionsResponse {
  shipping_options: ResolvedShippingOption[];
}

// ---------------------------------------------------------------------------
// Inquiry forms, inquiries and notification channels
// ---------------------------------------------------------------------------

export type InquiryFieldKind =
  | "text"
  | "textarea"
  | "email"
  | "phone"
  | "number"
  | "select"
  | "radio"
  | "checkbox"
  | "date"
  | "file"
  | "hidden"
  | "consent";

export interface InquiryFieldOption {
  value: string;
  label: string;
}

export interface InquiryFieldValidation {
  min_length?: number | null;
  max_length?: number | null;
  min?: number | null;
  max?: number | null;
}

export interface InquiryFileFieldConfig {
  max_files: number;
  max_bytes: number;
  accept: string[];
}

export interface InquiryFormField {
  key: string;
  kind: InquiryFieldKind;
  label: string;
  required: boolean;
  placeholder?: string | null;
  help?: string | null;
  default?: unknown;
  options: InquiryFieldOption[];
  validation: InquiryFieldValidation;
  file?: InquiryFileFieldConfig | null;
}

export interface InquiryFormSettings {
  submit_label: string;
  success_message: string;
  redirect_url?: string | null;
  consent_text?: string | null;
  consent_required: boolean;
  allow_attachments: boolean;
  notification_fields: string[];
}

export type InquiryFormStatus = "draft" | "published" | "archived";

export interface InquiryForm {
  id: string;
  name: string;
  handle: string;
  description?: string | null;
  status: InquiryFormStatus;
  current_version: number;
  published_version?: number | null;
  created_at: number;
  updated_at: number;
}

export interface InquiryFormRevision {
  id: string;
  form_id: string;
  version: number;
  fields: InquiryFormField[];
  settings: InquiryFormSettings;
  created_by?: string | null;
  created_at: number;
  published_at?: number | null;
}

export interface InquiryFormDetail {
  form: InquiryForm;
  draft?: InquiryFormRevision | null;
  published_revision?: InquiryFormRevision | null;
}

export type InquiryStatus = "new" | "read" | "replied" | "closed" | "spam";

export interface Inquiry {
  id: string;
  event_id: string;
  form_id: string;
  form_revision_id: string;
  form_version: number;
  status: InquiryStatus;
  name?: string | null;
  email?: string | null;
  phone?: string | null;
  subject?: string | null;
  message?: string | null;
  data: Record<string, unknown>;
  source: string;
  page_url?: string | null;
  locale?: string | null;
  idempotency_key?: string | null;
  metadata: Record<string, unknown>;
  created_at: number;
  updated_at: number;
}

export interface InquiryFile {
  id: string;
  inquiry_id?: string | null;
  field_key: string;
  original_name?: string | null;
  content_type: string;
  size_bytes: number;
  checksum_sha256: string;
  status: "pending" | "claimed" | "expired";
  created_at: number;
}

export interface UploadInquiryFileResponse {
  id: string;
  expires_at: number;
}

export type NotificationChannelKind = "feishu";

export interface NotificationChannel {
  id: string;
  kind: NotificationChannelKind;
  name: string;
  events: string[];
  is_active: boolean;
  config: Record<string, unknown>;
  endpoint_url_masked: string;
  signing_secret_configured: boolean;
  last_error?: string | null;
  last_success_at?: number | null;
  created_at: number;
  updated_at: number;
}

export interface NotificationChannelEvent {
  type: string;
  label: string;
  group: string;
  description: string;
}

export type NotificationDeliveryStatus = "pending" | "queued" | "sent" | "failed" | "skipped";

export interface NotificationChannelDelivery {
  id: string;
  channel_id: string;
  event_id: string;
  event_type: string;
  status: NotificationDeliveryStatus;
  job_id?: string | null;
  attempts: number;
  response_status?: number | null;
  response_body?: string | null;
  last_error?: string | null;
  created_at: number;
  updated_at: number;
  sent_at?: number | null;
}

export interface InquiryListResponse {
  data: Inquiry[];
  pagination: Pagination;
}

export interface CreateInquiryFormRequest {
  name: string;
  handle?: string;
  description?: string | null;
}

export interface UpdateInquiryFormRequest {
  name?: string;
  description?: string | null;
}

export interface SaveInquiryFormDraftRequest {
  fields: InquiryFormField[];
  settings: InquiryFormSettings;
}

export interface PublishInquiryFormRequest {
  version?: number;
}

export interface InquiryFormListQuery extends PaginationQuery {
  status?: InquiryFormStatus;
}

export interface InquiryListQuery extends PaginationQuery {
  status?: InquiryStatus;
  form_id?: string;
  query?: string;
  from?: number;
  to?: number;
}

export interface UpdateInquiryStatusRequest {
  status: InquiryStatus;
}

export interface InquiryUnreadCountResponse {
  count: number;
}

/** The detail endpoint may include the revision schema and claimed files. */
export interface InquiryDetail extends Inquiry {
  revision?: InquiryFormRevision | null;
  files?: InquiryFile[];
}

export interface CreateNotificationChannelRequest {
  kind: NotificationChannelKind;
  name: string;
  endpoint_url: string;
  signing_secret?: string | null;
  events: string[];
  is_active: boolean;
  config?: Record<string, unknown>;
}

export interface UpdateNotificationChannelRequest {
  name?: string;
  endpoint_url?: string;
  signing_secret?: string | null;
  events?: string[];
  is_active?: boolean;
  config?: Record<string, unknown>;
}

export interface NotificationChannelTestResponse {
  ok: boolean;
  status?: number | null;
  message?: string | null;
  error?: string | null;
}

export interface NotificationDeliveriesQuery {
  limit?: number;
}

// ---------------------------------------------------------------------------
// Public storefront inquiry contracts
// ---------------------------------------------------------------------------

export interface StoreInquiryFormSettings {
  submit_label: string;
  success_message: string;
  redirect_url?: string | null;
  consent_text?: string | null;
  consent_required: boolean;
  allow_attachments: boolean;
}

export interface StoreInquiryForm {
  handle: string;
  version: number;
  fields: InquiryFormField[];
  settings: StoreInquiryFormSettings;
}

export interface StoreInquiryFormResponse {
  form: StoreInquiryForm;
}

export interface CreateInquiryRequest {
  form_handle: string;
  name?: string | null;
  email?: string | null;
  phone?: string | null;
  subject?: string | null;
  message?: string | null;
  data?: Record<string, unknown>;
  files?: Record<string, string[]>;
  page_url?: string | null;
  locale?: string | null;
  consent?: boolean;
}

export interface CreateInquiryResponse {
  id: string;
  status?: InquiryStatus;
  event_id?: string;
  idempotency_key?: string | null;
}
