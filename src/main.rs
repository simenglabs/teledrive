use axum::{
    extract::{DefaultBodyLimit, State},
    http::HeaderMap,
    response::Response,
    routing::{delete, get, post, put},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tera::Tera;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod crypto;
mod db;
mod s3;
mod storage;
mod telegram;
mod web;

use config::Config;
use db::DbRepo;
use storage::StorageService;
use telegram::TelegramBot;
use web::WebState;

async fn root_handler(
    State(web_state): State<WebState>,
    headers: HeaderMap,
) -> Response {
    if let Some(accept) = headers.get(axum::http::header::ACCEPT) {
        if let Ok(accept_str) = accept.to_str() {
            if accept_str.contains("application/xml") || accept_str.contains("text/xml") {
                return s3::list_buckets_handler(State(web_state.storage)).await;
            }
        }
    }
    web::landing_handler(State(web_state)).await
}

use std::collections::HashMap;
use tera::{to_value, Value, Result as TeraResult};

fn truncate_filename(name: &str) -> String {
    let (base, ext) = match name.rfind('.') {
        Some(idx) if idx > 0 => (&name[..idx], &name[idx..]),
        _ => (name, ""),
    };

    let chars: Vec<char> = base.chars().collect();
    if chars.len() <= 13 {
        name.to_string()
    } else {
        let first_10: String = chars[..10].iter().collect();
        let last_3: String = chars[chars.len() - 3..].iter().collect();
        format!("{}...{}{}", first_10, last_3, ext)
    }
}

fn truncate_filename_filter(value: &Value, _args: &HashMap<String, Value>) -> TeraResult<Value> {
    let name = match value.as_str() {
        Some(s) => s,
        None => return Ok(value.clone()),
    };

    Ok(to_value(truncate_filename(name)).unwrap())
}

fn format_bytes_filter(value: &Value, _args: &HashMap<String, Value>) -> TeraResult<Value> {
    let bytes = match value.as_i64().or_else(|| value.as_f64().map(|f| f as i64)) {
        Some(b) => b,
        None => return Ok(value.clone()),
    };

    Ok(to_value(web::format_bytes(bytes)).unwrap())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg = Config::from_env().map_err(|e| format!("Config error: {}", e))?;
    tracing::info!("Starting Telegram S3 Storage Engine on port {}...", cfg.server_port);

    // Initialize Turso / libSQL DB
    let db_repo = DbRepo::new(&cfg.turso_database_url, cfg.turso_auth_token.as_deref())
        .await
        .map_err(|e| format!("Database init failed: {}", e))?;

    // Initialize Telegram Client (User MTProto mode with API_ID & Turso DB Session Persistence)
    let bot = TelegramBot::connect(
        cfg.telegram_api_id,
        cfg.telegram_api_hash.clone(),
        db_repo.clone(),
    )
    .await
    .map_err(|e| format!("Telegram connect failed: {}", e))?;

    // Initialize Storage Service
    let storage = StorageService::new(db_repo, bot, cfg.master_encryption_key);

    // Initialize Tera Templates
    let tera = match Tera::new("templates/**/*") {
        Ok(mut t) => {
            t.register_filter("truncate_filename", truncate_filename_filter);
            t.register_filter("format_bytes", format_bytes_filter);
            Arc::new(t)
        }
        Err(e) => {
            tracing::error!("Template parsing error: {}", e);
            std::process::exit(1);
        }
    };

    let web_state = WebState {
        storage: storage.clone(),
        tera,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Web Dashboard & Auth Routes
    let web_routes = Router::new()
        .route("/login", get(web::login_page_handler))
        .route("/api/auth/request-otp", post(web::request_otp_api_handler))
        .route("/api/auth/verify-otp", post(web::verify_otp_api_handler))
        .route("/api/auth/verify-totp", post(web::verify_totp_api_handler))
        .route("/logout", get(web::logout_handler))
        .route("/ui", get(web::dashboard_handler))
        .route("/ui/docs", get(web::api_docs_handler))
        .route("/ui/buckets", post(web::create_bucket_ui_handler))
        .route("/ui/buckets/:name", get(web::bucket_view_handler).delete(web::delete_bucket_ui_handler))
        .route("/ui/buckets/:name/upload", post(web::upload_file_ui_handler))
        .route("/ui/buckets/:name/download/*key", get(web::download_file_ui_handler))
        .route("/ui/buckets/:name/stream/*key", get(web::stream_file_ui_handler))
        .route("/ui/buckets/:name/files/*key", delete(web::delete_file_ui_handler))
        .with_state(web_state.clone());

    // S3 Compatibility API Routes
    let s3_routes = Router::new()
        .route("/:bucket", put(s3::create_bucket_handler).delete(s3::delete_bucket_handler).get(s3::list_objects_handler))
        .route("/:bucket/*key", put(s3::put_object_handler).get(s3::get_object_handler).delete(s3::delete_object_handler).head(s3::head_object_handler))
        .with_state(storage.clone());

    let app = Router::new()
        .route("/", get(root_handler).with_state(web_state))
        .merge(web_routes)
        .merge(s3_routes)
        .layer(DefaultBodyLimit::disable())
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.server_port));
    tracing::info!("Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
