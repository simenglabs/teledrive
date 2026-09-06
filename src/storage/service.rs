use md5::{Digest, Md5};

use crate::crypto::aes_gcm;
use crate::db::{Bucket, ChunkRecord, DbRepo, ObjectRecord};
use crate::telegram::TelegramBot;
use futures_util::stream::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::AsyncReadExt;

use super::chunker::{resolve_range_reads, ByteRange, CHUNK_SIZE};
use super::stream_util::ByteStreamReader;

#[derive(Clone)]
pub struct StorageService {
    repo: DbRepo,
    bot: TelegramBot,
    master_key: [u8; 32],
}

impl StorageService {
    pub fn new(repo: DbRepo, bot: TelegramBot, master_key: [u8; 32]) -> Self {
        Self { repo, bot, master_key }
    }

    pub async fn request_telegram_otp(&self, phone: &str) -> Result<String, String> {
        self.bot.request_otp(phone).await
    }

    pub async fn verify_telegram_otp(&self, code: &str) -> Result<String, String> {
        self.bot.verify_otp(code).await
    }

    pub async fn create_bucket(&self, name: &str) -> Result<Bucket, String> {
        let name_lower = name.to_lowercase();
        if let Some(existing) = self.repo.get_bucket_by_name(&name_lower).await? {
            return Ok(existing);
        }
        self.repo.create_bucket(&name_lower).await
    }

    pub async fn list_buckets(&self) -> Result<Vec<Bucket>, String> {
        self.repo.list_buckets().await
    }

    pub async fn delete_bucket(&self, name: &str) -> Result<bool, String> {
        let name_lower = name.to_lowercase();
        let objects = self.repo.list_objects(&name_lower, None).await?;
        if !objects.is_empty() {
            return Err(format!("Bucket '{}' is not empty", name_lower));
        }
        self.repo.delete_bucket(&name_lower).await
    }

    pub async fn list_objects(&self, bucket_name: &str, prefix: Option<&str>) -> Result<Vec<ObjectRecord>, String> {
        let bucket_name = bucket_name.to_lowercase();
        self.repo.list_objects(&bucket_name, prefix).await
    }

    pub async fn get_object_metadata(&self, bucket_name: &str, key: &str) -> Result<Option<ObjectRecord>, String> {
        let bucket_name = bucket_name.to_lowercase();
        self.repo.get_object(&bucket_name, key).await
    }

    /// Upload raw (unencrypted) bytes, so they can be streamed later without decrypt.
    #[allow(dead_code)]
    pub async fn put_object(
        &self,
        bucket_name: &str,
        key: &str,
        data: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<ObjectRecord, String> {
        self.put_object_reader(bucket_name, key, &mut std::io::Cursor::new(data), content_type)
            .await
    }

    /// Upload from any async reader, chunked at 45MB, stored raw in Telegram.
    /// The reader is consumed chunk-by-chunk — the payload is never fully buffered.
    pub async fn put_object_reader<R: tokio::io::AsyncRead + Unpin>(
        &self,
        bucket_name: &str,
        key: &str,
        reader: &mut R,
        content_type: Option<&str>,
    ) -> Result<ObjectRecord, String> {
        let bucket_name = bucket_name.to_lowercase();

        if self.repo.get_bucket_by_name(&bucket_name).await?.is_none() {
            self.repo.create_bucket(&bucket_name).await?;
        }

        // Remove stale record (note: old Telegram messages are cleaned by delete_object)
        let _ = self.repo.delete_object(&bucket_name, key).await;

        let mime = content_type
            .map(|s| s.to_string())
            .unwrap_or_else(|| mime_guess::from_path(key).first_or_octet_stream().to_string());

        let obj_record = self.repo.create_object(&bucket_name, key, 0, &mime, "\"\"").await?;

        let mut hasher = Md5::new();
        let mut total_size: i64 = 0;
        let mut part_number: i32 = 1;
        let mut buffer = vec![0u8; CHUNK_SIZE];

        loop {
            let n = reader
                .read(&mut buffer)
                .await
                .map_err(|e| format!("Failed reading upload stream: {}", e))?;
            if n == 0 {
                break;
            }

            let filename = format!("{}_part{}.bin", obj_record.id, part_number);

            let upload = match self.bot.upload_chunk(&bucket_name, buffer[..n].to_vec(), &filename).await {
                Ok(u) => u,
                Err(e) => {
                    let _ = self.repo.delete_object(&bucket_name, key).await;
                    return Err(format!("Failed uploading chunk {} to Telegram: {}", part_number, e));
                }
            };

            if let Err(e) = self.repo
                .add_chunk(
                    &obj_record.id,
                    part_number,
                    &upload.file_id,
                    upload.message_id,
                    n as i64,
                    "", // plaintext payload — no nonce needed
                )
                .await
            {
                let _ = self.repo.delete_object(&bucket_name, key).await;
                return Err(format!("Failed saving chunk {} metadata: {}", part_number, e));
            }

            hasher.update(&buffer[..n]);
            total_size += n as i64;
            part_number += 1;
        }

        let etag = format!("\"{:x}\"", hasher.finalize());
        self.repo
            .update_object_after_upload(&obj_record.id, total_size, &etag)
            .await?;

        Ok(obj_record)
    }

    /// Full in-memory fetch (kept for legacy encrypted objects & small files).
    #[allow(dead_code)]
    pub async fn get_object_data(&self, bucket_name: &str, key: &str) -> Result<(ObjectRecord, Vec<u8>), String> {
        let (obj, chunks) = self.get_object_chunks(bucket_name, key).await?;
        let bucket_name = bucket_name.to_lowercase();

        let mut full_bytes = Vec::with_capacity(obj.size.max(0) as usize);
        for chunk in chunks {
            let data = self.bot.download_chunk(&bucket_name, &chunk.telegram_file_id).await?;
            full_bytes.extend_from_slice(&decode_chunk(&chunk, data, &self.master_key)?);
        }

        Ok((obj, full_bytes))
    }

    /// Load chunk records ordered by part (needed for range mapping).
    pub async fn get_object_chunks(&self, bucket_name: &str, key: &str) -> Result<(ObjectRecord, Vec<ChunkRecord>), String> {
        let bucket_name = bucket_name.to_lowercase();
        let obj = self
            .repo
            .get_object(&bucket_name, key)
            .await?
            .ok_or_else(|| format!("Object '{}/{}' not found", bucket_name, key))?;

        let chunks = self.repo.get_chunks_for_object(&obj.id).await?;
        if chunks.is_empty() && obj.size > 0 {
            return Err(format!("Object '{}/{}' has missing data chunks in storage", bucket_name, key));
        }

        Ok((obj, chunks))
    }

    /// Stream an exact byte range of an object straight from Telegram storage.
    /// Chunks are fetched lazily and yielded as they arrive:
    /// no full-file download and no decryption wait for new (plaintext) uploads.
    pub fn stream_object_range(
        &self,
        bucket_name: &str,
        chunks: Vec<ChunkRecord>,
        range: ByteRange,
    ) -> impl Stream<Item = Result<bytes::Bytes, String>> + Send + 'static {
        let bot = self.bot.clone();
        let master_key = self.master_key;
        let bucket = bucket_name.to_string();

        let chunk_sizes: Vec<u64> = chunks.iter().map(|c| c.chunk_size as u64).collect();
        let reads = resolve_range_reads(&chunk_sizes, range);

        ObjectRangeStream::new(bot, bucket, master_key, chunks, reads)
    }

    pub async fn delete_object(&self, bucket_name: &str, key: &str) -> Result<bool, String> {
        let bucket_name = bucket_name.to_lowercase();
        // Fetch chunks BEFORE deleting the row (cascade would erase the message ids)
        let chunks = match self.repo.get_object(&bucket_name, key).await? {
            Some(obj) => self.repo.get_chunks_for_object(&obj.id).await?,
            None => return Ok(false),
        };

        if self.repo.delete_object(&bucket_name, key).await?.is_some() {
            for chunk in chunks {
                let _ = self.bot.delete_chunk(&bucket_name, chunk.telegram_message_id).await;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

fn decode_chunk(chunk: &ChunkRecord, data: Vec<u8>, master_key: &[u8; 32]) -> Result<Vec<u8>, String> {
    if chunk.nonce.is_empty() {
        return Ok(data); // plaintext upload — streamable as-is
    }

    // Legacy AES-256-GCM chunk: decrypt with stored nonce
    let nonce_bytes_vec = hex::decode(&chunk.nonce)
        .map_err(|e| format!("Invalid nonce hex in DB: {}", e))?;
    if nonce_bytes_vec.len() != 12 {
        return Err("Invalid nonce length in database".to_string());
    }
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(&nonce_bytes_vec);

    aes_gcm::decrypt(&data, &nonce_bytes, master_key)
}

/// Lazily pulls planned range reads out of Telegram storage in order, one partial
/// download per read, emitting HTTP-ready `Bytes` as soon as each read completes.
pub struct ObjectRangeStream {
    bot: TelegramBot,
    bucket: String,
    master_key: [u8; 32],
    chunks: Vec<ChunkRecord>,
    reads: std::vec::IntoIter<super::chunker::ChunkRead>,
    pending: Option<Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + Send>>>,
    pending_chunk: Option<ChunkRecord>,
}

impl ObjectRangeStream {
    fn new(
        bot: TelegramBot,
        bucket: String,
        master_key: [u8; 32],
        chunks: Vec<ChunkRecord>,
        reads: Vec<super::chunker::ChunkRead>,
    ) -> Self {
        Self {
            bot,
            bucket,
            master_key,
            chunks,
            reads: reads.into_iter(),
            pending: None,
            pending_chunk: None,
        }
    }
}

impl Stream for ObjectRangeStream {
    type Item = Result<bytes::Bytes, String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(fut) = self.pending.as_mut() {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(data)) => {
                        let chunk = self.pending_chunk.take().expect("pending chunk metadata");
                        self.pending = None;
                        let key = self.master_key;
                        return Poll::Ready(Some(match decode_chunk(&chunk, data, &key) {
                            Ok(decoded) => Ok(bytes::Bytes::from(decoded)),
                            Err(e) => Err(e),
                        }));
                    }
                    Poll::Ready(Err(e)) => {
                        self.pending = None;
                        return Poll::Ready(Some(Err(e)));
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }

            match self.reads.next() {
                Some(read) => {
                    let chunk = match self.chunks.get(read.chunk_index) {
                        Some(c) => c.clone(),
                        None => {
                            return Poll::Ready(Some(Err("Chunk metadata missing while streaming".to_string())))
                        }
                    };

                    let bot = self.bot.clone();
                    let bucket = self.bucket.clone();
                    let file_id = chunk.telegram_file_id.clone();
                    let offset = read.offset_in_chunk as usize;
                    let len = read.len as usize;

                    self.pending_chunk = Some(chunk);
                    self.pending = Some(Box::pin(async move {
                        bot.download_chunk_range(&bucket, &file_id, offset, len).await
                    }));
                }
                None => return Poll::Ready(None),
            }
        }
    }
}

/// Wrap a byte stream (e.g. a multipart field) as a tokio AsyncRead for Telegram upload.
pub fn wrap_stream_as_reader<S, E>(stream: S) -> ByteStreamReader<S>
where
    S: Stream<Item = Result<bytes::Bytes, E>>,
{
    ByteStreamReader::new(stream)
}
