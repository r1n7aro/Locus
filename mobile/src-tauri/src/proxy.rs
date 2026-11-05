//! `locus-remote` URI scheme: the WebView loads the desktop's mobile page
//! through this proxy, so every byte travels over the pinned TLS connection
//! and the device token never enters the page.

use tauri::http::{header, Request, Response, StatusCode};
use locus_remote_protocol as protocol;
use tauri::{AppHandle, Manager};

use crate::asset_cache::{self, CachedAsset};
use crate::commands::AppState;

pub const SCHEME: &str = "locus-remote";
/// Binary previews (`http://locus-binary.localhost/blob/{uuid}` in command
/// results, the desktop UI's own scheme) are fetched from the desktop's blob
/// route.
pub const BINARY_SCHEME: &str = "locus-binary";

/// Response headers relayed from the desktop; everything else is dropped.
const RELAYED_HEADERS: &[&str] = &[
    "content-type",
    "content-security-policy",
    "cache-control",
    "x-content-type-options",
    "referrer-policy",
];

/// URL of the remote page inside the WebView.
pub fn page_url() -> &'static str {
    if cfg!(any(windows, target_os = "android")) {
        "http://locus-remote.localhost/m/"
    } else {
        "locus-remote://localhost/m/"
    }
}

fn plain(status: StatusCode, body: String) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(body.into_bytes())
        .expect("static response builds")
}

fn unavailable(message: &str) -> Response<Vec<u8>> {
    let message = message.replace('&', "&amp;").replace('<', "&lt;");
    plain(
        StatusCode::SERVICE_UNAVAILABLE,
        format!(
            "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
             <body style=\"font:15px system-ui;padding:24px\"><p>{message}</p>\
             <p><a href=\"/__locus_remote/leave\">返回设备列表</a></p></body>"
        ),
    )
}

fn cached(asset: CachedAsset) -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, asset.content_type)
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(asset.body)
        .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, String::new()))
}

pub async fn handle(app: &AppHandle, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    if request.method() != tauri::http::Method::GET {
        return plain(StatusCode::METHOD_NOT_ALLOWED, String::new());
    }
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let state = app.state::<AppState>();
    if path_and_query.starts_with("/__locus_remote/leave") {
        state.leave(app);
        return plain(StatusCode::NO_CONTENT, String::new());
    }
    let Some(connection) = state.active_connection() else {
        return unavailable("尚未连接电脑。");
    };
    // Content-hashed bundle files: served locally, even while (re)connecting.
    let cacheable = asset_cache::cacheable_request(&path_and_query);
    if cacheable {
        if let Some(asset) = state.cache.get(&connection.desktop_id, &path_and_query).await {
            return cached(asset);
        }
    }
    let (client, token) = match connection.asset_client().await {
        Ok(pair) => pair,
        Err(error) => return unavailable(&error),
    };
    let upstream = match client.get_asset(&path_and_query, &token).await {
        Ok(asset) => asset,
        Err(error) => return unavailable(&error.to_string()),
    };
    let header_str = |name: &str| upstream.headers.get(name).and_then(|value| value.to_str().ok());
    if cacheable && asset_cache::cacheable_response(upstream.status.as_u16(), header_str("cache-control")) {
        if let Some(content_type) = header_str("content-type") {
            state.cache.put(&connection.desktop_id, &path_and_query, content_type, upstream.body.clone());
        }
    }
    let status = StatusCode::from_u16(upstream.status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for name in RELAYED_HEADERS {
        if let Some(value) = upstream.headers.get(*name) {
            builder = builder.header(*name, value.as_bytes());
        }
    }
    builder
        .body(upstream.body)
        .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, String::new()))
}

fn is_blob_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// `locus-binary` scheme: images and media the page embeds. Blobs expire on
/// the desktop after a few minutes, so they are not kept on disk.
pub async fn handle_binary(app: &AppHandle, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    if request.method() != tauri::http::Method::GET {
        return plain(StatusCode::METHOD_NOT_ALLOWED, String::new());
    }
    let Some(id) = request.uri().path().strip_prefix("/blob/").filter(|id| is_blob_id(id)) else {
        return plain(StatusCode::NOT_FOUND, String::new());
    };
    let state = app.state::<AppState>();
    let Some(connection) = state.active_connection() else {
        return plain(StatusCode::SERVICE_UNAVAILABLE, String::new());
    };
    let (client, token) = match connection.asset_client().await {
        Ok(pair) => pair,
        Err(_) => return plain(StatusCode::SERVICE_UNAVAILABLE, String::new()),
    };
    let upstream = match client.get_asset(&format!("{}{id}", protocol::BLOB_PREFIX), &token).await {
        Ok(asset) => asset,
        Err(_) => return plain(StatusCode::BAD_GATEWAY, String::new()),
    };
    if !upstream.status.is_success() {
        return plain(StatusCode::NOT_FOUND, String::new());
    }
    let content_type = upstream
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "private, max-age=600")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        // Never a document: an opened blob cannot run script or reach the IPC.
        .header("content-security-policy", "sandbox; default-src 'none'")
        .body(upstream.body)
        .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, String::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_uuid_blob_ids_are_forwarded() {
        assert!(is_blob_id("0f8fad5b-d9cb-469f-a165-70867728950e"));
        assert!(!is_blob_id("../remote/v1/ws"));
        assert!(!is_blob_id("0f8fad5b-d9cb-469f-a165-70867728950e/../x"));
    }
}
