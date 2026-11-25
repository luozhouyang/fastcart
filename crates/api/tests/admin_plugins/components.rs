use super::runtime::raw;
use super::support::*;
use super::*;

const COMPONENT_JS: &str = "export const hero = \"ready\";\n";

fn component_archive(document: &Value) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = Builder::new(&mut encoder);
        append(&mut builder, "plugin.json", document.to_string().as_bytes());
        append(&mut builder, "index.js", b"export default {};");
        append(
            &mut builder,
            "ui/settings.html",
            b"<!doctype html><p>settings</p>",
        );
        append(&mut builder, "components/hero.js", COMPONENT_JS.as_bytes());
        builder.finish().unwrap();
    }
    encoder.finish().unwrap()
}

fn component_manifest(id: &str, provides: Value) -> Value {
    let mut document = manifest(id, "1.0.0", &[]);
    document["capabilities"]["storefront"] = json!(["storefront.component"]);
    document["provides"] = provides;
    document
}

async fn fetch_headers(
    app: &Router,
    uri: &str,
    headers: &[(&str, String)],
) -> (StatusCode, axum::http::HeaderMap) {
    let mut builder = Request::builder().method("GET").uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    (response.status(), response.headers().clone())
}

#[tokio::test]
async fn components_require_an_enabled_plugin_and_expose_entries() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let document = component_manifest(
        "demo",
        json!([
            {
                "kind": "storefront_component",
                "id": "hero",
                "title": "Hero banner",
                "category": "marketing",
                "editor_manifest": { "fields": [{ "name": "title", "type": "text" }] },
                "entry": "components/hero.js",
                "requires": [
                    { "id": "chat", "version": 1 },
                    { "id": "search", "optional": true }
                ]
            },
            {
                "kind": "storefront_component",
                "id": "footer-note",
                "title": "Footer note"
            }
        ]),
    );
    let bytes = component_archive(&document);
    let (status, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    assert_eq!(installed["enabled"], false);
    let digest = installed["digest"].as_str().unwrap().to_owned();

    let (status, body, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/components",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["components"], json!([]), "disabled plugins are hidden");

    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/components",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let components = body["components"].as_array().expect("components array");
    assert_eq!(components.len(), 2);
    let ids: Vec<&str> = components
        .iter()
        .map(|component| component["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["footer-note", "hero"], "deterministic order");

    assert_eq!(
        components[0],
        json!({
            "plugin_id": "demo",
            "plugin_name": "Demo Plugin",
            "plugin_version": "1.0.0",
            "digest": digest.as_str(),
            "id": "footer-note",
            "title": "Footer note",
            "category": null,
            "editor_manifest": null,
            "entry": null,
            "requires": []
        })
    );
    assert_eq!(
        components[1],
        json!({
            "plugin_id": "demo",
            "plugin_name": "Demo Plugin",
            "plugin_version": "1.0.0",
            "digest": digest.as_str(),
            "id": "hero",
            "title": "Hero banner",
            "category": "marketing",
            "editor_manifest": { "fields": [{ "name": "title", "type": "text" }] },
            "entry": format!("/_fastcart/extensions/demo/{digest}/components/hero.js"),
            "requires": [
                { "id": "chat", "version": 1, "optional": false },
                { "id": "search", "version": null, "optional": true }
            ]
        })
    );
}

#[tokio::test]
async fn only_enabled_plugins_with_components_are_listed() {
    let fixture = fixture().await;
    let headers = fixture.headers();

    let demo = component_manifest(
        "demo",
        json!([{ "kind": "storefront_component", "id": "hero", "title": "Hero" }]),
    );
    let (status, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &component_archive(&demo),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let empty = manifest("empty", "1.0.0", &[]);
    let (status, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &component_archive(&empty),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/empty/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let hidden = component_manifest(
        "hidden",
        json!([{ "kind": "storefront_component", "id": "hero", "title": "Hidden" }]),
    );
    let (status, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &component_archive(&hidden),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    assert_eq!(installed["enabled"], false);

    let (status, body, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/components",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let components = body["components"].as_array().expect("components array");
    assert_eq!(components.len(), 1, "{body}");
    assert_eq!(components[0]["plugin_id"], "demo");
    assert_eq!(components[0]["id"], "hero");
}

#[tokio::test]
async fn components_route_requires_authentication() {
    let fixture = fixture().await;
    let (status, body, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/components",
        None,
        &[("x-store-id", fixture.store_id.clone())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"]["code"], "UNAUTHORIZED");
}

#[tokio::test]
async fn component_entry_is_served_by_the_storefront_extension_route() {
    let fixture = fixture().await;
    let headers = fixture.headers();

    let demo = component_manifest(
        "demo",
        json!([{
            "kind": "storefront_component",
            "id": "hero",
            "title": "Hero",
            "entry": "components/hero.js"
        }]),
    );
    let (status, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &component_archive(&demo),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    let digest = installed["digest"].as_str().unwrap().to_owned();
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let hidden = component_manifest(
        "hidden",
        json!([{
            "kind": "storefront_component",
            "id": "hero",
            "title": "Hidden",
            "entry": "components/hero.js"
        }]),
    );
    let (status, installed) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &component_archive(&hidden),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{installed}");
    assert_eq!(installed["enabled"], false);
    let hidden_digest = installed["digest"].as_str().unwrap().to_owned();

    let entry = format!("/_fastcart/extensions/demo/{digest}/components/hero.js");
    let (status, body) = raw(
        &fixture.app,
        "GET",
        &entry,
        vec![],
        &[("host", HOST.to_owned())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body, COMPONENT_JS.as_bytes());

    let (status, headers_map) =
        fetch_headers(&fixture.app, &entry, &[("host", HOST.to_owned())]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers_map
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(
        headers_map
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=31536000, immutable")
    );

    let disabled_entry = format!("/_fastcart/extensions/hidden/{hidden_digest}/components/hero.js");
    let (status, _) = raw(
        &fixture.app,
        "GET",
        &disabled_entry,
        vec![],
        &[("host", HOST.to_owned())],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
