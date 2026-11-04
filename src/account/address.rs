//! NOVARA account identifiers and derivation.

use crate::crypto::sha256;
use crate::account::network::NetworkId;

/// Domain separator for NOVARA account identifier version 1.
const ACCOUNT_ID_DOMAIN: &[u8] = b"NOVARA_ACCOUNT_ID_V1";

/// A unique 32-byte identifier for a NOVARA account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountId(pub [u8; 32]);

impl AccountId {
    /// Creates an account identifier from 32 bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Derives an individual account identifier from an
    /// Ed25519 public key and its NOVARA network.
    pub fn from_ed25519_public_key(
        network: NetworkId,
        public_key: &[u8; 32],
    ) -> Self {
        let mut input = Vec::with_capacity(
            ACCOUNT_ID_DOMAIN.len() + 1 + public_key.len(),
        );

        input.extend_from_slice(ACCOUNT_ID_DOMAIN);
        input.push(network.as_byte());
        input.extend_from_slice(public_key);

        Self(sha256(&input))
    }

    /// Returns the identifier's underlying bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_id_is_deterministic() {
        let public_key = [7u8; 32];

        let first =
            AccountId::from_ed25519_public_key(NetworkId::Testnet, &public_key);

        let second =
            AccountId::from_ed25519_public_key(NetworkId::Testnet, &public_key);

        assert_eq!(first, second);
    }

    #[test]
    fn account_id_differs_between_networks() {
        let public_key = [7u8; 32];

        let testnet =
            AccountId::from_ed25519_public_key(NetworkId::Testnet, &public_key);

        let mainnet =
            AccountId::from_ed25519_public_key(NetworkId::Mainnet, &public_key);

        assert_ne!(testnet, mainnet);
    }

    #[test]
    fn different_public_keys_produce_different_identifiers() {
        let first_key = [7u8; 32];
        let second_key = [8u8; 32];

        let first =
            AccountId::from_ed25519_public_key(NetworkId::Testnet, &first_key);

        let second =
            AccountId::from_ed25519_public_key(NetworkId::Testnet, &second_key);

        assert_ne!(first, second);
    }
                    }
