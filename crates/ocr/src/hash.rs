use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::Path;

/// Compute SHA-256 of a file via streaming reads (constant memory).
pub fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Compute SHA-256 of an in-memory byte slice.
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Encode a raw 32-byte hash as a lowercase hex string (64 chars).
pub fn to_hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// Derive the content-addressed storage path for a given hash.
/// Layout: `<base>/<first_2_hex_chars>/<full_hex>.<ext>`
pub fn attachment_path(
    attachments_dir: &Path,
    hash_hex: &str,
    ext: &str,
) -> std::path::PathBuf {
    attachments_dir
        .join(&hash_hex[..2])
        .join(format!("{hash_hex}.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_bytes_known_vector() {
        // SHA-256 of empty bytes is a known constant.
        let hash = sha256_bytes(b"");
        let hex = to_hex(&hash);
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_bytes_deterministic() {
        assert_eq!(sha256_bytes(b"hello"), sha256_bytes(b"hello"));
        assert_ne!(sha256_bytes(b"hello"), sha256_bytes(b"world"));
    }

    #[test]
    fn to_hex_length() {
        let hash = sha256_bytes(b"test");
        assert_eq!(to_hex(&hash).len(), 64);
    }

    #[test]
    fn attachment_path_layout() {
        let base = std::path::PathBuf::from("/data/attachments");
        let hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab";
        let path = attachment_path(&base, hash, "jpg");
        assert_eq!(path, std::path::PathBuf::from("/data/attachments/ab/abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab.jpg"));
    }
}
