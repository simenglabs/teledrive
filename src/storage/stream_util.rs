use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf};

/// Bridges a `Stream<Item = Result<Bytes, E>>` (HTTP body / multipart field)
/// into a tokio `AsyncRead`, so Telegram `upload_stream` can consume it
/// without ever buffering the whole payload in memory.
pub struct ByteStreamReader<S> {
    stream: S,
    current: Bytes,
}

impl<S> ByteStreamReader<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            current: Bytes::new(),
        }
    }
}

impl<S, E> AsyncRead for ByteStreamReader<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if !self.current.is_empty() {
                let len = std::cmp::min(self.current.len(), buf.remaining());
                let slice = self.current.split_to(len);
                buf.put_slice(&slice);
                return Poll::Ready(Ok(()));
            }

            match Pin::new(&mut self.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    if !bytes.is_empty() {
                        self.current = bytes;
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    let boxed: Box<dyn std::error::Error + Send + Sync> = e.into();
                    return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, boxed)));
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())), // EOF
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Reads up to `max` bytes into `buf` (buf is cleared first).
/// Returns the number of bytes read; 0 means EOF.
#[allow(dead_code)]
pub async fn read_up_to<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> io::Result<usize> {
    buf.clear();
    let mut total = 0usize;
    while total < max {
        let want = std::cmp::min(1024 * 1024, max - total);
        buf.resize(total + want, 0);
        let n = reader.read(&mut buf[total..]).await?;
        if n == 0 {
            break;
        }
        total += n;
    }
    buf.truncate(total);
    Ok(total)
}

/// Spools an inbound byte stream to a temp file (bounded memory),
/// returning the temp path and the exact byte size.
#[allow(dead_code)]
pub async fn spool_stream_to_tempfile<S, E>(mut stream: S) -> Result<(std::path::PathBuf, u64), String>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    let path = std::env::temp_dir().join(format!("mengdrive_spool_{}", rand::random::<u64>()));
    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|e| format!("Failed to create temp spool file: {}", e))?;

    let mut size = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Failed to read upload stream: {}", e))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write temp spool file: {}", e))?;
        size += chunk.len() as u64;
    }

    file.flush()
        .await
        .map_err(|e| format!("Failed to flush temp spool file: {}", e))?;
    Ok((path, size))
}
