import {
  createApiClient,
  type ApiClientOptions,
  type CallOptions,
  type RequestOptions,
} from "@fastcart/api-client";
import type {
  AvailabilityResponse,
  CancelOrderRequest,
  Category,
  Collection,
  CollectionProductsRequest,
  CommittedResponse,
  Coupon,
  CouponListQuery,
  CouponValidationResponse,
  CreateCategoryRequest,
  CreateCollectionRequest,
  CreateCouponRequest,
  CreateImageRequest,
  CreateInventoryItemRequest,
  CreateInventoryLocationRequest,
  CreatePriceListRequest,
  CreateProductRequest,
  CreatePromotionRequest,
  CreateReservationsRequest,
  CreateShippingOptionRequest,
  CreateShippingProfileRequest,
  CreateShippingRateRequest,
  CreateStoreRequest,
  CreateTaxCategoryRequest,
  CreateTaxRateRequest,
  CreateTaxRegionRequest,
  CreateVariantRequest,
  CreateWebhookRequest,
  CreatedWebhookEndpoint,
  CreateCustomerGroupRequest,
  Customer,
  CustomerAddress,
  CustomerAddressInput,
  CustomerListQuery,
  DeleteStoreRequest,
  Domain,
  DomainRequest,
  DestinationOptionsQuery,
  FulfillOrderRequest,
  ImageReorderRequest,
  InventoryAdjustmentRequest,
  InventoryItem,
  InventoryLevel,
  InventoryLevelsQuery,
  InventoryLocation,
  InventoryMovement,
  InventoryMovementsQuery,
  InventoryReservation,
  InventoryTransferRequest,
  InventoryTransferResponse,
  LoginRequest,
  LoginResponse,
  Membership,
  MembershipRequest,
  MeResponse,
  Order,
  OrderListQuery,
  OrderStatusHistory,
  PageResult,
  PaginationQuery,
  Payment,
  PaymentAttempt,
  PaymentListQuery,
  PaymentRefund,
  PaymentSession,
  PaymentWebhookRecord,
  Price,
  PriceList,
  PriceListQuery,
  PrimaryDomainRequest,
  Product,
  ProductImage,
  ProductListQuery,
  Promotion,
  PromotionListQuery,
  PromotionUsageResponse,
  ProvisioningResponse,
  Refund,
  RegisterThemeRequest,
  ReleasedResponse,
  ResolvedShippingOption,
  SalesChannel,
  SearchQuery,
  SelectedShippingRate,
  SelectRateQuery,
  SettingsUpdateRequest,
  ShippingOption,
  ShippingOptionsListQuery,
  ShippingProfile,
  ShippingRate,
  Store,
  StoreSettings,
  Tag,
  TagRequest,
  TaxCalculationResponse,
  TaxCalculateRequest,
  TaxCategory,
  TaxRate,
  TaxRateListQuery,
  TaxRegion,
  Theme,
  CreatePaymentRefundRequest,
  FailPaymentSessionRequest,
  UpdateCategoryRequest,
  UpdateCollectionRequest,
  UpdateCouponRequest,
  UpdateCustomerRequest,
  UpdateInventoryLocationRequest,
  UpdatePriceListRequest,
  UpdateProductRequest,
  UpdatePromotionRequest,
  UpdateShippingOptionRequest,
  UpdateShippingProfileRequest,
  UpdateShippingRateRequest,
  UpdateStoreRequest,
  UpdateTaxCategoryRequest,
  UpdateTaxRateRequest,
  UpdateTaxRegionRequest,
  UpdateThemeSettingsRequest,
  UpdateVariantRequest,
  UpdateWebhookRequest,
  Variant,
  WebhookDeliveriesQuery,
  WebhookDelivery,
  WebhookEndpoint,
} from "@fastcart/shared-types";

/** Options accepted by {@link createAdminClient}. */
export interface AdminClientOptions extends ApiClientOptions {
  /** Store selected with the `x-store-id` header on every admin request. */
  storeId: string;
  /** Double-submit CSRF token, or a provider called for every unsafe request. */
  csrfToken?: string | (() => string | null);
}

/** Per-call configuration shared by every admin method. */
export interface AdminRequestOptions extends CallOptions {
  /** Overrides the client-level CSRF token for this request. */
  csrfToken?: string;
}

/** Typed administration SDK for the `/api/v1/admin` and platform namespaces. */
export interface AdminClient {
  /** Store selected by this client. */
  readonly storeId: string;
  /** Updates the CSRF token captured from login or supplied out of band. */
  setCsrfToken(token: string | null): void;

  auth: {
    me(options?: AdminRequestOptions): Promise<MeResponse>;
    login(body: LoginRequest, options?: AdminRequestOptions): Promise<LoginResponse>;
    logout(options?: AdminRequestOptions): Promise<void>;
  };

  stores: {
    list(query?: PaginationQuery, options?: AdminRequestOptions): Promise<PageResult<Store>>;
    get(storeId: string, options?: AdminRequestOptions): Promise<Store>;
    create(body: CreateStoreRequest, options?: AdminRequestOptions): Promise<Store>;
    update(
      storeId: string,
      body: UpdateStoreRequest,
      options?: AdminRequestOptions,
    ): Promise<Store>;
    activate(storeId: string, options?: AdminRequestOptions): Promise<Store>;
    suspend(storeId: string, options?: AdminRequestOptions): Promise<Store>;
    resume(storeId: string, options?: AdminRequestOptions): Promise<Store>;
    provision(storeId: string, options?: AdminRequestOptions): Promise<ProvisioningResponse>;
    delete(
      storeId: string,
      body?: DeleteStoreRequest,
      options?: AdminRequestOptions,
    ): Promise<Store>;
    domains: {
      list(storeId: string, options?: AdminRequestOptions): Promise<Domain[]>;
      add(storeId: string, body: DomainRequest, options?: AdminRequestOptions): Promise<Domain>;
      remove(storeId: string, hostname: string, options?: AdminRequestOptions): Promise<void>;
      setPrimary(
        storeId: string,
        body: PrimaryDomainRequest,
        options?: AdminRequestOptions,
      ): Promise<Domain>;
    };
    memberships: {
      list(storeId: string, options?: AdminRequestOptions): Promise<Membership[]>;
      upsert(
        storeId: string,
        body: MembershipRequest,
        options?: AdminRequestOptions,
      ): Promise<Membership>;
      remove(storeId: string, userId: string, options?: AdminRequestOptions): Promise<void>;
    };
  };

  themes: {
    list(query?: PaginationQuery, options?: AdminRequestOptions): Promise<PageResult<Theme>>;
    active(options?: AdminRequestOptions): Promise<Theme>;
    get(id: string, options?: AdminRequestOptions): Promise<Theme>;
    register(body: RegisterThemeRequest, options?: AdminRequestOptions): Promise<Theme>;
    activate(id: string, options?: AdminRequestOptions): Promise<Theme>;
    rollback(options?: AdminRequestOptions): Promise<Theme>;
    delete(id: string, options?: AdminRequestOptions): Promise<void>;
    getSettings(id: string, options?: AdminRequestOptions): Promise<unknown>;
    updateSettings(
      id: string,
      body: UpdateThemeSettingsRequest,
      options?: AdminRequestOptions,
    ): Promise<Theme>;
  };

  products: {
    list(query?: ProductListQuery, options?: AdminRequestOptions): Promise<PageResult<Product>>;
    get(id: string, options?: AdminRequestOptions): Promise<Product>;
    create(body: CreateProductRequest, options?: AdminRequestOptions): Promise<Product>;
    update(id: string, body: UpdateProductRequest, options?: AdminRequestOptions): Promise<Product>;
    delete(id: string, options?: AdminRequestOptions): Promise<void>;
    publish(id: string, options?: AdminRequestOptions): Promise<Product>;
    archive(id: string, options?: AdminRequestOptions): Promise<Product>;
    search(query: SearchQuery, options?: AdminRequestOptions): Promise<PageResult<Product>>;
    getVariant(id: string, options?: AdminRequestOptions): Promise<Variant>;
    createVariant(
      productId: string,
      body: CreateVariantRequest,
      options?: AdminRequestOptions,
    ): Promise<Variant>;
    updateVariant(
      id: string,
      body: UpdateVariantRequest,
      options?: AdminRequestOptions,
    ): Promise<Variant>;
    deleteVariant(id: string, options?: AdminRequestOptions): Promise<void>;
    images: {
      list(productId: string, options?: AdminRequestOptions): Promise<ProductImage[]>;
      add(
        productId: string,
        body: CreateImageRequest,
        options?: AdminRequestOptions,
      ): Promise<ProductImage>;
      remove(imageId: string, options?: AdminRequestOptions): Promise<void>;
      reorder(
        productId: string,
        body: ImageReorderRequest,
        options?: AdminRequestOptions,
      ): Promise<ProductImage[]>;
    };
  };

  categories: {
    list(
      query?: PaginationQuery & { parent_id?: string },
      options?: AdminRequestOptions,
    ): Promise<PageResult<Category>>;
    get(id: string, options?: AdminRequestOptions): Promise<Category>;
    create(body: CreateCategoryRequest, options?: AdminRequestOptions): Promise<Category>;
    update(
      id: string,
      body: UpdateCategoryRequest,
      options?: AdminRequestOptions,
    ): Promise<Category>;
    delete(id: string, options?: AdminRequestOptions): Promise<void>;
  };

  collections: {
    list(query?: PaginationQuery, options?: AdminRequestOptions): Promise<PageResult<Collection>>;
    get(id: string, options?: AdminRequestOptions): Promise<Collection>;
    create(body: CreateCollectionRequest, options?: AdminRequestOptions): Promise<Collection>;
    update(
      id: string,
      body: UpdateCollectionRequest,
      options?: AdminRequestOptions,
    ): Promise<Collection>;
    delete(id: string, options?: AdminRequestOptions): Promise<void>;
    publish(id: string, options?: AdminRequestOptions): Promise<Collection>;
    unpublish(id: string, options?: AdminRequestOptions): Promise<Collection>;
    addProducts(
      id: string,
      body: CollectionProductsRequest,
      options?: AdminRequestOptions,
    ): Promise<void>;
    removeProduct(
      collectionId: string,
      productId: string,
      options?: AdminRequestOptions,
    ): Promise<void>;
  };

  tags: {
    list(query?: PaginationQuery, options?: AdminRequestOptions): Promise<PageResult<Tag>>;
    create(body: TagRequest, options?: AdminRequestOptions): Promise<Tag>;
    attach(
      productId: string,
      body: { tag_id: string },
      options?: AdminRequestOptions,
    ): Promise<Tag>;
    detach(productId: string, tagId: string, options?: AdminRequestOptions): Promise<void>;
  };

  orders: {
    list(query?: OrderListQuery, options?: AdminRequestOptions): Promise<PageResult<Order>>;
    get(id: string, options?: AdminRequestOptions): Promise<Order>;
    getByNumber(displayNumber: number, options?: AdminRequestOptions): Promise<Order>;
    confirm(id: string, options?: AdminRequestOptions): Promise<Order>;
    process(id: string, options?: AdminRequestOptions): Promise<Order>;
    cancel(id: string, body?: CancelOrderRequest, options?: AdminRequestOptions): Promise<Order>;
    fulfill(id: string, body: FulfillOrderRequest, options?: AdminRequestOptions): Promise<Order>;
    history(id: string, options?: AdminRequestOptions): Promise<OrderStatusHistory[]>;
    refunds: {
      list(id: string, options?: AdminRequestOptions): Promise<Refund[]>;
      create(
        id: string,
        body: { payment_id: string; amount: number; currency: string; reason?: string | null },
        options?: AdminRequestOptions,
      ): Promise<Order>;
    };
  };

  customers: {
    list(query?: CustomerListQuery, options?: AdminRequestOptions): Promise<PageResult<Customer>>;
    get(id: string, options?: AdminRequestOptions): Promise<Customer>;
    update(
      id: string,
      body: UpdateCustomerRequest,
      options?: AdminRequestOptions,
    ): Promise<Customer>;
    delete(id: string, options?: AdminRequestOptions): Promise<void>;
    addresses: {
      list(customerId: string, options?: AdminRequestOptions): Promise<CustomerAddress[]>;
      create(
        customerId: string,
        body: CustomerAddressInput,
        options?: AdminRequestOptions,
      ): Promise<CustomerAddress>;
      update(
        customerId: string,
        addressId: string,
        body: CustomerAddressInput,
        options?: AdminRequestOptions,
      ): Promise<CustomerAddress>;
      remove(customerId: string, addressId: string, options?: AdminRequestOptions): Promise<void>;
    };
    groups: {
      list(customerId: string, options?: AdminRequestOptions): Promise<unknown[]>;
      listAll(options?: AdminRequestOptions): Promise<unknown[]>;
      create(body: CreateCustomerGroupRequest, options?: AdminRequestOptions): Promise<unknown>;
    };
  };

  inventory: {
    locations: {
      list(
        query?: PaginationQuery,
        options?: AdminRequestOptions,
      ): Promise<PageResult<InventoryLocation>>;
      get(id: string, options?: AdminRequestOptions): Promise<InventoryLocation>;
      create(
        body: CreateInventoryLocationRequest,
        options?: AdminRequestOptions,
      ): Promise<InventoryLocation>;
      update(
        id: string,
        body: UpdateInventoryLocationRequest,
        options?: AdminRequestOptions,
      ): Promise<InventoryLocation>;
    };
    items: {
      list(
        query?: PaginationQuery,
        options?: AdminRequestOptions,
      ): Promise<PageResult<InventoryItem>>;
      get(id: string, options?: AdminRequestOptions): Promise<InventoryItem>;
      getByVariant(variantId: string, options?: AdminRequestOptions): Promise<InventoryItem>;
      create(
        body: CreateInventoryItemRequest,
        options?: AdminRequestOptions,
      ): Promise<InventoryItem>;
    };
    levels(query: InventoryLevelsQuery, options?: AdminRequestOptions): Promise<InventoryLevel[]>;
    availability(variantId: string, options?: AdminRequestOptions): Promise<AvailabilityResponse>;
    movements(
      query: InventoryMovementsQuery,
      options?: AdminRequestOptions,
    ): Promise<InventoryMovement[]>;
    adjust(
      body: InventoryAdjustmentRequest,
      options?: AdminRequestOptions,
    ): Promise<InventoryLevel>;
    transfer(
      body: InventoryTransferRequest,
      options?: AdminRequestOptions,
    ): Promise<InventoryTransferResponse>;
    reservations: {
      create(
        body: CreateReservationsRequest,
        options?: AdminRequestOptions,
      ): Promise<InventoryReservation[]>;
      release(
        body: { reservation_ids?: string[] },
        options?: AdminRequestOptions,
      ): Promise<ReleasedResponse>;
      commit(
        body: { checkout_id: string; order_id: string },
        options?: AdminRequestOptions,
      ): Promise<CommittedResponse>;
      expire(options?: AdminRequestOptions): Promise<ReleasedResponse>;
    };
  };

  promotions: {
    list(query?: PromotionListQuery, options?: AdminRequestOptions): Promise<PageResult<Promotion>>;
    get(id: string, options?: AdminRequestOptions): Promise<Promotion>;
    create(body: CreatePromotionRequest, options?: AdminRequestOptions): Promise<Promotion>;
    update(
      id: string,
      body: UpdatePromotionRequest,
      options?: AdminRequestOptions,
    ): Promise<Promotion>;
    delete(id: string, options?: AdminRequestOptions): Promise<void>;
    usage(id: string, options?: AdminRequestOptions): Promise<PromotionUsageResponse>;
  };

  coupons: {
    list(query?: CouponListQuery, options?: AdminRequestOptions): Promise<PageResult<Coupon>>;
    get(id: string, options?: AdminRequestOptions): Promise<Coupon>;
    create(body: CreateCouponRequest, options?: AdminRequestOptions): Promise<Coupon>;
    update(id: string, body: UpdateCouponRequest, options?: AdminRequestOptions): Promise<Coupon>;
    delete(id: string, options?: AdminRequestOptions): Promise<void>;
    validate(
      body: { code: string },
      options?: AdminRequestOptions,
    ): Promise<CouponValidationResponse>;
  };

  priceLists: {
    list(query?: PaginationQuery, options?: AdminRequestOptions): Promise<PageResult<PriceList>>;
    get(id: string, options?: AdminRequestOptions): Promise<PriceList>;
    create(body: CreatePriceListRequest, options?: AdminRequestOptions): Promise<PriceList>;
    update(
      id: string,
      body: UpdatePriceListRequest,
      options?: AdminRequestOptions,
    ): Promise<PriceList>;
    delete(id: string, options?: AdminRequestOptions): Promise<void>;
    prices(query?: PriceListQuery, options?: AdminRequestOptions): Promise<PageResult<Price>>;
  };

  shipping: {
    profiles: {
      list(
        query?: PaginationQuery,
        options?: AdminRequestOptions,
      ): Promise<PageResult<ShippingProfile>>;
      get(id: string, options?: AdminRequestOptions): Promise<ShippingProfile>;
      create(
        body: CreateShippingProfileRequest,
        options?: AdminRequestOptions,
      ): Promise<ShippingProfile>;
      update(
        id: string,
        body: UpdateShippingProfileRequest,
        options?: AdminRequestOptions,
      ): Promise<ShippingProfile>;
      delete(id: string, options?: AdminRequestOptions): Promise<void>;
    };
    options: {
      list(
        query?: ShippingOptionsListQuery & PaginationQuery,
        options?: AdminRequestOptions,
      ): Promise<PageResult<ShippingOption>>;
      get(id: string, options?: AdminRequestOptions): Promise<ShippingOption>;
      create(
        body: CreateShippingOptionRequest,
        options?: AdminRequestOptions,
      ): Promise<ShippingOption>;
      update(
        id: string,
        body: UpdateShippingOptionRequest,
        options?: AdminRequestOptions,
      ): Promise<ShippingOption>;
      delete(id: string, options?: AdminRequestOptions): Promise<void>;
      forDestination(
        query: DestinationOptionsQuery,
        options?: AdminRequestOptions,
      ): Promise<ResolvedShippingOption[]>;
      selectRate(
        id: string,
        query: SelectRateQuery,
        options?: AdminRequestOptions,
      ): Promise<SelectedShippingRate | null>;
      locations: {
        add(
          id: string,
          body: { location_id: string },
          options?: AdminRequestOptions,
        ): Promise<void>;
        remove(id: string, locationId: string, options?: AdminRequestOptions): Promise<void>;
      };
    };
    rates: {
      list(optionId: string, options?: AdminRequestOptions): Promise<ShippingRate[]>;
      get(id: string, options?: AdminRequestOptions): Promise<ShippingRate>;
      create(
        optionId: string,
        body: CreateShippingRateRequest,
        options?: AdminRequestOptions,
      ): Promise<ShippingRate>;
      update(
        id: string,
        body: UpdateShippingRateRequest,
        options?: AdminRequestOptions,
      ): Promise<ShippingRate>;
      delete(id: string, options?: AdminRequestOptions): Promise<void>;
    };
  };

  tax: {
    categories: {
      list(
        query?: PaginationQuery,
        options?: AdminRequestOptions,
      ): Promise<PageResult<TaxCategory>>;
      get(id: string, options?: AdminRequestOptions): Promise<TaxCategory>;
      create(body: CreateTaxCategoryRequest, options?: AdminRequestOptions): Promise<TaxCategory>;
      update(
        id: string,
        body: UpdateTaxCategoryRequest,
        options?: AdminRequestOptions,
      ): Promise<TaxCategory>;
      delete(id: string, options?: AdminRequestOptions): Promise<void>;
    };
    regions: {
      list(query?: PaginationQuery, options?: AdminRequestOptions): Promise<PageResult<TaxRegion>>;
      get(id: string, options?: AdminRequestOptions): Promise<TaxRegion>;
      create(body: CreateTaxRegionRequest, options?: AdminRequestOptions): Promise<TaxRegion>;
      update(
        id: string,
        body: UpdateTaxRegionRequest,
        options?: AdminRequestOptions,
      ): Promise<TaxRegion>;
      delete(id: string, options?: AdminRequestOptions): Promise<void>;
    };
    rates: {
      list(query: TaxRateListQuery, options?: AdminRequestOptions): Promise<TaxRate[]>;
      get(id: string, options?: AdminRequestOptions): Promise<TaxRate>;
      create(body: CreateTaxRateRequest, options?: AdminRequestOptions): Promise<TaxRate>;
      update(
        id: string,
        body: UpdateTaxRateRequest,
        options?: AdminRequestOptions,
      ): Promise<TaxRate>;
      delete(id: string, options?: AdminRequestOptions): Promise<void>;
    };
    calculate(
      body: TaxCalculateRequest,
      options?: AdminRequestOptions,
    ): Promise<TaxCalculationResponse>;
  };

  payments: {
    list(query?: PaymentListQuery, options?: AdminRequestOptions): Promise<PageResult<Payment>>;
    get(id: string, options?: AdminRequestOptions): Promise<Payment>;
    refunds: {
      list(paymentId: string, options?: AdminRequestOptions): Promise<PaymentRefund[]>;
      create(
        paymentId: string,
        body: CreatePaymentRefundRequest,
        options?: AdminRequestOptions,
      ): Promise<PaymentRefund>;
    };
    sessions: {
      get(id: string, options?: AdminRequestOptions): Promise<PaymentSession>;
      capture(id: string, options?: AdminRequestOptions): Promise<Payment>;
      cancel(id: string, options?: AdminRequestOptions): Promise<PaymentSession>;
      fail(
        id: string,
        body?: FailPaymentSessionRequest,
        options?: AdminRequestOptions,
      ): Promise<PaymentSession>;
      attempts(id: string, options?: AdminRequestOptions): Promise<PaymentAttempt[]>;
    };
    webhooks: {
      list(
        query?: PaginationQuery,
        options?: AdminRequestOptions,
      ): Promise<PageResult<PaymentWebhookRecord>>;
      get(id: string, options?: AdminRequestOptions): Promise<PaymentWebhookRecord>;
    };
  };

  webhooks: {
    list(options?: AdminRequestOptions): Promise<WebhookEndpoint[]>;
    get(id: string, options?: AdminRequestOptions): Promise<WebhookEndpoint>;
    create(
      body: CreateWebhookRequest,
      options?: AdminRequestOptions,
    ): Promise<CreatedWebhookEndpoint>;
    update(
      id: string,
      body: UpdateWebhookRequest,
      options?: AdminRequestOptions,
    ): Promise<WebhookEndpoint>;
    delete(id: string, options?: AdminRequestOptions): Promise<void>;
    rotateSecret(id: string, options?: AdminRequestOptions): Promise<{ secret: string }>;
    deliveries(
      id: string,
      query?: WebhookDeliveriesQuery,
      options?: AdminRequestOptions,
    ): Promise<WebhookDelivery[]>;
    replay(deliveryId: string, options?: AdminRequestOptions): Promise<WebhookDelivery>;
  };

  salesChannels: {
    list(query?: PaginationQuery, options?: AdminRequestOptions): Promise<PageResult<SalesChannel>>;
    get(id: string, options?: AdminRequestOptions): Promise<SalesChannel>;
    create(
      body: { name: string; description?: string | null; metadata?: Record<string, unknown> },
      options?: AdminRequestOptions,
    ): Promise<SalesChannel>;
    update(
      id: string,
      body: { name?: string; description?: string; metadata?: Record<string, unknown> },
      options?: AdminRequestOptions,
    ): Promise<SalesChannel>;
    disable(id: string, options?: AdminRequestOptions): Promise<SalesChannel>;
    enable(id: string, options?: AdminRequestOptions): Promise<SalesChannel>;
  };

  settings: {
    get(options?: AdminRequestOptions): Promise<StoreSettings>;
    update(body: SettingsUpdateRequest, options?: AdminRequestOptions): Promise<StoreSettings>;
    getOne(key: string, options?: AdminRequestOptions): Promise<{ key: string; value: string }>;
    set(
      key: string,
      value: string,
      options?: AdminRequestOptions,
    ): Promise<{ key: string; value: string }>;
    delete(key: string, options?: AdminRequestOptions): Promise<void>;
  };
}

type AdminInit = CallOptions & {
  method?: string;
  query?: unknown;
  body?: unknown;
  csrfToken?: string;
};

const UNSAFE_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"]);

/**
 * Creates a typed administration client.
 *
 * The client sends `x-store-id` on every request, attaches the CSRF token to
 * unsafe requests, and captures the token returned by `auth.login` so callers
 * do not have to thread it through manually. Pass an `AbortSignal` through any
 * method's final options argument to cancel the request.
 */
export function createAdminClient(options: AdminClientOptions): AdminClient {
  let sessionCsrf = typeof options.csrfToken === "string" ? options.csrfToken : null;

  function csrfTokenForRequest(override?: string): string | null {
    if (override !== undefined) {
      return override;
    }
    if (typeof options.csrfToken === "function") {
      return options.csrfToken();
    }
    return sessionCsrf;
  }

  const headers = new Headers(options.headers);
  headers.set("x-store-id", options.storeId);

  const client = createApiClient({ ...options, headers });

  function request<T>(path: string, init: AdminInit = {}): Promise<T> {
    const method = (init.method ?? "GET").toUpperCase();
    const requestHeaders = new Headers(init.headers);
    if (UNSAFE_METHODS.has(method)) {
      const csrf = csrfTokenForRequest(init.csrfToken);
      if (csrf !== null && csrf.length > 0 && !requestHeaders.has("x-csrf-token")) {
        requestHeaders.set("x-csrf-token", csrf);
      }
    }
    return client.request<T>(path, { ...init, headers: requestHeaders } as RequestOptions);
  }

  const id = (value: string): string => encodeURIComponent(value);

  const login = async (
    body: LoginRequest,
    requestOptions?: AdminRequestOptions,
  ): Promise<LoginResponse> => {
    const response = await request<LoginResponse>("/api/v1/admin/auth/login", {
      ...requestOptions,
      method: "POST",
      body,
    });
    sessionCsrf = response.csrf_token;
    return response;
  };

  return {
    storeId: options.storeId,
    setCsrfToken: (token) => {
      sessionCsrf = token;
    },

    auth: {
      me: (requestOptions) => request<MeResponse>("/api/v1/admin/auth/me", requestOptions),
      login,
      logout: (requestOptions) =>
        request<void>("/api/v1/admin/auth/logout", {
          ...requestOptions,
          method: "POST",
        }),
    },

    stores: {
      list: (query, requestOptions) =>
        request<PageResult<Store>>("/api/v1/admin/stores", {
          ...requestOptions,
          query,
        }),
      get: (storeId, requestOptions) =>
        request<Store>(`/api/v1/platform/stores/${id(storeId)}`, requestOptions),
      create: (body, requestOptions) =>
        request<Store>("/api/v1/platform/stores", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      update: (storeId, body, requestOptions) =>
        request<Store>(`/api/v1/platform/stores/${id(storeId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      activate: (storeId, requestOptions) =>
        request<Store>(`/api/v1/platform/stores/${id(storeId)}/activate`, {
          ...requestOptions,
          method: "POST",
        }),
      suspend: (storeId, requestOptions) =>
        request<Store>(`/api/v1/platform/stores/${id(storeId)}/suspend`, {
          ...requestOptions,
          method: "POST",
        }),
      resume: (storeId, requestOptions) =>
        request<Store>(`/api/v1/platform/stores/${id(storeId)}/resume`, {
          ...requestOptions,
          method: "POST",
        }),
      provision: (storeId, requestOptions) =>
        request<ProvisioningResponse>(`/api/v1/platform/stores/${id(storeId)}/provision`, {
          ...requestOptions,
          method: "POST",
        }),
      delete: (storeId, body, requestOptions) =>
        request<Store>(`/api/v1/platform/stores/${id(storeId)}/delete`, {
          ...requestOptions,
          method: "POST",
          body: body ?? {},
        }),
      domains: {
        list: (storeId, requestOptions) =>
          request<Domain[]>(`/api/v1/platform/stores/${id(storeId)}/domains`, requestOptions),
        add: (storeId, body, requestOptions) =>
          request<Domain>(`/api/v1/platform/stores/${id(storeId)}/domains`, {
            ...requestOptions,
            method: "POST",
            body,
          }),
        remove: (storeId, hostname, requestOptions) =>
          request<void>(`/api/v1/platform/stores/${id(storeId)}/domains/${id(hostname)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
        setPrimary: (storeId, body, requestOptions) =>
          request<Domain>(`/api/v1/platform/stores/${id(storeId)}/domains/primary`, {
            ...requestOptions,
            method: "POST",
            body,
          }),
      },
      memberships: {
        list: (storeId, requestOptions) =>
          request<Membership[]>(
            `/api/v1/platform/stores/${id(storeId)}/memberships`,
            requestOptions,
          ),
        upsert: (storeId, body, requestOptions) =>
          request<Membership>(`/api/v1/platform/stores/${id(storeId)}/memberships`, {
            ...requestOptions,
            method: "POST",
            body,
          }),
        remove: (storeId, userId, requestOptions) =>
          request<void>(`/api/v1/platform/stores/${id(storeId)}/memberships/${id(userId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
      },
    },

    themes: {
      list: (query, requestOptions) =>
        request<PageResult<Theme>>("/api/v1/admin/themes", {
          ...requestOptions,
          query,
        }),
      active: (requestOptions) => request<Theme>("/api/v1/admin/themes/active", requestOptions),
      get: (themeId, requestOptions) =>
        request<Theme>(`/api/v1/admin/themes/${id(themeId)}`, requestOptions),
      register: (body, requestOptions) =>
        request<Theme>("/api/v1/admin/themes", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      activate: (themeId, requestOptions) =>
        request<Theme>(`/api/v1/admin/themes/${id(themeId)}/activate`, {
          ...requestOptions,
          method: "POST",
        }),
      rollback: (requestOptions) =>
        request<Theme>("/api/v1/admin/themes/rollback", {
          ...requestOptions,
          method: "POST",
        }),
      delete: (themeId, requestOptions) =>
        request<void>(`/api/v1/admin/themes/${id(themeId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
      getSettings: async (themeId, requestOptions) => {
        const theme = await request<Theme>(`/api/v1/admin/themes/${id(themeId)}`, requestOptions);
        return theme.settings;
      },
      updateSettings: (themeId, body, requestOptions) =>
        request<Theme>(`/api/v1/admin/themes/${id(themeId)}/settings`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
    },

    products: {
      list: (query, requestOptions) =>
        request<PageResult<Product>>("/api/v1/admin/catalog/products", {
          ...requestOptions,
          query,
        }),
      get: (productId, requestOptions) =>
        request<Product>(`/api/v1/admin/catalog/products/${id(productId)}`, requestOptions),
      create: (body, requestOptions) =>
        request<Product>("/api/v1/admin/catalog/products", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      update: (productId, body, requestOptions) =>
        request<Product>(`/api/v1/admin/catalog/products/${id(productId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      delete: (productId, requestOptions) =>
        request<void>(`/api/v1/admin/catalog/products/${id(productId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
      publish: (productId, requestOptions) =>
        request<Product>(`/api/v1/admin/catalog/products/${id(productId)}/publish`, {
          ...requestOptions,
          method: "POST",
        }),
      archive: (productId, requestOptions) =>
        request<Product>(`/api/v1/admin/catalog/products/${id(productId)}/archive`, {
          ...requestOptions,
          method: "POST",
        }),
      search: (query, requestOptions) =>
        request<PageResult<Product>>("/api/v1/admin/catalog/search", {
          ...requestOptions,
          query,
        }),
      getVariant: (variantId, requestOptions) =>
        request<Variant>(`/api/v1/admin/catalog/variants/${id(variantId)}`, requestOptions),
      createVariant: (productId, body, requestOptions) =>
        request<Variant>(`/api/v1/admin/catalog/products/${id(productId)}/variants`, {
          ...requestOptions,
          method: "POST",
          body,
        }),
      updateVariant: (variantId, body, requestOptions) =>
        request<Variant>(`/api/v1/admin/catalog/variants/${id(variantId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      deleteVariant: (variantId, requestOptions) =>
        request<void>(`/api/v1/admin/catalog/variants/${id(variantId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
      images: {
        list: (productId, requestOptions) =>
          request<ProductImage[]>(
            `/api/v1/admin/catalog/products/${id(productId)}/images`,
            requestOptions,
          ),
        add: (productId, body, requestOptions) =>
          request<ProductImage>(`/api/v1/admin/catalog/products/${id(productId)}/images`, {
            ...requestOptions,
            method: "POST",
            body,
          }),
        remove: (imageId, requestOptions) =>
          request<void>(`/api/v1/admin/catalog/images/${id(imageId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
        reorder: (productId, body, requestOptions) =>
          request<ProductImage[]>(
            `/api/v1/admin/catalog/products/${id(productId)}/images/reorder`,
            {
              ...requestOptions,
              method: "POST",
              body,
            },
          ),
      },
    },

    categories: {
      list: (query, requestOptions) =>
        request<PageResult<Category>>("/api/v1/admin/catalog/categories", {
          ...requestOptions,
          query,
        }),
      get: (categoryId, requestOptions) =>
        request<Category>(`/api/v1/admin/catalog/categories/${id(categoryId)}`, requestOptions),
      create: (body, requestOptions) =>
        request<Category>("/api/v1/admin/catalog/categories", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      update: (categoryId, body, requestOptions) =>
        request<Category>(`/api/v1/admin/catalog/categories/${id(categoryId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      delete: (categoryId, requestOptions) =>
        request<void>(`/api/v1/admin/catalog/categories/${id(categoryId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
    },

    collections: {
      list: (query, requestOptions) =>
        request<PageResult<Collection>>("/api/v1/admin/catalog/collections", {
          ...requestOptions,
          query,
        }),
      get: (collectionId, requestOptions) =>
        request<Collection>(
          `/api/v1/admin/catalog/collections/${id(collectionId)}`,
          requestOptions,
        ),
      create: (body, requestOptions) =>
        request<Collection>("/api/v1/admin/catalog/collections", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      update: (collectionId, body, requestOptions) =>
        request<Collection>(`/api/v1/admin/catalog/collections/${id(collectionId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      delete: (collectionId, requestOptions) =>
        request<void>(`/api/v1/admin/catalog/collections/${id(collectionId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
      publish: (collectionId, requestOptions) =>
        request<Collection>(`/api/v1/admin/catalog/collections/${id(collectionId)}/publish`, {
          ...requestOptions,
          method: "POST",
        }),
      unpublish: (collectionId, requestOptions) =>
        request<Collection>(`/api/v1/admin/catalog/collections/${id(collectionId)}/unpublish`, {
          ...requestOptions,
          method: "POST",
        }),
      addProducts: (collectionId, body, requestOptions) =>
        request<void>(`/api/v1/admin/catalog/collections/${id(collectionId)}/products`, {
          ...requestOptions,
          method: "POST",
          body,
        }),
      removeProduct: (collectionId, productId, requestOptions) =>
        request<void>(
          `/api/v1/admin/catalog/collections/${id(collectionId)}/products/${id(productId)}`,
          {
            ...requestOptions,
            method: "DELETE",
          },
        ),
    },

    tags: {
      list: (query, requestOptions) =>
        request<PageResult<Tag>>("/api/v1/admin/catalog/tags", {
          ...requestOptions,
          query,
        }),
      create: (body, requestOptions) =>
        request<Tag>("/api/v1/admin/catalog/tags", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      attach: (productId, body, requestOptions) =>
        request<Tag>(`/api/v1/admin/catalog/products/${id(productId)}/tags`, {
          ...requestOptions,
          method: "POST",
          body,
        }),
      detach: (productId, tagId, requestOptions) =>
        request<void>(`/api/v1/admin/catalog/products/${id(productId)}/tags/${id(tagId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
    },

    orders: {
      list: (query, requestOptions) =>
        request<PageResult<Order>>("/api/v1/admin/orders", {
          ...requestOptions,
          query,
        }),
      get: (orderId, requestOptions) =>
        request<Order>(`/api/v1/admin/orders/${id(orderId)}`, requestOptions),
      getByNumber: (displayNumber, requestOptions) =>
        request<Order>(`/api/v1/admin/orders/by-number/${displayNumber}`, requestOptions),
      confirm: (orderId, requestOptions) =>
        request<Order>(`/api/v1/admin/orders/${id(orderId)}/confirm`, {
          ...requestOptions,
          method: "POST",
        }),
      process: (orderId, requestOptions) =>
        request<Order>(`/api/v1/admin/orders/${id(orderId)}/process`, {
          ...requestOptions,
          method: "POST",
        }),
      cancel: (orderId, body, requestOptions) =>
        request<Order>(`/api/v1/admin/orders/${id(orderId)}/cancel`, {
          ...requestOptions,
          method: "POST",
          body: body ?? {},
        }),
      fulfill: (orderId, body, requestOptions) =>
        request<Order>(`/api/v1/admin/orders/${id(orderId)}/fulfill`, {
          ...requestOptions,
          method: "POST",
          body,
        }),
      history: (orderId, requestOptions) =>
        request<OrderStatusHistory[]>(
          `/api/v1/admin/orders/${id(orderId)}/history`,
          requestOptions,
        ),
      refunds: {
        list: (orderId, requestOptions) =>
          request<Refund[]>(`/api/v1/admin/orders/${id(orderId)}/refunds`, requestOptions),
        create: (orderId, body, requestOptions) =>
          request<Order>(`/api/v1/admin/orders/${id(orderId)}/refunds`, {
            ...requestOptions,
            method: "POST",
            body,
          }),
      },
    },

    customers: {
      list: (query, requestOptions) =>
        request<PageResult<Customer>>("/api/v1/admin/customers", {
          ...requestOptions,
          query,
        }),
      get: (customerId, requestOptions) =>
        request<Customer>(`/api/v1/admin/customers/${id(customerId)}`, requestOptions),
      update: (customerId, body, requestOptions) =>
        request<Customer>(`/api/v1/admin/customers/${id(customerId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      delete: (customerId, requestOptions) =>
        request<void>(`/api/v1/admin/customers/${id(customerId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
      addresses: {
        list: (customerId, requestOptions) =>
          request<CustomerAddress[]>(
            `/api/v1/admin/customers/${id(customerId)}/addresses`,
            requestOptions,
          ),
        create: (customerId, body, requestOptions) =>
          request<CustomerAddress>(`/api/v1/admin/customers/${id(customerId)}/addresses`, {
            ...requestOptions,
            method: "POST",
            body,
          }),
        update: (customerId, addressId, body, requestOptions) =>
          request<CustomerAddress>(
            `/api/v1/admin/customers/${id(customerId)}/addresses/${id(addressId)}`,
            {
              ...requestOptions,
              method: "PATCH",
              body,
            },
          ),
        remove: (customerId, addressId, requestOptions) =>
          request<void>(`/api/v1/admin/customers/${id(customerId)}/addresses/${id(addressId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
      },
      groups: {
        list: (customerId, requestOptions) =>
          request<unknown[]>(`/api/v1/admin/customers/${id(customerId)}/groups`, requestOptions),
        listAll: (requestOptions) =>
          request<unknown[]>("/api/v1/admin/customer-groups", requestOptions),
        create: (body, requestOptions) =>
          request<unknown>("/api/v1/admin/customer-groups", {
            ...requestOptions,
            method: "POST",
            body,
          }),
      },
    },

    inventory: {
      locations: {
        list: (query, requestOptions) =>
          request<PageResult<InventoryLocation>>("/api/v1/admin/inventory/locations", {
            ...requestOptions,
            query,
          }),
        get: (locationId, requestOptions) =>
          request<InventoryLocation>(
            `/api/v1/admin/inventory/locations/${id(locationId)}`,
            requestOptions,
          ),
        create: (body, requestOptions) =>
          request<InventoryLocation>("/api/v1/admin/inventory/locations", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        update: (locationId, body, requestOptions) =>
          request<InventoryLocation>(`/api/v1/admin/inventory/locations/${id(locationId)}`, {
            ...requestOptions,
            method: "PATCH",
            body,
          }),
      },
      items: {
        list: (query, requestOptions) =>
          request<PageResult<InventoryItem>>("/api/v1/admin/inventory/items", {
            ...requestOptions,
            query,
          }),
        get: (itemId, requestOptions) =>
          request<InventoryItem>(`/api/v1/admin/inventory/items/${id(itemId)}`, requestOptions),
        getByVariant: (variantId, requestOptions) =>
          request<InventoryItem>(
            `/api/v1/admin/inventory/items/by-variant/${id(variantId)}`,
            requestOptions,
          ),
        create: (body, requestOptions) =>
          request<InventoryItem>("/api/v1/admin/inventory/items", {
            ...requestOptions,
            method: "POST",
            body,
          }),
      },
      levels: (query, requestOptions) =>
        request<InventoryLevel[]>("/api/v1/admin/inventory/levels", {
          ...requestOptions,
          query,
        }),
      availability: (variantId, requestOptions) =>
        request<AvailabilityResponse>(
          `/api/v1/admin/inventory/availability/${id(variantId)}`,
          requestOptions,
        ),
      movements: (query, requestOptions) =>
        request<InventoryMovement[]>("/api/v1/admin/inventory/movements", {
          ...requestOptions,
          query,
        }),
      adjust: (body, requestOptions) =>
        request<InventoryLevel>("/api/v1/admin/inventory/adjustments", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      transfer: (body, requestOptions) =>
        request<InventoryTransferResponse>("/api/v1/admin/inventory/transfers", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      reservations: {
        create: (body, requestOptions) =>
          request<InventoryReservation[]>("/api/v1/admin/inventory/reservations", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        release: (body, requestOptions) =>
          request<ReleasedResponse>("/api/v1/admin/inventory/reservations/release", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        commit: (body, requestOptions) =>
          request<CommittedResponse>("/api/v1/admin/inventory/reservations/commit", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        expire: (requestOptions) =>
          request<ReleasedResponse>("/api/v1/admin/inventory/reservations/expire", {
            ...requestOptions,
            method: "POST",
          }),
      },
    },

    promotions: {
      list: (query, requestOptions) =>
        request<PageResult<Promotion>>("/api/v1/admin/promotions", {
          ...requestOptions,
          query,
        }),
      get: (promotionId, requestOptions) =>
        request<Promotion>(`/api/v1/admin/promotions/${id(promotionId)}`, requestOptions),
      create: (body, requestOptions) =>
        request<Promotion>("/api/v1/admin/promotions", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      update: (promotionId, body, requestOptions) =>
        request<Promotion>(`/api/v1/admin/promotions/${id(promotionId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      delete: (promotionId, requestOptions) =>
        request<void>(`/api/v1/admin/promotions/${id(promotionId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
      usage: (promotionId, requestOptions) =>
        request<PromotionUsageResponse>(
          `/api/v1/admin/promotions/${id(promotionId)}/usage`,
          requestOptions,
        ),
    },

    coupons: {
      list: (query, requestOptions) =>
        request<PageResult<Coupon>>("/api/v1/admin/coupons", {
          ...requestOptions,
          query,
        }),
      get: (couponId, requestOptions) =>
        request<Coupon>(`/api/v1/admin/coupons/${id(couponId)}`, requestOptions),
      create: (body, requestOptions) =>
        request<Coupon>("/api/v1/admin/coupons", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      update: (couponId, body, requestOptions) =>
        request<Coupon>(`/api/v1/admin/coupons/${id(couponId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      delete: (couponId, requestOptions) =>
        request<void>(`/api/v1/admin/coupons/${id(couponId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
      validate: (body, requestOptions) =>
        request<CouponValidationResponse>("/api/v1/admin/coupons/validate", {
          ...requestOptions,
          method: "POST",
          body,
        }),
    },

    priceLists: {
      list: (query, requestOptions) =>
        request<PageResult<PriceList>>("/api/v1/admin/price-lists", {
          ...requestOptions,
          query,
        }),
      get: (priceListId, requestOptions) =>
        request<PriceList>(`/api/v1/admin/price-lists/${id(priceListId)}`, requestOptions),
      create: (body, requestOptions) =>
        request<PriceList>("/api/v1/admin/price-lists", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      update: (priceListId, body, requestOptions) =>
        request<PriceList>(`/api/v1/admin/price-lists/${id(priceListId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      delete: (priceListId, requestOptions) =>
        request<void>(`/api/v1/admin/price-lists/${id(priceListId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
      prices: (query, requestOptions) =>
        request<PageResult<Price>>("/api/v1/admin/prices", {
          ...requestOptions,
          query,
        }),
    },

    shipping: {
      profiles: {
        list: (query, requestOptions) =>
          request<PageResult<ShippingProfile>>("/api/v1/admin/shipping/profiles", {
            ...requestOptions,
            query,
          }),
        get: (profileId, requestOptions) =>
          request<ShippingProfile>(
            `/api/v1/admin/shipping/profiles/${id(profileId)}`,
            requestOptions,
          ),
        create: (body, requestOptions) =>
          request<ShippingProfile>("/api/v1/admin/shipping/profiles", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        update: (profileId, body, requestOptions) =>
          request<ShippingProfile>(`/api/v1/admin/shipping/profiles/${id(profileId)}`, {
            ...requestOptions,
            method: "PATCH",
            body,
          }),
        delete: (profileId, requestOptions) =>
          request<void>(`/api/v1/admin/shipping/profiles/${id(profileId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
      },
      options: {
        list: (query, requestOptions) =>
          request<PageResult<ShippingOption>>("/api/v1/admin/shipping/options", {
            ...requestOptions,
            query,
          }),
        get: (optionId, requestOptions) =>
          request<ShippingOption>(`/api/v1/admin/shipping/options/${id(optionId)}`, requestOptions),
        create: (body, requestOptions) =>
          request<ShippingOption>("/api/v1/admin/shipping/options", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        update: (optionId, body, requestOptions) =>
          request<ShippingOption>(`/api/v1/admin/shipping/options/${id(optionId)}`, {
            ...requestOptions,
            method: "PATCH",
            body,
          }),
        delete: (optionId, requestOptions) =>
          request<void>(`/api/v1/admin/shipping/options/${id(optionId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
        forDestination: (query, requestOptions) =>
          request<ResolvedShippingOption[]>("/api/v1/admin/shipping/options-list", {
            ...requestOptions,
            query,
          }),
        selectRate: (optionId, query, requestOptions) =>
          request<SelectedShippingRate | null>(
            `/api/v1/admin/shipping/options/${id(optionId)}/rate`,
            {
              ...requestOptions,
              query,
            },
          ),
        locations: {
          add: (optionId, body, requestOptions) =>
            request<void>(`/api/v1/admin/shipping/options/${id(optionId)}/locations`, {
              ...requestOptions,
              method: "POST",
              body,
            }),
          remove: (optionId, locationId, requestOptions) =>
            request<void>(
              `/api/v1/admin/shipping/options/${id(optionId)}/locations/${id(locationId)}`,
              {
                ...requestOptions,
                method: "DELETE",
              },
            ),
        },
      },
      rates: {
        list: (optionId, requestOptions) =>
          request<ShippingRate[]>(
            `/api/v1/admin/shipping/options/${id(optionId)}/rates`,
            requestOptions,
          ),
        get: (rateId, requestOptions) =>
          request<ShippingRate>(`/api/v1/admin/shipping/rates/${id(rateId)}`, requestOptions),
        create: (optionId, body, requestOptions) =>
          request<ShippingRate>(`/api/v1/admin/shipping/options/${id(optionId)}/rates`, {
            ...requestOptions,
            method: "POST",
            body,
          }),
        update: (rateId, body, requestOptions) =>
          request<ShippingRate>(`/api/v1/admin/shipping/rates/${id(rateId)}`, {
            ...requestOptions,
            method: "PATCH",
            body,
          }),
        delete: (rateId, requestOptions) =>
          request<void>(`/api/v1/admin/shipping/rates/${id(rateId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
      },
    },

    tax: {
      categories: {
        list: (query, requestOptions) =>
          request<PageResult<TaxCategory>>("/api/v1/admin/tax/categories", {
            ...requestOptions,
            query,
          }),
        get: (categoryId, requestOptions) =>
          request<TaxCategory>(`/api/v1/admin/tax/categories/${id(categoryId)}`, requestOptions),
        create: (body, requestOptions) =>
          request<TaxCategory>("/api/v1/admin/tax/categories", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        update: (categoryId, body, requestOptions) =>
          request<TaxCategory>(`/api/v1/admin/tax/categories/${id(categoryId)}`, {
            ...requestOptions,
            method: "PATCH",
            body,
          }),
        delete: (categoryId, requestOptions) =>
          request<void>(`/api/v1/admin/tax/categories/${id(categoryId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
      },
      regions: {
        list: (query, requestOptions) =>
          request<PageResult<TaxRegion>>("/api/v1/admin/tax/regions", {
            ...requestOptions,
            query,
          }),
        get: (regionId, requestOptions) =>
          request<TaxRegion>(`/api/v1/admin/tax/regions/${id(regionId)}`, requestOptions),
        create: (body, requestOptions) =>
          request<TaxRegion>("/api/v1/admin/tax/regions", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        update: (regionId, body, requestOptions) =>
          request<TaxRegion>(`/api/v1/admin/tax/regions/${id(regionId)}`, {
            ...requestOptions,
            method: "PATCH",
            body,
          }),
        delete: (regionId, requestOptions) =>
          request<void>(`/api/v1/admin/tax/regions/${id(regionId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
      },
      rates: {
        list: (query, requestOptions) =>
          request<TaxRate[]>("/api/v1/admin/tax/rates", {
            ...requestOptions,
            query,
          }),
        get: (rateId, requestOptions) =>
          request<TaxRate>(`/api/v1/admin/tax/rates/${id(rateId)}`, requestOptions),
        create: (body, requestOptions) =>
          request<TaxRate>("/api/v1/admin/tax/rates", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        update: (rateId, body, requestOptions) =>
          request<TaxRate>(`/api/v1/admin/tax/rates/${id(rateId)}`, {
            ...requestOptions,
            method: "PATCH",
            body,
          }),
        delete: (rateId, requestOptions) =>
          request<void>(`/api/v1/admin/tax/rates/${id(rateId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
      },
      calculate: (body, requestOptions) =>
        request<TaxCalculationResponse>("/api/v1/admin/tax/calculate", {
          ...requestOptions,
          method: "POST",
          body,
        }),
    },

    payments: {
      list: (query, requestOptions) =>
        request<PageResult<Payment>>("/api/v1/admin/payments", {
          ...requestOptions,
          query,
        }),
      get: (paymentId, requestOptions) =>
        request<Payment>(`/api/v1/admin/payments/${id(paymentId)}`, requestOptions),
      refunds: {
        list: (paymentId, requestOptions) =>
          request<PaymentRefund[]>(
            `/api/v1/admin/payments/${id(paymentId)}/refunds`,
            requestOptions,
          ),
        create: (paymentId, body, requestOptions) =>
          request<PaymentRefund>(`/api/v1/admin/payments/${id(paymentId)}/refunds`, {
            ...requestOptions,
            method: "POST",
            body,
          }),
      },
      sessions: {
        get: (sessionId, requestOptions) =>
          request<PaymentSession>(
            `/api/v1/admin/payment-sessions/${id(sessionId)}`,
            requestOptions,
          ),
        capture: (sessionId, requestOptions) =>
          request<Payment>(`/api/v1/admin/payment-sessions/${id(sessionId)}/capture`, {
            ...requestOptions,
            method: "POST",
          }),
        cancel: (sessionId, requestOptions) =>
          request<PaymentSession>(`/api/v1/admin/payment-sessions/${id(sessionId)}/cancel`, {
            ...requestOptions,
            method: "POST",
          }),
        fail: (sessionId, body, requestOptions) =>
          request<PaymentSession>(`/api/v1/admin/payment-sessions/${id(sessionId)}/fail`, {
            ...requestOptions,
            method: "POST",
            body: body ?? {},
          }),
        attempts: (sessionId, requestOptions) =>
          request<PaymentAttempt[]>(
            `/api/v1/admin/payment-sessions/${id(sessionId)}/attempts`,
            requestOptions,
          ),
      },
      webhooks: {
        list: (query, requestOptions) =>
          request<PageResult<PaymentWebhookRecord>>("/api/v1/admin/payment-webhooks", {
            ...requestOptions,
            query,
          }),
        get: (webhookId, requestOptions) =>
          request<PaymentWebhookRecord>(
            `/api/v1/admin/payment-webhooks/${id(webhookId)}`,
            requestOptions,
          ),
      },
    },

    webhooks: {
      list: (requestOptions) =>
        request<WebhookEndpoint[]>("/api/v1/admin/webhooks", requestOptions),
      get: (webhookId, requestOptions) =>
        request<WebhookEndpoint>(`/api/v1/admin/webhooks/${id(webhookId)}`, requestOptions),
      create: (body, requestOptions) =>
        request<CreatedWebhookEndpoint>("/api/v1/admin/webhooks", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      update: (webhookId, body, requestOptions) =>
        request<WebhookEndpoint>(`/api/v1/admin/webhooks/${id(webhookId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      delete: (webhookId, requestOptions) =>
        request<void>(`/api/v1/admin/webhooks/${id(webhookId)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
      rotateSecret: (webhookId, requestOptions) =>
        request<{ secret: string }>(`/api/v1/admin/webhooks/${id(webhookId)}/rotate-secret`, {
          ...requestOptions,
          method: "POST",
        }),
      deliveries: (webhookId, query, requestOptions) =>
        request<WebhookDelivery[]>(`/api/v1/admin/webhooks/${id(webhookId)}/deliveries`, {
          ...requestOptions,
          query,
        }),
      replay: (deliveryId, requestOptions) =>
        request<WebhookDelivery>(`/api/v1/admin/webhook-deliveries/${id(deliveryId)}/replay`, {
          ...requestOptions,
          method: "POST",
        }),
    },

    salesChannels: {
      list: (query, requestOptions) =>
        request<PageResult<SalesChannel>>("/api/v1/admin/sales-channels", {
          ...requestOptions,
          query,
        }),
      get: (channelId, requestOptions) =>
        request<SalesChannel>(`/api/v1/admin/sales-channels/${id(channelId)}`, requestOptions),
      create: (body, requestOptions) =>
        request<SalesChannel>("/api/v1/admin/sales-channels", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      update: (channelId, body, requestOptions) =>
        request<SalesChannel>(`/api/v1/admin/sales-channels/${id(channelId)}`, {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      disable: (channelId, requestOptions) =>
        request<SalesChannel>(`/api/v1/admin/sales-channels/${id(channelId)}/disable`, {
          ...requestOptions,
          method: "POST",
        }),
      enable: (channelId, requestOptions) =>
        request<SalesChannel>(`/api/v1/admin/sales-channels/${id(channelId)}/enable`, {
          ...requestOptions,
          method: "POST",
        }),
    },

    settings: {
      get: (requestOptions) => request<StoreSettings>("/api/v1/admin/settings", requestOptions),
      update: (body, requestOptions) =>
        request<StoreSettings>("/api/v1/admin/settings", {
          ...requestOptions,
          method: "PUT",
          body,
        }),
      getOne: (key, requestOptions) =>
        request<{ key: string; value: string }>(
          `/api/v1/admin/settings/${id(key)}`,
          requestOptions,
        ),
      set: (key, value, requestOptions) =>
        request<{ key: string; value: string }>(`/api/v1/admin/settings/${id(key)}`, {
          ...requestOptions,
          method: "PUT",
          body: { value },
        }),
      delete: (key, requestOptions) =>
        request<void>(`/api/v1/admin/settings/${id(key)}`, {
          ...requestOptions,
          method: "DELETE",
        }),
    },
  };
}
