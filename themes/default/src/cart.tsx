import type { Cart, Order, ResolvedShippingOption } from "@fastcart/shared-types";
import { createStorefrontClient } from "@fastcart/storefront-sdk";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { useThemeRuntime } from "./context";
import { DEFAULT_COUNTRY_CODE, normalizeCountryCode } from "./countries";
import { apiBaseUrlFromSettings } from "./runtime";

/** Steps rendered inside the cart drawer. */
export type CartStep = "cart" | "details" | "confirmation";

/** Customer-entered checkout fields for the shipping address. */
export interface CheckoutDetails {
  email: string;
  firstName: string;
  lastName: string;
  address1: string;
  city: string;
  postalCode: string;
  countryCode: string;
}

/** Maximum quantity selectable on a product page. */
export const MAX_ADD_QUANTITY = 10;

/** Maximum quantity for an existing cart line. */
export const MAX_LINE_QUANTITY = 99;

/** State and actions shared by add-to-cart controls and the cart drawer. */
export interface CartContextValue {
  cart: Cart | null;
  itemCount: number;
  currencyCode: string | null;
  isOpen: boolean;
  step: CartStep;
  order: Order | null;
  checkoutDetails: CheckoutDetails | null;
  shippingOptions: ResolvedShippingOption[];
  selectedShippingOptionId: string | null;
  detailsReady: boolean;
  loading: boolean;
  updating: boolean;
  savingDetails: boolean;
  placingOrder: boolean;
  busy: boolean;
  error: string | null;
  addItem(variantId: string, quantity?: number): Promise<void>;
  setQuantity(itemId: string, quantity: number): Promise<void>;
  removeItem(itemId: string): Promise<void>;
  setEmail(email: string): Promise<void>;
  updateCheckoutDetails(details: CheckoutDetails): Promise<void>;
  selectShippingOption(optionId: string): Promise<void>;
  setStep(step: CartStep): void;
  editDetails(): void;
  checkout(): Promise<void>;
  open(): void;
  close(): void;
  continueShopping(): void;
}

/** Props accepted by the cart provider. */
export interface CartProviderProps {
  apiBaseUrl?: string | null | undefined;
  children?: ReactNode;
}

const CART_STORAGE_KEY = "fastcart:cart-id";

const CartContext = createContext<CartContextValue | null>(null);

/**
 * Owns the storefront cart and the checkout funnel.
 *
 * The provider is SSR-safe: every browser API is touched from an effect or an
 * event handler, and the initial state is an empty cart so the server output
 * and the first hydrated render agree. The persisted cart is loaded after
 * hydration and never rendered as part of the initial HTML.
 */
export function CartProvider({ apiBaseUrl, children }: CartProviderProps) {
  const runtime = useThemeRuntime();
  const resolvedApiBaseUrl =
    apiBaseUrl ?? runtime.apiBaseUrl ?? apiBaseUrlFromSettings(runtime.settings);
  const client = useMemo(
    () => createStorefrontClient(resolvedApiBaseUrl ? { baseUrl: resolvedApiBaseUrl } : {}),
    [resolvedApiBaseUrl],
  );

  const [cart, setCart] = useState<Cart | null>(null);
  const [currencyCode, setCurrencyCode] = useState<string | null>(null);
  const [isOpen, setIsOpen] = useState(false);
  const [step, setStepState] = useState<CartStep>("cart");
  const [order, setOrder] = useState<Order | null>(null);
  const [checkoutDetails, setCheckoutDetails] = useState<CheckoutDetails | null>(null);
  const [shippingOptions, setShippingOptions] = useState<ResolvedShippingOption[]>([]);
  const [selectedShippingOptionId, setSelectedShippingOptionId] = useState<string | null>(null);
  const [detailsReady, setDetailsReady] = useState(false);
  const [loading, setLoading] = useState(true);
  const [updating, setUpdating] = useState(false);
  const [savingDetails, setSavingDetails] = useState(false);
  const [placingOrder, setPlacingOrder] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const cartIdRef = useRef<string | null>(null);
  const currencyRef = useRef<string | null>(null);
  const countryRef = useRef<string>(DEFAULT_COUNTRY_CODE);
  const contextRequestRef = useRef<Promise<string> | null>(null);
  const bootstrapRef = useRef<Promise<void> | null>(null);
  const updateLockRef = useRef(false);
  const detailsLockRef = useRef(false);
  const checkoutLockRef = useRef(false);
  const checkoutIdRef = useRef<string | null>(null);

  const applyCart = useCallback((next: Cart) => {
    setCart(next);
    cartIdRef.current = next.id;
    persistCartId(next.id);
    if (next.currency_code) {
      currencyRef.current = next.currency_code;
      setCurrencyCode(next.currency_code);
    }
    const destination = next.shipping_address?.country_code;
    if (destination) {
      countryRef.current = normalizeCountryCode(destination);
    }
  }, []);

  const ensureCurrency = useCallback(async (): Promise<string> => {
    const current = currencyRef.current;
    if (current) {
      return current;
    }
    const pending = contextRequestRef.current;
    if (pending) {
      return pending;
    }
    const request = client
      .context()
      .then((context) => {
        const code = context.currency_code;
        if (typeof code !== "string" || code.trim().length === 0) {
          throw new Error("The store currency is unavailable.");
        }
        const normalized = code.trim().toUpperCase();
        currencyRef.current = normalized;
        setCurrencyCode(normalized);
        return normalized;
      })
      .catch((cause: unknown) => {
        contextRequestRef.current = null;
        throw cause;
      });
    contextRequestRef.current = request;
    return request;
  }, [client]);

  const bootstrap = useCallback(async () => {
    const storedCartId = readStoredCartId();
    if (storedCartId) {
      cartIdRef.current = storedCartId;
    }
    try {
      await ensureCurrency();
      if (!storedCartId) {
        return;
      }
      try {
        const response = await client.cart.get(storedCartId);
        if (response.cart.status !== "open") {
          if (cartIdRef.current === storedCartId) {
            cartIdRef.current = null;
          }
          persistCartId(null);
          return;
        }
        applyCart(response.cart);
        const method = response.cart.shipping_methods[0];
        if (method) {
          setSelectedShippingOptionId(method.shipping_option_id);
        }
      } catch (cause) {
        if (isNotFound(cause)) {
          if (cartIdRef.current === storedCartId) {
            cartIdRef.current = null;
          }
          persistCartId(null);
        }
      }
    } catch {
      // The next user action retries the context request.
    } finally {
      setLoading(false);
    }
  }, [applyCart, client, ensureCurrency]);

  const ensureBootstrap = useCallback((): Promise<void> => {
    const pending = bootstrapRef.current;
    if (pending) {
      return pending;
    }
    const request = bootstrap();
    bootstrapRef.current = request;
    return request;
  }, [bootstrap]);

  useEffect(() => {
    void ensureBootstrap();
  }, [ensureBootstrap]);

  function resetCheckoutState() {
    setDetailsReady(false);
    setCheckoutDetails(null);
    setShippingOptions([]);
    setSelectedShippingOptionId(null);
    checkoutIdRef.current = null;
  }

  function handleStaleCart(message = "Your cart expired. Please add your items again.") {
    cartIdRef.current = null;
    persistCartId(null);
    setCart(null);
    resetCheckoutState();
    setStepState("cart");
    setError(message);
  }

  async function createCart(): Promise<Cart> {
    const currency = currencyRef.current ?? (await ensureCurrency());
    const response = await client.cart.create({ currency_code: currency });
    applyCart(response.cart);
    return response.cart;
  }

  function destinationCountry(): string {
    return normalizeCountryCode(checkoutDetails?.countryCode ?? countryRef.current);
  }

  async function addItem(variantId: string, quantity = 1): Promise<void> {
    const normalizedVariantId = typeof variantId === "string" ? variantId.trim() : "";
    if (normalizedVariantId.length === 0) {
      const message = "Choose a product variant first.";
      setError(message);
      throw new Error(message);
    }
    if (updateLockRef.current) {
      return;
    }
    updateLockRef.current = true;
    setUpdating(true);
    setError(null);
    const normalizedQuantity = clampQuantity(quantity, MAX_ADD_QUANTITY);
    try {
      await ensureBootstrap();
      let cartId = cartIdRef.current;
      if (!cartId) {
        const created = await createCart();
        cartId = created.id;
      }

      let response;
      try {
        response = await client.cart.addItem(cartId, {
          variant_id: normalizedVariantId,
          quantity: normalizedQuantity,
          country_code: destinationCountry(),
        });
      } catch (cause) {
        if (!isNotFound(cause)) {
          throw cause;
        }
        handleStaleCart();
        const created = await createCart();
        if (!created.id) {
          throw new Error("Your cart could not be created.", { cause });
        }
        response = await client.cart.addItem(created.id, {
          variant_id: normalizedVariantId,
          quantity: normalizedQuantity,
          country_code: destinationCountry(),
        });
      }

      applyCart(response.cart);
      resetCheckoutState();
      setOrder(null);
      setStepState("cart");
      setError(null);
      setIsOpen(true);
    } catch (cause) {
      const message = errorMessage(cause, "We couldn't add this item to your cart.");
      setError(message);
      throw new Error(message, { cause });
    } finally {
      updateLockRef.current = false;
      setUpdating(false);
    }
  }

  async function setQuantity(itemId: string, quantity: number): Promise<void> {
    if (updateLockRef.current) {
      return;
    }
    const cartId = cartIdRef.current;
    if (!cartId) {
      const message = "Your cart is empty.";
      setError(message);
      throw new Error(message);
    }
    // A quantity of zero or less removes the line; positive values are clamped.
    const normalizedQuantity = quantity <= 0 ? 0 : clampQuantity(quantity, MAX_LINE_QUANTITY);
    updateLockRef.current = true;
    setUpdating(true);
    setError(null);
    try {
      const destination = { country_code: destinationCountry() };
      const response =
        normalizedQuantity <= 0
          ? await client.cart.removeItem(cartId, itemId, destination)
          : await client.cart.updateItem(
              cartId,
              itemId,
              { quantity: normalizedQuantity },
              destination,
            );
      applyCart(response.cart);
      resetCheckoutState();
      if (response.cart.items.length === 0) {
        setStepState("cart");
      }
    } catch (cause) {
      if (isNotFound(cause)) {
        handleStaleCart();
        throw new Error("Your cart expired. Please add your items again.", { cause });
      }
      const message = errorMessage(cause, "We couldn't update your cart.");
      setError(message);
      throw new Error(message, { cause });
    } finally {
      updateLockRef.current = false;
      setUpdating(false);
    }
  }

  async function removeItem(itemId: string): Promise<void> {
    await setQuantity(itemId, 0);
  }

  async function setEmail(email: string): Promise<void> {
    const normalizedEmail = email.trim();
    const cartId = cartIdRef.current;
    if (!cartId) {
      const message = "Your cart is empty.";
      setError(message);
      throw new Error(message);
    }
    if (normalizedEmail.length === 0) {
      const message = "Enter an email address.";
      setError(message);
      throw new Error(message);
    }
    setError(null);
    try {
      const response = await client.cart.setEmail(cartId, { email: normalizedEmail });
      applyCart(response.cart);
    } catch (cause) {
      if (isNotFound(cause)) {
        handleStaleCart();
        throw new Error("Your cart expired. Please add your items again.", { cause });
      }
      const message = errorMessage(cause, "We couldn't save your email address.");
      setError(message);
      throw new Error(message, { cause });
    }
  }

  async function updateCheckoutDetails(details: CheckoutDetails): Promise<void> {
    if (detailsLockRef.current) {
      return;
    }
    const cartId = cartIdRef.current;
    if (!cartId) {
      const message = "Your cart is empty.";
      setError(message);
      throw new Error(message);
    }
    const normalized = normalizeCheckoutDetails(details);
    if (normalized.email.length === 0) {
      const message = "Enter an email address.";
      setError(message);
      throw new Error(message);
    }
    detailsLockRef.current = true;
    setSavingDetails(true);
    setError(null);
    countryRef.current = normalized.countryCode;
    try {
      const emailResponse = await client.cart.setEmail(cartId, { email: normalized.email });
      applyCart(emailResponse.cart);

      const addressResponse = await client.cart.setAddress(cartId, {
        type: "shipping",
        first_name: normalized.firstName,
        last_name: normalized.lastName,
        address_1: normalized.address1,
        city: normalized.city,
        postal_code: normalized.postalCode,
        country_code: normalized.countryCode,
      });
      applyCart(addressResponse.cart);

      const optionsResponse = await client.shipping.options({
        country_code: normalized.countryCode,
        subtotal: addressResponse.cart.totals.subtotal,
      });
      const options = Array.isArray(optionsResponse.shipping_options)
        ? optionsResponse.shipping_options
        : [];
      const existingOptionId = addressResponse.cart.shipping_methods[0]?.shipping_option_id ?? null;
      const existingOption =
        existingOptionId !== null && options.some((option) => option.option.id === existingOptionId)
          ? existingOptionId
          : null;

      setCheckoutDetails(normalized);
      setShippingOptions(options);
      setSelectedShippingOptionId(existingOption);
      setDetailsReady(true);
      setStepState("details");
      checkoutIdRef.current = null;
      setError(null);
    } catch (cause) {
      if (isNotFound(cause)) {
        handleStaleCart();
        throw new Error("Your cart expired. Please add your items again.", { cause });
      }
      const message = errorMessage(cause, "We couldn't save your checkout details.");
      setError(message);
      throw new Error(message, { cause });
    } finally {
      detailsLockRef.current = false;
      setSavingDetails(false);
    }
  }

  async function selectShippingOption(optionId: string): Promise<void> {
    if (updateLockRef.current || detailsLockRef.current || checkoutLockRef.current) {
      return;
    }
    const cartId = cartIdRef.current;
    if (!cartId) {
      const message = "Your cart is empty.";
      setError(message);
      throw new Error(message);
    }
    updateLockRef.current = true;
    setUpdating(true);
    setError(null);
    try {
      const response = await client.cart.setShippingMethod(cartId, {
        shipping_option_id: optionId,
        country_code: destinationCountry(),
      });
      applyCart(response.cart);
      setSelectedShippingOptionId(optionId);
      checkoutIdRef.current = null;
      setError(null);
    } catch (cause) {
      if (isNotFound(cause)) {
        handleStaleCart();
        throw new Error("Your cart expired. Please add your items again.", { cause });
      }
      const message = errorMessage(cause, "We couldn't select that shipping method.");
      setError(message);
      throw new Error(message, { cause });
    } finally {
      updateLockRef.current = false;
      setUpdating(false);
    }
  }

  async function checkout(): Promise<void> {
    if (checkoutLockRef.current || updateLockRef.current || detailsLockRef.current) {
      return;
    }
    const cartId = cartIdRef.current;
    if (!cartId || !cart || cart.items.length === 0) {
      const message = "Your cart is empty.";
      setError(message);
      throw new Error(message);
    }
    if (!detailsReady || !checkoutDetails || !selectedShippingOptionId) {
      const message = "Choose a shipping method before placing your order.";
      setError(message);
      throw new Error(message);
    }

    checkoutLockRef.current = true;
    setPlacingOrder(true);
    setError(null);
    try {
      const hasShippingMethod = cart.shipping_methods.some(
        (method) => method.shipping_option_id === selectedShippingOptionId,
      );
      if (!hasShippingMethod) {
        const response = await client.cart.setShippingMethod(cartId, {
          shipping_option_id: selectedShippingOptionId,
          country_code: checkoutDetails.countryCode,
        });
        applyCart(response.cart);
        checkoutIdRef.current = null;
      }

      let checkoutId = checkoutIdRef.current;
      if (!checkoutId) {
        const created = await client.checkout.create({
          cart_id: cartId,
          email: checkoutDetails.email,
          payment_provider: "manual",
          country_code: checkoutDetails.countryCode,
        });
        checkoutId = created.checkout.id;
        checkoutIdRef.current = checkoutId;
      }

      const completed = await client.checkout.complete(checkoutId, {
        country_code: checkoutDetails.countryCode,
      });

      checkoutIdRef.current = null;
      cartIdRef.current = null;
      persistCartId(null);
      setOrder(completed.order);
      setCart(null);
      resetCheckoutState();
      setStepState("confirmation");
      setIsOpen(true);
      setError(null);
    } catch (cause) {
      if (isNotFound(cause)) {
        handleStaleCart();
        throw new Error("Your cart expired. Please add your items again.", { cause });
      }
      const message = errorMessage(cause, "We couldn't place your order.");
      setError(message);
      throw new Error(message, { cause });
    } finally {
      checkoutLockRef.current = false;
      setPlacingOrder(false);
    }
  }

  function changeStep(next: CartStep) {
    if (next === "details" && (!cart || cart.items.length === 0)) {
      return;
    }
    if (next === "confirmation" && !order) {
      return;
    }
    setError(null);
    setStepState(next);
  }

  function editDetails() {
    setDetailsReady(false);
    setCheckoutDetails(null);
    setShippingOptions([]);
    setSelectedShippingOptionId(null);
    checkoutIdRef.current = null;
    setError(null);
  }

  function open() {
    setError(null);
    setIsOpen(true);
  }

  function close() {
    setIsOpen(false);
  }

  function continueShopping() {
    setOrder(null);
    resetCheckoutState();
    setStepState("cart");
    setError(null);
    setIsOpen(false);
  }

  const itemCount = cart?.items.reduce((total, item) => total + item.quantity, 0) ?? 0;
  const busy = updating || savingDetails || placingOrder;

  const value: CartContextValue = {
    cart,
    itemCount,
    currencyCode,
    isOpen,
    step,
    order,
    checkoutDetails,
    shippingOptions,
    selectedShippingOptionId,
    detailsReady,
    loading,
    updating,
    savingDetails,
    placingOrder,
    busy,
    error,
    addItem,
    setQuantity,
    removeItem,
    setEmail,
    updateCheckoutDetails,
    selectShippingOption,
    setStep: changeStep,
    editDetails,
    checkout,
    open,
    close,
    continueShopping,
  };

  return <CartContext.Provider value={value}>{children}</CartContext.Provider>;
}

/** Returns the cart context, throwing when rendered outside a provider. */
export function useCart(): CartContextValue {
  const value = useContext(CartContext);
  if (!value) {
    throw new Error("useCart must be used within a CartProvider");
  }
  return value;
}

/**
 * Returns the cart context when one is available.
 *
 * Theme components are also exported for isolated editor previews, which may
 * render them without the page renderer. Those previews keep their original
 * presentation instead of showing controls that cannot reach a storefront.
 */
export function useOptionalCart(): CartContextValue | null {
  return useContext(CartContext);
}

function readStoredCartId(): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const value = window.localStorage.getItem(CART_STORAGE_KEY);
    return value && value.trim().length > 0 ? value : null;
  } catch {
    return null;
  }
}

function persistCartId(cartId: string | null): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    if (cartId) {
      window.localStorage.setItem(CART_STORAGE_KEY, cartId);
    } else {
      window.localStorage.removeItem(CART_STORAGE_KEY);
    }
  } catch {
    // Storage can be unavailable in private browsing or sandboxed frames.
  }
}

function clampQuantity(quantity: number, maximum: number): number {
  if (!Number.isFinite(quantity)) {
    return 1;
  }
  return Math.min(maximum, Math.max(1, Math.trunc(quantity)));
}

function normalizeCheckoutDetails(details: CheckoutDetails): CheckoutDetails {
  return {
    email: details.email.trim(),
    firstName: details.firstName.trim(),
    lastName: details.lastName.trim(),
    address1: details.address1.trim(),
    city: details.city.trim(),
    postalCode: details.postalCode.trim(),
    countryCode: normalizeCountryCode(details.countryCode),
  };
}

function isNotFound(error: unknown): boolean {
  return isRecord(error) && error.status === 404;
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message.trim().length > 0 ? error.message : fallback;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
