//! OpenAPI document smoke tests.
//!
//! Detailed structural coverage lives in the unit tests inside
//! `src/openapi.rs`; this target verifies that the public module and HTTP
//! handler expose a valid OpenAPI 3.1 document.

#[tokio::test]
async fn document_is_openapi_31() {
    let document = fastcart_api::openapi::document();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(document["servers"][0]["url"], "/api/v1");
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
