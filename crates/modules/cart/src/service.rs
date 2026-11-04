//! SQLite-backed cart application service.
//!
//! Every mutation first checks that the cart is open, writes inside a short
//! transaction and returns a freshly loaded aggregate. Totals are always
//! derived from stored rows by [`calculate_totals`]; no total is ever accepted
//! from an input payload.

use std::collections::BTreeMap;

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CartId, CurrencyCode, CustomerId, LineItemId, Metadata, PageResult, Pagination,
    ProductId, PromotionId, ShippingOptionId, VariantId, normalize_email, now_timestamp,
    validate_email, validate_length, validate_metadata, validate_non_negative_amount,
    validate_quantity,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::{FromRow, Sqlite, Transaction};

use crate::domain::{
    AddItemInput, Address, AddressInput, AdjustmentInput, Cart, CartAdjustment, CartDiscount,
    CartItem, CartShippingMethod, CartStatus, CartTaxLine, CartTotals, CreateCartInput,
    DEFAULT_CART_TTL_SECONDS, DiscountInput, SetShippingMethodInput, TaxLineInput, UpdateItemInput,
    recalculate_totals,
};

const CART_COLUMNS: &str = "SELECT id, currency_code, customer_id, email, sales_channel_id, \
     status, shipping_address, billing_address, metadata, created_at, updated_at, completed_at, \
     expires_at FROM carts WHERE id = ?";

const ITEM_COLUMNS: &str = "SELECT id, cart_id, product_id, variant_id, quantity, title, \
     variant_title, sku, thumbnail, unit_price, currency_code, is_tax_inclusive, \
     requires_shipping, metadata FROM cart_items WHERE cart_id = ? ORDER BY created_at ASC, id ASC";

const ITEM_ADJUSTMENT_COLUMNS: &str = "SELECT a.cart_item_id, a.promotion_id, a.code, a.amount, \
     a.description FROM cart_item_adjustments a JOIN cart_items i ON i.id = a.cart_item_id \
     WHERE i.cart_id = ? ORDER BY a.created_at ASC, a.id ASC";

const DISCOUNT_COLUMNS: &str = "SELECT promotion_id, code, amount, is_automatic, metadata \
     FROM cart_discounts WHERE cart_id = ? ORDER BY created_at ASC, id ASC";

const SHIPPING_METHOD_COLUMNS: &str = "SELECT shipping_option_id, name, amount, \
     currency_code, is_tax_inclusive, data FROM cart_shipping_methods WHERE cart_id = ? \
     ORDER BY created_at ASC, id ASC";

const SHIPPING_ADJUSTMENT_COLUMNS: &str = "SELECT a.shipping_method_id, a.promotion_id, a.code, \
     a.amount, a.description FROM cart_shipping_method_adjustments a \
     JOIN cart_shipping_methods m ON m.id = a.shipping_method_id WHERE m.cart_id = ? \
     ORDER BY a.created_at ASC, a.id ASC";

const TAX_LINE_COLUMNS: &str = "SELECT name, rate_bps, amount, is_tax_inclusive, cart_item_id, \
     shipping_method_id FROM cart_tax_lines WHERE cart_id = ? ORDER BY created_at ASC, id ASC";

const ADDRESS_COLUMNS: &str = "SELECT type, first_name, last_name, company, address_1, \
     address_2, city, province, postal_code, country_code, phone, metadata \
     FROM cart_addresses WHERE cart_id = ?";

/// SQLite-backed service owning carts and their commercial snapshots.
#[derive(Clone, Debug)]
pub struct CartService {
    db: StoreDatabase,
}

impl CartService {
    /// Creates a service bound to an already-migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Creates an anonymous or customer cart.
    pub async fn create_cart(&self, input: CreateCartInput) -> Result<Cart, AppError> {
        let currency = CurrencyCode::new(input.currency_code)?;
        validate_metadata(&input.metadata)?;

        let email = match input.email {
            Some(value) => {
                validate_email(&value, "email")?;
                Some(normalize_email(&value))
            }
            None => None,
        };

        let ttl = input.expires_in_seconds.unwrap_or(DEFAULT_CART_TTL_SECONDS);
        if ttl < 0 {
            return Err(AppError::validation(
                "expires_in_seconds must not be negative",
            ));
        }

        let now = now_timestamp();
        let cart_id = CartId::new();
        let expires_at = now.saturating_add(ttl);

        sqlx::query(
            "INSERT INTO carts (id, currency_code, customer_id, email, sales_channel_id, status, \
             shipping_address, billing_address, metadata, created_at, updated_at, completed_at, \
             expires_at) VALUES (?, ?, ?, ?, ?, 'open', '{}', '{}', ?, ?, ?, NULL, ?)",
        )
        .bind(cart_id.as_str())
        .bind(currency.as_str())
        .bind(input.customer_id.as_ref().map(CustomerId::as_str))
        .bind(email.as_deref())
        .bind(input.sales_channel_id.as_deref())
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .bind(expires_at)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;

        self.get_cart(&cart_id).await
    }

    /// Loads the full aggregate and recalculates its totals.
    pub async fn get_cart(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        let row = self.cart_row(cart_id).await?;
        let items = self.load_items(cart_id).await?;
        let discounts = self.load_discounts(cart_id).await?;
        let shipping_methods = self.load_shipping_methods(cart_id).await?;
        let shipping_adjustments = self.load_shipping_adjustments(cart_id).await?;
        let tax_lines = self.load_tax_lines(cart_id).await?;
        let addresses = self.load_addresses(cart_id).await?;

        let item_adjustments = items
            .iter()
            .flat_map(|item| item.adjustments.iter().cloned())
            .collect::<Vec<_>>();

        let currency_code = CurrencyCode::new(row.currency_code.as_str())?;
        let mut cart = Cart {
            id: CartId::from_string(row.id),
            currency_code: currency_code.clone(),
            customer_id: row.customer_id.map(CustomerId::from_string),
            email: row.email,
            sales_channel_id: row.sales_channel_id,
            status: CartStatus::from_db(&row.status),
            shipping_address: addresses
                .shipping
                .unwrap_or_else(|| parse_address(&row.shipping_address)),
            billing_address: addresses
                .billing
                .unwrap_or_else(|| parse_address(&row.billing_address)),
            items,
            discounts,
            shipping_methods,
            item_adjustments,
            shipping_adjustments,
            tax_lines,
            totals: CartTotals::zero(currency_code),
            metadata: Metadata::from_json_str(&row.metadata),
            created_at: row.created_at,
            updated_at: row.updated_at,
            completed_at: row.completed_at,
            expires_at: row.expires_at,
        };
        recalculate_totals(&mut cart);
        Ok(cart)
    }

    /// Adds a line snapshot, merging into an existing variant line.
    ///
    /// When the variant is already present its quantity is incremented and the
    /// stored price snapshot is kept; workflows reprice explicitly with
    /// [`Self::set_item_unit_price`].
    pub async fn add_item(&self, cart_id: &CartId, input: AddItemInput) -> Result<Cart, AppError> {
        let cart = self.ensure_open(cart_id).await?;
        let cart_currency = CurrencyCode::new(cart.currency_code.as_str())?;
        let item_currency = CurrencyCode::new(input.currency_code.as_str())?;
        if item_currency != cart_currency {
            return Err(AppError::conflict(format!(
                "cart currency {cart_currency} does not match item currency {item_currency}"
            )));
        }
        validate_quantity(input.quantity, false)?;
        validate_non_negative_amount(input.unit_price, "unit_price")?;
        validate_length(&input.title, "title", 255)?;
        validate_metadata(&input.metadata)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;

        let existing: Option<(String, i64)> = sqlx::query_as(
            "SELECT id, quantity FROM cart_items WHERE cart_id = ? AND variant_id = ?",
        )
        .bind(cart_id.as_str())
        .bind(input.variant_id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;

        if let Some((item_id, quantity)) = existing {
            let new_quantity = quantity
                .checked_add(input.quantity)
                .ok_or_else(|| AppError::validation("quantity overflow"))?;
            validate_quantity(new_quantity, false)?;
            sqlx::query("UPDATE cart_items SET quantity = ?, updated_at = ? WHERE id = ?")
                .bind(new_quantity)
                .bind(now)
                .bind(item_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        } else {
            sqlx::query(
                "INSERT INTO cart_items (id, cart_id, product_id, variant_id, quantity, title, \
                 variant_title, sku, thumbnail, unit_price, currency_code, is_tax_inclusive, \
                 requires_shipping, metadata, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(LineItemId::new().as_str())
            .bind(cart_id.as_str())
            .bind(input.product_id.as_str())
            .bind(input.variant_id.as_str())
            .bind(input.quantity)
            .bind(&input.title)
            .bind(input.variant_title.as_deref())
            .bind(input.sku.as_deref())
            .bind(input.thumbnail.as_deref())
            .bind(input.unit_price)
            .bind(item_currency.as_str())
            .bind(input.is_tax_inclusive)
            .bind(input.requires_shipping)
            .bind(input.metadata.to_json_string())
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Sets a line quantity. A quantity of zero removes the line.
    pub async fn update_item_quantity(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
        input: UpdateItemInput,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_quantity(input.quantity, true)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let found: Option<String> =
            sqlx::query_scalar("SELECT id FROM cart_items WHERE id = ? AND cart_id = ?")
                .bind(item_id.as_str())
                .bind(cart_id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
        if found.is_none() {
            return Err(AppError::not_found("cart item", item_id));
        }

        if input.quantity == 0 {
            sqlx::query("DELETE FROM cart_items WHERE id = ? AND cart_id = ?")
                .bind(item_id.as_str())
                .bind(cart_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            sqlx::query("DELETE FROM cart_tax_lines WHERE cart_id = ? AND cart_item_id = ?")
                .bind(cart_id.as_str())
                .bind(item_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        } else {
            sqlx::query(
                "UPDATE cart_items SET quantity = ?, updated_at = ? WHERE id = ? AND cart_id = ?",
            )
            .bind(input.quantity)
            .bind(now)
            .bind(item_id.as_str())
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Removes one line item.
    pub async fn remove_item(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;

        let result = sqlx::query("DELETE FROM cart_items WHERE id = ? AND cart_id = ?")
            .bind(item_id.as_str())
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("cart item", item_id));
        }
        sqlx::query("DELETE FROM cart_tax_lines WHERE cart_id = ? AND cart_item_id = ?")
            .bind(cart_id.as_str())
            .bind(item_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Removes every line item from the cart.
    pub async fn clear_items(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;

        sqlx::query("DELETE FROM cart_items WHERE cart_id = ?")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("DELETE FROM cart_tax_lines WHERE cart_id = ? AND cart_item_id IS NOT NULL")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Attaches an authenticated customer and optionally updates the email.
    ///
    /// A cart that already belongs to a different customer is rejected with a
    /// conflict rather than silently transferred.
    pub async fn attach_customer(
        &self,
        cart_id: &CartId,
        customer_id: &CustomerId,
        email: Option<String>,
    ) -> Result<Cart, AppError> {
        let row = self.ensure_open(cart_id).await?;
        if let Some(existing) = row.customer_id.as_deref()
            && existing != customer_id.as_str()
        {
            return Err(AppError::conflict(
                "cart already belongs to a different customer",
            ));
        }

        let email = match email {
            Some(value) => {
                validate_email(&value, "email")?;
                Some(normalize_email(&value))
            }
            None => None,
        };

        let now = now_timestamp();
        sqlx::query(
            "UPDATE carts SET customer_id = ?, email = COALESCE(?, email), updated_at = ? \
             WHERE id = ?",
        )
        .bind(customer_id.as_str())
        .bind(email.as_deref())
        .bind(now)
        .bind(cart_id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;

        self.get_cart(cart_id).await
    }

    /// Sets the contact email.
    pub async fn set_email(&self, cart_id: &CartId, email: &str) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_email(email, "email")?;
        let email = normalize_email(email);
        let now = now_timestamp();
        sqlx::query("UPDATE carts SET email = ?, updated_at = ? WHERE id = ?")
            .bind(email)
            .bind(now)
            .bind(cart_id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Sets the shipping address snapshot.
    pub async fn set_shipping_address(
        &self,
        cart_id: &CartId,
        input: AddressInput,
    ) -> Result<Cart, AppError> {
        self.set_address(cart_id, "shipping", input).await
    }

    /// Sets the billing address snapshot.
    pub async fn set_billing_address(
        &self,
        cart_id: &CartId,
        input: AddressInput,
    ) -> Result<Cart, AppError> {
        self.set_address(cart_id, "billing", input).await
    }

    /// Replaces the cart's shipping method and clears its shipping adjustments.
    pub async fn set_shipping_method(
        &self,
        cart_id: &CartId,
        input: SetShippingMethodInput,
    ) -> Result<Cart, AppError> {
        let cart = self.ensure_open(cart_id).await?;
        let cart_currency = CurrencyCode::new(cart.currency_code.as_str())?;
        let shipping_currency = CurrencyCode::new(input.currency_code.as_str())?;
        if shipping_currency != cart_currency {
            return Err(AppError::conflict(format!(
                "cart currency {cart_currency} does not match shipping currency {shipping_currency}"
            )));
        }
        validate_non_negative_amount(input.amount, "shipping amount")?;
        validate_length(&input.name, "name", 255)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;

        sqlx::query("DELETE FROM cart_shipping_methods WHERE cart_id = ?")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query(
            "INSERT INTO cart_shipping_methods (id, cart_id, shipping_option_id, name, amount, \
             currency_code, is_tax_inclusive, data, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row_id("shm"))
        .bind(cart_id.as_str())
        .bind(input.shipping_option_id.as_str())
        .bind(&input.name)
        .bind(input.amount)
        .bind(shipping_currency.as_str())
        .bind(input.is_tax_inclusive)
        .bind(json_string(&input.data))
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Adds a cart-level discount snapshot.
    pub async fn add_discount(
        &self,
        cart_id: &CartId,
        input: DiscountInput,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_discount_input(&input)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "INSERT INTO cart_discounts (id, cart_id, promotion_id, code, amount, is_automatic, \
             metadata, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row_id("disc"))
        .bind(cart_id.as_str())
        .bind(input.promotion_id.as_ref().map(PromotionId::as_str))
        .bind(input.code.as_deref())
        .bind(input.amount)
        .bind(input.is_automatic)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Removes every discount produced by a promotion.
    pub async fn remove_discount(
        &self,
        cart_id: &CartId,
        promotion_id: &PromotionId,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;

        let result =
            sqlx::query("DELETE FROM cart_discounts WHERE cart_id = ? AND promotion_id = ?")
                .bind(cart_id.as_str())
                .bind(promotion_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("cart discount", promotion_id));
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Replaces every cart-level discount, as promotion workflows do.
    pub async fn replace_discounts(
        &self,
        cart_id: &CartId,
        discounts: Vec<DiscountInput>,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        for discount in &discounts {
            validate_discount_input(discount)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query("DELETE FROM cart_discounts WHERE cart_id = ?")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        for discount in discounts {
            sqlx::query(
                "INSERT INTO cart_discounts (id, cart_id, promotion_id, code, amount, \
                 is_automatic, metadata, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row_id("disc"))
            .bind(cart_id.as_str())
            .bind(discount.promotion_id.as_ref().map(PromotionId::as_str))
            .bind(discount.code.as_deref())
            .bind(discount.amount)
            .bind(discount.is_automatic)
            .bind(discount.metadata.to_json_string())
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Appends one adjustment to a line item.
    pub async fn add_item_adjustment(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
        input: AdjustmentInput,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_adjustment_input(&input)?;

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        ensure_item_belongs_to_cart(&mut tx, cart_id, item_id).await?;
        insert_adjustment(
            &mut tx,
            "cart_item_adjustments",
            "cart_item_id",
            item_id.as_str(),
            &input,
            now,
        )
        .await?;

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Replaces every adjustment attached to one line item.
    pub async fn replace_item_adjustments(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
        adjustments: Vec<AdjustmentInput>,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        for adjustment in &adjustments {
            validate_adjustment_input(adjustment)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        ensure_item_belongs_to_cart(&mut tx, cart_id, item_id).await?;
        sqlx::query("DELETE FROM cart_item_adjustments WHERE cart_item_id = ?")
            .bind(item_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        for adjustment in adjustments {
            insert_adjustment(
                &mut tx,
                "cart_item_adjustments",
                "cart_item_id",
                item_id.as_str(),
                &adjustment,
                now,
            )
            .await?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Replaces the shipping adjustments on the cart's single shipping method.
    pub async fn replace_shipping_adjustments(
        &self,
        cart_id: &CartId,
        adjustments: Vec<AdjustmentInput>,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        for adjustment in &adjustments {
            validate_adjustment_input(adjustment)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let method_id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM cart_shipping_methods WHERE cart_id = ? \
             ORDER BY created_at ASC, id ASC LIMIT 1",
        )
        .bind(cart_id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        let Some(method_id) = method_id else {
            return Err(AppError::not_found("shipping method", cart_id));
        };

        sqlx::query("DELETE FROM cart_shipping_method_adjustments WHERE shipping_method_id = ?")
            .bind(&method_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        for adjustment in adjustments {
            insert_adjustment(
                &mut tx,
                "cart_shipping_method_adjustments",
                "shipping_method_id",
                &method_id,
                &adjustment,
                now,
            )
            .await?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Replaces every tax line on the cart.
    pub async fn replace_tax_lines(
        &self,
        cart_id: &CartId,
        lines: Vec<TaxLineInput>,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        for line in &lines {
            validate_tax_line_input(line)?;
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query("DELETE FROM cart_tax_lines WHERE cart_id = ?")
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        for line in lines {
            sqlx::query(
                "INSERT INTO cart_tax_lines (id, cart_id, cart_item_id, shipping_method_id, name, \
                 rate_bps, amount, is_tax_inclusive, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row_id("tax"))
            .bind(cart_id.as_str())
            .bind(line.cart_item_id.as_ref().map(LineItemId::as_str))
            .bind(line.shipping_method_id.as_deref())
            .bind(&line.name)
            .bind(line.rate_bps)
            .bind(line.amount)
            .bind(line.is_tax_inclusive)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Updates a line's unit price snapshot.
    ///
    /// Workflows call this after obtaining an authoritative price from the
    /// pricing module; the supplied currency must match the cart currency.
    pub async fn set_item_unit_price(
        &self,
        cart_id: &CartId,
        item_id: &LineItemId,
        unit_price: i64,
        currency_code: &str,
    ) -> Result<Cart, AppError> {
        let cart = self.ensure_open(cart_id).await?;
        validate_non_negative_amount(unit_price, "unit_price")?;
        let cart_currency = CurrencyCode::new(cart.currency_code.as_str())?;
        let price_currency = CurrencyCode::new(currency_code)?;
        if price_currency != cart_currency {
            return Err(AppError::conflict(format!(
                "cart currency {cart_currency} does not match price currency {price_currency}"
            )));
        }

        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query(
            "UPDATE cart_items SET unit_price = ?, updated_at = ? WHERE id = ? AND cart_id = ?",
        )
        .bind(unit_price)
        .bind(now)
        .bind(item_id.as_str())
        .bind(cart_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("cart item", item_id));
        }

        Self::touch_cart(&mut tx, cart_id, now).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    /// Recalculates and returns the current totals.
    pub async fn recalculate(&self, cart_id: &CartId) -> Result<CartTotals, AppError> {
        Ok(self.get_cart(cart_id).await?.totals)
    }

    /// Marks an open cart as completed.
    pub async fn complete_cart(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE carts SET status = 'completed', completed_at = ?, updated_at = ? \
             WHERE id = ? AND status = 'open'",
        )
        .bind(now)
        .bind(now)
        .bind(cart_id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;

        if result.rows_affected() == 0 {
            let _ = self.cart_row(cart_id).await?;
            return Err(AppError::conflict("only open carts can be completed"));
        }
        self.get_cart(cart_id).await
    }

    /// Expires every open cart whose expiration timestamp passed.
    ///
    /// Returns the number of carts transitioned to `expired`.
    pub async fn expire_due(&self, now: i64) -> Result<u64, AppError> {
        let result = sqlx::query(
            "UPDATE carts SET status = 'expired', updated_at = ? \
             WHERE status = 'open' AND expires_at IS NOT NULL AND expires_at <= ?",
        )
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected())
    }

    /// Lists carts, optionally filtered by customer.
    pub async fn list_carts(
        &self,
        customer_id: Option<&CustomerId>,
        pagination: Pagination,
    ) -> Result<PageResult<Cart>, AppError> {
        let (count, ids): (i64, Vec<String>) = match customer_id {
            Some(customer) => {
                let count: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM carts WHERE customer_id = ?")
                        .bind(customer.as_str())
                        .fetch_one(self.db.pool())
                        .await
                        .map_err(database_error)?;
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT id FROM carts WHERE customer_id = ? ORDER BY created_at DESC, id DESC \
                     LIMIT ? OFFSET ?",
                )
                .bind(customer.as_str())
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?;
                (count, ids)
            }
            None => {
                let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM carts")
                    .fetch_one(self.db.pool())
                    .await
                    .map_err(database_error)?;
                let ids: Vec<String> = sqlx::query_scalar(
                    "SELECT id FROM carts ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?",
                )
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?;
                (count, ids)
            }
        };

        let mut carts = Vec::with_capacity(ids.len());
        for id in ids {
            carts.push(self.get_cart(&CartId::from_string(id)).await?);
        }
        Ok(PageResult::new(carts, count, pagination))
    }

    async fn set_address(
        &self,
        cart_id: &CartId,
        address_type: &str,
        input: AddressInput,
    ) -> Result<Cart, AppError> {
        self.ensure_open(cart_id).await?;
        validate_metadata(&input.metadata)?;
        let address = input.try_into_address()?;
        let now = now_timestamp();

        let column = match address_type {
            "shipping" => "shipping_address",
            _ => "billing_address",
        };

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "INSERT INTO cart_addresses (id, cart_id, type, first_name, last_name, company, \
             address_1, address_2, city, province, postal_code, country_code, phone, metadata, \
             created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (cart_id, type) DO UPDATE SET first_name = excluded.first_name, \
             last_name = excluded.last_name, company = excluded.company, \
             address_1 = excluded.address_1, address_2 = excluded.address_2, city = excluded.city, \
             province = excluded.province, postal_code = excluded.postal_code, \
             country_code = excluded.country_code, phone = excluded.phone, \
             metadata = excluded.metadata, updated_at = excluded.updated_at",
        )
        .bind(format!("{cart_id}_{address_type}"))
        .bind(cart_id.as_str())
        .bind(address_type)
        .bind(address.first_name.as_deref())
        .bind(address.last_name.as_deref())
        .bind(address.company.as_deref())
        .bind(address.address_1.as_deref())
        .bind(address.address_2.as_deref())
        .bind(address.city.as_deref())
        .bind(address.province.as_deref())
        .bind(address.postal_code.as_deref())
        .bind(address.country_code.as_deref())
        .bind(address.phone.as_deref())
        .bind(address.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        let update = format!("UPDATE carts SET {column} = ?, updated_at = ? WHERE id = ?");
        sqlx::query(&update)
            .bind(json_string(&address))
            .bind(now)
            .bind(cart_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;

        tx.commit().await.map_err(database_error)?;
        self.get_cart(cart_id).await
    }

    async fn cart_row(&self, cart_id: &CartId) -> Result<CartRow, AppError> {
        sqlx::query_as::<_, CartRow>(CART_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("cart", cart_id))
    }

    async fn ensure_open(&self, cart_id: &CartId) -> Result<CartRow, AppError> {
        let row = self.cart_row(cart_id).await?;
        if !CartStatus::from_db(&row.status).is_open() {
            return Err(AppError::conflict("only open carts can be modified"));
        }
        Ok(row)
    }

    async fn load_items(&self, cart_id: &CartId) -> Result<Vec<CartItem>, AppError> {
        let rows = sqlx::query_as::<_, CartItemRow>(ITEM_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let adjustment_rows = sqlx::query_as::<_, ItemAdjustmentRow>(ITEM_ADJUSTMENT_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut grouped: BTreeMap<String, Vec<CartAdjustment>> = BTreeMap::new();
        for row in adjustment_rows {
            grouped
                .entry(row.cart_item_id)
                .or_default()
                .push(CartAdjustment {
                    promotion_id: row.promotion_id.map(PromotionId::from_string),
                    code: row.code,
                    amount: row.amount,
                    description: row.description,
                });
        }

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let adjustments = grouped.remove(&row.id).unwrap_or_default();
            items.push(CartItem {
                id: LineItemId::from_string(row.id),
                cart_id: CartId::from_string(row.cart_id),
                product_id: ProductId::from_string(row.product_id),
                variant_id: VariantId::from_string(row.variant_id),
                quantity: row.quantity,
                title: row.title,
                variant_title: row.variant_title,
                sku: row.sku,
                thumbnail: row.thumbnail,
                unit_price: row.unit_price,
                currency_code: CurrencyCode::new(row.currency_code)?,
                is_tax_inclusive: row.is_tax_inclusive,
                requires_shipping: row.requires_shipping,
                metadata: Metadata::from_json_str(&row.metadata),
                adjustments,
            });
        }
        Ok(items)
    }

    async fn load_discounts(&self, cart_id: &CartId) -> Result<Vec<CartDiscount>, AppError> {
        let rows = sqlx::query_as::<_, DiscountRow>(DISCOUNT_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(rows
            .into_iter()
            .map(|row| CartDiscount {
                promotion_id: row.promotion_id.map(PromotionId::from_string),
                code: row.code,
                amount: row.amount,
                is_automatic: row.is_automatic,
                metadata: Metadata::from_json_str(&row.metadata),
            })
            .collect())
    }

    async fn load_shipping_methods(
        &self,
        cart_id: &CartId,
    ) -> Result<Vec<CartShippingMethod>, AppError> {
        let rows = sqlx::query_as::<_, ShippingMethodRow>(SHIPPING_METHOD_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut methods = Vec::with_capacity(rows.len());
        for row in rows {
            methods.push(CartShippingMethod {
                shipping_option_id: ShippingOptionId::from_string(row.shipping_option_id),
                name: row.name,
                amount: row.amount,
                currency: CurrencyCode::new(row.currency_code)?,
                is_tax_inclusive: row.is_tax_inclusive,
                data: parse_data(&row.data),
            });
        }
        Ok(methods)
    }

    async fn load_shipping_adjustments(
        &self,
        cart_id: &CartId,
    ) -> Result<Vec<CartAdjustment>, AppError> {
        let rows = sqlx::query_as::<_, ShippingAdjustmentRow>(SHIPPING_ADJUSTMENT_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(rows
            .into_iter()
            .map(|row| CartAdjustment {
                promotion_id: row.promotion_id.map(PromotionId::from_string),
                code: row.code,
                amount: row.amount,
                description: row.description,
            })
            .collect())
    }

    async fn load_tax_lines(&self, cart_id: &CartId) -> Result<Vec<CartTaxLine>, AppError> {
        let rows = sqlx::query_as::<_, TaxLineRow>(TAX_LINE_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(rows
            .into_iter()
            .map(|row| CartTaxLine {
                name: row.name,
                rate_bps: row.rate_bps,
                amount: row.amount,
                is_tax_inclusive: row.is_tax_inclusive,
                cart_item_id: row.cart_item_id.map(LineItemId::from_string),
                shipping_method_id: row.shipping_method_id,
            })
            .collect())
    }

    async fn load_addresses(&self, cart_id: &CartId) -> Result<Addresses, AppError> {
        let rows = sqlx::query_as::<_, AddressRow>(ADDRESS_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut addresses = Addresses::default();
        for row in rows {
            let address = Address {
                first_name: row.first_name,
                last_name: row.last_name,
                company: row.company,
                address_1: row.address_1,
                address_2: row.address_2,
                city: row.city,
                province: row.province,
                postal_code: row.postal_code,
                country_code: row.country_code,
                phone: row.phone,
                metadata: Metadata::from_json_str(&row.metadata),
            };
            match row.address_type.as_str() {
                "shipping" => addresses.shipping = Some(address),
                "billing" => addresses.billing = Some(address),
                _ => {}
            }
        }
        Ok(addresses)
    }

    async fn touch_cart(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
        now: i64,
    ) -> Result<(), AppError> {
        sqlx::query("UPDATE carts SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(cart_id.as_str())
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
        Ok(())
    }
}

#[derive(Default)]
struct Addresses {
    shipping: Option<Address>,
    billing: Option<Address>,
}

#[derive(Debug, FromRow)]
struct CartRow {
    id: String,
    currency_code: String,
    customer_id: Option<String>,
    email: Option<String>,
    sales_channel_id: Option<String>,
    status: String,
    shipping_address: String,
    billing_address: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
    completed_at: Option<i64>,
    expires_at: Option<i64>,
}

#[derive(Debug, FromRow)]
struct CartItemRow {
    id: String,
    cart_id: String,
    product_id: String,
    variant_id: String,
    quantity: i64,
    title: String,
    variant_title: Option<String>,
    sku: Option<String>,
    thumbnail: Option<String>,
    unit_price: i64,
    currency_code: String,
    is_tax_inclusive: bool,
    requires_shipping: bool,
    metadata: String,
}

#[derive(Debug, FromRow)]
struct ItemAdjustmentRow {
    cart_item_id: String,
    promotion_id: Option<String>,
    code: Option<String>,
    amount: i64,
    description: Option<String>,
}

#[derive(Debug, FromRow)]
struct DiscountRow {
    promotion_id: Option<String>,
    code: Option<String>,
    amount: i64,
    is_automatic: bool,
    metadata: String,
}

#[derive(Debug, FromRow)]
struct ShippingMethodRow {
    shipping_option_id: String,
    name: String,
    amount: i64,
    currency_code: String,
    is_tax_inclusive: bool,
    data: String,
}

#[derive(Debug, FromRow)]
struct ShippingAdjustmentRow {
    promotion_id: Option<String>,
    code: Option<String>,
    amount: i64,
    description: Option<String>,
}

#[derive(Debug, FromRow)]
struct TaxLineRow {
    name: String,
    rate_bps: i64,
    amount: i64,
    is_tax_inclusive: bool,
    cart_item_id: Option<String>,
    shipping_method_id: Option<String>,
}

#[derive(Debug, FromRow)]
struct AddressRow {
    #[sqlx(rename = "type")]
    address_type: String,
    first_name: Option<String>,
    last_name: Option<String>,
    company: Option<String>,
    address_1: Option<String>,
    address_2: Option<String>,
    city: Option<String>,
    province: Option<String>,
    postal_code: Option<String>,
    country_code: Option<String>,
    phone: Option<String>,
    metadata: String,
}

async fn ensure_item_belongs_to_cart(
    tx: &mut Transaction<'_, Sqlite>,
    cart_id: &CartId,
    item_id: &LineItemId,
) -> Result<(), AppError> {
    let found: Option<String> =
        sqlx::query_scalar("SELECT id FROM cart_items WHERE id = ? AND cart_id = ?")
            .bind(item_id.as_str())
            .bind(cart_id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
    if found.is_none() {
        return Err(AppError::not_found("cart item", item_id));
    }
    Ok(())
}

async fn insert_adjustment(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
    parent_column: &str,
    parent_id: &str,
    input: &AdjustmentInput,
    created_at: i64,
) -> Result<(), AppError> {
    let sql = format!(
        "INSERT INTO {table} (id, {parent_column}, promotion_id, code, amount, description, \
         created_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
    );
    sqlx::query(&sql)
        .bind(row_id("adj"))
        .bind(parent_id)
        .bind(input.promotion_id.as_ref().map(PromotionId::as_str))
        .bind(input.code.as_deref())
        .bind(input.amount)
        .bind(input.description.as_deref())
        .bind(created_at)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    Ok(())
}

fn validate_discount_input(input: &DiscountInput) -> Result<(), AppError> {
    validate_non_negative_amount(input.amount, "discount amount")?;
    if let Some(code) = &input.code {
        validate_length(code, "code", 120)?;
    }
    validate_metadata(&input.metadata)
}

fn validate_adjustment_input(input: &AdjustmentInput) -> Result<(), AppError> {
    validate_non_negative_amount(input.amount, "adjustment amount")?;
    if let Some(code) = &input.code {
        validate_length(code, "code", 120)?;
    }
    if let Some(description) = &input.description {
        validate_length(description, "description", 500)?;
    }
    Ok(())
}

fn validate_tax_line_input(input: &TaxLineInput) -> Result<(), AppError> {
    validate_length(&input.name, "name", 160)?;
    if input.rate_bps < 0 {
        return Err(AppError::validation("rate_bps must not be negative"));
    }
    validate_non_negative_amount(input.amount, "tax amount")
}

fn json_string<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned())
}

fn parse_address(raw: &str) -> Address {
    serde_json::from_str(raw).unwrap_or_default()
}

fn parse_data(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

/// Generates an opaque surrogate key for a child row.
///
/// The kernel identifier generator supplies the unique, time-ordered suffix;
/// the `cart_` prefix it normally carries is replaced with a table hint.
fn row_id(prefix: &str) -> String {
    let generated = CartId::new();
    let raw = generated.as_str();
    let suffix = raw.strip_prefix("cart_").unwrap_or(raw);
    format!("{prefix}_{suffix}")
}

#[cfg(test)]
mod tests {
    use fastcart_infrastructure::StoreDatabase;
    use fastcart_kernel::{
        CurrencyCode, CustomerId, LineItemId, Metadata, Pagination, ProductId, PromotionId,
        ShippingOptionId, StoreId, VariantId,
    };

    use super::CartService;
    use crate::domain::{
        AddItemInput, AddressInput, AdjustmentInput, Cart, CartStatus, CreateCartInput,
        DiscountInput, SetShippingMethodInput, TaxLineInput, UpdateItemInput,
    };

    async fn service() -> CartService {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        CartService::new(db)
    }

    fn create_input(currency: &str) -> CreateCartInput {
        CreateCartInput::new(currency)
    }

    async fn create_cart(service: &CartService, currency: &str) -> Cart {
        service.create_cart(create_input(currency)).await.unwrap()
    }

    fn variant() -> VariantId {
        VariantId::new()
    }

    fn item_input(
        currency: &str,
        variant_id: VariantId,
        unit_price: i64,
        quantity: i64,
    ) -> AddItemInput {
        AddItemInput {
            product_id: ProductId::new(),
            variant_id,
            title: "Test product".to_owned(),
            variant_title: None,
            sku: Some("SKU-1".to_owned()),
            thumbnail: None,
            unit_price,
            currency_code: currency.to_owned(),
            quantity,
            is_tax_inclusive: false,
            requires_shipping: true,
            metadata: Metadata::empty(),
        }
    }

    fn shipping_input(currency: &str, amount: i64) -> SetShippingMethodInput {
        SetShippingMethodInput {
            shipping_option_id: ShippingOptionId::new(),
            name: "Standard".to_owned(),
            amount,
            currency_code: currency.to_owned(),
            is_tax_inclusive: false,
            data: serde_json::json!({ "provider": "test" }),
        }
    }

    fn adjustment(amount: i64) -> AdjustmentInput {
        AdjustmentInput {
            promotion_id: Some(PromotionId::new()),
            code: Some("PROMO".to_owned()),
            amount,
            description: Some("Line promotion".to_owned()),
        }
    }

    fn tax_line(amount: i64, inclusive: bool) -> TaxLineInput {
        TaxLineInput {
            name: "Sales tax".to_owned(),
            rate_bps: 1_000,
            amount,
            is_tax_inclusive: inclusive,
            cart_item_id: None,
            shipping_method_id: None,
        }
    }

    #[tokio::test]
    async fn creates_and_gets_a_cart() {
        let service = service().await;
        let cart = create_cart(&service, "usd").await;

        assert_eq!(cart.currency_code.as_str(), "USD");
        assert_eq!(cart.status, CartStatus::Open);
        assert!(cart.items.is_empty());
        assert_eq!(cart.totals.total, 0);
        assert!(cart.expires_at.is_some());

        let loaded = service.get_cart(&cart.id).await.unwrap();
        assert_eq!(loaded.id, cart.id);
    }

    #[tokio::test]
    async fn adding_the_same_variant_increments_quantity() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let variant = variant();

        let cart = service
            .add_item(&cart.id, item_input("USD", variant.clone(), 1_000, 2))
            .await
            .unwrap();
        assert_eq!(cart.items.len(), 1);
        assert_eq!(cart.items[0].quantity, 2);

        let cart = service
            .add_item(&cart.id, item_input("USD", variant, 1_000, 3))
            .await
            .unwrap();
        assert_eq!(cart.items.len(), 1);
        assert_eq!(cart.items[0].quantity, 5);
        assert_eq!(cart.totals.item_subtotal, 5_000);
    }

    #[tokio::test]
    async fn updates_removes_and_clears_items() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 500, 1))
            .await
            .unwrap();
        let item_id = cart.items[0].id.clone();

        let cart = service
            .update_item_quantity(&cart.id, &item_id, UpdateItemInput { quantity: 4 })
            .await
            .unwrap();
        assert_eq!(cart.items[0].quantity, 4);
        assert_eq!(cart.totals.item_subtotal, 2_000);

        let cart = service
            .update_item_quantity(&cart.id, &item_id, UpdateItemInput { quantity: 0 })
            .await
            .unwrap();
        assert!(cart.items.is_empty());

        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 100, 1))
            .await
            .unwrap();
        assert_eq!(cart.items.len(), 1);
        let cart = service.clear_items(&cart.id).await.unwrap();
        assert!(cart.items.is_empty());
        assert_eq!(cart.totals.total, 0);
    }

    #[tokio::test]
    async fn rejects_a_mismatched_item_currency() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;

        let error = service
            .add_item(&cart.id, item_input("EUR", variant(), 1_000, 1))
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn validates_quantities_and_unknown_items() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;

        let error = service
            .add_item(&cart.id, item_input("USD", variant(), 100, 0))
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));

        let missing = LineItemId::new();
        let error = service.remove_item(&cart.id, &missing).await.unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn calculates_totals_from_stored_snapshots() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 1_000, 2))
            .await
            .unwrap();
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 500, 1))
            .await
            .unwrap();

        assert_eq!(cart.totals.item_subtotal, 2_500);
        assert_eq!(cart.totals.subtotal, 2_500);
        assert_eq!(cart.totals.discount_total, 0);
        assert_eq!(cart.totals.total, 2_500);
    }

    #[tokio::test]
    async fn caps_line_adjustments_at_the_line_subtotal() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
            .await
            .unwrap();
        let item_id = cart.items[0].id.clone();

        let cart = service
            .add_item_adjustment(&cart.id, &item_id, adjustment(1_500))
            .await
            .unwrap();
        assert_eq!(cart.totals.item_discount_total, 1_000);
        assert_eq!(cart.totals.discount_total, 1_000);
        assert_eq!(cart.totals.total, 0);
    }

    #[tokio::test]
    async fn caps_cart_discounts_at_the_item_subtotal() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
            .await
            .unwrap();

        let cart = service
            .add_discount(
                &cart.id,
                DiscountInput {
                    promotion_id: Some(PromotionId::new()),
                    code: Some("SAVE".to_owned()),
                    amount: 1_500,
                    is_automatic: false,
                    metadata: Metadata::empty(),
                },
            )
            .await
            .unwrap();

        assert_eq!(cart.discounts.len(), 1);
        assert_eq!(cart.totals.discount_total, 1_000);
        assert_eq!(cart.totals.total, 0);
    }

    #[tokio::test]
    async fn setting_a_shipping_method_replaces_the_previous_selection() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
            .await
            .unwrap();

        let cart = service
            .set_shipping_method(&cart.id, shipping_input("USD", 500))
            .await
            .unwrap();
        assert_eq!(cart.shipping_methods.len(), 1);
        assert_eq!(cart.totals.shipping_subtotal, 500);
        assert_eq!(cart.totals.shipping_total, 500);
        assert_eq!(cart.totals.total, 1_500);

        let cart = service
            .set_shipping_method(&cart.id, shipping_input("USD", 800))
            .await
            .unwrap();
        assert_eq!(cart.shipping_methods.len(), 1);
        assert_eq!(cart.shipping_methods[0].amount, 800);
        assert_eq!(cart.totals.total, 1_800);
    }

    #[tokio::test]
    async fn shipping_adjustments_reduce_the_shipping_total() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
            .await
            .unwrap();
        let cart = service
            .set_shipping_method(&cart.id, shipping_input("USD", 500))
            .await
            .unwrap();

        let cart = service
            .replace_shipping_adjustments(&cart.id, vec![adjustment(200)])
            .await
            .unwrap();
        assert_eq!(cart.shipping_adjustments.len(), 1);
        assert_eq!(cart.totals.shipping_discount_total, 200);
        assert_eq!(cart.totals.shipping_total, 300);
        assert_eq!(cart.totals.total, 1_300);
    }

    #[tokio::test]
    async fn inclusive_tax_is_informational_and_not_added_to_the_total() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
            .await
            .unwrap();
        let cart = service
            .replace_tax_lines(&cart.id, vec![tax_line(100, true)])
            .await
            .unwrap();

        assert_eq!(cart.tax_lines.len(), 1);
        assert_eq!(cart.totals.tax_total, 100);
        assert_eq!(cart.totals.total, 1_000);
    }

    #[tokio::test]
    async fn exclusive_tax_is_added_to_the_total() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
            .await
            .unwrap();
        let cart = service
            .replace_tax_lines(&cart.id, vec![tax_line(100, false), tax_line(50, true)])
            .await
            .unwrap();

        assert_eq!(cart.totals.tax_total, 150);
        assert_eq!(cart.totals.total, 1_100);
    }

    #[tokio::test]
    async fn stores_shipping_and_billing_addresses_separately() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;

        let shipping = AddressInput {
            first_name: Some("Ada".to_owned()),
            city: Some("London".to_owned()),
            country_code: Some("gb".to_owned()),
            ..AddressInput::default()
        };
        let billing = AddressInput {
            first_name: Some("Grace".to_owned()),
            city: Some("Arlington".to_owned()),
            country_code: Some("us".to_owned()),
            ..AddressInput::default()
        };

        let cart = service
            .set_shipping_address(&cart.id, shipping)
            .await
            .unwrap();
        let cart = service
            .set_billing_address(&cart.id, billing)
            .await
            .unwrap();

        assert_eq!(cart.shipping_address.first_name.as_deref(), Some("Ada"));
        assert_eq!(cart.shipping_address.country_code.as_deref(), Some("GB"));
        assert_eq!(cart.billing_address.first_name.as_deref(), Some("Grace"));
        assert_eq!(cart.billing_address.country_code.as_deref(), Some("US"));
    }

    #[tokio::test]
    async fn attaching_a_different_customer_conflicts() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let first = CustomerId::new();
        let second = CustomerId::new();

        let cart = service
            .attach_customer(&cart.id, &first, Some("One@Example.com".to_owned()))
            .await
            .unwrap();
        assert_eq!(cart.customer_id.as_ref(), Some(&first));
        assert_eq!(cart.email.as_deref(), Some("one@example.com"));

        let error = service
            .attach_customer(&cart.id, &second, None)
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn completes_and_expires_carts_and_rejects_mutations() {
        let service = service().await;
        let open = create_cart(&service, "USD").await;
        let completed = service.complete_cart(&open.id).await.unwrap();
        assert_eq!(completed.status, CartStatus::Completed);
        assert!(completed.completed_at.is_some());

        let error = service
            .add_item(&open.id, item_input("USD", variant(), 100, 1))
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

        let expiring = service
            .create_cart(CreateCartInput {
                expires_in_seconds: Some(10),
                ..create_input("USD")
            })
            .await
            .unwrap();
        let count = service.expire_due(i64::MAX).await.unwrap();
        assert_eq!(count, 1);

        let expired = service.get_cart(&expiring.id).await.unwrap();
        assert_eq!(expired.status, CartStatus::Expired);
        let error = service
            .add_item(&expiring.id, item_input("USD", variant(), 100, 1))
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn lists_carts_for_a_customer() {
        let service = service().await;
        let customer = CustomerId::new();
        let other = CustomerId::new();

        let first = service
            .create_cart(CreateCartInput {
                customer_id: Some(customer.clone()),
                ..create_input("USD")
            })
            .await
            .unwrap();
        service
            .add_item(&first.id, item_input("USD", variant(), 250, 2))
            .await
            .unwrap();
        service
            .create_cart(CreateCartInput {
                customer_id: Some(customer.clone()),
                ..create_input("EUR")
            })
            .await
            .unwrap();
        service
            .create_cart(CreateCartInput {
                customer_id: Some(other),
                ..create_input("USD")
            })
            .await
            .unwrap();

        let page = service
            .list_carts(Some(&customer), Pagination::default())
            .await
            .unwrap();
        assert_eq!(page.count, 2);
        assert_eq!(page.data.len(), 2);
        let with_items = page
            .data
            .iter()
            .find(|cart| !cart.items.is_empty())
            .unwrap();
        assert_eq!(with_items.totals.item_subtotal, 500);
    }

    #[tokio::test]
    async fn reprices_items_and_replaces_adjustments() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
            .await
            .unwrap();
        let item_id = cart.items[0].id.clone();

        let cart = service
            .set_item_unit_price(&cart.id, &item_id, 750, "usd")
            .await
            .unwrap();
        assert_eq!(cart.items[0].unit_price, 750);

        let cart = service
            .replace_item_adjustments(&cart.id, &item_id, vec![adjustment(100)])
            .await
            .unwrap();
        assert_eq!(cart.items[0].adjustments.len(), 1);
        assert_eq!(cart.totals.item_discount_total, 100);

        let cart = service
            .replace_item_adjustments(&cart.id, &item_id, vec![adjustment(50)])
            .await
            .unwrap();
        assert_eq!(cart.items[0].adjustments.len(), 1);
        assert_eq!(cart.totals.item_discount_total, 50);

        let error = service
            .set_item_unit_price(&cart.id, &item_id, 100, "EUR")
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn replaces_and_removes_discounts() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
            .await
            .unwrap();
        let promotion = PromotionId::new();

        let cart = service
            .replace_discounts(
                &cart.id,
                vec![DiscountInput {
                    promotion_id: Some(promotion.clone()),
                    code: Some("SAVE10".to_owned()),
                    amount: 100,
                    is_automatic: false,
                    metadata: Metadata::empty(),
                }],
            )
            .await
            .unwrap();
        assert_eq!(cart.discounts.len(), 1);
        assert_eq!(cart.totals.total, 900);

        let cart = service.remove_discount(&cart.id, &promotion).await.unwrap();
        assert!(cart.discounts.is_empty());
        assert_eq!(cart.totals.total, 1_000);

        let error = service
            .remove_discount(&cart.id, &promotion)
            .await
            .unwrap_err();
        assert!(matches!(error, fastcart_kernel::AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn recalculate_returns_current_totals() {
        let service = service().await;
        let cart = create_cart(&service, "USD").await;
        let cart = service
            .add_item(&cart.id, item_input("USD", variant(), 300, 3))
            .await
            .unwrap();

        let totals = service.recalculate(&cart.id).await.unwrap();
        assert_eq!(totals.item_subtotal, 900);
        assert_eq!(totals.currency_code, CurrencyCode::new("USD").unwrap());

        let missing = fastcart_kernel::CartId::new();
        assert!(service.recalculate(&missing).await.is_err());
    }
}
