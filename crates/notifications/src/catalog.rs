//! Embedded templates and the data-driven event-to-template catalog.

use minijinja::value::Serde;
use minijinja::{AutoEscape, Environment, Value};

use crate::NotificationError;
use crate::context::TemplateContext;

/// One registered notification template family.
struct EmbeddedTemplate {
    base: &'static str,
    subject: &'static str,
    text: &'static str,
    html: &'static str,
}

macro_rules! embedded {
    ($base:literal) => {
        EmbeddedTemplate {
            base: $base,
            subject: include_str!(concat!("../templates/", $base, ".subject")),
            text: include_str!(concat!("../templates/", $base, ".txt")),
            html: include_str!(concat!("../templates/", $base, ".html")),
        }
    };
}

/// Every template shipped with the crate.
///
/// Adding a notification type only requires a template family here plus a rule
/// for the event in [`RULES`].
const TEMPLATES: &[EmbeddedTemplate] = &[
    embedded!("order_confirmation"),
    embedded!("order_canceled"),
    embedded!("payment_receipt"),
    embedded!("payment_failed"),
    embedded!("shipping_confirmation"),
    embedded!("refund_confirmation"),
    embedded!("after_sales_update"),
];

/// A rule maps one outbox event type to a notification template family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationRule {
    /// Outbox event type, for example `order.placed`.
    pub event_type: &'static str,
    /// Template family used for the subject, text and HTML bodies.
    pub template: &'static str,
}

impl NotificationRule {
    const fn new(event_type: &'static str, template: &'static str) -> Self {
        Self {
            event_type,
            template,
        }
    }
}

/// The default event mapping.
///
/// Order creation is the customer confirmation trigger: `order.confirmed` and
/// `order.processing_started` are audit actions rather than outbox events.
/// `payment.refunded` is intentionally not mapped because `order.refunded`
/// already produces the customer-facing confirmation.
pub const RULES: &[NotificationRule] = &[
    NotificationRule::new("order.placed", "order_confirmation"),
    NotificationRule::new("order.canceled", "order_canceled"),
    NotificationRule::new("order.fulfilled", "shipping_confirmation"),
    NotificationRule::new("order.refunded", "refund_confirmation"),
    NotificationRule::new("payment.captured", "payment_receipt"),
    NotificationRule::new("payment.failed", "payment_failed"),
    NotificationRule::new("order.return_received", "after_sales_update"),
    NotificationRule::new("order.return_refunded", "after_sales_update"),
    NotificationRule::new("order.claim_accepted", "after_sales_update"),
    NotificationRule::new("order.claim_refunded", "after_sales_update"),
    NotificationRule::new("order.claim_fulfilled", "after_sales_update"),
    NotificationRule::new("order.exchange_accepted", "after_sales_update"),
    NotificationRule::new("order.exchange_fulfilled", "after_sales_update"),
];

/// Returns the rule configured for an event type, when any.
#[must_use]
pub fn rule_for(event_type: &str) -> Option<&'static NotificationRule> {
    RULES.iter().find(|rule| rule.event_type == event_type)
}

/// Rendered notification bodies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RenderedNotification {
    pub subject: String,
    pub text: String,
    pub html: String,
}

/// An embedded template registry with HTML auto-escaping enabled.
#[derive(Clone, Debug)]
pub struct TemplateRegistry {
    env: Environment<'static>,
}

impl TemplateRegistry {
    /// Registers the templates bundled with the crate.
    #[must_use]
    pub fn embedded() -> Self {
        let mut registry = Self::empty();
        registry
            .with_all(TEMPLATES.iter())
            .expect("bundled notification templates are valid and unique");
        registry
    }

    /// Creates an empty registry with HTML auto-escaping configured.
    #[must_use]
    pub fn empty() -> Self {
        let mut env = Environment::new();
        env.set_auto_escape_callback(|name| {
            if name.ends_with(".html") {
                AutoEscape::Html
            } else {
                AutoEscape::None
            }
        });
        Self { env }
    }

    /// Adds or replaces one template family.
    ///
    /// The `base` name is used with `.subject`, `.txt` and `.html` suffixes.
    pub fn add_template(
        &mut self,
        base: &'static str,
        subject: &'static str,
        text: &'static str,
        html: &'static str,
    ) -> Result<(), NotificationError> {
        self.with_one(&EmbeddedTemplate {
            base,
            subject,
            text,
            html,
        })
    }

    fn with_all<'a>(
        &mut self,
        templates: impl Iterator<Item = &'a EmbeddedTemplate>,
    ) -> Result<(), NotificationError> {
        for template in templates {
            self.with_one(template)?;
        }
        Ok(())
    }

    fn with_one(&mut self, template: &EmbeddedTemplate) -> Result<(), NotificationError> {
        let subject = template_name(template.base, "subject");
        let text = template_name(template.base, "txt");
        let html = template_name(template.base, "html");
        self.env.add_template_owned(subject, template.subject)?;
        self.env.add_template_owned(text, template.text)?;
        self.env.add_template_owned(html, template.html)?;
        Ok(())
    }

    /// Renders the subject, text and HTML bodies for a template family.
    pub(crate) fn render(
        &self,
        base: &str,
        context: &TemplateContext,
    ) -> Result<RenderedNotification, NotificationError> {
        let value = Value::from(Serde(context));
        let subject = self
            .env
            .get_template(&template_name(base, "subject"))?
            .render(value.clone())?;
        let text = self
            .env
            .get_template(&template_name(base, "txt"))?
            .render(value.clone())?;
        let html = self
            .env
            .get_template(&template_name(base, "html"))?
            .render(value)?;
        Ok(RenderedNotification {
            subject,
            text,
            html,
        })
    }
}

impl Default for TemplateRegistry {
    fn default() -> Self {
        Self::embedded()
    }
}

fn template_name(base: &str, suffix: &str) -> String {
    format!("{base}.{suffix}")
}

#[cfg(test)]
mod tests {
    use super::TemplateRegistry;
    use crate::{TemplateContext, TemplateItem};

    fn context() -> TemplateContext {
        TemplateContext {
            event_type: "order.placed".to_owned(),
            event_id: "event_1".to_owned(),
            display_number: Some(42),
            currency_code: Some("USD".to_owned()),
            total: Some("USD 49.99".to_owned()),
            items: vec![TemplateItem {
                title: "<script>alert(1)</script>".to_owned(),
                variant_title: Some("Large & \"red\"".to_owned()),
                quantity: 2,
                total: "USD 49.99".to_owned(),
            }],
            ..TemplateContext::default()
        }
    }

    #[test]
    fn renders_text_and_escapes_html() {
        let registry = TemplateRegistry::embedded();
        let rendered = registry.render("order_confirmation", &context()).unwrap();

        assert!(rendered.subject.contains("Order #42"));
        assert!(rendered.text.contains("<script>alert(1)</script>"));
        assert!(rendered.text.contains("USD 49.99"));
        assert!(
            rendered.html.contains("&lt;script&gt;"),
            "html was {}",
            rendered.html
        );
        assert!(
            rendered.html.contains("Large &amp; &quot;red&quot;"),
            "html was {}",
            rendered.html
        );
        assert!(!rendered.html.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn templates_are_available_for_every_rule() {
        let registry = TemplateRegistry::embedded();
        let context = TemplateContext::default();
        for rule in super::RULES {
            registry
                .render(rule.template, &context)
                .unwrap_or_else(|error| panic!("{} failed to render: {error}", rule.template));
        }
    }
}
