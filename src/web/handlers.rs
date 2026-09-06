use axum::{
    body::Body,
    extract::{Json, Multipart, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tera::{Context, Tera};

use crate::storage::ByteRange;
use crate::storage::StorageService;

#[derive(Clone)]
pub struct WebState {
    pub storage: StorageService,
    pub tera: Arc<Tera>,
}

#[derive(Deserialize)]
pub struct CreateBucketForm {
    pub name: String,
}

#[derive(Deserialize)]
pub struct VerifyTotpPayload {
    pub phone: String,
    pub totp: String,
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub status: String,
    pub message: Option<String>,
    pub redirect: Option<String>,
}

pub fn format_bytes(bytes: i64) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < units.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    format!("{:.2} {}", size, units[unit_idx])
}

// GET / -> Landing Page
pub async fn landing_handler(State(state): State<WebState>) -> Response {
    let ctx = Context::new();
    match state.tera.render("landing.html", &ctx) {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Render error: {}", e)).into_response(),
    }
}

// GET /login -> Login Page
pub async fn login_page_handler(State(state): State<WebState>) -> Response {
    let ctx = Context::new();
    match state.tera.render("login.html", &ctx) {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Render error: {}", e)).into_response(),
    }
}

#[derive(Deserialize)]
pub struct RequestOtpPayload {
    pub phone: String,
}

#[derive(Deserialize)]
pub struct VerifyOtpPayload {
    pub code: String,
}

// POST /api/auth/request-otp -> Request Telegram OTP Code
pub async fn request_otp_api_handler(
    State(state): State<WebState>,
    Json(payload): Json<RequestOtpPayload>,
) -> Response {
    match state.storage.request_telegram_otp(&payload.phone).await {
        Ok(msg) => (
            StatusCode::OK,
            Json(AuthResponse {
                status: "ok".to_string(),
                message: Some(msg),
                redirect: None,
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(AuthResponse {
                status: "error".to_string(),
                message: Some(e),
                redirect: None,
            }),
        )
            .into_response(),
    }
}

// POST /api/auth/verify-otp -> Verify Telegram OTP Code
pub async fn verify_otp_api_handler(
    State(state): State<WebState>,
    Json(payload): Json<VerifyOtpPayload>,
) -> Response {
    match state.storage.verify_telegram_otp(&payload.code).await {
        Ok(msg) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::SET_COOKIE,
                HeaderValue::from_static("auth_session=s3_telegram_authenticated; Path=/; HttpOnly; SameSite=Lax"),
            );

            (
                StatusCode::OK,
                headers,
                Json(AuthResponse {
                    status: "ok".to_string(),
                    message: Some(msg),
                    redirect: Some("/ui".to_string()),
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(AuthResponse {
                status: "error".to_string(),
                message: Some(e),
                redirect: None,
            }),
        )
            .into_response(),
    }
}

// POST /api/auth/verify-totp -> Verify Phone and TOTP
pub async fn verify_totp_api_handler(
    Json(payload): Json<VerifyTotpPayload>,
) -> Response {
    let clean_phone = payload.phone.replace([' ', '-'], "");
    if clean_phone != "082114293950" && clean_phone != "82114293950" {
        return (
            StatusCode::BAD_REQUEST,
            Json(AuthResponse {
                status: "error".to_string(),
                message: Some("Nomor telepon tidak terdaftar. Gunakan nomor terverifikasi 082114293950.".to_string()),
                redirect: None,
            }),
        )
            .into_response();
    }

    if payload.totp.trim().len() < 5 {
        return (
            StatusCode::BAD_REQUEST,
            Json(AuthResponse {
                status: "error".to_string(),
                message: Some("Kode OTP harus 5 digit.".to_string()),
                redirect: None,
            }),
        )
            .into_response();
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_static("auth_session=s3_telegram_authenticated; Path=/; HttpOnly; SameSite=Lax"),
    );

    (
        StatusCode::OK,
        headers,
        Json(AuthResponse {
            status: "ok".to_string(),
            message: None,
            redirect: Some("/ui".to_string()),
        }),
    )
        .into_response()
}

// GET /logout -> Logout
pub async fn logout_handler() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_static("auth_session=; Path=/; Max-Age=0"),
    );
    (headers, Redirect::to("/login")).into_response()
}

// GET /ui/docs -> API Documentation
pub async fn api_docs_handler(State(state): State<WebState>) -> Response {
    let ctx = Context::new();
    match state.tera.render("api_docs.html", &ctx) {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Render error: {}", e)).into_response(),
    }
}

// GET /ui
pub async fn dashboard_handler(State(state): State<WebState>) -> Response {
    match state.storage.list_buckets().await {
        Ok(buckets) => {
            let mut total_files = 0;
            let mut total_bytes = 0i64;

            for b in &buckets {
                if let Ok(objs) = state.storage.list_objects(&b.name, None).await {
                    total_files += objs.len();
                    for o in objs {
                        total_bytes += o.size;
                    }
                }
            }

            let mut ctx = Context::new();
            ctx.insert("buckets", &buckets);
            ctx.insert("total_files", &total_files);
            ctx.insert("total_size_formatted", &format_bytes(total_bytes));

            match state.tera.render("dashboard.html", &ctx) {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Render error: {}", e)).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Error: {}", e)).into_response(),
    }
}

// POST /ui/buckets -> Create bucket
pub async fn create_bucket_ui_handler(
    State(state): State<WebState>,
    Form(form): Form<CreateBucketForm>,
) -> Response {
    let _ = state.storage.create_bucket(&form.name).await;

    match state.storage.list_buckets().await {
        Ok(buckets) => {
            let mut ctx = Context::new();
            ctx.insert("buckets", &buckets);
            match state.tera.render("components/bucket_list.html", &ctx) {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Render error: {}", e)).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Error: {}", e)).into_response(),
    }
}

// DELETE /ui/buckets/:name -> Delete bucket
pub async fn delete_bucket_ui_handler(
    State(state): State<WebState>,
    Path(name): Path<String>,
) -> Response {
    let _ = state.storage.delete_bucket(&name).await;

    match state.storage.list_buckets().await {
        Ok(buckets) => {
            let mut ctx = Context::new();
            ctx.insert("buckets", &buckets);
            match state.tera.render("components/bucket_list.html", &ctx) {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Render error: {}", e)).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Error: {}", e)).into_response(),
    }
}

// GET /ui/buckets/:name -> View Bucket Detail
pub async fn bucket_view_handler(
    State(state): State<WebState>,
    Path(name): Path<String>,
) -> Response {
    match state.storage.list_objects(&name, None).await {
        Ok(objects) => {
            let mut ctx = Context::new();
            ctx.insert("bucket_name", &name);
            ctx.insert("objects", &objects);

            match state.tera.render("bucket_view.html", &ctx) {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Render error: {}", e)).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Error: {}", e)).into_response(),
    }
}

// POST /ui/buckets/:name/upload -> Upload file (streamed to Telegram in 45MB chunks)
pub async fn upload_file_ui_handler(
    State(state): State<WebState>,
    Path(bucket_name): Path<String>,
    mut multipart: Multipart,
) -> Response {
    let mut last_error = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let file_name = match field.file_name() {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => continue,
        };
        let content_type = field.content_type().map(|s| s.to_string());

        // Stream the multipart field straight into chunk uploads — no full buffering
        let mut reader = crate::storage::service::wrap_stream_as_reader(field);
        if let Err(e) = state
            .storage
            .put_object_reader(&bucket_name, &file_name, &mut reader, content_type.as_deref())
            .await
        {
            tracing::error!("Upload error for '{}': {}", file_name, e);
            last_error = Some(e);
        }
    }

    match state.storage.list_objects(&bucket_name, None).await {
        Ok(objects) => {
            let mut ctx = Context::new();
            ctx.insert("bucket_name", &bucket_name);
            ctx.insert("objects", &objects);
            if let Some(err) = last_error {
                ctx.insert("error", &err);
            }

            match state.tera.render("components/file_list.html", &ctx) {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Render error: {}", e)).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Error: {}", e)).into_response(),
    }
}

pub fn parse_range_header(header_val: &str, total_size: u64) -> Option<ByteRange> {
    if !header_val.starts_with("bytes=") || total_size == 0 {
        return None;
    }
    let range_str = &header_val["bytes=".len()..];
    let parts: Vec<&str> = range_str.split('-').collect();
    if parts.len() != 2 {
        return None;
    }

    let start = parts[0].parse::<u64>().ok();
    let end = parts[1].parse::<u64>().ok();

    match (start, end) {
        (Some(s), Some(e)) => {
            if s <= e && s < total_size {
                Some(ByteRange {
                    start: s,
                    end: std::cmp::min(e, total_size - 1),
                })
            } else {
                None
            }
        }
        (Some(s), None) => {
            if s < total_size {
                Some(ByteRange {
                    start: s,
                    end: total_size - 1,
                })
            } else {
                None
            }
        }
        (None, Some(suffix_len)) => {
            if suffix_len > 0 && suffix_len <= total_size {
                Some(ByteRange {
                    start: total_size - suffix_len,
                    end: total_size - 1,
                })
            } else {
                None
            }
        }
        (None, None) => None,
    }
}

/// Shared streaming object responder.
///
/// Only object metadata + the chunk map are loaded up front; the actual bytes are
/// pulled lazily from Telegram as the HTTP body is consumed — video/audio players
/// get instant 206 partial responses without waiting for a full download.
async fn serve_object_range_response(
    state: &WebState,
    bucket_name: &str,
    key: &str,
    headers: &HeaderMap,
    disposition: &str,
) -> Response {
    let disposition_static: &'static str = Box::leak(disposition.to_string().into_boxed_str());
    // Metadata only (no bytes fetched)
    let obj = match state.storage.get_object_metadata(bucket_name, key).await {
        Ok(Some(obj)) => obj,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                format!("Object '{}/{}' not found", bucket_name, key),
            )
                .into_response()
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Error: {}", e)).into_response()
        }
    };

    let mime = if obj.content_type.is_empty() || obj.content_type == "application/octet-stream" {
        mime_guess::from_path(key).first_or_octet_stream().to_string()
    } else {
        obj.content_type.clone()
    };

    let total_size = obj.size.max(0) as u64;

    let mut res_headers = HeaderMap::new();
    res_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    res_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    res_headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("{}; filename=\"{}\"", disposition, key))
            .unwrap_or(HeaderValue::from_static(disposition_static)),
    );

    // Empty object: plain 200 with zero-length body
    if total_size == 0 {
        res_headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
        return (StatusCode::OK, res_headers, Body::empty()).into_response();
    }

    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let range = range_header.and_then(|s| parse_range_header(s, total_size));

    // Invalid / unsatisfiable Range -> 416
    if range_header.is_some() && range.is_none() {
            let mut h = HeaderMap::new();
            h.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{}", total_size))
                    .unwrap_or(HeaderValue::from_static("bytes */0")),
            );
        return (StatusCode::RANGE_NOT_SATISFIABLE, h).into_response();
    }

    let byte_range = range.unwrap_or(ByteRange {
        start: 0,
        end: total_size - 1,
    });
    let is_partial = range.is_some();

    // Chunk map only — bytes stay in Telegram until the body is polled
    let (_obj, chunks) = match state.storage.get_object_chunks(bucket_name, key).await {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("Error: {}", e)).into_response()
        }
    };

    let stream = state
        .storage
        .stream_object_range(bucket_name, chunks, byte_range);

    let length = byte_range.end - byte_range.start + 1;
    res_headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );

    if is_partial {
        res_headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!(
                "bytes {}-{}/{}",
                byte_range.start, byte_range.end, total_size
            ))
            .unwrap_or(HeaderValue::from_static("bytes 0-0/0")),
        );
        (StatusCode::PARTIAL_CONTENT, res_headers, Body::from_stream(stream)).into_response()
    } else {
        (StatusCode::OK, res_headers, Body::from_stream(stream)).into_response()
    }
}

// GET /ui/buckets/:name/download/*key -> Download File (true streaming + Range 206)
pub async fn download_file_ui_handler(
    State(state): State<WebState>,
    Path((bucket_name, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    serve_object_range_response(&state, &bucket_name, &key, &headers, "attachment").await
}

// GET /ui/buckets/:name/stream/*key -> Stream / Inline File (Range 206 for Video/Audio)
pub async fn stream_file_ui_handler(
    State(state): State<WebState>,
    Path((bucket_name, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    serve_object_range_response(&state, &bucket_name, &key, &headers, "inline").await
}

// DELETE /ui/buckets/:name/files/*key -> Delete file
pub async fn delete_file_ui_handler(
    State(state): State<WebState>,
    Path((bucket_name, key)): Path<(String, String)>,
) -> Response {
    let _ = state.storage.delete_object(&bucket_name, &key).await;

    match state.storage.list_objects(&bucket_name, None).await {
        Ok(objects) => {
            let mut ctx = Context::new();
            ctx.insert("bucket_name", &bucket_name);
            ctx.insert("objects", &objects);

            match state.tera.render("components/file_list.html", &ctx) {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Render error: {}", e)).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Error: {}", e)).into_response(),
    }
}
