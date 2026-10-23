//! Content-addressed blob store for message attachments (ARCHITECTURE §7).
//!
//! Blobs are addressed by their SHA-256 digest and stored under
//! `<data_dir>/blobs/<first two hex chars>/<rest>`. Deduplication is free:
//! identical bytes share one file. Read-only snapshot for the GUI; the
//! writer is the sync engine (later phase); until then these items are
//! intentionally unused.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Where blobs live inside the data directory.
pub const BLOB_DIR: &str = "blobs";

/// Errors from the blob store.
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("stored blob digest does not match requested digest")]
    DigestMismatch,
    #[error("blob not found: {0}")]
    NotFound(String),
}

/// Handle for writing one blob: feeds bytes through the hasher then
/// materializes the file once `finish` is called.
#[derive(Default)]
pub struct BlobWriter {
    hasher: Sha256,
    bytes: Vec<u8>,
}

impl BlobWriter {
    /// Starts accumulating one blob.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds more bytes into the blob.
    pub fn write(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
        self.bytes.extend_from_slice(chunk);
    }

    /// Finalizes: writes `<data_dir>/blobs/<xx>/<rest>` if absent,
    /// returns the hex digest address. Idempotent for identical bytes.
    pub fn finish(self, data_dir: &Path) -> Result<String, BlobError> {
        let digest = self.hasher.finalize();
        let hex = hex::encode(digest);
        let dir = data_dir.join(BLOB_DIR).join(&hex[..2]);
        let path = dir.join(&hex[2..]);
        std::fs::create_dir_all(&dir)?;
        if !path.exists() {
            std::fs::write(&path, &self.bytes)?;
        }
        Ok(hex)
    }
}

/// Resolves the on-disk path for a digest address.
pub fn blob_path(data_dir: &Path, digest: &str) -> PathBuf {
    data_dir
        .join(BLOB_DIR)
        .join(&digest[..2])
        .join(&digest[2..])
}

/// Reads a stored blob back by digest.
pub fn read_blob(data_dir: &Path, digest: &str) -> Result<Vec<u8>, BlobError> {
    let path = blob_path(data_dir, digest);
    let bytes = std::fs::read(&path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            BlobError::NotFound(digest.to_string())
        } else {
            BlobError::Io(err)
        }
    })?;
    // Verify integrity on read: a corrupted blob is worse than a miss.
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual = format!("{:x}", hasher.finalize());
    if actual != digest {
        return Err(BlobError::DigestMismatch);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = temp_dir();
        let payload = b"attachment bytes";
        let mut writer = BlobWriter::new();
        writer.write(&payload[..8]);
        writer.write(&payload[8..]);
        let digest = writer.finish(dir.path()).expect("finish");
        assert_eq!(digest.len(), 64);
        let back = read_blob(dir.path(), &digest).expect("read");
        assert_eq!(back, payload);
    }

    #[test]
    fn identical_bytes_share_one_file() {
        let dir = temp_dir();
        let mut first = BlobWriter::new();
        first.write(b"dup");
        let d1 = first.finish(dir.path()).expect("finish 1");
        let mut second = BlobWriter::new();
        second.write(b"dup");
        let d2 = second.finish(dir.path()).expect("finish 2");
        assert_eq!(d1, d2);
        let path = blob_path(dir.path(), &d1);
        assert!(path.is_file(), "blob file must exist: {path:?}");
        // Only one file on disk for identical bytes.
        let prefix = &d1[..2];
        let count: u32 = std::fs::read_dir(dir.path().join(BLOB_DIR))
            .expect("read blobs dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .filter(|name| name == prefix)
            .count() as u32;
        assert_eq!(count, 1);
    }

    #[test]
    fn read_missing_blob_is_not_found() {
        let dir = temp_dir();
        let err = read_blob(
            dir.path(),
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(matches!(err, BlobError::NotFound(_)));
    }

    #[test]
    fn path_has_two_levels() {
        let dir = temp_dir();
        let tail = "c".repeat(62);
        let path: PathBuf = blob_path(dir.path(), &format!("ab{tail}"));
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), &tail);
        assert_eq!(
            path.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap(),
            "ab"
        );
    }
}
