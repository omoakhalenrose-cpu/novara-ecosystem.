//! NOVARA account identifiers and derivation.

use crate::account::network::NetworkId;
use crate::crypto::ed25519::Ed25519Identity;
use crate::crypto::sha256;

/// Domain separator for NOVARA account identifier version 1.
const ACCOUNT_ID_DOMAIN: &[u8] = b"NOVARA_ACCOUNT_ID_V1";

/// An error encountered while deriving an account identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountIdError {
    InvalidPublicKey,
}

/// A unique 32-byte identifier for a NOVARA account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountId(pub [u8; 32]);

impl AccountId {
    /// Creates an account identifier from 32 bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Derives an individual account identifier from a valid Ed25519 public key.
    pub fn try_from_ed25519_public_key(
        network: NetworkId,
        public_key: &[u8; 32],
    ) -> Result<Self, AccountIdError> {
        if !Ed25519Identity::is_valid_public_key(public_key) {
            return Err(AccountIdError::InvalidPublicKey);
        }

        let mut input = Vec::with_capacity(ACCOUNT_ID_DOMAIN.len() + 1 + public_key.len());

        input.extend_from_slice(ACCOUNT_ID_DOMAIN);
        input.push(network.as_byte());
        input.extend_from_slice(public_key);

        Ok(Self(sha256(&input)))
    }

    /// Returns the identifier's underlying bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::ed25519::Ed25519Identity;

    #[test]
    fn account_id_is_deterministic() {
        let identity = Ed25519Identity::generate();
        let public_key = identity.public_key();

        let first =
            AccountId::try_from_ed25519_public_key(NetworkId::Testnet, &public_key).unwrap();

        let second =
            AccountId::try_from_ed25519_public_key(NetworkId::Testnet, &public_key).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn account_id_differs_between_networks() {
        let identity = Ed25519Identity::generate();
        let public_key = identity.public_key();

        let testnet =
            AccountId::try_from_ed25519_public_key(NetworkId::Testnet, &public_key).unwrap();

        let mainnet =
            AccountId::try_from_ed25519_public_key(NetworkId::Mainnet, &public_key).unwrap();

        assert_ne!(testnet, mainnet);
    }

    #[test]
    fn different_public_keys_produce_different_identifiers() {
        let first_identity = Ed25519Identity::generate();
        let second_identity = Ed25519Identity::generate();

        let first = AccountId::try_from_ed25519_public_key(
            NetworkId::Testnet,
            &first_identity.public_key(),
        )
        .unwrap();

        let second = AccountId::try_from_ed25519_public_key(
            NetworkId::Testnet,
            &second_identity.public_key(),
        )
        .unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn weak_public_key_is_rejected() {
        let invalid_key = [0u8; 32];

        let result = AccountId::try_from_ed25519_public_key(NetworkId::Testnet, &invalid_key);

        assert_eq!(result, Err(AccountIdError::InvalidPublicKey));
        }
}
