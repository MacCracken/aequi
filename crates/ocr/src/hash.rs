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
pub fn attachment_path(attachments_dir: &Path, hash_hex: &str, ext: &str) -> std::path::PathBuf {
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

    #[test]
    fn sha256_file_matches_sha256_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");
        let data = b"hello world";
        std::fs::write(&file_path, data).unwrap();

        let file_hash = sha256_file(&file_path).unwrap();
        let bytes_hash = sha256_bytes(data);
        assert_eq!(file_hash, bytes_hash);
    }

    #[test]
    fn sha256_file_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("empty.bin");
        std::fs::write(&file_path, b"").unwrap();

        let hash = sha256_file(&file_path).unwrap();
        let hex = to_hex(&hash);
        // Known SHA-256 of empty data
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_file_nonexistent_returns_error() {
        let result = sha256_file(Path::new("/nonexistent/file.bin"));
        assert!(result.is_err());
    }

    #[test]
    fn sha256_file_large_data() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("large.bin");
        // Write data larger than the 8192-byte buffer to test the loop
        let data = vec![0xABu8; 20_000];
        std::fs::write(&file_path, &data).unwrap();

        let file_hash = sha256_file(&file_path).unwrap();
        let bytes_hash = sha256_bytes(&data);
        assert_eq!(file_hash, bytes_hash);
    }

    #[test]
    fn to_hex_known_value() {
        let hash = sha256_bytes(b"hello");
        let hex = to_hex(&hash);
        assert_eq!(
            hex,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn attachment_path_different_extensions() {
        let base = std::path::PathBuf::from("/attachments");
        let hash = "aabbccdd00112233aabbccdd00112233aabbccdd00112233aabbccdd00112233";

        let png_path = attachment_path(&base, hash, "png");
        let pdf_path = attachment_path(&base, hash, "pdf");

        assert!(png_path.to_str().unwrap().ends_with(".png"));
        assert!(pdf_path.to_str().unwrap().ends_with(".pdf"));
        // Same prefix dir
        assert_eq!(png_path.parent(), pdf_path.parent());
    }

    #[test]
    fn attachment_path_uses_first_two_hex_chars_as_subdir() {
        let base = std::path::PathBuf::from("/store");
        let hash = "ff11223344556677ff11223344556677ff11223344556677ff11223344556677";
        let path = attachment_path(&base, hash, "tiff");
        let parent = path.parent().unwrap();
        assert_eq!(parent.file_name().unwrap().to_str().unwrap(), "ff");
    }
}
