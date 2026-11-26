//! OpenAPI document smoke tests.
//!
//! Detailed structural coverage lives in the unit tests inside
//! `src/openapi.rs`; this target verifies that the public module and HTTP
//! handler expose a valid OpenAPI 3.1 document.

#[tokio::test]
async fn document_is_openapi_31() {
    let document = fastcart_api::openapi::document();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(document["servers"][0]["url"], "/");
    assert!(
        document["paths"]
            .as_object()
            .is_some_and(|paths| !paths.is_empty())
    );
}

#[tokio::test]
async fn handler_serves_the_document() {
    let response = fastcart_api::openapi::handler().await;
    assert_eq!(response.0["openapi"], "3.1.0");
}

/// Refreshes the snapshot consumed by the admin SDK parity check.
///
/// The test is inert unless `FASTCART_WRITE_OPENAPI_SNAPSHOT=1`, so the
/// regular suite never mutates the working tree. Regenerate with:
///
/// ```text
/// FASTCART_WRITE_OPENAPI_SNAPSHOT=1 cargo test -p fastcart-api --test openapi writes_openapi_snapshot
/// ```
#[test]
fn writes_openapi_snapshot() {
    if std::env::var("FASTCART_WRITE_OPENAPI_SNAPSHOT").as_deref() != Ok("1") {
        return;
    }

    let document = fastcart_api::openapi::document();
    let snapshot = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/admin-sdk/openapi.snapshot.json");
    let mut serialized = serde_json::to_string_pretty(&document).expect("serialize document");
    serialized.push('\n');
    std::fs::write(&snapshot, serialized).expect("write OpenAPI snapshot");
    eprintln!("wrote {}", snapshot.display());
}
