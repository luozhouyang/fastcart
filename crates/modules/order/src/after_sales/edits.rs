use super::*;

/// A proposed order edit with recalculated totals.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderEdit {
    /// Edit identifier.
    pub id: OrderEditId,
    /// Owning order.
    pub order_id: OrderId,
    /// Edit lifecycle state.
    pub status: OrderEditStatus,
    /// Optional reason for the edit.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Order subtotal before the edit.
    pub before_subtotal: i64,
    /// Order subtotal after the edit.
    pub after_subtotal: i64,
    /// Order total when the edit was prepared. It may be stale if another
    /// change landed before confirmation; `difference_total` records the change
    /// the confirmation actually made.
    pub before_total: i64,
    /// Order total the edit is expected to produce, rewritten to the actual
    /// total when it is confirmed.
    pub after_total: i64,
    /// Price difference the confirmed edit settled on the order: the order
    /// total after confirmation minus the total that was current when the
    /// confirmation transaction began. A positive difference is money the
    /// customer owes, a negative one is money owed to them. `None` until the
    /// edit is confirmed.
    pub difference_total: Option<i64>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Confirmation timestamp in Unix seconds.
    pub confirmed_at: Option<Timestamp>,
    /// Cancellation timestamp in Unix seconds.
    pub canceled_at: Option<Timestamp>,
    /// Edit lines.
    pub items: Vec<OrderEditItem>,
}

/// One line of an order edit.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderEditItem {
    /// Line identifier.
    pub id: EventId,
    /// Owning edit.
    pub edit_id: OrderEditId,
    /// Historical order line for remove/update lines.
    pub order_item_id: Option<LineItemId>,
    /// Kind of change.
    pub action: OrderEditAction,
    /// Product identifier snapshot.
    pub product_id: Option<ProductId>,
    /// Variant identifier snapshot.
    pub variant_id: Option<VariantId>,
    /// Product title snapshot.
    pub product_title: String,
    /// Variant title snapshot.
    pub variant_title: Option<String>,
    /// SKU snapshot.
    pub sku: Option<String>,
    /// Unit price in minor units.
    pub unit_price: i64,
    /// Quantity before the edit.
    pub before_quantity: i64,
    /// Quantity after the edit.
    pub after_quantity: i64,
    /// Whether the line takes stock, snapshotted when the edit was prepared.
    pub requires_shipping: bool,
    /// Line state.
    pub status: OrderEditItemStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for one order edit line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderEditItemInput {
    /// Historical order line for remove/update lines.
    pub order_item_id: Option<LineItemId>,
    /// Kind of change.
    pub action: OrderEditAction,
    /// Product identifier snapshot for added lines.
    pub product_id: Option<ProductId>,
    /// Variant identifier snapshot for added lines.
    pub variant_id: Option<VariantId>,
    /// Product title snapshot for added lines.
    pub product_title: Option<String>,
    /// Variant title snapshot for added lines.
    pub variant_title: Option<String>,
    /// SKU snapshot for added lines.
    pub sku: Option<String>,
    /// Unit price for added lines; ignored for existing lines.
    pub unit_price: Option<i64>,
    /// Expected current quantity for remove/update lines.
    pub before_quantity: Option<i64>,
    /// Quantity after the edit.
    pub after_quantity: i64,
    /// Whether an added line takes stock. Ignored for existing lines, which
    /// inherit the value from the order line they rewrite.
    pub requires_shipping: Option<bool>,
}

/// Input for [`crate::OrderService::create_order_edit`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateOrderEditInput {
    /// Optional reason for the edit.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Edit lines.
    pub items: Vec<OrderEditItemInput>,
}
