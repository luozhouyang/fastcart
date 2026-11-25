use super::*;

#[derive(Clone, Debug)]
pub(crate) struct Captured {
    pub(crate) headers: HeaderMap,
    pub(crate) body: String,
}

#[derive(Clone)]
pub(crate) struct MockState {
    pub(crate) captured: Arc<Mutex<Vec<Captured>>>,
    pub(crate) status: StatusCode,
}

pub(crate) async fn receive(
    State(state): State<MockState>,
    headers: HeaderMap,
    body: String,
) -> StatusCode {
    state
        .captured
        .lock()
        .unwrap()
        .push(Captured { headers, body });
    state.status
}

pub(crate) async fn spawn_mock(status: StatusCode) -> (String, MockState) {
    let state = MockState {
        captured: Arc::default(),
        status,
    };
    let app = Router::new()
        .route("/hook", post(receive))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        serve(listener, app).await.unwrap();
    });
    (format!("http://{address}/hook"), state)
}

pub(crate) async fn service() -> WebhookService {
    WebhookService::new(
        StoreDatabase::in_memory(StoreId::new())
            .await
            .expect("in-memory store database"),
    )
    .with_private_networks_allowed(true)
}

pub(crate) fn test_cipher() -> SecretCipher {
    SecretCipher::from_hex(&"ab".repeat(32)).expect("valid test key")
}

pub(crate) fn expected_signature(secret: &str, timestamp: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.{body}").as_bytes());
    format!("v1={}", hex::encode(mac.finalize().into_bytes()))
}

pub(crate) async fn endpoint(service: &WebhookService, url: &str, events: Vec<&str>) -> String {
    service
        .create_endpoint(CreateWebhookEndpointInput {
            url: url.to_owned(),
            events: events.into_iter().map(str::to_owned).collect(),
            ..Default::default()
        })
        .await
        .unwrap()
        .secret
}
