use libsql::{Builder, Connection, Database};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::schema::INIT_SCHEMA;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bucket {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectRecord {
    pub id: String,
    pub bucket_name: String,
    pub key: String,
    pub size: i64,
    pub content_type: String,
    pub etag: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub id: String,
    pub object_id: String,
    pub part_number: i32,
    pub telegram_file_id: String,
    pub telegram_message_id: i64,
    pub chunk_size: i64,
    pub nonce: String,
}

#[derive(Clone)]
pub struct DbRepo {
    db: Arc<Database>,
}

impl DbRepo {
    pub async fn new(db_url: &str, auth_token: Option<&str>) -> Result<Self, String> {
        let db = if db_url.starts_with("libsql://") || db_url.starts_with("https://") {
            Builder::new_remote(db_url.to_string(), auth_token.unwrap_or_default().to_string())
                .build()
                .await
                .map_err(|e| format!("Failed to connect to remote Turso DB: {}", e))?
        } else {
            Builder::new_local(db_url)
                .build()
                .await
                .map_err(|e| format!("Failed to open local SQLite DB: {}", e))?
        };

        let repo = Self { db: Arc::new(db) };
        repo.init_schema().await?;
        Ok(repo)
    }

    pub async fn get_connection(&self) -> Result<Connection, String> {
        self.db
            .connect()
            .map_err(|e| format!("Failed to get DB connection: {}", e))
    }

    async fn init_schema(&self) -> Result<(), String> {
        let conn = self.get_connection().await?;
        conn.execute_batch(INIT_SCHEMA)
            .await
            .map_err(|e| format!("Failed to execute schema batch: {}", e))?;
        Ok(())
    }

    // --- Bucket Operations ---

    pub async fn create_bucket(&self, name: &str) -> Result<Bucket, String> {
        let conn = self.get_connection().await?;
        let id = format!("bkt_{}", rand::random::<u32>());
        let created_at = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO buckets (id, name, created_at) VALUES (?1, ?2, ?3)",
            libsql::params![id.clone(), name.to_string(), created_at.clone()],
        )
        .await
        .map_err(|e| format!("Failed to create bucket: {}", e))?;

        Ok(Bucket {
            id,
            name: name.to_string(),
            created_at,
        })
    }

    pub async fn list_buckets(&self) -> Result<Vec<Bucket>, String> {
        let conn = self.get_connection().await?;
        let mut rows = conn
            .query("SELECT id, name, created_at FROM buckets ORDER BY name ASC", ())
            .await
            .map_err(|e| format!("Failed to query buckets: {}", e))?;

        let mut buckets = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| format!("Row error: {}", e))? {
            buckets.push(Bucket {
                id: row.get(0).unwrap_or_default(),
                name: row.get(1).unwrap_or_default(),
                created_at: row.get(2).unwrap_or_default(),
            });
        }

        Ok(buckets)
    }

    pub async fn get_bucket_by_name(&self, name: &str) -> Result<Option<Bucket>, String> {
        let conn = self.get_connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, name, created_at FROM buckets WHERE name = ?1",
                libsql::params![name.to_string()],
            )
            .await
            .map_err(|e| format!("Failed to get bucket: {}", e))?;

        if let Some(row) = rows.next().await.map_err(|e| format!("Row error: {}", e))? {
            Ok(Some(Bucket {
                id: row.get(0).unwrap_or_default(),
                name: row.get(1).unwrap_or_default(),
                created_at: row.get(2).unwrap_or_default(),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn delete_bucket(&self, name: &str) -> Result<bool, String> {
        let conn = self.get_connection().await?;
        let affected = conn
            .execute(
                "DELETE FROM buckets WHERE name = ?1",
                libsql::params![name.to_string()],
            )
            .await
            .map_err(|e| format!("Failed to delete bucket: {}", e))?;

        Ok(affected > 0)
    }

    // --- Object Operations ---

    pub async fn create_object(
        &self,
        bucket_name: &str,
        key: &str,
        size: i64,
        content_type: &str,
        etag: &str,
    ) -> Result<ObjectRecord, String> {
        let conn = self.get_connection().await?;
        let id = format!("obj_{}", rand::random::<u64>());
        let created_at = chrono::Utc::now().to_rfc3339();

        // Replace existing object if exists
        conn.execute(
            "INSERT OR REPLACE INTO objects (id, bucket_name, key, size, content_type, etag, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            libsql::params![
                id.clone(),
                bucket_name.to_string(),
                key.to_string(),
                size,
                content_type.to_string(),
                etag.to_string(),
                created_at.clone()
            ],
        )
        .await
        .map_err(|e| format!("Failed to create object record: {}", e))?;

        Ok(ObjectRecord {
            id,
            bucket_name: bucket_name.to_string(),
            key: key.to_string(),
            size,
            content_type: content_type.to_string(),
            etag: etag.to_string(),
            created_at,
        })
    }

    /// Finalize an object after streamed upload: set real size + etag.
    pub async fn update_object_after_upload(
        &self,
        object_id: &str,
        size: i64,
        etag: &str,
    ) -> Result<ObjectRecord, String> {
        let conn = self.get_connection().await?;
        conn.execute(
            "UPDATE objects SET size = ?1, etag = ?2 WHERE id = ?3",
            libsql::params![size, etag.to_string(), object_id.to_string()],
        )
        .await
        .map_err(|e| format!("Failed to finalize object: {}", e))?;

        self.get_object_by_id(object_id)
            .await?
            .ok_or_else(|| "Object vanished during upload".to_string())
    }

    pub async fn get_object_by_id(&self, object_id: &str) -> Result<Option<ObjectRecord>, String> {
        let conn = self.get_connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, bucket_name, key, size, content_type, etag, created_at 
                 FROM objects WHERE id = ?1",
                libsql::params![object_id.to_string()],
            )
            .await
            .map_err(|e| format!("Failed to query object by id: {}", e))?;

        if let Some(row) = rows.next().await.map_err(|e| format!("Row error: {}", e))? {
            Ok(Some(ObjectRecord {
                id: row.get(0).unwrap_or_default(),
                bucket_name: row.get(1).unwrap_or_default(),
                key: row.get(2).unwrap_or_default(),
                size: row.get(3).unwrap_or_default(),
                content_type: row.get(4).unwrap_or_default(),
                etag: row.get(5).unwrap_or_default(),
                created_at: row.get(6).unwrap_or_default(),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_object(&self, bucket_name: &str, key: &str) -> Result<Option<ObjectRecord>, String> {
        let conn = self.get_connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, bucket_name, key, size, content_type, etag, created_at 
                 FROM objects WHERE bucket_name = ?1 AND key = ?2",
                libsql::params![bucket_name.to_string(), key.to_string()],
            )
            .await
            .map_err(|e| format!("Failed to query object: {}", e))?;

        if let Some(row) = rows.next().await.map_err(|e| format!("Row error: {}", e))? {
            Ok(Some(ObjectRecord {
                id: row.get(0).unwrap_or_default(),
                bucket_name: row.get(1).unwrap_or_default(),
                key: row.get(2).unwrap_or_default(),
                size: row.get(3).unwrap_or_default(),
                content_type: row.get(4).unwrap_or_default(),
                etag: row.get(5).unwrap_or_default(),
                created_at: row.get(6).unwrap_or_default(),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn list_objects(&self, bucket_name: &str, prefix: Option<&str>) -> Result<Vec<ObjectRecord>, String> {
        let conn = self.get_connection().await?;
        let sql = if let Some(p) = prefix {
            let pattern = format!("{}%", p);
            format!(
                "SELECT id, bucket_name, key, size, content_type, etag, created_at 
                 FROM objects WHERE bucket_name = ?1 AND key LIKE '{}' ORDER BY key ASC",
                pattern
            )
        } else {
            "SELECT id, bucket_name, key, size, content_type, etag, created_at 
             FROM objects WHERE bucket_name = ?1 ORDER BY key ASC".to_string()
        };

        let mut rows = conn
            .query(&sql, libsql::params![bucket_name.to_string()])
            .await
            .map_err(|e| format!("Failed to list objects: {}", e))?;

        let mut objects = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| format!("Row error: {}", e))? {
            objects.push(ObjectRecord {
                id: row.get(0).unwrap_or_default(),
                bucket_name: row.get(1).unwrap_or_default(),
                key: row.get(2).unwrap_or_default(),
                size: row.get(3).unwrap_or_default(),
                content_type: row.get(4).unwrap_or_default(),
                etag: row.get(5).unwrap_or_default(),
                created_at: row.get(6).unwrap_or_default(),
            });
        }

        Ok(objects)
    }

    pub async fn delete_object(&self, bucket_name: &str, key: &str) -> Result<Option<ObjectRecord>, String> {
        if let Some(obj) = self.get_object(bucket_name, key).await? {
            let conn = self.get_connection().await?;
            conn.execute(
                "DELETE FROM objects WHERE id = ?1",
                libsql::params![obj.id.clone()],
            )
            .await
            .map_err(|e| format!("Failed to delete object: {}", e))?;

            Ok(Some(obj))
        } else {
            Ok(None)
        }
    }

    // --- Chunk Operations ---

    pub async fn add_chunk(
        &self,
        object_id: &str,
        part_number: i32,
        telegram_file_id: &str,
        telegram_message_id: i64,
        chunk_size: i64,
        nonce: &str,
    ) -> Result<ChunkRecord, String> {
        let conn = self.get_connection().await?;
        let id = format!("chk_{}", rand::random::<u64>());

        conn.execute(
            "INSERT INTO chunks (id, object_id, part_number, telegram_file_id, telegram_message_id, chunk_size, nonce)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            libsql::params![
                id.clone(),
                object_id.to_string(),
                part_number,
                telegram_file_id.to_string(),
                telegram_message_id,
                chunk_size,
                nonce.to_string()
            ],
        )
        .await
        .map_err(|e| format!("Failed to insert chunk: {}", e))?;

        Ok(ChunkRecord {
            id,
            object_id: object_id.to_string(),
            part_number,
            telegram_file_id: telegram_file_id.to_string(),
            telegram_message_id,
            chunk_size,
            nonce: nonce.to_string(),
        })
    }

    pub async fn get_chunks_for_object(&self, object_id: &str) -> Result<Vec<ChunkRecord>, String> {
        let conn = self.get_connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, object_id, part_number, telegram_file_id, telegram_message_id, chunk_size, nonce
                 FROM chunks WHERE object_id = ?1 ORDER BY part_number ASC",
                libsql::params![object_id.to_string()],
            )
            .await
            .map_err(|e| format!("Failed to query chunks: {}", e))?;

        let mut chunks = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| format!("Row error: {}", e))? {
            chunks.push(ChunkRecord {
                id: row.get(0).unwrap_or_default(),
                object_id: row.get(1).unwrap_or_default(),
                part_number: row.get(2).unwrap_or_default(),
                telegram_file_id: row.get(3).unwrap_or_default(),
                telegram_message_id: row.get(4).unwrap_or_default(),
                chunk_size: row.get(5).unwrap_or_default(),
                nonce: row.get(6).unwrap_or_default(),
            });
        }

        Ok(chunks)
    }
}
