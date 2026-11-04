//! Rendering context and amount formatting.

use fastcart_kernel::{CurrencyCode, DomainEventEnvelope};
use fastcart_module_order::{Order, OrderFulfillment};
use serde::Serialize;
use serde_json::Value;

/// One line item exposed to templates.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TemplateItem {
    /// Product title.
    pub title: String,
    /// Variant title, when known.
    pub variant_title: Option<String>,
    /// Purchased quantity.
    pub quantity: i64,
    /// Formatted line total.
    pub total: String,
}

/// Values available to every notification template.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TemplateContext {
    /// Outbox event type.
    pub event_type: String,
    /// Outbox event identifier.
    pub event_id: String,
    /// Event creation timestamp.
    pub occurred_at: i64,
    /// Related order identifier.
    pub order_id: Option<String>,
    /// Customer-facing sequential order number.
    pub display_number: Option<i64>,
    /// Order currency.
    pub currency_code: Option<String>,
    /// Formatted order total.
    pub total: Option<String>,
    /// Formatted event amount, when the payload carries one.
    pub amount: Option<String>,
    /// Formatted refund amount for refund events.
    pub refund_amount: Option<String>,
    /// Shipment tracking numbers.
    pub tracking_numbers: Vec<String>,
    /// Purchased or fulfilled line items.
    pub items: Vec<TemplateItem>,
    /// Current order lifecycle status.
    pub order_status: Option<String>,
    /// Current order payment status.
    pub payment_status: Option<String>,
    /// Current order fulfillment status.
    pub fulfillment_status: Option<String>,
    /// Current order refund status.
    pub refund_status: Option<String>,
    /// Fulfillment or payment provider identifier.
    pub provider_id: Option<String>,
    /// After-sales or event-specific status.
    pub status: Option<String>,
    /// Human-readable event reason, when present.
    pub reason: Option<String>,
}

impl TemplateContext {
    /// Builds a context from an outbox event and the order it references.
    #[must_use]
    pub fn from_event(event: &DomainEventEnvelope, order: Option<&Order>) -> Self {
        let payload = &event.data;
        let order_id = payload
            .get("order_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| order.map(|order| order.id.as_str().to_owned()));
        let currency_code = order
            .map(|order| order.currency_code.clone())
            .or_else(|| string_field(payload, &["currency_code", "currency"]));
        let amount = payload
            .get("amount")
            .and_then(Value::as_i64)
            .map(|amount| format_amount(amount, currency_code.as_deref()));
        let refund_amount = payload
            .get("amount")
            .and_then(Value::as_i64)
            .or_else(|| payload.get("refunded_amount").and_then(Value::as_i64))
            .map(|amount| format_amount(amount, currency_code.as_deref()));
        let items = order.map_or_else(Vec::new, |order| {
            order
                .items
                .iter()
                .map(|item| TemplateItem {
                    title: item.product_title.clone(),
                    variant_title: item.variant_title.clone(),
                    quantity: item.quantity,
                    total: format_amount(item.total, Some(&order.currency_code)),
                })
                .collect()
        });

        Self {
            event_type: event.event_type.clone(),
            event_id: event.id.as_str().to_owned(),
            occurred_at: event.occurred_at,
            order_id,
            display_number: order
                .map(|order| order.display_number)
                .or_else(|| payload.get("display_number").and_then(Value::as_i64)),
            currency_code,
            total: order.map(|order| format_amount(order.total, Some(&order.currency_code))),
            amount,
            refund_amount,
            tracking_numbers: tracking_numbers(payload, order),
            items,
            order_status: order.map(|order| order.status.as_str().to_owned()),
            payment_status: order.map(|order| order.payment_status.as_str().to_owned()),
            fulfillment_status: order
                .map(|order| order.fulfillment_status.as_str().to_owned())
                .or_else(|| string_field(payload, &["fulfillment_status"])),
            refund_status: order.map(|order| order.refund_status.as_str().to_owned()),
            provider_id: string_field(payload, &["provider_id", "provider"]),
            status: string_field(payload, &["status", "order_status"]),
            reason: string_field(payload, &["reason"]),
        }
    }
}

fn string_field(payload: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| payload.get(*name).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn tracking_numbers(payload: &Value, order: Option<&Order>) -> Vec<String> {
    if let Some(numbers) = payload.get("tracking_numbers").and_then(Value::as_array) {
        let numbers: Vec<String> = numbers
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        if !numbers.is_empty() {
            return numbers;
        }
    }

    let Some(order) = order else {
        return Vec::new();
    };
    let fulfillment = payload
        .get("fulfillment_id")
        .and_then(Value::as_str)
        .and_then(|id| {
            order
                .fulfillments
                .iter()
                .find(|fulfillment| fulfillment.id.as_str() == id)
        })
        .or_else(|| order.fulfillments.last());
    fulfillment.map_or_else(Vec::new, |fulfillment: &OrderFulfillment| {
        fulfillment.tracking_numbers.clone()
    })
}

/// Formats minor currency units without floating point arithmetic.
///
/// The currency code is prefixed when available; otherwise the raw minor unit
/// amount is rendered so a template still has a useful value.
#[must_use]
pub fn format_amount(amount_minor: i64, currency: Option<&str>) -> String {
    let Some(currency) = currency.filter(|value| !value.trim().is_empty()) else {
        return amount_minor.to_string();
    };
    let (currency, digits) = match CurrencyCode::new(currency) {
        Ok(code) => {
            let digits = code.minor_unit_digits();
            (code.as_str().to_owned(), digits)
        }
        Err(_) => (currency.trim().to_ascii_uppercase(), 2),
    };
    if digits == 0 {
        return format!("{currency} {amount_minor}");
    }
    let scale = 10_u64.saturating_pow(digits);
    let magnitude = amount_minor.unsigned_abs();
    let major = magnitude / scale;
    let minor = magnitude % scale;
    let sign = if amount_minor < 0 { "-" } else { "" };
    format!(
        "{sign}{currency} {major}.{minor:0width$}",
        width = digits as usize
    )
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::DomainEventEnvelope;
    use serde_json::json;

    use super::{TemplateContext, format_amount};

    #[test]
    fn formats_minor_units_for_two_and_zero_decimal_currencies() {
        assert_eq!(format_amount(1234, Some("usd")), "USD 12.34");
        assert_eq!(format_amount(-5, Some("USD")), "-USD 0.05");
        assert_eq!(format_amount(500, Some("JPY")), "JPY 500");
        assert_eq!(format_amount(9, None), "9");
    }

    #[test]
    fn builds_context_from_payload_and_order_fields() {
        let event = DomainEventEnvelope::new(
            "payment.captured",
            None,
            json!({
                "order_id": "order_1",
                "amount": 2599,
                "currency_code": "EUR",
                "provider": "stripe"
            }),
        );
        let context = TemplateContext::from_event(&event, None);

        assert_eq!(context.order_id.as_deref(), Some("order_1"));
        assert_eq!(context.amount.as_deref(), Some("EUR 25.99"));
        assert_eq!(context.provider_id.as_deref(), Some("stripe"));
        assert_eq!(context.currency_code.as_deref(), Some("EUR"));
    }
}
