use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use grammers_client::{
    media::Media,
    message::InputMessage,
    sender::SenderPool,
    Client,
};
use grammers_session::types::PeerRef;

/// Telegram upload.GetFile requires offsets divisible by 4 KB; 1 MB is safely divisible.
const TG_OFFSET_ALIGN: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct UploadResult {
    pub file_id: String,
    pub message_id: i64,
}

use crate::db::repo::DbRepo;
use super::session_store::TursoSessionManager;

#[derive(Clone)]
pub struct PendingAuth {
    pub phone: String,
    pub token: Arc<grammers_client::client::LoginToken>,
}

#[derive(Clone)]
pub struct TelegramBot {
    client: Client,
    api_hash: String,
    session_mgr: Arc<TursoSessionManager>,
    bucket_peers: Arc<Mutex<HashMap<String, PeerRef>>>,
    pending_auth: Arc<Mutex<Option<PendingAuth>>>,
}

impl TelegramBot {
    pub async fn connect(
        api_id: i32,
        api_hash: String,
        repo: DbRepo,
    ) -> Result<Self, String> {
        let session_mgr = Arc::new(TursoSessionManager::new(repo, "telegram.session"));
        let session = session_mgr.open_sqlite_session().await?;
        let pool = SenderPool::new(session, api_id);
        let client = Client::new(pool.handle);

        let runner = pool.runner;
        tokio::spawn(async move {
            runner.run().await;
        });

        info!("Telegram MTProto initialized using API_ID ({}) & Turso DB Session Persistence", api_id);

        let bot = Self {
            client,
            api_hash,
            session_mgr,
            bucket_peers: Arc::new(Mutex::new(HashMap::new())),
            pending_auth: Arc::new(Mutex::new(None)),
        };

        Ok(bot)
    }

    #[allow(dead_code)]
    pub async fn save_session_to_db(&self) -> Result<(), String> {
        self.session_mgr.save_session().await
    }

    pub async fn request_otp(&self, phone: &str) -> Result<String, String> {
        let clean_phone = phone.replace([' ', '-'], "");
        let formatted_phone = if clean_phone.starts_with("08") {
            format!("+62{}", &clean_phone[1..])
        } else if clean_phone.starts_with("8") {
            format!("+62{}", clean_phone)
        } else if !clean_phone.starts_with('+') {
            format!("+{}", clean_phone)
        } else {
            clean_phone
        };

        info!("Sending Telegram OTP request to phone {}...", formatted_phone);
        let token = self
            .client
            .request_login_code(&formatted_phone, &self.api_hash)
            .await
            .map_err(|e| format!("Failed to request Telegram OTP code: {}", e))?;

        let mut lock = self.pending_auth.lock().await;
        *lock = Some(PendingAuth {
            phone: formatted_phone.clone(),
            token: Arc::new(token),
        });

        Ok(format!("Telegram OTP code successfully sent to {}", formatted_phone))
    }

    pub async fn verify_otp(&self, code: &str) -> Result<String, String> {
        let pending = {
            let lock = self.pending_auth.lock().await;
            lock.clone().ok_or_else(|| "OTP request session not found. Please click 'Send Telegram OTP Code' first.".to_string())?
        };

        info!("Verifying Telegram OTP code for phone {}...", pending.phone);
        match self.client.sign_in(&pending.token, code.trim()).await {
            Ok(user) => {
                let user_disp = user.username().map(|u| format!("@{}", u)).unwrap_or_else(|| format!("User {}", user.id()));
                info!("Telegram User signed in successfully: {}", user_disp);
                
                // Clear pending auth only after successful sign-in
                let mut lock = self.pending_auth.lock().await;
                *lock = None;

                // Immediately persist the authenticated session into Turso DB!
                if let Err(e) = self.session_mgr.save_session().await {
                    info!("Warning: Failed to save session to Turso DB after login: {}", e);
                }

                Ok(format!("Telegram authentication successful for {}", user_disp))
            }
            Err(e) => Err(format!("Failed to verify Telegram OTP code: {}. Please double-check your OTP code.", e)),
        }
    }

    pub async fn get_peer_for_bucket(&self, bucket_name: &str) -> Result<PeerRef, String> {
        let bucket = bucket_name.to_lowercase();
        let mut lock = self.bucket_peers.lock().await;

        if let Some(peer_ref) = lock.get(&bucket) {
            return Ok(peer_ref.clone());
        }

        if let Some(peer_ref) = lock.get("me").cloned() {
            lock.insert(bucket, peer_ref.clone());
            return Ok(peer_ref);
        }

        // Saved Messages ('me') peer resolution
        info!("Resolving Telegram Saved Messages ('me') peer for bucket '{}'...", bucket);
        let me = match self.client.get_me().await {
            Ok(user) => user,
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("AUTH_KEY_UNREGISTERED") {
                    return Err(
                        "Telegram MTProto session is not authenticated (AUTH_KEY_UNREGISTERED). Please authenticate using Telegram OTP on /login.".to_string()
                    );
                }
                return Err(format!("Failed to get me from Telegram: {}", e));
            }
        };

        let peer_ref = me
            .to_ref()
            .await
            .map_err(|e| format!("Failed to get me PeerRef: {}", e))?
            .ok_or_else(|| "Me PeerRef unavailable".to_string())?;

        lock.insert(bucket.clone(), peer_ref.clone());
        lock.insert("default".to_string(), peer_ref.clone());
        lock.insert("me".to_string(), peer_ref.clone());
        Ok(peer_ref)
    }

    pub async fn upload_chunk(&self, bucket_name: &str, data: Vec<u8>, filename: &str) -> Result<UploadResult, String> {
        let peer = self.get_peer_for_bucket(bucket_name).await?;
        let size = data.len();
        let mut cursor = Cursor::new(data);

        let uploaded_file = self
            .client
            .upload_stream(&mut cursor, size, filename.to_string())
            .await
            .map_err(|e| format!("Failed to upload stream to Telegram: {}", e))?;

        let input_msg = InputMessage::new().document(uploaded_file);
        let msg = self
            .client
            .send_message(peer, input_msg)
            .await
            .map_err(|e| format!("Failed to send document message to Telegram: {}", e))?;

        let msg_id = msg.id() as i64;
        let file_id = msg_id.to_string();

        Ok(UploadResult {
            file_id,
            message_id: msg_id,
        })
    }

    /// Resolve the media object stored in a Telegram message by its message id.
    pub async fn get_media(&self, bucket_name: &str, file_id: &str) -> Result<Media, String> {
        let peer = self.get_peer_for_bucket(bucket_name).await?;
        let msg_id: i32 = file_id
            .parse::<i32>()
            .map_err(|e| format!("Invalid message id '{}': {}", file_id, e))?;

        let messages = self
            .client
            .get_messages_by_id(peer, &[msg_id])
            .await
            .map_err(|e| format!("Failed to get message by id {}: {}", msg_id, e))?;

        let msg = messages
            .into_iter()
            .next()
            .flatten()
            .ok_or_else(|| format!("Message {} not found", msg_id))?;

        msg.media()
            .ok_or_else(|| format!("Message {} has no media attachment", msg_id))
    }

    /// Download a full chunk (all bytes) from Telegram.
    #[allow(dead_code)]
    pub async fn download_chunk(&self, bucket_name: &str, file_id: &str) -> Result<Vec<u8>, String> {
        let media = self.get_media(bucket_name, file_id).await?;

        let mut download = self.client.iter_download(&media);
        let mut bytes = Vec::new();
        while let Some(chunk) = download
            .next()
            .await
            .map_err(|e| format!("Error downloading chunk from Telegram: {}", e))?
        {
            bytes.extend(chunk);
        }

        Ok(bytes)
    }

    /// Download at most `len` bytes starting at `offset` from a stored Telegram media.
    /// Uses MTProto upload.GetFile partial reads (offsets must be 4 KB-aligned; 1 MB is safe).
    pub async fn download_chunk_range(
        &self,
        bucket_name: &str,
        file_id: &str,
        offset: usize,
        len: usize,
    ) -> Result<Vec<u8>, String> {
        if len == 0 {
            return Ok(Vec::new());
        }

        let media = self.get_media(bucket_name, file_id).await?;
        self.read_media_range(media, offset, len).await
    }

    /// Partial read directly from an already-resolved media object.
    pub async fn read_media_range(&self, media: Media, offset: usize, len: usize) -> Result<Vec<u8>, String> {
        if len == 0 {
            return Ok(Vec::new());
        }

        let tg_offset = (offset / TG_OFFSET_ALIGN) * TG_OFFSET_ALIGN;
        let chunk_size = 512 * 1024; // valid GetFile limit, divisible by MIN_CHUNK_SIZE

        let skip = (tg_offset / chunk_size) as i32;
        let read_len = len + (offset - tg_offset);

        let mut download = self
            .client
            .iter_download(&media)
            .chunk_size(chunk_size as i32)
            .skip_chunks(skip);

        let mut out = Vec::with_capacity(read_len);
        while out.len() < read_len {
            let piece = match download.next().await {
                Ok(Some(p)) => p,
                Ok(None) => break,
                Err(e) => return Err(format!("Error downloading range from Telegram: {}", e)),
            };
            out.extend_from_slice(&piece);
        }

        if out.len() <= offset - tg_offset {
            return Ok(Vec::new());
        }
        let start = offset - tg_offset;
        let end = std::cmp::min(start + len, out.len());
        Ok(out[start..end].to_vec())
    }

    pub async fn delete_chunk(&self, bucket_name: &str, message_id: i64) -> Result<(), String> {
        let peer = self.get_peer_for_bucket(bucket_name).await?;
        let msg_id = message_id as i32;
        self.client
            .delete_messages(peer, &[msg_id])
            .await
            .map_err(|e| format!("Failed to delete message {}: {}", msg_id, e))?;
        Ok(())
    }
}
