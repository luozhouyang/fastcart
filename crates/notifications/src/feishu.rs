//! Feishu (`open.feishu.cn`) and Lark (`open.larksuite.com`) custom-bot client.
//!
//! The adapter renders card JSON 2.0 messages and posts them to an operator
//! supplied webhook URL. Endpoint URLs and signing secrets are never included
//! in error strings so they cannot leak through delivery records or logs.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use fastcart_kernel::{AppError, Timestamp, now_timestamp};
use hmac::{Hmac, Mac};
use reqwest::Url;
use serde_json::{Value, json};
use sha2::Sha256;

use crate::channel::{ChannelError, RenderContext};

/// Maximum accepted endpoint URL length in bytes.
pub const MAX_ENDPOINT_URL_BYTES: usize = 2048;
/// Maximum response bytes persisted with a delivery.
pub const MAX_RESPONSE_BODY_BYTES: usize = 2048;
/// Upper bound for a serialized card body.
pub const MAX_CARD_BYTES: usize = 20 * 1024;

/// Hosts a Feishu/Lark bot webhook may target.
const ALLOWED_HOSTS: &[&str] = &["open.feishu.cn", "open.larksuite.com"];

/// Validates a Feishu/Lark webhook URL.
///
/// Only HTTPS URLs on the documented bot hosts are accepted. Credentials,
/// whitespace, control characters and oversized URLs are rejected. The
/// `allow_local` escape hatch exists for tests and local development only; it
/// permits plain HTTP and loopback hosts.
pub(crate) fn validate_endpoint_url(raw: &str, allow_local: bool) -> Result<String, AppError> {
    if raw.is_empty() {
        return Err(AppError::validation("feishu endpoint url is required"));
    }
    if raw.len() > MAX_ENDPOINT_URL_BYTES {
        return Err(AppError::validation(format!(
            "feishu endpoint url must not exceed {MAX_ENDPOINT_URL_BYTES} bytes"
        )));
    }
    if raw
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(AppError::validation(
            "feishu endpoint url must not contain whitespace or control characters",
        ));
    }

    let parsed = Url::parse(raw)
        .map_err(|error| AppError::validation(format!("invalid feishu endpoint url: {error}")))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::validation(
            "feishu endpoint url must not include credentials",
        ));
    }
    let Some(host) = parsed.host_str().filter(|host| !host.is_empty()) else {
        return Err(AppError::validation(
            "feishu endpoint url must include a host",
        ));
    };
    let normalized_host = host.trim_matches(['[', ']']).to_ascii_lowercase();

    let local = is_loopback(&normalized_host);
    if allow_local && local {
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(AppError::validation(
                "local feishu endpoint url must use http or https",
            ));
        }
    } else {
        if parsed.scheme() != "https" {
            return Err(AppError::validation("feishu endpoint url must use https"));
        }
        if !ALLOWED_HOSTS.contains(&normalized_host.as_str()) {
            return Err(AppError::validation(
                "feishu endpoint url must target open.feishu.cn or open.larksuite.com",
            ));
        }
    }

    Ok(raw.to_owned())
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Computes the Feishu webhook signature.
///
/// `HMAC-SHA256(key = "{timestamp}\n{secret}", data = "")`, base64 encoded.
pub(crate) fn sign(secret: &str, timestamp: Timestamp) -> Result<String, ChannelError> {
    let key = format!("{timestamp}\n{secret}");
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
        .map_err(|_| ChannelError::Permanent("feishu signing key is invalid".to_owned()))?;
    mac.update(&[]);
    Ok(BASE64.encode(mac.finalize().into_bytes()))
}

/// Adds the top-level `timestamp` and `sign` fields when a secret is set.
fn signed_body(
    message: &Value,
    secret: Option<&str>,
    timestamp: Timestamp,
) -> Result<Value, ChannelError> {
    let mut body = message.as_object().cloned().ok_or_else(|| {
        ChannelError::Permanent("feishu message must be a JSON object".to_owned())
    })?;
    if let Some(secret) = secret.filter(|secret| !secret.is_empty()) {
        body.insert("timestamp".to_owned(), json!(timestamp.to_string()));
        body.insert("sign".to_owned(), json!(sign(secret, timestamp)?));
    }
    Ok(Value::Object(body))
}

/// Escapes a value before it is embedded in a `lark_md` element.
///
/// HTML-significant characters are entity encoded and markdown control
/// characters that could start links, code spans or emphasis are backslash
/// escaped. Newlines collapse to spaces so a value cannot splice extra lines
/// into the card.
pub(crate) fn escape_lark_md(value: &str) -> String {
    let html_escaped = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let mut escaped = String::with_capacity(html_escaped.len());
    for character in html_escaped.chars() {
        match character {
            '\n' | '\r' | '\t' => escaped.push(' '),
            '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '(' | ')' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Truncates a value to at most `max` characters, adding an ellipsis when cut.
fn clip(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    let mut clipped: String = value.chars().take(max.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

/// Values needed to render an `order.placed` card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OrderCardData {
    pub display_number: String,
    pub order_id: Option<String>,
    pub amount: String,
    pub customer: String,
    pub item_count: i64,
    pub created_at: Timestamp,
}

/// Values needed to render an `inquiry.created` card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InquiryCardData {
    pub form_handle: String,
    pub inquiry_id: Option<String>,
    pub name: String,
    pub contact: String,
    pub subject: String,
    pub message: String,
    pub attachment_count: i64,
    pub custom_fields: Vec<(String, String)>,
}

/// Renders a green `order.placed` card.
pub(crate) fn render_order_card(
    data: &OrderCardData,
    context: &RenderContext,
) -> Result<Value, ChannelError> {
    let title = format!(
        "新订单 #{}",
        escape_lark_md(&clip(&data.display_number, 80))
    );
    let fields = [
        format!("**金额**：{}", escape_lark_md(&clip(&data.amount, 120))),
        format!(
            "**客户邮箱**：{}",
            escape_lark_md(&clip(&data.customer, 320))
        ),
        format!("**商品数**：{}", data.item_count.max(0)),
        format!("**创建时间**：{}", data.created_at),
    ]
    .join("\n");

    let action = data
        .order_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .and_then(|id| {
            context
                .admin_base_url
                .as_deref()
                .map(|base| action_button("查看订单", join_url(base, &format!("orders/{id}"))))
        });

    card(title, "green", fields, action)
}

/// Renders a blue `inquiry.created` card.
pub(crate) fn render_inquiry_card(
    data: &InquiryCardData,
    context: &RenderContext,
) -> Result<Value, ChannelError> {
    let handle = clip(&data.form_handle, 120);
    let handle = if handle.trim().is_empty() {
        "—".to_owned()
    } else {
        handle
    };
    let title = format!("新留言 · {}", escape_lark_md(&handle));
    let excerpt = clip(&data.message, 300);
    let mut lines = vec![
        format!("**姓名**：{}", escape_lark_md(&clip(&data.name, 200))),
        format!(
            "**联系方式**：{}",
            escape_lark_md(&clip(&data.contact, 320))
        ),
        format!("**主题**：{}", escape_lark_md(&clip(&data.subject, 200))),
    ];
    for (label, value) in &data.custom_fields {
        lines.push(format!(
            "**{}**：{}",
            escape_lark_md(&clip(label, 80)),
            escape_lark_md(&clip(value, 200))
        ));
    }
    lines.push(format!("**留言**：{}", escape_lark_md(&excerpt)));
    lines.push(format!("**附件**：{}", data.attachment_count.max(0)));
    let fields = lines.join("\n");

    let action = data
        .inquiry_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .and_then(|id| {
            context
                .admin_base_url
                .as_deref()
                .map(|base| action_button("查看留言", join_url(base, &format!("inbox/{id}"))))
        });

    card(title, "blue", fields, action)
}

fn card(
    title: String,
    template: &str,
    fields: String,
    action: Option<Value>,
) -> Result<Value, ChannelError> {
    let mut elements = vec![json!({
        "tag": "div",
        "text": {"tag": "lark_md", "content": fields}
    })];
    if let Some(action) = action {
        elements.push(action);
    }
    let body = json!({
        "msg_type": "interactive",
        "card": {
            "schema": "2.0",
            "config": {"update_multi": true},
            "header": {
                "title": {"tag": "plain_text", "content": title},
                "template": template
            },
            "body": {"elements": elements}
        }
    });

    let size = serde_json::to_vec(&body)
        .map_err(|_| ChannelError::Permanent("feishu card serialization failed".to_owned()))?
        .len();
    if size > MAX_CARD_BYTES {
        return Err(ChannelError::Permanent(format!(
            "feishu card exceeds {MAX_CARD_BYTES} bytes"
        )));
    }
    Ok(body)
}

fn action_button(label: &str, url: String) -> Value {
    json!({
        "tag": "action",
        "actions": [{
            "tag": "button",
            "text": {"tag": "plain_text", "content": label},
            "type": "primary",
            "url": url
        }]
    })
}

fn join_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Result of one HTTP send attempt.
pub(crate) struct SendResult {
    /// Delivery outcome.
    pub result: Result<(), ChannelError>,
    /// HTTP response status, when a response was received.
    pub status: Option<i64>,
    /// Truncated response body or transport error summary.
    pub body: Option<String>,
}

/// Posts a message to a Feishu bot webhook.
///
/// The signing timestamp is generated at send time so a queued delivery can
/// wait without invalidating the signature. Redirects are disabled by the
/// service's HTTP client.
pub(crate) async fn send(
    client: &reqwest::Client,
    endpoint_url: &str,
    secret: Option<&str>,
    message: &Value,
) -> SendResult {
    let body = match signed_body(message, secret, now_timestamp()) {
        Ok(body) => body,
        Err(error) => {
            return SendResult {
                result: Err(error),
                status: None,
                body: None,
            };
        }
    };

    let response = client
        .post(endpoint_url)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await;

    let response = match response {
        Ok(response) => response,
        Err(error) => {
            let summary = truncate_bytes(error.without_url().to_string().as_bytes());
            return SendResult {
                result: Err(ChannelError::Retryable(
                    "feishu request could not be completed".to_owned(),
                )),
                status: None,
                body: Some(summary),
            };
        }
    };

    let status_code = response.status();
    let status = i64::from(status_code.as_u16());
    let response_body = read_response_body(response).await;

    let result = if status_code.is_success() {
        classify_response(&response_body)
    } else if status_code.is_server_error()
        || status_code == reqwest::StatusCode::REQUEST_TIMEOUT
        || status_code == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        Err(ChannelError::Retryable(format!(
            "feishu responded with HTTP {status}"
        )))
    } else {
        Err(ChannelError::Permanent(format!(
            "feishu responded with HTTP {status}"
        )))
    };

    SendResult {
        result,
        status: Some(status),
        body: Some(response_body),
    }
}

fn classify_response(body: &str) -> Result<(), ChannelError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ChannelError::Permanent("feishu returned a non-JSON response".to_owned()))?;
    let Some(code) = value.get("code").and_then(Value::as_i64) else {
        return Err(ChannelError::Permanent(
            "feishu response is missing the code field".to_owned(),
        ));
    };
    classify_code(code, value.get("msg").and_then(Value::as_str))
}

fn classify_code(code: i64, message: Option<&str>) -> Result<(), ChannelError> {
    if code == 0 {
        return Ok(());
    }
    let detail = message.map_or_else(String::new, |message| format!(": {}", clip(message, 160)));
    match code {
        // Rate limited.
        11232 => Err(ChannelError::Retryable(format!(
            "feishu rate limited the request ({code}){detail}"
        ))),
        // Sign or timestamp mismatch.
        19021 => Err(ChannelError::Retryable(format!(
            "feishu rejected the signature or timestamp ({code}){detail}"
        ))),
        // IP not allowed, keyword mismatch.
        19022 | 19024 => Err(ChannelError::Permanent(format!(
            "feishu permanently rejected the request ({code}){detail}"
        ))),
        _ => Err(ChannelError::Permanent(format!(
            "feishu rejected the request ({code}){detail}"
        ))),
    }
}

async fn read_response_body(response: reqwest::Response) -> String {
    match response.bytes().await {
        Ok(body) => truncate_bytes(&body),
        Err(_) => String::new(),
    }
}

/// Truncates a byte slice to the delivery response limit on a UTF-8 boundary.
pub(crate) fn truncate_bytes(body: &[u8]) -> String {
    let end = body.len().min(MAX_RESPONSE_BODY_BYTES);
    let end = (0..=end)
        .rev()
        .find(|index| {
            body.get(*index)
                .is_none_or(|byte| *byte & 0b1100_0000 != 0b1000_0000)
        })
        .unwrap_or(0);
    String::from_utf8_lossy(&body[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::Timestamp;
    use serde_json::json;

    use super::{
        InquiryCardData, OrderCardData, classify_code, clip, escape_lark_md, render_inquiry_card,
        render_order_card, sign, validate_endpoint_url,
    };
    use crate::channel::{ChannelError, RenderContext};

    #[test]
    fn signature_matches_the_documented_vector() {
        let signature = sign("test-secret", 1_700_000_000).unwrap();
        assert_eq!(signature, "mbm4Y4oluIPQ00qlBIhX8vAZ0EKv3nw0LuTb91jPL84=");
    }

    #[test]
    fn endpoint_allowlist_rejects_plaintext_and_foreign_hosts() {
        assert!(
            validate_endpoint_url("https://open.feishu.cn/open-apis/bot/v2/hook/abc", false)
                .is_ok()
        );
        assert!(
            validate_endpoint_url(
                "https://open.larksuite.com/open-apis/bot/v2/hook/abc",
                false
            )
            .is_ok()
        );

        for url in [
            "http://open.feishu.cn/open-apis/bot/v2/hook/abc",
            "https://example.com/open-apis/bot/v2/hook/abc",
            "https://open.feishu.cn.evil.example/hook",
            "https://user:pass@open.feishu.cn/hook",
            "https://open.feishu.cn/hook with space",
            "https://open.feishu.cn/hook\t",
        ] {
            assert!(
                validate_endpoint_url(url, false).is_err(),
                "{url} should be rejected"
            );
        }

        let oversized = format!("https://open.feishu.cn/{}", "a".repeat(2_100));
        assert!(validate_endpoint_url(&oversized, false).is_err());

        // The local escape hatch is limited to loopback hosts.
        assert!(validate_endpoint_url("http://127.0.0.1:8080/hook", true).is_ok());
        assert!(validate_endpoint_url("http://example.com/hook", true).is_err());
    }

    #[test]
    fn escapes_markdown_and_html_control_characters() {
        assert_eq!(
            escape_lark_md("<script>alert('x')</script> & *bold* [link](x)"),
            "&lt;script&gt;alert\\('x'\\)&lt;/script&gt; &amp; \\*bold\\* \\[link\\]\\(x\\)"
        );
        assert_eq!(escape_lark_md("line\nbreak"), "line break");
    }

    #[test]
    fn renders_order_card_fields_and_deep_link() {
        let data = OrderCardData {
            display_number: "42".to_owned(),
            order_id: Some("order_1".to_owned()),
            amount: "USD 19.99".to_owned(),
            customer: "<script>alert(1)</script>@example.com".to_owned(),
            item_count: 2,
            created_at: 1_700_000_000,
        };
        let context = RenderContext {
            admin_base_url: Some("https://admin.example.com/".to_owned()),
            locale: "zh-CN".to_owned(),
        };
        let card = render_order_card(&data, &context).unwrap();
        let serialized = card.to_string();

        assert!(serialized.contains("新订单 #42"));
        assert!(serialized.contains("USD 19.99"));
        assert!(serialized.contains("&lt;script&gt;"));
        assert!(!serialized.contains("<script>"));
        assert!(serialized.contains("https://admin.example.com/orders/order_1"));
        assert!(serialized.contains("\"template\":\"green\""));
        assert!(serialized.contains("\"schema\":\"2.0\""));
        assert!(serialized.contains("\"update_multi\":true"));
    }

    #[test]
    fn renders_inquiry_card_and_truncates_the_message() {
        let data = InquiryCardData {
            form_handle: "contact".to_owned(),
            inquiry_id: Some("inq_1".to_owned()),
            name: "<script>alert(1)</script>".to_owned(),
            contact: "buyer@example.com".to_owned(),
            subject: "尺寸".to_owned(),
            message: "x".repeat(400),
            attachment_count: 3,
            custom_fields: vec![("公司".to_owned(), "Acme".to_owned())],
        };
        let context = RenderContext {
            admin_base_url: Some("https://admin.example.com".to_owned()),
            locale: "zh-CN".to_owned(),
        };
        let card = render_inquiry_card(&data, &context).unwrap();
        let serialized = card.to_string();

        assert!(serialized.contains("新留言 · contact"));
        assert!(serialized.contains("buyer@example.com"));
        assert!(serialized.contains("&lt;script&gt;"));
        assert!(serialized.contains("公司"));
        assert!(serialized.contains("Acme"));
        assert!(serialized.contains("https://admin.example.com/inbox/inq_1"));
        assert!(serialized.contains("\"template\":\"blue\""));
        assert!(serialized.contains("**附件**：3"));
        assert!(!serialized.contains(&"x".repeat(400)));
        assert!(clip(&"x".repeat(400), 300).chars().count() <= 300);
    }

    #[test]
    fn classifies_feishu_error_codes() {
        assert_eq!(classify_code(0, None), Ok(()));
        assert!(matches!(
            classify_code(11232, None),
            Err(ChannelError::Retryable(_))
        ));
        assert!(matches!(
            classify_code(19021, None),
            Err(ChannelError::Retryable(_))
        ));
        assert!(matches!(
            classify_code(19022, None),
            Err(ChannelError::Permanent(_))
        ));
        assert!(matches!(
            classify_code(19024, None),
            Err(ChannelError::Permanent(_))
        ));
        assert!(matches!(
            classify_code(12345, None),
            Err(ChannelError::Permanent(_))
        ));
    }

    #[test]
    fn cards_never_include_raw_script_markup() {
        let data = InquiryCardData {
            form_handle: "<script>".to_owned(),
            inquiry_id: None,
            name: "a".to_owned(),
            contact: "b".to_owned(),
            subject: "c".to_owned(),
            message: html_escape_check(),
            attachment_count: 0,
            custom_fields: Vec::new(),
        };
        let context = RenderContext::default();
        let serialized = render_inquiry_card(&data, &context).unwrap().to_string();
        assert!(!serialized.contains("<script>"));
        assert!(serialized.contains("&lt;script&gt;"));
    }

    fn html_escape_check() -> String {
        "<script>alert('x')</script>".to_owned()
    }

    #[test]
    fn timestamps_are_rendered_as_integers() {
        let data = OrderCardData {
            display_number: "1".to_owned(),
            order_id: None,
            amount: "USD 1.00".to_owned(),
            customer: "a@example.com".to_owned(),
            item_count: 1,
            created_at: 1_700_000_000 as Timestamp,
        };
        let serialized = render_order_card(&data, &RenderContext::default())
            .unwrap()
            .to_string();
        assert!(serialized.contains("1700000000"));
        assert!(!serialized.contains("\"tag\":\"action\""));
        assert_eq!(json!({"ok": true})["ok"], true);
    }
}
