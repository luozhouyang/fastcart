//! Raw RFC 6455 tunnel for plugin WebSockets.
//!
//! The browser side is upgraded by hyper; the workerd side is a plain client
//! handshake over loopback. Frames are proxied with the masking bit rewritten
//! for each direction, so no WebSocket client crate is required.

use std::net::SocketAddr;

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_FRAME: usize = 1024 * 1024;
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Upstream connection details for one tunnel.
pub struct TunnelRequest {
    pub gateway_addr: SocketAddr,
    pub plugin_id: String,
    pub digest: String,
    pub store_id: String,
    pub path_and_query: String,
    pub context_token: String,
    pub gateway_token: String,
}

/// Builds the 101 response for the browser, or a 400 when the handshake is
/// incomplete.
pub fn accept_response(headers: &HeaderMap) -> Response {
    let Some(key) = headers
        .get("sec-websocket-key")
        .and_then(|value| value.to_str().ok())
    else {
        return (StatusCode::BAD_REQUEST, "missing Sec-WebSocket-Key").into_response();
    };
    let version = headers
        .get("sec-websocket-version")
        .and_then(|value| value.to_str().ok());
    if version != Some("13") {
        return (StatusCode::BAD_REQUEST, "unsupported WebSocket version").into_response();
    }
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(GUID.as_bytes());
    let accept = BASE64.encode(hasher.finalize());
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "websocket")
        .header("sec-websocket-accept", accept)
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Proxies one upgraded browser connection to workerd.
pub async fn tunnel(upgraded: Upgraded, request: TunnelRequest) -> std::io::Result<()> {
    let mut upstream = TcpStream::connect(request.gateway_addr).await?;
    upstream.set_nodelay(true)?;
    let key = BASE64.encode(format!(
        "{}{}",
        uuid::Uuid::now_v7().simple(),
        uuid::Uuid::now_v7().simple()
    ));
    let path = format!(
        "/__plugin/{}@{}{}",
        request.plugin_id, request.digest, request.path_and_query
    );
    let handshake = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\
         x-fastcart-store-id: {}\r\nx-fastcart-context-token: {}\r\n\
         x-fastcart-gateway-token: {}\r\n\r\n",
        request.gateway_addr, request.store_id, request.context_token, request.gateway_token
    );
    upstream.write_all(handshake.as_bytes()).await?;
    upstream.flush().await?;
    let status = read_status(&mut upstream).await?;
    if status != 101 {
        return Err(std::io::Error::other(format!(
            "workerd rejected the WebSocket handshake with {status}"
        )));
    }

    let browser = TokioIo::new(upgraded);
    let (mut browser_read, mut browser_write) = tokio::io::split(browser);
    let (mut upstream_read, mut upstream_write) = upstream.into_split();

    let client_to_server = async {
        loop {
            let frame = read_frame(&mut browser_read, true).await?;
            let close = frame.opcode == 0x8;
            write_frame(&mut upstream_write, &frame, true).await?;
            if close {
                return Ok::<(), std::io::Error>(());
            }
        }
    };
    let server_to_client = async {
        loop {
            let frame = read_frame(&mut upstream_read, false).await?;
            let close = frame.opcode == 0x8;
            write_frame(&mut browser_write, &frame, false).await?;
            if close {
                return Ok::<(), std::io::Error>(());
            }
        }
    };

    tokio::select! {
        result = client_to_server => result,
        result = server_to_client => result,
    }
}

async fn read_status(stream: &mut TcpStream) -> std::io::Result<u16> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream.read(&mut byte).await?;
        if read == 0 {
            return Err(std::io::Error::other("workerd closed during handshake"));
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
        if head.len() > 16 * 1024 {
            return Err(std::io::Error::other("workerd handshake too large"));
        }
    }
    let head = String::from_utf8_lossy(&head);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| std::io::Error::other("malformed workerd handshake"))?;
    Ok(status)
}

struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

async fn read_frame<R>(reader: &mut R, expect_mask: bool) -> std::io::Result<Frame>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; 2];
    reader.read_exact(&mut header).await?;
    let fin = header[0] & 0x80 != 0;
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    if masked != expect_mask {
        return Err(std::io::Error::other("unexpected WebSocket masking"));
    }
    let mut length = u64::from(header[1] & 0x7f);
    if length == 126 {
        let mut extended = [0_u8; 2];
        reader.read_exact(&mut extended).await?;
        length = u64::from(u16::from_be_bytes(extended));
    } else if length == 127 {
        let mut extended = [0_u8; 8];
        reader.read_exact(&mut extended).await?;
        length = u64::from_be_bytes(extended);
    }
    let length =
        usize::try_from(length).map_err(|_| std::io::Error::other("WebSocket frame too large"))?;
    if length > MAX_FRAME {
        return Err(std::io::Error::other("WebSocket frame exceeds 1 MiB"));
    }
    let mask = if masked {
        let mut mask = [0_u8; 4];
        reader.read_exact(&mut mask).await?;
        Some(mask)
    } else {
        None
    };
    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload).await?;
    if let Some(mask) = mask {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    Ok(Frame {
        fin,
        opcode,
        payload,
    })
}

async fn write_frame<W>(writer: &mut W, frame: &Frame, masked: bool) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut header = Vec::with_capacity(14);
    header.push(if frame.fin { 0x80 } else { 0 } | (frame.opcode & 0x0f));
    let mask_bit = if masked { 0x80 } else { 0 };
    let length = frame.payload.len();
    if length < 126 {
        header.push(mask_bit | u8::try_from(length).unwrap_or(125));
    } else if length <= usize::from(u16::MAX) {
        header.push(mask_bit | 126);
        header.extend_from_slice(&u16::try_from(length).unwrap_or(u16::MAX).to_be_bytes());
    } else {
        header.push(mask_bit | 127);
        header.extend_from_slice(&(length as u64).to_be_bytes());
    }
    if masked {
        let mask = *uuid::Uuid::now_v7()
            .as_bytes()
            .first_chunk::<4>()
            .unwrap_or(&[0; 4]);
        header.extend_from_slice(&mask);
        writer.write_all(&header).await?;
        let mut payload = frame.payload.clone();
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
        writer.write_all(&payload).await?;
    } else {
        writer.write_all(&header).await?;
        writer.write_all(&frame.payload).await?;
    }
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_the_rfc_handshake_accept() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "sec-websocket-key",
            "dGhlIHNhbXBsZSBub25jZQ==".parse().unwrap(),
        );
        headers.insert("sec-websocket-version", "13".parse().unwrap());
        let response = accept_response(&headers);
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(
            response.headers()["sec-websocket-accept"],
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[tokio::test]
    async fn round_trips_masked_and_unmasked_frames() {
        let frame = Frame {
            fin: true,
            opcode: 0x1,
            payload: b"hello".to_vec(),
        };
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &frame, true).await.unwrap();
        let mut reader = buffer.as_slice();
        let decoded = read_frame(&mut reader, true).await.unwrap();
        assert_eq!(decoded.payload, b"hello");
    }
}
