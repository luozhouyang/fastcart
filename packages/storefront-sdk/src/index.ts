import {
  createApiClient,
  type ApiClientOptions,
  type CallOptions,
  type RequestOptions,
} from "@fastcart/api-client";
import type {
  AddItemRequest,
  Cart,
  CartItem,
  CartResponse,
  CategoryListResponse,
  ChangePasswordRequest,
  CheckoutCompleteResponse,
  CheckoutHistoryResponse,
  CheckoutResponse,
  Collection,
  CollectionListResponse,
  CompleteCheckoutRequest,
  CreateCartRequest,
  CreateCheckoutRequest,
  CreateInquiryRequest,
  CreateInquiryResponse,
  CustomerAddressInput,
  CustomerAddressListResponse,
  CustomerAddressResponse,
  CustomerGroupsResponse,
  CustomerLoginRequest,
  CustomerResponse,
  CustomerSessionResponse,
  DestinationQuery,
  DiscountRequest,
  Money,
  NavigationResponse,
  OrderListResponse,
  OrderResponse,
  PaginationQuery,
  Product,
  ProductListResponse,
  ProductResponse,
  ProductSearchResponse,
  RegisterCustomerRequest,
  SetAddressRequest,
  SetEmailRequest,
  SetShippingMethodRequest,
  ShippingOptionsQuery,
  ShippingOptionsResponse,
  StoreInquiryFormResponse,
  UploadInquiryFileResponse,
  StorePageResponse,
  StoreProductListQuery,
  StoreSearchQuery,
  StoreThemeResponse,
  UpdateItemRequest,
  UpdateProfileRequest,
  VariantAvailabilityResponse,
} from "@fastcart/shared-types";

/** Options accepted by {@link createStorefrontClient}. */
export interface StorefrontClientOptions extends ApiClientOptions {
  /** Customer CSRF token, or a provider called for every unsafe request. */
  csrfToken?: string | (() => string | null);
}

/** Per-call configuration shared by every storefront method. */
export interface StorefrontRequestOptions extends CallOptions {
  /** Overrides the client-level CSRF token for this request. */
  csrfToken?: string;
}

/** A storefront product, including an optional resolved display price. */
export interface StoreProduct extends Product {
  price?: Money | null;
}

/** Storefront collection shape. */
export type StoreCollection = Collection;

/** Backwards-compatible cart line shape. */
export type CartLine = Pick<CartItem, "variant_id" | "quantity">;

/** Storefront cart aggregate. */
export type StoreCart = Cart;

/** Typed storefront SDK for the public `/api/v1/store` namespace. */
export interface StorefrontClient {
  /** Updates the CSRF token captured from register or login. */
  setCsrfToken(token: string | null): void;

  products: {
    list(
      query?: StoreProductListQuery,
      options?: StorefrontRequestOptions,
    ): Promise<ProductListResponse<StoreProduct>>;
    get(handle: string, options?: StorefrontRequestOptions): Promise<ProductResponse<StoreProduct>>;
    getByHandle(
      handle: string,
      options?: StorefrontRequestOptions,
    ): Promise<ProductResponse<StoreProduct>>;
    search(
      query: StoreSearchQuery,
      options?: StorefrontRequestOptions,
    ): Promise<ProductSearchResponse<StoreProduct>>;
    availability(
      handle: string,
      options?: StorefrontRequestOptions,
    ): Promise<VariantAvailabilityResponse>;
  };

  categories: {
    list(
      query?: PaginationQuery,
      options?: StorefrontRequestOptions,
    ): Promise<CategoryListResponse>;
  };

  collections: {
    list(
      query?: PaginationQuery,
      options?: StorefrontRequestOptions,
    ): Promise<CollectionListResponse>;
  };

  cart: {
    create(body: CreateCartRequest, options?: StorefrontRequestOptions): Promise<CartResponse>;
    get(cartId: string, options?: StorefrontRequestOptions): Promise<CartResponse>;
    addItem(
      cartId: string,
      body: AddItemRequest,
      options?: StorefrontRequestOptions,
    ): Promise<CartResponse>;
    updateItem(
      cartId: string,
      itemId: string,
      body: UpdateItemRequest,
      query?: DestinationQuery,
      options?: StorefrontRequestOptions,
    ): Promise<CartResponse>;
    removeItem(
      cartId: string,
      itemId: string,
      query?: DestinationQuery,
      options?: StorefrontRequestOptions,
    ): Promise<CartResponse>;
    clearItems(
      cartId: string,
      query?: DestinationQuery,
      options?: StorefrontRequestOptions,
    ): Promise<CartResponse>;
    setEmail(
      cartId: string,
      body: SetEmailRequest | string,
      options?: StorefrontRequestOptions,
    ): Promise<CartResponse>;
    attachCustomer(cartId: string, options?: StorefrontRequestOptions): Promise<CartResponse>;
    setAddress(
      cartId: string,
      body: SetAddressRequest,
      options?: StorefrontRequestOptions,
    ): Promise<CartResponse>;
    setShippingMethod(
      cartId: string,
      body: SetShippingMethodRequest,
      options?: StorefrontRequestOptions,
    ): Promise<CartResponse>;
    applyDiscount(
      cartId: string,
      body: DiscountRequest | string,
      query?: DestinationQuery,
      options?: StorefrontRequestOptions,
    ): Promise<CartResponse>;
    removeDiscount(
      cartId: string,
      code: string,
      query?: DestinationQuery,
      options?: StorefrontRequestOptions,
    ): Promise<CartResponse>;
  };

  customer: {
    register(
      body: RegisterCustomerRequest,
      options?: StorefrontRequestOptions,
    ): Promise<CustomerSessionResponse>;
    login(
      body: CustomerLoginRequest,
      options?: StorefrontRequestOptions,
    ): Promise<CustomerSessionResponse>;
    logout(options?: StorefrontRequestOptions): Promise<void>;
    me(options?: StorefrontRequestOptions): Promise<CustomerResponse>;
    updateMe(
      body: UpdateProfileRequest,
      options?: StorefrontRequestOptions,
    ): Promise<CustomerResponse>;
    changePassword(body: ChangePasswordRequest, options?: StorefrontRequestOptions): Promise<void>;
    addresses: {
      list(options?: StorefrontRequestOptions): Promise<CustomerAddressListResponse>;
      create(
        body: CustomerAddressInput,
        options?: StorefrontRequestOptions,
      ): Promise<CustomerAddressResponse>;
      update(
        addressId: string,
        body: CustomerAddressInput,
        options?: StorefrontRequestOptions,
      ): Promise<CustomerAddressResponse>;
      remove(addressId: string, options?: StorefrontRequestOptions): Promise<void>;
    };
    orders: {
      list(query?: PaginationQuery, options?: StorefrontRequestOptions): Promise<OrderListResponse>;
      get(orderId: string, options?: StorefrontRequestOptions): Promise<OrderResponse>;
    };
    groups(options?: StorefrontRequestOptions): Promise<CustomerGroupsResponse>;
  };

  content: {
    page(handle: string, options?: StorefrontRequestOptions): Promise<StorePageResponse>;
    navigation(handle: string, options?: StorefrontRequestOptions): Promise<NavigationResponse>;
  };

  shipping: {
    options(
      query: ShippingOptionsQuery,
      options?: StorefrontRequestOptions,
    ): Promise<ShippingOptionsResponse>;
  };

  theme: {
    active(options?: StorefrontRequestOptions): Promise<StoreThemeResponse>;
  };

  checkout: {
    create(
      body: CreateCheckoutRequest,
      options?: StorefrontRequestOptions,
    ): Promise<CheckoutResponse>;
    get(checkoutId: string, options?: StorefrontRequestOptions): Promise<CheckoutResponse>;
    complete(
      checkoutId: string,
      body?: CompleteCheckoutRequest,
      options?: StorefrontRequestOptions,
    ): Promise<CheckoutCompleteResponse>;
    cancel(checkoutId: string, options?: StorefrontRequestOptions): Promise<CheckoutResponse>;
    history(
      checkoutId: string,
      options?: StorefrontRequestOptions,
    ): Promise<CheckoutHistoryResponse>;
  };

  inquiryForms: {
    get(handle: string, options?: StorefrontRequestOptions): Promise<StoreInquiryFormResponse>;
  };

  inquiries: {
    create(
      body: CreateInquiryRequest,
      options?: StorefrontRequestOptions,
    ): Promise<CreateInquiryResponse>;
  };

  inquiryFiles: {
    upload(
      file: File,
      formHandle: string,
      fieldKey: string,
      options?: StorefrontRequestOptions,
    ): Promise<UploadInquiryFileResponse>;
  };

  settings: {
    public(options?: StorefrontRequestOptions): Promise<Record<string, string>>;
  };
}

type StorefrontInit = CallOptions & {
  method?: string;
  query?: unknown;
  body?: unknown;
  csrfToken?: string;
};

const UNSAFE_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"]);

/**
 * Creates a typed storefront client.
 *
 * Cookies are sent by default (`credentials: "include"`) because customers are
 * authenticated with store-scoped session cookies. The client captures the CSRF
 * token returned by register/login and attaches it to unsafe requests. Pass an
 * `AbortSignal` through any method's final options argument to cancel.
 */
export function createStorefrontClient(options: StorefrontClientOptions = {}): StorefrontClient {
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

  const client = createApiClient({
    ...options,
    credentials: options.credentials ?? "include",
  });

  function request<T>(path: string, init: StorefrontInit = {}): Promise<T> {
    const method = (init.method ?? "GET").toUpperCase();
    const headers = new Headers(init.headers);
    if (UNSAFE_METHODS.has(method)) {
      const csrf = csrfTokenForRequest(init.csrfToken);
      if (csrf !== null && csrf.length > 0 && !headers.has("x-csrf-token")) {
        headers.set("x-csrf-token", csrf);
      }
    }
    return client.request<T>(path, { ...init, headers } as RequestOptions);
  }

  const id = (value: string): string => encodeURIComponent(value);

  const captureSession = (response: CustomerSessionResponse): CustomerSessionResponse => {
    sessionCsrf = response.csrf_token;
    return response;
  };

  return {
    setCsrfToken: (token) => {
      sessionCsrf = token;
    },

    products: {
      list: (query, requestOptions) =>
        request<ProductListResponse<StoreProduct>>("/api/v1/store/products", {
          ...requestOptions,
          query,
        }),
      get: (handle, requestOptions) =>
        request<ProductResponse<StoreProduct>>(
          `/api/v1/store/products/${id(handle)}`,
          requestOptions,
        ),
      getByHandle: (handle, requestOptions) =>
        request<ProductResponse<StoreProduct>>(
          `/api/v1/store/products/${id(handle)}`,
          requestOptions,
        ),
      search: (query, requestOptions) =>
        request<ProductSearchResponse<StoreProduct>>("/api/v1/store/search", {
          ...requestOptions,
          query,
        }),
      availability: (handle, requestOptions) =>
        request<VariantAvailabilityResponse>(
          `/api/v1/store/products/${id(handle)}/availability`,
          requestOptions,
        ),
    },

    categories: {
      list: (query, requestOptions) =>
        request<CategoryListResponse>("/api/v1/store/categories", {
          ...requestOptions,
          query,
        }),
    },

    collections: {
      list: (query, requestOptions) =>
        request<CollectionListResponse>("/api/v1/store/collections", {
          ...requestOptions,
          query,
        }),
    },

    cart: {
      create: (body, requestOptions) =>
        request<CartResponse>("/api/v1/store/carts", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      get: (cartId, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}`, requestOptions),
      addItem: (cartId, body, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/items`, {
          ...requestOptions,
          method: "POST",
          body,
        }),
      updateItem: (cartId, itemId, body, query, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/items/${id(itemId)}`, {
          ...requestOptions,
          method: "PATCH",
          query,
          body,
        }),
      removeItem: (cartId, itemId, query, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/items/${id(itemId)}`, {
          ...requestOptions,
          method: "DELETE",
          query,
        }),
      clearItems: (cartId, query, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/items`, {
          ...requestOptions,
          method: "DELETE",
          query,
        }),
      setEmail: (cartId, body, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/email`, {
          ...requestOptions,
          method: "POST",
          body: typeof body === "string" ? { email: body } : body,
        }),
      attachCustomer: (cartId, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/customer`, {
          ...requestOptions,
          method: "POST",
        }),
      setAddress: (cartId, body, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/addresses`, {
          ...requestOptions,
          method: "POST",
          body,
        }),
      setShippingMethod: (cartId, body, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/shipping-method`, {
          ...requestOptions,
          method: "POST",
          body,
        }),
      applyDiscount: (cartId, body, query, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/discounts`, {
          ...requestOptions,
          method: "POST",
          query,
          body: typeof body === "string" ? { code: body } : body,
        }),
      removeDiscount: (cartId, code, query, requestOptions) =>
        request<CartResponse>(`/api/v1/store/carts/${id(cartId)}/discounts/${id(code)}`, {
          ...requestOptions,
          method: "DELETE",
          query,
        }),
    },

    customer: {
      register: async (body, requestOptions) =>
        captureSession(
          await request<CustomerSessionResponse>("/api/v1/store/customers", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        ),
      login: async (body, requestOptions) =>
        captureSession(
          await request<CustomerSessionResponse>("/api/v1/store/customers/login", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        ),
      logout: (requestOptions) =>
        request<void>("/api/v1/store/customers/logout", {
          ...requestOptions,
          method: "POST",
        }),
      me: (requestOptions) =>
        request<CustomerResponse>("/api/v1/store/customers/me", requestOptions),
      updateMe: (body, requestOptions) =>
        request<CustomerResponse>("/api/v1/store/customers/me", {
          ...requestOptions,
          method: "PATCH",
          body,
        }),
      changePassword: (body, requestOptions) =>
        request<void>("/api/v1/store/customers/me/password", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      addresses: {
        list: (requestOptions) =>
          request<CustomerAddressListResponse>(
            "/api/v1/store/customers/me/addresses",
            requestOptions,
          ),
        create: (body, requestOptions) =>
          request<CustomerAddressResponse>("/api/v1/store/customers/me/addresses", {
            ...requestOptions,
            method: "POST",
            body,
          }),
        update: (addressId, body, requestOptions) =>
          request<CustomerAddressResponse>(
            `/api/v1/store/customers/me/addresses/${id(addressId)}`,
            {
              ...requestOptions,
              method: "PATCH",
              body,
            },
          ),
        remove: (addressId, requestOptions) =>
          request<void>(`/api/v1/store/customers/me/addresses/${id(addressId)}`, {
            ...requestOptions,
            method: "DELETE",
          }),
      },
      orders: {
        list: (query, requestOptions) =>
          request<OrderListResponse>("/api/v1/store/customers/me/orders", {
            ...requestOptions,
            query,
          }),
        get: (orderId, requestOptions) =>
          request<OrderResponse>(
            `/api/v1/store/customers/me/orders/${id(orderId)}`,
            requestOptions,
          ),
      },
      groups: (requestOptions) =>
        request<CustomerGroupsResponse>("/api/v1/store/customers/me/groups", requestOptions),
    },

    content: {
      page: (handle, requestOptions) =>
        request<StorePageResponse>(`/api/v1/store/pages/${id(handle)}`, requestOptions),
      navigation: (handle, requestOptions) =>
        request<NavigationResponse>(`/api/v1/store/navigation/${id(handle)}`, requestOptions),
    },

    shipping: {
      options: (query, requestOptions) =>
        request<ShippingOptionsResponse>("/api/v1/store/shipping/options", {
          ...requestOptions,
          query,
        }),
    },

    theme: {
      active: (requestOptions) =>
        request<StoreThemeResponse>("/api/v1/store/theme", requestOptions),
    },

    checkout: {
      create: (body, requestOptions) =>
        request<CheckoutResponse>("/api/v1/store/checkouts", {
          ...requestOptions,
          method: "POST",
          body,
        }),
      get: (checkoutId, requestOptions) =>
        request<CheckoutResponse>(`/api/v1/store/checkouts/${id(checkoutId)}`, requestOptions),
      complete: (checkoutId, body, requestOptions) =>
        request<CheckoutCompleteResponse>(`/api/v1/store/checkouts/${id(checkoutId)}/complete`, {
          ...requestOptions,
          method: "POST",
          body: body ?? {},
        }),
      cancel: (checkoutId, requestOptions) =>
        request<CheckoutResponse>(`/api/v1/store/checkouts/${id(checkoutId)}/cancel`, {
          ...requestOptions,
          method: "POST",
        }),
      history: (checkoutId, requestOptions) =>
        request<CheckoutHistoryResponse>(
          `/api/v1/store/checkouts/${id(checkoutId)}/history`,
          requestOptions,
        ),
    },

    inquiryForms: {
      get: (handle, requestOptions) =>
        request<StoreInquiryFormResponse>(
          `/api/v1/store/inquiry-forms/${id(handle)}`,
          requestOptions,
        ),
    },

    inquiries: {
      create: (body, requestOptions) =>
        request<CreateInquiryResponse>("/api/v1/store/inquiries", {
          ...requestOptions,
          method: "POST",
          body,
        }),
    },

    inquiryFiles: {
      upload: (file, formHandle, fieldKey, requestOptions) => {
        const body = new FormData();
        body.append("file", file);
        return request<UploadInquiryFileResponse>("/api/v1/store/inquiry-files", {
          ...requestOptions,
          method: "POST",
          query: { form_handle: formHandle, field_key: fieldKey },
          body,
        });
      },
    },

    settings: {
      public: (requestOptions) =>
        request<Record<string, string>>("/api/v1/store/settings/public", requestOptions),
    },
  };
}
