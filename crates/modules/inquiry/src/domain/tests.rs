use super::*;

fn field(key: &str, kind: FieldKind) -> InquiryFormField {
    InquiryFormField {
        key: key.to_owned(),
        kind,
        label: key.to_owned(),
        required: false,
        placeholder: None,
        help: None,
        default: None,
        options: Vec::new(),
        validation: FieldValidation::default(),
        file: None,
    }
}

fn revision(fields: Vec<InquiryFormField>) -> InquiryFormRevision {
    InquiryFormRevision {
        id: InquiryRevisionId::from_string("ifrev_1"),
        form_id: InquiryFormId::from_string("iform_1"),
        version: 1,
        fields,
        settings: InquiryFormSettings::default(),
        created_by: None,
        created_at: 0,
        published_at: None,
    }
}

#[test]
fn validates_keys_and_contact_coverage() {
    let mut name = field("name", FieldKind::Text);
    name.required = true;
    let email = field("email", FieldKind::Email);
    assert!(validate_fields(&[name, email]).is_ok());

    assert!(validate_fields(&[field("Name", FieldKind::Text)]).is_err());
    assert!(validate_fields(&[field("company", FieldKind::Text)]).is_err());
}

#[test]
fn rejects_unknown_and_reserved_custom_data() {
    let email = field("email", FieldKind::Email);
    let revision = revision(vec![email]);
    assert!(
        validate_submission(
            &revision,
            None,
            Some("a@example.com"),
            None,
            None,
            None,
            &serde_json::json!({"unknown": "x"}),
            false,
        )
        .is_err()
    );
    assert!(
        validate_submission(
            &revision,
            None,
            Some("a@example.com"),
            None,
            None,
            None,
            &serde_json::json!({"email": "b@example.com"}),
            false,
        )
        .is_err()
    );
}

#[test]
fn normalizes_email_and_custom_values() {
    let mut email = field("email", FieldKind::Email);
    email.required = true;
    let mut company = field("company", FieldKind::Text);
    company.validation.max_length = Some(20);
    let revision = revision(vec![email, company]);

    let normalized = validate_submission(
        &revision,
        None,
        Some(" Buyer@Example.com "),
        None,
        None,
        None,
        &serde_json::json!({"company": " Acme "}),
        false,
    )
    .unwrap();
    assert_eq!(normalized.email.as_deref(), Some("buyer@example.com"));
    assert_eq!(normalized.data, serde_json::json!({"company": " Acme "}));
}

#[test]
fn validates_choices_and_numbers() {
    let mut plan = field("plan", FieldKind::Select);
    plan.options = vec![FieldOption {
        value: "pro".to_owned(),
        label: "Pro".to_owned(),
    }];
    let mut seats = field("seats", FieldKind::Number);
    seats.validation.min = Some(1.0);
    seats.validation.max = Some(10.0);
    let revision = revision(vec![field("email", FieldKind::Email), plan, seats]);

    assert!(
        validate_submission(
            &revision,
            None,
            Some("a@example.com"),
            None,
            None,
            None,
            &serde_json::json!({"plan": "pro", "seats": 5}),
            false,
        )
        .is_ok()
    );
    assert!(
        validate_submission(
            &revision,
            None,
            Some("a@example.com"),
            None,
            None,
            None,
            &serde_json::json!({"plan": "enterprise", "seats": 5}),
            false,
        )
        .is_err()
    );
    assert!(
        validate_submission(
            &revision,
            None,
            Some("a@example.com"),
            None,
            None,
            None,
            &serde_json::json!({"plan": "pro", "seats": 50}),
            false,
        )
        .is_err()
    );
}
