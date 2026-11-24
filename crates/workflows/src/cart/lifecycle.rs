//! Cart creation, lookup and completion.

use super::*;

impl CartWorkflow {
    /// Creates an empty cart.
    pub async fn create_cart(&self, input: CreateCartInput) -> Result<Cart, AppError> {
        self.cart.create_cart(input).await
    }
}

impl CartWorkflow {
    /// Reads a cart with its stored totals.
    pub async fn get_cart(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        self.cart.get_cart(cart_id).await
    }
}

impl CartWorkflow {
    /// Completes a cart after an order was placed.
    pub async fn complete_cart(&self, cart_id: &CartId) -> Result<Cart, AppError> {
        self.cart.complete_cart(cart_id).await
    }
}
