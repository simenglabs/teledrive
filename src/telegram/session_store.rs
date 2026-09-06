use crate::db::repo::DbRepo;
use grammers_session::storages::SqliteSession;
use std::sync::Arc;
use tracing::info;

pub struct TursoSessionManager {
    repo: DbRepo,
    session_path: String,
}

impl TursoSessionManager {
    pub fn new(repo: DbRepo, session_path: &str) -> Self {
        Self {
            repo,
            session_path: session_path.to_string(),
        }
    }

    pub async fn restore_session(&self) -> Result<(), String> {
        let conn = self.repo.get_connection().await?;
        let mut rows = conn
            .query("SELECT session_data FROM telegram_sessions WHERE id = 'active_session'", ())
            .await
            .map_err(|e| format!("Failed to query telegram_sessions from Turso DB: {}", e))?;

        if let Some(row) = rows.next().await.map_err(|e| format!("Row error: {}", e))? {
            let session_bytes: Vec<u8> = row.get(0).map_err(|e| format!("Failed to get session_data: {}", e))?;
            if !session_bytes.is_empty() {
                tokio::fs::write(&self.session_path, &session_bytes)
                    .await
                    .map_err(|e| format!("Failed to write session file to disk: {}", e))?;
                info!("Successfully restored Telegram session ({} bytes) from Turso DB to {}", session_bytes.len(), self.session_path);
            }
        }
        Ok(())
    }

    pub async fn save_session(&self) -> Result<(), String> {
        if !std::path::Path::new(&self.session_path).exists() {
            return Ok(());
        }

        let session_bytes = tokio::fs::read(&self.session_path)
            .await
            .map_err(|e| format!("Failed to read session file {}: {}", self.session_path, e))?;

        if session_bytes.is_empty() {
            return Ok(());
        }

        let bytes_len = session_bytes.len();
        let conn = self.repo.get_connection().await?;
        let updated_at = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT OR REPLACE INTO telegram_sessions (id, session_data, updated_at) VALUES ('active_session', ?1, ?2)",
            libsql::params![session_bytes, updated_at],
        )
        .await
        .map_err(|e| format!("Failed to save session to Turso DB: {}", e))?;

        info!("Successfully saved Telegram session ({} bytes) to Turso DB", bytes_len);
        Ok(())
    }

    pub async fn open_sqlite_session(&self) -> Result<Arc<SqliteSession>, String> {
        let _ = self.restore_session().await;
        let session = SqliteSession::open(&self.session_path)
            .await
            .map_err(|e| format!("Failed to open SqliteSession at {}: {}", self.session_path, e))?;
        Ok(Arc::new(session))
    }
}
