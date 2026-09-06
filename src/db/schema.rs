pub const INIT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS buckets (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_buckets_name ON buckets(name);

CREATE TABLE IF NOT EXISTS objects (
    id TEXT PRIMARY KEY,
    bucket_name TEXT NOT NULL,
    key TEXT NOT NULL,
    size INTEGER NOT NULL,
    content_type TEXT NOT NULL,
    etag TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_objects_bucket_key ON objects(bucket_name, key);

CREATE TABLE IF NOT EXISTS chunks (
    id TEXT PRIMARY KEY,
    object_id TEXT NOT NULL,
    part_number INTEGER NOT NULL,
    telegram_file_id TEXT NOT NULL,
    telegram_message_id INTEGER NOT NULL,
    chunk_size INTEGER NOT NULL,
    nonce TEXT NOT NULL,
    FOREIGN KEY(object_id) REFERENCES objects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_chunks_object_part ON chunks(object_id, part_number);

CREATE TABLE IF NOT EXISTS telegram_sessions (
    id TEXT PRIMARY KEY,
    session_data BLOB NOT NULL,
    updated_at TEXT NOT NULL
);
"#;
