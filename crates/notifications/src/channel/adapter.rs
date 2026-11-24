//! Feishu message rendering.

use super::*;

pub(super) struct FeishuAdapter;

#[async_trait]
impl ChannelAdapter for FeishuAdapter {
    async fn render(
        &self,
        db: &StoreDatabase,
        event: &DomainEventEnvelope,
        context: &RenderContext,
    ) -> Result<Value, ChannelError> {
        match event.event_type.as_str() {
            "order.placed" => {
                let data = order_card_data(db, event).await;
                feishu::render_order_card(&data, context)
            }
            "inquiry.created" => {
                let data = inquiry_card_data(event);
                feishu::render_inquiry_card(&data, context)
            }
            other => Err(ChannelError::Permanent(format!(
                "unsupported notification event: {other}"
            ))),
        }
    }
}

async fn order_card_data(db: &StoreDatabase, event: &DomainEventEnvelope) -> OrderCardData {
    let order_id = event
        .data
        .get("order_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let order = match order_id.as_deref() {
        Some(id) => OrderService::new(db.clone())
            .get(&OrderId::from_string(id))
            .await
            .ok(),
        None => None,
    };

    if let Some(order) = order {
        return OrderCardData {
            display_number: order.display_number.to_string(),
            order_id: Some(order.id.as_str().to_owned()),
            amount: format_amount(order.total, Some(&order.currency_code)),
            customer: order.email,
            item_count: order.items.iter().map(|item| item.quantity).sum(),
            created_at: order.created_at,
        };
    }

    let payload = &event.data;
    let currency = payload_string_field(payload, &["currency_code", "currency"]);
    let amount = payload
        .get("total")
        .and_then(Value::as_i64)
        .map(|total| format_amount(total, currency))
        .or_else(|| {
            payload
                .get("total")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            payload
                .get("amount")
                .and_then(Value::as_i64)
                .map(|total| format_amount(total, currency))
        })
        .unwrap_or_else(|| "—".to_owned());
    let item_count = payload
        .get("item_count")
        .and_then(Value::as_i64)
        .or_else(|| {
            payload.get("items").and_then(Value::as_array).map(|items| {
                items
                    .iter()
                    .map(|item| item.get("quantity").and_then(Value::as_i64).unwrap_or(1))
                    .sum()
            })
        })
        .unwrap_or(0);

    OrderCardData {
        display_number: payload
            .get("display_number")
            .and_then(Value::as_i64)
            .map(|number| number.to_string())
            .unwrap_or_else(|| "—".to_owned()),
        order_id,
        amount,
        customer: payload_string_field(payload, &["email", "customer_email"])
            .unwrap_or("—")
            .to_owned(),
        item_count,
        created_at: event.occurred_at,
    }
}

fn inquiry_card_data(event: &DomainEventEnvelope) -> InquiryCardData {
    let payload = &event.data;
    let contact = payload_string_field(payload, &["email", "phone", "contact"])
        .unwrap_or("—")
        .to_owned();
    let attachment_count = payload
        .get("attachment_count")
        .and_then(Value::as_i64)
        .or_else(|| {
            payload
                .get("attachments")
                .and_then(Value::as_array)
                .map(|files| files.len() as i64)
        })
        .or_else(|| {
            payload
                .get("files")
                .and_then(Value::as_object)
                .map(|files| {
                    files
                        .values()
                        .filter_map(Value::as_array)
                        .map(|values| values.len() as i64)
                        .sum()
                })
        })
        .unwrap_or(0);

    let custom_fields = payload
        .get("custom_fields")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let label = item.get("label").and_then(Value::as_str)?;
                    let value = item.get("value").and_then(Value::as_str)?;
                    Some((label.to_owned(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();

    InquiryCardData {
        form_handle: payload_string_field(payload, &["form_handle"])
            .unwrap_or("—")
            .to_owned(),
        inquiry_id: payload
            .get("inquiry_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        name: payload_string_field(payload, &["name"])
            .unwrap_or("—")
            .to_owned(),
        contact,
        subject: payload_string_field(payload, &["subject"])
            .unwrap_or("—")
            .to_owned(),
        message: payload_string_field(payload, &["message"])
            .unwrap_or("")
            .to_owned(),
        attachment_count,
        custom_fields,
    }
}

pub(super) fn payload_string<'a>(payload: &'a Value, key: &str) -> Result<&'a str, ChannelError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ChannelError::Permanent(format!("job payload is missing {key}")))
}

fn payload_string_field<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
}
