use axum::{
    body::Body,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct WebAssets;

pub async fn static_handler(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let path = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };

    match WebAssets::get(path).or_else(|| WebAssets::get("index.html")) {
        Some(asset) => {
            let content_type = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();
            let mut response = Response::new(Body::from(asset.data.into_owned()));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(&content_type)
                    .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
            );
            let cache = if path == "index.html" {
                "no-cache"
            } else {
                "public, max-age=31536000, immutable"
            };
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
            response
        }
        None => (StatusCode::NOT_FOUND, "Web assets are not built").into_response(),
    }
}
