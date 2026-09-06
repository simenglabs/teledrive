use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub telegram_api_id: i32,
    pub telegram_api_hash: String,
    #[allow(dead_code)]
    pub telegram_chat_id: String,
    pub turso_database_url: String,
    pub turso_auth_token: Option<String>,
    pub master_encryption_key: [u8; 32],
    pub server_port: u16,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        dotenvy::dotenv().ok();

        let telegram_api_id = env::var("TELEGRAM_API_ID")
            .ok()
            .and_then(|id| id.parse().ok())
            .unwrap_or(24185293);

        let telegram_api_hash = env::var("TELEGRAM_API_HASH")
            .unwrap_or_else(|_| "a3f1234567890abcdef1234567890abc".to_string());

        let telegram_chat_id = env::var("TELEGRAM_CHAT_ID")
            .unwrap_or_else(|_| "me".to_string());
        let turso_database_url = env::var("TURSO_DATABASE_URL")
            .unwrap_or_else(|_| "file:local.db".to_string());
        let turso_auth_token = env::var("TURSO_AUTH_TOKEN").ok().filter(|s| !s.is_empty());
        
        let hex_key = env::var("MASTER_ENCRYPTION_KEY")
            .unwrap_or_else(|_| "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string());
        
        let key_bytes = hex::decode(&hex_key)
            .map_err(|e| format!("Invalid MASTER_ENCRYPTION_KEY hex: {}", e))?;

        if key_bytes.len() != 32 {
            return Err("MASTER_ENCRYPTION_KEY must be 32 bytes (64 hex characters)".to_string());
        }

        let mut master_encryption_key = [0u8; 32];
        master_encryption_key.copy_from_slice(&key_bytes);

        let server_port = env::var("SERVER_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3060);

        Ok(Config {
            telegram_api_id,
            telegram_api_hash,
            telegram_chat_id,
            turso_database_url,
            turso_auth_token,
            master_encryption_key,
            server_port,
        })
    }
}
