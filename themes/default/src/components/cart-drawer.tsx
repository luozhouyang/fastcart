import type { Cart, CartItem, Order } from "@fastcart/shared-types";
import { useEffect, useRef, useState, type FormEvent } from "react";

import { MAX_LINE_QUANTITY, useCart, type CheckoutDetails } from "../cart";
import { COUNTRIES, DEFAULT_COUNTRY_CODE, countryLabel, normalizeCountryCode } from "../countries";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

const FIELD_IDS: Record<keyof CheckoutDetails, string> = {
  email: "fc-checkout-email",
  firstName: "fc-checkout-first-name",
  lastName: "fc-checkout-last-name",
  address1: "fc-checkout-address1",
  city: "fc-checkout-city",
  postalCode: "fc-checkout-postal-code",
  countryCode: "fc-checkout-country",
};

const FIELD_ORDER: Array<keyof CheckoutDetails> = [
  "email",
  "firstName",
  "lastName",
  "address1",
  "city",
  "postalCode",
  "countryCode",
];

/**
 * Renders the fixed cart launcher and the modal cart/checkout drawer.
 *
 * The drawer is only mounted while open, so the server and the first client
 * render contain the launcher with an empty count. All cart contents arrive
 * after hydration.
 */
export function CartDrawer() {
  const { isOpen, itemCount, open, close, step, error } = useCart();
  const panelRef = useRef<HTMLElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const previousStepRef = useRef(step);
  const closeRef = useRef(close);
  closeRef.current = close;

  useEffect(() => {
    if (!isOpen) {
      return;
    }
    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const frame = window.requestAnimationFrame(() => closeButtonRef.current?.focus());

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeRef.current();
        return;
      }
      if (event.key !== "Tab" || !panelRef.current) {
        return;
      }
      const focusable = Array.from(
        panelRef.current.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      );
      if (focusable.length === 0) {
        event.preventDefault();
        panelRef.current.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (!first || !last) {
        return;
      }
      if (event.shiftKey) {
        if (document.activeElement === first || document.activeElement === panelRef.current) {
          event.preventDefault();
          last.focus();
        }
      } else if (document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
      document.body.style.overflow = previousOverflow;
      previouslyFocused?.focus();
    };
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) {
      previousStepRef.current = step;
      return;
    }
    if (previousStepRef.current !== step) {
      previousStepRef.current = step;
      headingRef.current?.focus();
    }
  }, [isOpen, step]);

  const title =
    step === "confirmation" ? "Order confirmed" : step === "details" ? "Checkout" : "Your cart";
  const noun = itemCount === 1 ? "item" : "items";

  return (
    <>
      <button
        type="button"
        className="fc-button fc-button--primary fc-cart-launcher"
        aria-haspopup="dialog"
        aria-expanded={isOpen}
        aria-label={`Open cart, ${itemCount} ${noun}`}
        onClick={open}
      >
        <span aria-hidden="true">Cart</span>
        <span className="fc-cart-launcher-count" aria-hidden="true">
          {itemCount}
        </span>
      </button>

      {isOpen ? (
        <div className="fc-cart-layer">
          <div className="fc-cart-backdrop" aria-hidden="true" onClick={close} />
          <aside
            ref={panelRef}
            className="fc-cart-drawer"
            role="dialog"
            aria-modal="true"
            aria-labelledby="fc-cart-title"
            tabIndex={-1}
          >
            <header className="fc-cart-header">
              <h2 id="fc-cart-title" ref={headingRef} tabIndex={-1}>
                {title}
              </h2>
              <button
                ref={closeButtonRef}
                type="button"
                className="fc-button fc-button--ghost fc-cart-close"
                aria-label="Close cart"
                onClick={close}
              >
                ×
              </button>
            </header>
            {error ? (
              <p className="fc-cart-error" role="alert">
                {error}
              </p>
            ) : null}
            <CartDrawerBody />
          </aside>
        </div>
      ) : null}
    </>
  );
}

function CartDrawerBody() {
  const { cart, step, order } = useCart();
  if (step === "confirmation" && order) {
    return <ConfirmationView order={order} />;
  }
  if (step === "details" && cart && cart.items.length > 0) {
    return <CheckoutView cart={cart} />;
  }
  return <CartView />;
}

function CartView() {
  const { cart, loading, updating, setStep, setQuantity, removeItem, close } = useCart();

  if (loading) {
    return (
      <div className="fc-cart-scroll">
        <p className="fc-cart-loading" role="status">
          Loading your cart…
        </p>
      </div>
    );
  }

  if (!cart || cart.items.length === 0) {
    return (
      <div className="fc-cart-scroll fc-cart-empty">
        <p>Your cart is empty.</p>
        <p className="fc-cart-hint">Add a product to start checkout.</p>
        <button type="button" className="fc-button fc-button--primary" onClick={close}>
          Continue shopping
        </button>
      </div>
    );
  }

  return (
    <>
      <div className="fc-cart-scroll">
        <ul className="fc-cart-lines">
          {cart.items.map((item) => (
            <CartLine
              key={item.id}
              item={item}
              updating={updating}
              onQuantityChange={(quantity) => {
                void setQuantity(item.id, quantity).catch(() => undefined);
              }}
              onRemove={() => {
                void removeItem(item.id).catch(() => undefined);
              }}
            />
          ))}
        </ul>
      </div>
      <div className="fc-cart-footer">
        <CartTotals cart={cart} totalLabel="Estimated total" />
        <div className="fc-cart-button-row">
          <button
            type="button"
            className="fc-button fc-button--primary"
            onClick={() => setStep("details")}
            disabled={updating}
          >
            Continue to checkout
          </button>
        </div>
      </div>
    </>
  );
}

function CartLine({
  item,
  updating,
  onQuantityChange,
  onRemove,
}: {
  item: CartItem;
  updating: boolean;
  onQuantityChange: (quantity: number) => void;
  onRemove: () => void;
}) {
  const lineTotal = formatMoney(item.unit_price * item.quantity, item.currency_code);
  return (
    <li className="fc-cart-line">
      {item.thumbnail ? (
        <img src={item.thumbnail} alt="" loading="lazy" />
      ) : (
        <span className="fc-cart-line-thumb" aria-hidden="true" />
      )}
      <div>
        <p className="fc-cart-line-title">{item.title}</p>
        {item.variant_title ? <p className="fc-cart-line-meta">{item.variant_title}</p> : null}
        <p className="fc-cart-line-meta">{formatMoney(item.unit_price, item.currency_code)}</p>
        <div className="fc-cart-line-actions">
          <div className="fc-cart-qty">
            <button
              type="button"
              aria-label={`Decrease quantity of ${item.title}`}
              onClick={() => onQuantityChange(item.quantity - 1)}
              disabled={updating || item.quantity <= 1}
            >
              −
            </button>
            <span aria-live="polite">{item.quantity}</span>
            <button
              type="button"
              aria-label={`Increase quantity of ${item.title}`}
              onClick={() => onQuantityChange(item.quantity + 1)}
              disabled={updating || item.quantity >= MAX_LINE_QUANTITY}
            >
              +
            </button>
          </div>
          <button
            type="button"
            className="fc-button fc-button--ghost fc-cart-remove"
            onClick={onRemove}
            disabled={updating}
            aria-label={`Remove ${item.title} from cart`}
          >
            Remove
          </button>
        </div>
      </div>
      <p className="fc-cart-line-total">{lineTotal}</p>
    </li>
  );
}

function CheckoutView({ cart }: { cart: Cart }) {
  const { detailsReady } = useCart();
  return detailsReady ? <ShippingReview cart={cart} /> : <DetailsForm cart={cart} />;
}

function DetailsForm({ cart }: { cart: Cart }) {
  const { updateCheckoutDetails, savingDetails, setStep } = useCart();
  const [form, setForm] = useState<CheckoutDetails>(() => detailsFromCart(cart));
  const [errors, setErrors] = useState<Record<string, string>>({});

  function updateField(key: keyof CheckoutDetails, value: string) {
    setForm((current) => ({ ...current, [key]: value }));
    setErrors((current) => {
      if (!(key in current)) {
        return current;
      }
      const next = { ...current };
      delete next[key];
      return next;
    });
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (savingDetails) {
      return;
    }
    const normalized = normalizeForm(form);
    const nextErrors = validateCheckoutDetails(normalized);
    setForm(normalized);
    if (Object.keys(nextErrors).length > 0) {
      setErrors(nextErrors);
      focusFirstInvalidField(nextErrors);
      return;
    }
    setErrors({});
    try {
      await updateCheckoutDetails(normalized);
    } catch {
      // The provider exposes the API error in the drawer banner.
    }
  }

  const countryExists = COUNTRIES.some((country) => country.code === form.countryCode);
  const countryOptions = countryExists
    ? COUNTRIES
    : [...COUNTRIES, { code: form.countryCode, label: form.countryCode }];

  return (
    <form className="fc-cart-form" onSubmit={handleSubmit} noValidate>
      <div className="fc-cart-scroll">
        <p className="fc-cart-hint">
          We use this address for delivery and order updates. All fields marked * are required.
        </p>

        <div className="fc-cart-field">
          <label htmlFor={FIELD_IDS.email}>Email *</label>
          <input
            id={FIELD_IDS.email}
            type="email"
            autoComplete="email"
            value={form.email}
            required
            aria-invalid={errors.email ? "true" : undefined}
            aria-describedby={errors.email ? `${FIELD_IDS.email}-error` : undefined}
            onChange={(event) => updateField("email", event.target.value)}
          />
          {errors.email ? (
            <p id={`${FIELD_IDS.email}-error`} className="fc-cart-field-error">
              {errors.email}
            </p>
          ) : null}
        </div>

        <div className="fc-cart-field-row">
          <div className="fc-cart-field">
            <label htmlFor={FIELD_IDS.firstName}>First name *</label>
            <input
              id={FIELD_IDS.firstName}
              type="text"
              autoComplete="given-name"
              value={form.firstName}
              required
              aria-invalid={errors.firstName ? "true" : undefined}
              aria-describedby={errors.firstName ? `${FIELD_IDS.firstName}-error` : undefined}
              onChange={(event) => updateField("firstName", event.target.value)}
            />
            {errors.firstName ? (
              <p id={`${FIELD_IDS.firstName}-error`} className="fc-cart-field-error">
                {errors.firstName}
              </p>
            ) : null}
          </div>
          <div className="fc-cart-field">
            <label htmlFor={FIELD_IDS.lastName}>Last name *</label>
            <input
              id={FIELD_IDS.lastName}
              type="text"
              autoComplete="family-name"
              value={form.lastName}
              required
              aria-invalid={errors.lastName ? "true" : undefined}
              aria-describedby={errors.lastName ? `${FIELD_IDS.lastName}-error` : undefined}
              onChange={(event) => updateField("lastName", event.target.value)}
            />
            {errors.lastName ? (
              <p id={`${FIELD_IDS.lastName}-error`} className="fc-cart-field-error">
                {errors.lastName}
              </p>
            ) : null}
          </div>
        </div>

        <div className="fc-cart-field">
          <label htmlFor={FIELD_IDS.address1}>Address *</label>
          <input
            id={FIELD_IDS.address1}
            type="text"
            autoComplete="address-line1"
            value={form.address1}
            required
            aria-invalid={errors.address1 ? "true" : undefined}
            aria-describedby={errors.address1 ? `${FIELD_IDS.address1}-error` : undefined}
            onChange={(event) => updateField("address1", event.target.value)}
          />
          {errors.address1 ? (
            <p id={`${FIELD_IDS.address1}-error`} className="fc-cart-field-error">
              {errors.address1}
            </p>
          ) : null}
        </div>

        <div className="fc-cart-field-row">
          <div className="fc-cart-field">
            <label htmlFor={FIELD_IDS.city}>City *</label>
            <input
              id={FIELD_IDS.city}
              type="text"
              autoComplete="address-level2"
              value={form.city}
              required
              aria-invalid={errors.city ? "true" : undefined}
              aria-describedby={errors.city ? `${FIELD_IDS.city}-error` : undefined}
              onChange={(event) => updateField("city", event.target.value)}
            />
            {errors.city ? (
              <p id={`${FIELD_IDS.city}-error`} className="fc-cart-field-error">
                {errors.city}
              </p>
            ) : null}
          </div>
          <div className="fc-cart-field">
            <label htmlFor={FIELD_IDS.postalCode}>Postal code *</label>
            <input
              id={FIELD_IDS.postalCode}
              type="text"
              autoComplete="postal-code"
              value={form.postalCode}
              required
              aria-invalid={errors.postalCode ? "true" : undefined}
              aria-describedby={errors.postalCode ? `${FIELD_IDS.postalCode}-error` : undefined}
              onChange={(event) => updateField("postalCode", event.target.value)}
            />
            {errors.postalCode ? (
              <p id={`${FIELD_IDS.postalCode}-error`} className="fc-cart-field-error">
                {errors.postalCode}
              </p>
            ) : null}
          </div>
        </div>

        <div className="fc-cart-field">
          <label htmlFor={FIELD_IDS.countryCode}>Country *</label>
          <select
            id={FIELD_IDS.countryCode}
            autoComplete="country"
            value={form.countryCode}
            required
            aria-invalid={errors.countryCode ? "true" : undefined}
            aria-describedby={errors.countryCode ? `${FIELD_IDS.countryCode}-error` : undefined}
            onChange={(event) => updateField("countryCode", event.target.value)}
          >
            {countryOptions.map((country) => (
              <option key={country.code} value={country.code}>
                {country.label}
              </option>
            ))}
          </select>
          {errors.countryCode ? (
            <p id={`${FIELD_IDS.countryCode}-error`} className="fc-cart-field-error">
              {errors.countryCode}
            </p>
          ) : null}
        </div>
      </div>

      <div className="fc-cart-footer">
        <div className="fc-cart-button-row">
          <button type="submit" className="fc-button fc-button--primary" disabled={savingDetails}>
            {savingDetails ? "Saving…" : "Continue to shipping"}
          </button>
          <button
            type="button"
            className="fc-button fc-button--secondary"
            onClick={() => setStep("cart")}
            disabled={savingDetails}
          >
            Back to cart
          </button>
        </div>
      </div>
    </form>
  );
}

function ShippingReview({ cart }: { cart: Cart }) {
  const {
    checkoutDetails,
    shippingOptions,
    selectedShippingOptionId,
    placingOrder,
    busy,
    selectShippingOption,
    checkout,
    editDetails,
    setStep,
  } = useCart();
  const details = checkoutDetails ?? detailsFromCart(cart);
  const selectedOption =
    shippingOptions.find((option) => option.option.id === selectedShippingOptionId) ?? null;
  const canPlaceOrder = selectedOption !== null && !busy;

  async function handlePlaceOrder() {
    try {
      await checkout();
    } catch {
      // The provider exposes the API error in the drawer banner.
    }
  }

  return (
    <>
      <div className="fc-cart-scroll">
        <section className="fc-cart-section" aria-labelledby="fc-cart-contact-title">
          <div className="fc-cart-section-head">
            <h3 id="fc-cart-contact-title">Contact</h3>
            <button
              type="button"
              className="fc-button fc-button--ghost"
              onClick={editDetails}
              disabled={busy}
            >
              Edit
            </button>
          </div>
          <p className="fc-cart-address">{details.email || cart.email || ""}</p>
        </section>

        <section className="fc-cart-section" aria-labelledby="fc-cart-address-title">
          <h3 id="fc-cart-address-title">Ship to</h3>
          <p className="fc-cart-address">
            {addressLines(details).map((line, index) => (
              <span key={`${line}-${index}`}>
                {line}
                <br />
              </span>
            ))}
          </p>
        </section>

        <fieldset className="fc-cart-shipping-options">
          <legend>Shipping method</legend>
          {shippingOptions.length === 0 ? (
            <p className="fc-cart-hint" role="status">
              No shipping options are available for this destination. Edit your details and try
              another country.
            </p>
          ) : (
            shippingOptions.map((option) => (
              <label key={option.option.id} className="fc-cart-shipping-option">
                <input
                  type="radio"
                  name="fc-shipping-option"
                  value={option.option.id}
                  checked={selectedShippingOptionId === option.option.id}
                  disabled={busy}
                  onChange={() => {
                    void selectShippingOption(option.option.id).catch(() => undefined);
                  }}
                />
                <span className="fc-cart-shipping-option-name">{option.option.name}</span>
                <span>{formatMoney(option.amount_minor, option.currency)}</span>
              </label>
            ))
          )}
        </fieldset>
      </div>

      <div className="fc-cart-footer">
        <CartTotals cart={cart} totalLabel="Total" />
        <div className="fc-cart-button-row">
          <button
            type="button"
            className="fc-button fc-button--primary"
            onClick={handlePlaceOrder}
            disabled={!canPlaceOrder}
          >
            {placingOrder ? "Placing order…" : "Place order"}
          </button>
          <button
            type="button"
            className="fc-button fc-button--secondary"
            onClick={() => setStep("cart")}
            disabled={busy}
          >
            Back to cart
          </button>
        </div>
      </div>
    </>
  );
}

function ConfirmationView({ order }: { order: Order }) {
  const { continueShopping } = useCart();
  return (
    <>
      <div className="fc-cart-scroll">
        <div className="fc-cart-confirmation" role="status" aria-live="polite">
          <p className="fc-cart-eyebrow">Order confirmed</p>
          <p>Thank you. Your order is confirmed and a receipt will be sent by email.</p>
          <dl className="fc-cart-totals">
            <div>
              <dt>Order number</dt>
              <dd>#{order.display_number}</dd>
            </div>
            <div>
              <dt>Total</dt>
              <dd>{formatMoney(order.total, order.currency_code)}</dd>
            </div>
            <div>
              <dt>Email</dt>
              <dd>{order.email}</dd>
            </div>
          </dl>
        </div>
      </div>
      <div className="fc-cart-footer">
        <div className="fc-cart-button-row">
          <button type="button" className="fc-button fc-button--primary" onClick={continueShopping}>
            Continue shopping
          </button>
        </div>
      </div>
    </>
  );
}

function CartTotals({ cart, totalLabel }: { cart: Cart; totalLabel: string }) {
  return (
    <dl className="fc-cart-totals">
      <div>
        <dt>Subtotal</dt>
        <dd>{formatMoney(cart.totals.item_subtotal, cart.currency_code)}</dd>
      </div>
      {cart.totals.discount_total > 0 ? (
        <div>
          <dt>Discount</dt>
          <dd>−{formatMoney(cart.totals.discount_total, cart.currency_code)}</dd>
        </div>
      ) : null}
      {cart.totals.shipping_total > 0 ? (
        <div>
          <dt>Shipping</dt>
          <dd>{formatMoney(cart.totals.shipping_total, cart.currency_code)}</dd>
        </div>
      ) : null}
      <div className="fc-cart-grand-total">
        <dt>{totalLabel}</dt>
        <dd>{formatMoney(cart.totals.total, cart.currency_code)}</dd>
      </div>
    </dl>
  );
}

function detailsFromCart(cart: Cart | null): CheckoutDetails {
  const address = cart?.shipping_address;
  return {
    email: cart?.email ?? "",
    firstName: address?.first_name ?? "",
    lastName: address?.last_name ?? "",
    address1: address?.address_1 ?? "",
    city: address?.city ?? "",
    postalCode: address?.postal_code ?? "",
    countryCode: normalizeCountryCode(address?.country_code ?? DEFAULT_COUNTRY_CODE),
  };
}

function normalizeForm(form: CheckoutDetails): CheckoutDetails {
  return {
    email: form.email.trim(),
    firstName: form.firstName.trim(),
    lastName: form.lastName.trim(),
    address1: form.address1.trim(),
    city: form.city.trim(),
    postalCode: form.postalCode.trim(),
    countryCode: normalizeCountryCode(form.countryCode),
  };
}

function validateCheckoutDetails(details: CheckoutDetails): Record<string, string> {
  const errors: Record<string, string> = {};
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(details.email)) {
    errors.email = "Enter a valid email address.";
  }
  if (details.firstName.length === 0) {
    errors.firstName = "Enter a first name.";
  }
  if (details.lastName.length === 0) {
    errors.lastName = "Enter a last name.";
  }
  if (details.address1.length === 0) {
    errors.address1 = "Enter a street address.";
  }
  if (details.city.length === 0) {
    errors.city = "Enter a city.";
  }
  if (details.postalCode.length === 0) {
    errors.postalCode = "Enter a postal code.";
  }
  if (!/^[A-Z]{2}$/.test(details.countryCode)) {
    errors.countryCode = "Choose a country.";
  }
  return errors;
}

function focusFirstInvalidField(errors: Record<string, string>) {
  for (const key of FIELD_ORDER) {
    if (errors[key]) {
      document.getElementById(FIELD_IDS[key])?.focus();
      return;
    }
  }
}

function addressLines(details: CheckoutDetails): string[] {
  const name = [details.firstName, details.lastName].filter(Boolean).join(" ");
  const locality = [details.city, details.postalCode].filter(Boolean).join(", ");
  return [name, details.address1, locality, countryLabel(details.countryCode)].filter(
    (line) => line.length > 0,
  );
}

function formatMoney(amountMinor: number, currencyCode: string): string {
  const currency = currencyCode.toUpperCase();
  try {
    const formatter = new Intl.NumberFormat(undefined, {
      style: "currency",
      currency,
    });
    const digits = formatter.resolvedOptions().maximumFractionDigits ?? 2;
    return formatter.format(amountMinor / 10 ** digits);
  } catch {
    return `${(amountMinor / 100).toFixed(2)} ${currency}`;
  }
}
