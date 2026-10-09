//! SHA-256 helpers. Every file in an evidence root and every parsed record is hashed with these.

use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::Path;

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Streams `path` through SHA-256 in 64 KiB chunks. Returns `(hex_digest, size_in_bytes)`.
pub fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn hashes_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hashes_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn file_hash_matches_bytes_hash_and_reports_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.bin");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::File::create(&p).unwrap().write_all(&data).unwrap();
        let (digest, size) = sha256_file(&p).unwrap();
        assert_eq!(digest, sha256_hex(&data));
        assert_eq!(size, data.len() as u64);
    }
}
