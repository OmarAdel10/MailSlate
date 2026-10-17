//! Async frame codec for the tokio-based daemon and IPC client.
//!
//! Mirrors [`crate::framing`] exactly — same wire format, same 64 MiB cap,
//! same error taxonomy — but runs on tokio's `AsyncRead`/`AsyncWrite`.
//! Enabled by the `tokio` feature.

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::framing::{FrameError, check_len};

/// Reads one frame's raw bytes; errors on truncation or an oversized header.
pub async fn read_frame_async<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, FrameError> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).await?;
    let len = check_len(u32::from_le_bytes(header) as u64)?;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

/// Reads one frame and deserializes it as JSON `T`.
pub async fn read_json_frame_async<R: AsyncRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
) -> Result<T, FrameError> {
    let payload = read_frame_async(reader).await?;
    Ok(serde_json::from_slice(&payload)?)
}

/// Serializes `value` to JSON and writes one frame to `writer`.
pub async fn write_json_frame_async<W: AsyncWrite + Unpin, T: Serialize + ?Sized>(
    writer: &mut W,
    value: &T,
) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(value)?;
    let len = check_len(payload.len() as u64)?;
    writer.write_all(&(len as u32).to_le_bytes()).await?;
    writer.write_all(&payload).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_FRAME_BYTES;
    use crate::framing::FrameError;
    use serde_json::{Value, json};
    use std::io;

    #[tokio::test]
    async fn roundtrip_over_duplex() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let value = json!({"id": 7, "method": "ping", "params": null});
        write_json_frame_async(&mut a, &value).await.unwrap();
        let back: Value = read_json_frame_async(&mut b).await.unwrap();
        assert_eq!(back, value);
    }

    #[tokio::test]
    async fn oversized_header_is_rejected_before_allocation() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&((MAX_FRAME_BYTES as u32) + 1).to_le_bytes())
            .await
            .unwrap();
        match read_frame_async(&mut b).await {
            Err(FrameError::TooLarge { len }) => {
                assert_eq!(len, MAX_FRAME_BYTES as u64 + 1);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn truncated_header_is_unexpected_eof() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&[0u8, 1]).await.unwrap();
        drop(a);
        match read_frame_async(&mut b).await {
            Err(FrameError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof),
            other => panic!("expected Io(UnexpectedEof), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn truncated_body_is_unexpected_eof() {
        // A frame whose declared length exceeds the bytes actually written
        // must fail with UnexpectedEof, mirroring the sync codec's test.
        let value = json!({"hello": "world"});
        let mut frame = Vec::new();
        crate::write_json_frame(&mut frame, &value).unwrap();
        frame.pop();

        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&frame).await.unwrap();
        drop(a);
        match read_json_frame_async::<_, Value>(&mut b).await {
            Err(FrameError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof),
            other => panic!("expected Io(UnexpectedEof), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_json_payload_is_a_json_error() {
        let payload = b"not json";
        let (mut a, mut b) = tokio::io::duplex(64);
        let mut buf = (payload.len() as u32).to_le_bytes().to_vec();
        buf.extend_from_slice(payload);
        a.write_all(&buf).await.unwrap();
        match read_json_frame_async::<_, Value>(&mut b).await {
            Err(FrameError::Json(_)) => {}
            other => panic!("expected Json error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn oversized_payload_is_too_large_on_write() {
        let payload = "x".repeat(MAX_FRAME_BYTES + 1);
        let (mut a, _b) = tokio::io::duplex(64);
        assert!(matches!(
            write_json_frame_async(&mut a, &payload).await,
            Err(FrameError::TooLarge { .. })
        ));
    }
}
