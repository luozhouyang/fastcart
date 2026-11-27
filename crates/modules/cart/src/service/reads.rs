//! Transaction-scoped reads shared by the cart mutations.

use super::*;

impl CartService {
    pub(super) async fn cart_row(&self, cart_id: &CartId) -> Result<CartRow, AppError> {
        sqlx::query_as::<_, CartRow>(CART_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("cart", cart_id))
    }
}

impl CartService {
    pub(super) async fn cart_row_tx(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<CartRow, AppError> {
        sqlx::query_as::<_, CartRow>(CART_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("cart", cart_id))
    }
}

impl CartService {
    pub(super) async fn ensure_open(&self, cart_id: &CartId) -> Result<CartRow, AppError> {
        let row = self.cart_row(cart_id).await?;
        Self::check_open(&row)?;
        Ok(row)
    }
}

impl CartService {
    pub(super) async fn ensure_open_tx(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<CartRow, AppError> {
        let row = Self::cart_row_tx(tx, cart_id).await?;
        Self::check_open(&row)?;
        Ok(row)
    }
}

impl CartService {
    pub(super) fn check_open(row: &CartRow) -> Result<(), AppError> {
        if !CartStatus::from_db(&row.status).is_open()
            || row
                .expires_at
                .is_some_and(|expires_at| expires_at <= now_timestamp())
        {
            return Err(AppError::conflict("only open carts can be modified"));
        }
        Ok(())
    }
}

impl CartService {
    pub(super) async fn load_items(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<Vec<CartItem>, AppError> {
        let rows = sqlx::query_as::<_, CartItemRow>(ITEM_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(&mut **tx)
            .await
            .map_err(database_error)?;
        let adjustment_rows = sqlx::query_as::<_, ItemAdjustmentRow>(ITEM_ADJUSTMENT_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(&mut **tx)
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
}

impl CartService {
    pub(super) async fn load_discounts(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<Vec<CartDiscount>, AppError> {
        let rows = sqlx::query_as::<_, DiscountRow>(DISCOUNT_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(&mut **tx)
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
}

impl CartService {
    /// Loads the applied gift cards in application order.
    ///
    /// `applied_amount` is left at zero here: it is derived from the card
    /// amounts, balances and the cart's payable total by
    /// [`recalculate_totals`](crate::recalculate_totals), which runs at the end
    /// of every aggregate load.
    pub(super) async fn load_gift_cards(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<Vec<CartGiftCard>, AppError> {
        let rows = sqlx::query_as::<_, GiftCardRow>(GIFT_CARD_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(&mut **tx)
            .await
            .map_err(database_error)?;
        Ok(rows
            .into_iter()
            .map(|row| CartGiftCard {
                gift_card_id: row.gift_card_id,
                masked_code: row.masked_code,
                amount: row.amount_minor,
                balance: row.balance_minor,
                applied_amount: 0,
            })
            .collect())
    }
}

impl CartService {
    pub(super) async fn load_shipping_methods(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<Vec<CartShippingMethod>, AppError> {
        let rows = sqlx::query_as::<_, ShippingMethodRow>(SHIPPING_METHOD_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(&mut **tx)
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
                data: parse_data(&row.data)?,
            });
        }
        Ok(methods)
    }
}

impl CartService {
    pub(super) async fn load_shipping_adjustments(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<Vec<CartAdjustment>, AppError> {
        let rows = sqlx::query_as::<_, ShippingAdjustmentRow>(SHIPPING_ADJUSTMENT_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(&mut **tx)
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
}

impl CartService {
    pub(super) async fn load_tax_lines(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<Vec<CartTaxLine>, AppError> {
        let rows = sqlx::query_as::<_, TaxLineRow>(TAX_LINE_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(&mut **tx)
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
}

impl CartService {
    pub(super) async fn load_addresses(
        tx: &mut Transaction<'_, Sqlite>,
        cart_id: &CartId,
    ) -> Result<Addresses, AppError> {
        let rows = sqlx::query_as::<_, AddressRow>(ADDRESS_COLUMNS)
            .bind(cart_id.as_str())
            .fetch_all(&mut **tx)
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
}

impl CartService {
    pub(super) async fn touch_cart(
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
