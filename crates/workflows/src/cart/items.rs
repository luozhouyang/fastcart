//! Cart line, shipping and coupon mutations.

use super::*;

impl CartWorkflow {
    /// Adds a variant at the authoritative price and recalculates the cart.
    pub async fn add_item(
        &self,
        cart_id: &CartId,
        variant_id: &VariantId,
        quantity: i64,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;
        let variant = self.catalog.get_variant(variant_id.as_str()).await?;
        let product = self.catalog.get_product(&variant.product_id).await?;
        let price = self
            .pricing
            .calculate_price(
                variant_id.as_str(),
                &PriceContext {
                    currency: cart.currency_code.clone(),
                    quantity,
                    customer_group_id: context.customer_group_ids.first().cloned(),
                    sales_channel_id: cart
                        .sales_channel_id
                        .clone()
                        .or_else(|| context.sales_channel_id.clone()),
                    at: context.at,
                },
            )
            .await?
            .ok_or_else(|| {
                AppError::conflict(format!(
                    "no price is configured for variant {} in {}",
                    variant_id, cart.currency_code
                ))
            })?;

        self.cart
            .add_item(
                cart_id,
                AddItemInput {
                    product_id: fastcart_kernel::ProductId::from_string(product.id.clone()),
                    variant_id: variant_id.clone(),
                    title: product.title.clone(),
                    variant_title: Some(variant.title.clone()),
                    sku: variant.sku.clone(),
                    thumbnail: product.thumbnail.clone(),
                    unit_price: price.amount,
                    currency_code: price.currency.to_string(),
                    quantity,
                    is_tax_inclusive: context.prices_include_tax,
                    requires_shipping: true,
                    metadata: Metadata::empty(),
                },
            )
            .await?;
        self.recalculate(cart_id, context).await
    }
}

impl CartWorkflow {
    /// Selects a shipping option at the resolved rate and recalculates.
    pub async fn set_shipping_method(
        &self,
        cart_id: &CartId,
        option_id: &ShippingOptionId,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;
        let option = self.shipping.get_option(option_id).await?;
        let rate = self
            .shipping
            .select_rate(
                option_id,
                &context.country_code,
                context.province_code.as_deref(),
                cart.totals.item_subtotal,
            )
            .await?
            .ok_or_else(|| {
                AppError::conflict(format!(
                    "shipping option {} is not available for this destination",
                    option.id
                ))
            })?;
        if rate.currency != cart.currency_code {
            return Err(AppError::conflict(
                "shipping option currency does not match the cart currency",
            ));
        }
        self.cart
            .set_shipping_method(
                cart_id,
                SetShippingMethodInput {
                    shipping_option_id: option.id.clone(),
                    name: option.name.clone(),
                    amount: rate.amount_minor,
                    currency_code: rate.currency.to_string(),
                    is_tax_inclusive: context.prices_include_tax,
                    data: serde_json::json!({
                        "rate_id": rate.rate_id,
                        "provider_id": option.provider_id,
                        "provider_data": option.provider_data,
                    }),
                },
            )
            .await?;
        self.recalculate(cart_id, context).await
    }
}

impl CartWorkflow {
    /// Applies a coupon code and recalculates discounts.
    ///
    /// A code already on the cart from an automatic promotion is neither a
    /// repeat nor a second discount: the promotion writes its own code onto the
    /// cart-level row it owns, so entering the code recalculates and returns the
    /// cart as it already is. A code the customer applied themselves keeps the
    /// conflict, because that is a genuine repeat.
    pub async fn apply_coupon(
        &self,
        cart_id: &CartId,
        code: &str,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;
        let coupon = self.promotion.validate_coupon(code, context.at).await?;
        if let Some(existing) = cart
            .discounts
            .iter()
            .find(|discount| discount.code.as_deref() == Some(coupon.code.as_str()))
        {
            if existing.is_automatic {
                return self.recalculate(cart_id, context).await;
            }
            return Err(AppError::conflict("coupon is already applied"));
        }
        self.promotion.get_promotion(&coupon.promotion_id).await?;
        self.cart
            .add_discount(
                cart_id,
                DiscountInput {
                    promotion_id: Some(PromotionId::from_string(coupon.promotion_id.clone())),
                    code: Some(coupon.code.clone()),
                    amount: 0,
                    is_automatic: false,
                    metadata: Metadata::empty(),
                },
            )
            .await?;
        self.recalculate(cart_id, context).await
    }
}

impl CartWorkflow {
    /// Removes a coupon code and recalculates discounts.
    ///
    /// Only a code the customer applied is a coupon to remove. A code carried by
    /// an automatic promotion is refused instead of deleted: the row belongs to
    /// the promotion, the very next recalculation re-derives it, and a
    /// successful response would claim a change that cannot hold. Retiring the
    /// promotion, not clearing the cart row, is what takes such a discount away.
    pub async fn remove_coupon(
        &self,
        cart_id: &CartId,
        code: &str,
        context: &CartContext,
    ) -> Result<Cart, AppError> {
        let cart = self.cart.get_cart(cart_id).await?;
        let normalized = code.trim().to_ascii_uppercase();
        let discount = cart
            .discounts
            .iter()
            .find(|discount| discount.code.as_deref() == Some(normalized.as_str()))
            .ok_or_else(|| AppError::not_found("coupon", &normalized))?;
        if discount.is_automatic {
            return Err(AppError::conflict(format!(
                "coupon {normalized} is applied automatically by a promotion and cannot be removed"
            )));
        }
        let promotion_id = discount
            .promotion_id
            .clone()
            .ok_or_else(|| AppError::not_found("coupon promotion", &normalized))?;
        self.cart.remove_discount(cart_id, &promotion_id).await?;
        self.recalculate(cart_id, context).await
    }
}
