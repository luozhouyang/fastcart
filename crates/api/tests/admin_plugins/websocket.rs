use super::support::*;
use super::*;

const WS_PLUGIN_JS: &str = r#"
export default {
  async fetch(request) {
    if (request.headers.get("upgrade") !== "websocket") {
      return new Response("expected upgrade", { status: 400 });
    }
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    server.accept();
    server.send("welcome");
    server.addEventListener("message", (event) => server.send(`echo:${event.data}`));
    return new Response(null, { status: 101, webSocket: client });
  },
};
"#;

async fn start_ws_commerce() -> SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0_u8; 8192];
                loop {
                    let Ok(read) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if let Some(position) =
                        buffer.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        let head = String::from_utf8_lossy(&buffer[..position]).to_string();
                        let length = head
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if buffer.len() < position + 4 + length {
                            continue;
                        }
                        let body = serde_json::json!({
                            "compatibilityDate": "2026-11-01",
                            "mainModule": "index.js",
                            "modules": { "index.js": WS_PLUGIN_JS },
                        })
                        .to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        return;
                    }
                }
            });
        }
    });
    addr
}

async fn websocket_handshake(stream: &mut tokio::net::TcpStream, path: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: shop.example.com\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream.read(&mut byte).await.unwrap();
        assert_eq!(read, 1);
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
}

async fn read_ws_text(stream: &mut tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt;
    let mut header = [0_u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    let mut length = usize::from(header[1] & 0x7f);
    if length == 126 {
        let mut extended = [0_u8; 2];
        stream.read_exact(&mut extended).await.unwrap();
        length = usize::from(u16::from_be_bytes(extended));
    }
    assert_eq!(header[1] & 0x80, 0, "server frames are unmasked");
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await.unwrap();
    String::from_utf8(payload).unwrap()
}

async fn write_ws_text(stream: &mut tokio::net::TcpStream, text: &str) {
    use tokio::io::AsyncWriteExt;
    let payload = text.as_bytes();
    let mask = [1_u8, 2, 3, 4];
    let mut frame = vec![0x81, 0x80 | u8::try_from(payload.len()).unwrap()];
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream.write_all(&frame).await.unwrap();
}

#[tokio::test]
async fn websocket_upgrade_is_tunnelled_to_the_plugin() {
    let binary = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/workerd/bin/workerd");
    if !binary.is_file() {
        eprintln!("skipping: pinned workerd binary is absent");
        return;
    }
    let commerce = start_ws_commerce().await;
    let fixture = fixture_options(true, Some(commerce)).await;
    let headers = fixture.headers();
    let bytes = archive(&manifest("demo", "1.0.0", &[]), WS_PLUGIN_JS);
    let (status, _) = upload(
        &fixture.app,
        "/api/v1/admin/plugins/install",
        &headers,
        &bytes,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/plugins/demo/enable",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = fixture.app.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        websocket_handshake(&mut stream, "/apps/demo/ws"),
    )
    .await
    .expect("handshake timeout");
    let welcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        read_ws_text(&mut stream),
    )
    .await
    .expect("welcome timeout");
    assert_eq!(welcome, "welcome");

    write_ws_text(&mut stream, "hello").await;
    let echo = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        read_ws_text(&mut stream),
    )
    .await
    .expect("echo timeout");
    assert_eq!(echo, "echo:hello");
}
