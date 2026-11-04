//! NOVARA cryptographic foundation.

pub mod ed25519;

use sha2::{Digest, Sha256};

/// Computes the SHA-256 hash of arbitrary data.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(data);

    let mut result = [0u8; 32];
    result.copy_from_slice(&digest);

    result
}

/// Converts a 32-byte hash into a hexadecimal string.
pub fn hash_to_hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}
