//! Response construction, asset serving and cache helpers.

use super::*;

pub(super) async fn serve_asset(
    root: &std::path::Path,
    path: &str,
    headers: &axum::http::HeaderMap,
) -> Response {
    use axum::http::header::{
        ACCEPT_RANGES, CACHE_CONTROL, CONTENT_RANGE, ETAG, IF_NONE_MATCH, RANGE,
    };

    let mut relative = std::path::PathBuf::new();
    for component in std::path::Path::new(path).components() {
        match component {
            std::path::Component::Normal(segment) => relative.push(segment),
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    }
    if relative.as_os_str().is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(root) = tokio::fs::canonicalize(root).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(full) = tokio::fs::canonicalize(root.join(&relative)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !full.starts_with(&root) || !full.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(metadata) = tokio::fs::metadata(&full).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let len = metadata.len();
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs());
    let etag = format!("\"{len:x}-{modified:x}\"");
    let cache_control = "public, max-age=31536000, immutable";
    if headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag))
    {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(ETAG, &etag)
            .header(CACHE_CONTROL, cache_control)
            .header(ACCEPT_RANGES, "bytes")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::NOT_MODIFIED.into_response());
    }
    let Ok(bytes) = tokio::fs::read(&full).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let range = headers
        .get(RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, len));
    let (status, body, content_range) = match range {
        Some(Ok((start, end))) => (
            StatusCode::PARTIAL_CONTENT,
            bytes[start..=end].to_vec(),
            Some(format!("bytes {start}-{end}/{len}")),
        ),
        Some(Err(())) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(CONTENT_RANGE, format!("bytes */{len}"))
                .body(Body::empty())
                .unwrap_or_else(|_| StatusCode::RANGE_NOT_SATISFIABLE.into_response());
        }
        None => (StatusCode::OK, bytes, None),
    };
    let mut builder = Response::builder()
        .status(status)
        .header(axum::http::header::CONTENT_TYPE, content_type(&full))
        .header(CACHE_CONTROL, cache_control)
        .header(ETAG, &etag)
        .header(ACCEPT_RANGES, "bytes");
    if let Some(content_range) = content_range {
        builder = builder.header(CONTENT_RANGE, content_range);
    }
    builder.body(Body::from(body)).unwrap_or_else(|error| {
        tracing::error!(error = %error, "failed to build asset response");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
}

/// Parses one byte range. Returns `Ok((start, end))` or `Err` when unsatisfiable.
fn parse_range(value: &str, len: u64) -> Option<Result<(usize, usize), ()>> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') {
        return Some(Err(()));
    }
    let (start, end) = value.split_once('-')?;
    if len == 0 {
        return Some(Err(()));
    }
    let (start, end) = if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        if suffix == 0 {
            return Some(Err(()));
        }
        let suffix = suffix.min(len);
        (len - suffix, len - 1)
    } else {
        let start: u64 = start.parse().ok()?;
        let end = if end.is_empty() {
            len - 1
        } else {
            end.parse::<u64>().ok()?.min(len - 1)
        };
        if start > end || start >= len {
            return Some(Err(()));
        }
        (start, end)
    };
    Some(Ok((
        usize::try_from(start).ok()?,
        usize::try_from(end).ok()?,
    )))
}

pub(super) fn cache_key(request: &axum::http::Request<Body>, store_id: &str) -> Option<String> {
    if request.method() != axum::http::Method::GET {
        return None;
    }
    if request
        .headers()
        .contains_key(axum::http::header::AUTHORIZATION)
        || request.headers().contains_key(axum::http::header::COOKIE)
        || request
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("no-cache"))
    {
        return None;
    }
    let language = request
        .headers()
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    Some(format!(
        "{store_id}:{}:{language}",
        request
            .uri()
            .path_and_query()
            .map_or("/", axum::http::uri::PathAndQuery::as_str)
    ))
}

pub(super) fn cache_ttl(headers: &axum::http::HeaderMap) -> Option<Duration> {
    if headers.contains_key(axum::http::header::SET_COOKIE) {
        return None;
    }
    let value = headers
        .get(axum::http::header::CACHE_CONTROL)?
        .to_str()
        .ok()?;
    let lower = value.to_ascii_lowercase();
    if lower.contains("no-store") || lower.contains("private") {
        return None;
    }
    let mut ttl = None;
    for part in lower.split(',') {
        let part = part.trim();
        let Some(seconds) = part
            .strip_prefix("s-maxage=")
            .or_else(|| part.strip_prefix("max-age="))
        else {
            continue;
        };
        let Ok(seconds) = seconds.trim().parse::<u64>() else {
            continue;
        };
        ttl = Some(Duration::from_secs(seconds));
        if part.starts_with("s-maxage=") {
            break;
        }
    }
    ttl.filter(|ttl| !ttl.is_zero())
}

pub(super) async fn buffer_body(body: SsrBody) -> Option<bytes::Bytes> {
    match body {
        SsrBody::Full(bytes) => (bytes.len() <= MAX_CACHE_ENTRY_BYTES).then_some(bytes),
        SsrBody::Stream(mut stream) => {
            let mut output = bytes::BytesMut::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.ok()?;
                if output.len() + chunk.len() > MAX_CACHE_ENTRY_BYTES {
                    return None;
                }
                output.extend_from_slice(&chunk);
            }
            Some(output.freeze())
        }
    }
}

fn content_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "html" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub(super) fn into_response(response: SsrResponse) -> Response {
    let mut builder = Response::builder().status(response.status);
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in &response.headers {
            headers.append(name, value.clone());
        }
        if !headers.contains_key(axum::http::header::CACHE_CONTROL) {
            headers.insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
        }
    }
    let body = match response.body {
        SsrBody::Full(bytes) => Body::from(bytes),
        SsrBody::Stream(stream) => Body::from_stream(stream),
    };
    builder.body(body).unwrap_or_else(|error| {
        tracing::error!(error = %error, "failed to build SSR response");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
}

pub(super) fn error_response(error: &SsrError) -> Response {
    let (status, code) = match error {
        SsrError::Health(_)
        | SsrError::NotRunning
        | SsrError::StartupTimeout(_)
        | SsrError::EarlyExit(_)
        | SsrError::Start(_) => (StatusCode::SERVICE_UNAVAILABLE, "SSR_RUNTIME_UNAVAILABLE"),
        SsrError::InvalidRequest(_)
        | SsrError::InvalidRuntimeConfig(_)
        | SsrError::UnknownGroup(_)
        | SsrError::AlreadyRunning(_)
        | SsrError::RestartLimit(_) => (StatusCode::BAD_GATEWAY, "SSR_RUNTIME_ERROR"),
        _ => (StatusCode::BAD_GATEWAY, "SSR_REQUEST_FAILED"),
    };
    tracing::warn!(error = %error, status = %status, "universal SSR request failed");
    (status, axum::Json(json!({ "error": code }))).into_response()
}
