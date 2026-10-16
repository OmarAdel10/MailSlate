//! Length-prefixed JSON framing (ARCHITECTURE.md §5.1).
//!
//! Every frame is a 32-bit little-endian unsigned length followed by exactly
//! that many bytes of UTF-8 JSON. Frames are capped at [`MAX_FRAME_BYTES`];
//! larger payloads must travel through the staged-file mechanism instead of
//! the socket.

use std::fmt;
use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Hard cap on a single frame: 64 MiB (ARCHITECTURE.md §5.1).
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Framing failure.
#[derive(Debug)]
pub enum FrameError {
    /// Underlying read/write failure, including clean EOF mid-frame.
    Io(io::Error),
    /// Payload was not valid JSON for the expected type.
    Json(serde_json::Error),
    /// Declared frame length exceeds [`MAX_FRAME_BYTES`].
    TooLarge {
        /// Length the peer declared.
        len: u64,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "frame I/O error: {e}"),
            Self::Json(e) => write!(f, "frame JSON error: {e}"),
            Self::TooLarge { len } => write!(
                f,
                "frame of {len} bytes exceeds the {MAX_FRAME_BYTES}-byte cap"
            ),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::TooLarge { .. } => None,
        }
    }
}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for FrameError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

pub(crate) fn check_len(len: u64) -> Result<usize, FrameError> {
    if len > MAX_FRAME_BYTES as u64 {
        return Err(FrameError::TooLarge { len });
    }
    Ok(len as usize)
}

/// Serializes `value` to JSON and writes one frame to `writer`.
pub fn write_json_frame<W: Write, T: Serialize + ?Sized>(
    writer: &mut W,
    value: &T,
) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(value)?;
    let len = check_len(payload.len() as u64)?;
    writer.write_all(&(len as u32).to_le_bytes())?;
    writer.write_all(&payload)?;
    Ok(())
}

/// Reads one frame's raw bytes; errors on truncation or an oversized header.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, FrameError> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header)?;
    let len = check_len(u32::from_le_bytes(header) as u64)?;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

/// Reads one frame and deserializes it as JSON `T`.
pub fn read_json_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, FrameError> {
    let payload = read_frame(reader)?;
    Ok(serde_json::from_slice(&payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::io::Cursor;

    #[test]
    fn roundtrip_through_a_cursor() {
        let mut buf = Vec::new();
        write_json_frame(&mut buf, &json!({"a": 1})).unwrap();
        assert_eq!(&buf[..4], 7u32.to_le_bytes().as_slice());
        assert_eq!(&buf[4..], br#"{"a":1}"#);
        let mut cursor = Cursor::new(buf);
        let value: Value = read_json_frame(&mut cursor).unwrap();
        assert_eq!(value, json!({"a": 1}));
    }

    #[test]
    fn oversized_header_is_rejected_before_allocation() {
        let header = ((MAX_FRAME_BYTES as u32) + 1).to_le_bytes();
        let mut cursor = Cursor::new(header.to_vec());
        match read_frame(&mut cursor) {
            Err(FrameError::TooLarge { len }) => {
                assert_eq!(len, MAX_FRAME_BYTES as u64 + 1);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn truncated_header_is_unexpected_eof() {
        let mut cursor = Cursor::new(vec![0u8, 1]);
        match read_frame(&mut cursor) {
            Err(FrameError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof),
            other => panic!("expected Io(UnexpectedEof), got {other:?}"),
        }
    }

    #[test]
    fn truncated_body_is_unexpected_eof() {
        let mut buf = Vec::new();
        write_json_frame(&mut buf, &json!({"hello": true})).unwrap();
        buf.truncate(buf.len() - 1);
        let mut cursor = Cursor::new(buf);
        match read_frame(&mut cursor) {
            Err(FrameError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof),
            other => panic!("expected Io(UnexpectedEof), got {other:?}"),
        }
    }

    #[test]
    fn invalid_json_payload_is_a_json_error() {
        let payload = b"not json";
        let mut buf = payload.len().to_le_bytes().to_vec();
        buf.extend_from_slice(payload);
        let mut cursor = Cursor::new(buf);
        match read_json_frame::<_, Value>(&mut cursor) {
            Err(FrameError::Json(_)) => {}
            other => panic!("expected Json error, got {other:?}"),
        }
    }

    #[test]
    fn len_boundary_is_checked_precisely() {
        assert_eq!(check_len(MAX_FRAME_BYTES as u64).unwrap(), MAX_FRAME_BYTES);
        assert!(matches!(
            check_len(MAX_FRAME_BYTES as u64 + 1),
            Err(FrameError::TooLarge { .. })
        ));
    }
}
