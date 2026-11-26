use super::*;

/// A return request for one order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderReturn {
    /// Return identifier.
    pub id: ReturnId,
    /// Owning order.
    pub order_id: OrderId,
    /// Return lifecycle state.
    pub status: ReturnStatus,
    /// Optional reason for the return.
    pub reason: Option<String>,
    /// Refund created for this return, when linked.
    pub refund_id: Option<RefundId>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Submission timestamp in Unix seconds.
    pub requested_at: Option<Timestamp>,
    /// Timestamp at which every requested item was received.
    pub received_at: Option<Timestamp>,
    /// Timestamp at which the return was written off.
    pub written_off_at: Option<Timestamp>,
    /// Cancellation timestamp in Unix seconds.
    pub canceled_at: Option<Timestamp>,
    /// Returned lines.
    pub items: Vec<OrderReturnItem>,
}

/// One line of a return request.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderReturnItem {
    /// Line identifier.
    pub id: EventId,
    /// Owning return.
    pub return_id: ReturnId,
    /// Historical order line being returned.
    pub order_item_id: LineItemId,
    /// Requested return quantity.
    pub quantity: i64,
    /// Quantity received so far.
    pub received_quantity: i64,
    /// Quantity written off so far.
    pub written_off_quantity: i64,
    /// Optional line-level reason.
    pub reason: Option<String>,
    /// Line state.
    pub status: ReturnItemStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for one returned line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReturnItemInput {
    /// Historical order line being returned.
    pub order_item_id: LineItemId,
    /// Requested return quantity.
    pub quantity: i64,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Input for [`crate::OrderService::create_return`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateReturnInput {
    /// Optional reason for the return.
    ///
    /// Used when `reason_id` is absent; the free-text reason remains supported
    /// for callers that do not maintain configured reasons.
    pub reason: Option<String>,
    /// Configured return reason to snapshot into
    /// [`crate::OrderReturn::reason`], when supplied.
    ///
    /// The service validates that the reason exists and stores its label as free
    /// text. It takes precedence over `reason`.
    #[serde(default)]
    pub reason_id: Option<ReturnReasonId>,
    /// Refund created for this return, when linked.
    pub refund_id: Option<RefundId>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Returned lines.
    pub items: Vec<ReturnItemInput>,
}

/// Input for one received return line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReceiveReturnItemInput {
    /// Return line's historical order line.
    pub order_item_id: LineItemId,
    /// Incremental quantity received.
    pub quantity: i64,
}

/// Input for [`crate::OrderService::receive_return`].
///
/// An empty `items` list receives every outstanding quantity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReceiveReturnInput {
    /// Lines to receive.
    pub items: Vec<ReceiveReturnItemInput>,
}

/// Input for one written-off return line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WriteOffReturnItemInput {
    /// Return line's historical order line.
    pub order_item_id: LineItemId,
    /// Incremental quantity to write off.
    pub quantity: i64,
}

/// Input for [`crate::OrderService::write_off_return`].
///
/// An empty `items` list writes off every received quantity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WriteOffReturnInput {
    /// Lines to write off.
    pub items: Vec<WriteOffReturnItemInput>,
}
