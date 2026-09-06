use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::storage::StorageService;
use super::xml_responses::*;

#[derive(Debug, Deserialize)]
pub struct ListObjectsQuery {
    pub prefix: Option<String>,
}

fn s3_xml_response(status: StatusCode, xml_body: String) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    (status, headers, xml_body).into_response()
}

fn s3_error(status: StatusCode, code: &str, message: &str, resource: &str) -> Response {
    let err = S3XmlError {
        code: code.to_string(),
        message: message.to_string(),
        resource: resource.to_string(),
        request_id: "0000000000000000",
    };
    s3_xml_response(status, render_xml(&err))
}

// GET / -> ListAllMyBuckets
pub async fn list_buckets_handler(State(storage): State<StorageService>) -> Response {
    match storage.list_buckets().await {
        Ok(buckets) => {
            let xml_buckets = buckets
                .into_iter()
                .map(|b| XmlBucket {
                    name: b.name,
                    creation_date: b.created_at,
                })
                .collect();

            let result = ListAllMyBucketsResult {
                xmlns: "http://s3.amazonaws.com/doc/2006-03-01/",
                owner: Owner {
                    id: "telegram-s3-owner",
                    display_name: "Telegram Drive",
                },
                buckets: BucketsWrapper { list: xml_buckets },
            };

            s3_xml_response(StatusCode::OK, render_xml(&result))
        }
        Err(e) => s3_error(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", &e, "/"),
    }
}

// PUT /{bucket} -> CreateBucket
pub async fn create_bucket_handler(
    State(storage): State<StorageService>,
    Path(bucket): Path<String>,
) -> Response {
    match storage.create_bucket(&bucket).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => s3_error(StatusCode::BAD_REQUEST, "BucketAlreadyExists", &e, &format!("/{}", bucket)),
    }
}

// DELETE /{bucket} -> DeleteBucket
pub async fn delete_bucket_handler(
    State(storage): State<StorageService>,
    Path(bucket): Path<String>,
) -> Response {
    match storage.delete_bucket(&bucket).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => s3_error(StatusCode::NOT_FOUND, "NoSuchBucket", "Bucket not found", &format!("/{}", bucket)),
        Err(e) => s3_error(StatusCode::CONFLICT, "BucketNotEmpty", &e, &format!("/{}", bucket)),
    }
}

// GET /{bucket} -> ListObjects / ListObjectsV2
pub async fn list_objects_handler(
    State(storage): State<StorageService>,
    Path(bucket): Path<String>,
    Query(query): Query<ListObjectsQuery>,
) -> Response {
    let prefix = query.prefix.as_deref();
    match storage.list_objects(&bucket, prefix).await {
        Ok(objects) => {
            let contents = objects
                .into_iter()
                .map(|o| XmlContent {
                    key: o.key,
                    last_modified: o.created_at,
                    etag: o.etag,
                    size: o.size,
                    storage_class: "STANDARD",
                })
                .collect();

            let result = ListBucketResult {
                xmlns: "http://s3.amazonaws.com/doc/2006-03-01/",
                name: bucket,
                prefix: query.prefix.unwrap_or_default(),
                max_keys: 1000,
                is_truncated: false,
                contents,
            };

            s3_xml_response(StatusCode::OK, render_xml(&result))
        }
        Err(e) => s3_error(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", &e, &format!("/{}", bucket)),
    }
}

// HEAD /{bucket}/{*key} -> HeadObject
pub async fn head_object_handler(
    State(storage): State<StorageService>,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    match storage.get_object_metadata(&bucket, &key).await {
        Ok(Some(obj)) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_str(&obj.content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")),
            );
            headers.insert(
                axum::http::header::CONTENT_LENGTH,
                HeaderValue::from_str(&obj.size.to_string()).unwrap_or(HeaderValue::from_static("0")),
            );
            headers.insert(
                axum::http::header::ETAG,
                HeaderValue::from_str(&obj.etag).unwrap_or(HeaderValue::from_static("\"\"")),
            );
            (StatusCode::OK, headers).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// PUT /{bucket}/{*key} -> PutObject
pub async fn put_object_handler(
    State(storage): State<StorageService>,
    Path((bucket, key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let content_type = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let mut reader =
        crate::storage::service::wrap_stream_as_reader(request.into_body().into_data_stream());
    match storage
        .put_object_reader(&bucket, &key, &mut reader, content_type.as_deref())
        .await
    {
        Ok(obj) => {
            let mut res_headers = HeaderMap::new();
            res_headers.insert(
                axum::http::header::ETAG,
                HeaderValue::from_str(&obj.etag).unwrap_or(HeaderValue::from_static("\"\"")),
            );
            (StatusCode::OK, res_headers).into_response()
        }
        Err(e) => s3_error(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", &e, &format!("/{}/{}", bucket, key)),
    }
}

// GET /{bucket}/{*key} -> GetObject
pub async fn get_object_handler(
    State(storage): State<StorageService>,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    // Streamed: bytes are pulled from Telegram lazily as the body is consumed.
    let bucket_lower = bucket.to_lowercase();
    match storage.get_object_metadata(&bucket, &key).await {
        Ok(Some(_obj)) => {
            let (_obj, chunks) = match storage.get_object_chunks(&bucket, &key).await {
                Ok(v) => v,
                Err(e) => {
                    return s3_error(
                        StatusCode::NOT_FOUND,
                        "NoSuchKey",
                        &e,
                        &format!("/{}/{}", bucket, key),
                    )
                }
            };
            let total = chunks_total_size(&chunks);
            let stream = storage.stream_object_range(
                &bucket_lower,
                chunks,
                crate::storage::ByteRange { start: 0, end: total.saturating_sub(1) },
            );

            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_str(&object_content_type(&key)).unwrap_or(HeaderValue::from_static("application/octet-stream")),
            );
            headers.insert(
                axum::http::header::CONTENT_LENGTH,
                HeaderValue::from_str(&total.to_string()).unwrap_or(HeaderValue::from_static("0")),
            );
            headers.insert(
                axum::http::header::ETAG,
                HeaderValue::from_str("\"\"").unwrap_or(HeaderValue::from_static("\"\"")),
            );
            (StatusCode::OK, headers, Body::from_stream(stream)).into_response()
        }
        Ok(None) => s3_error(
            StatusCode::NOT_FOUND,
            "NoSuchKey",
            "The specified key does not exist.",
            &format!("/{}/{}", bucket, key),
        ),
        Err(e) => s3_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "InternalError",
            &e,
            &format!("/{}/{}", bucket, key),
        ),
    }
}

fn chunks_total_size(chunks: &[crate::db::ChunkRecord]) -> u64 {
    chunks.iter().map(|c| c.chunk_size.max(0) as u64).sum()
}

fn object_content_type(key: &str) -> String {
    mime_guess::from_path(key).first_or_octet_stream().to_string()
}

// DELETE /{bucket}/{*key} -> DeleteObject
pub async fn delete_object_handler(
    State(storage): State<StorageService>,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    match storage.delete_object(&bucket, &key).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => s3_error(StatusCode::NOT_FOUND, "NoSuchKey", "The specified key does not exist.", &format!("/{}/{}", bucket, key)),
        Err(e) => s3_error(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", &e, &format!("/{}/{}", bucket, key)),
    }
}
