//! NOVARA transaction data model and canonical encoding.

use crate::account::{address::AccountId, network::NetworkId};

pub mod signing;
pub mod validation;

/// Domain separator for NOVARA transaction encoding version 1.
const TRANSACTION_DOMAIN: &[u8] = b"NOVARA_TRANSACTION_V1";

/// Identifies a digital asset supported by NOVARA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetId(pub [u8; 32]);

impl AssetId {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Identifies a particular NOVARA chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChainId(pub [u8; 32]);

impl ChainId {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Represents a NOVARA transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub version: u16,
    pub network: NetworkId,
    pub chain_id: ChainId,
    pub sender: AccountId,
    pub recipient: AccountId,
    pub asset_id: AssetId,
    pub amount: u128,
    pub nonce: u64,
}

/// Errors returned when constructing a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionError {
    ZeroAmount,
}

impl Transaction {
    /// Creates a transaction after checking its basic amount requirement.
    pub fn new(
        version: u16,
        network: NetworkId,
        chain_id: ChainId,
        sender: AccountId,
        recipient: AccountId,
        asset_id: AssetId,
        amount: u128,
        nonce: u64,
    ) -> Result<Self, TransactionError> {
        if amount == 0 {
            return Err(TransactionError::ZeroAmount);
        }

        Ok(Self {
            version,
            network,
            chain_id,
            sender,
            recipient,
            asset_id,
            amount,
            nonce,
        })
    }

    /// Encodes the transaction into its canonical byte representation.
    ///
    /// All multi-byte integers use big-endian byte order.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes =
            Vec::with_capacity(TRANSACTION_DOMAIN.len() + 2 + 1 + 32 + 32 + 32 + 32 + 16 + 8);

        bytes.extend_from_slice(TRANSACTION_DOMAIN);
        bytes.extend_from_slice(&self.version.to_be_bytes());
        bytes.push(self.network.as_byte());
        bytes.extend_from_slice(self.chain_id.as_bytes());
        bytes.extend_from_slice(self.sender.as_bytes());
        bytes.extend_from_slice(self.recipient.as_bytes());
        bytes.extend_from_slice(self.asset_id.as_bytes());
        bytes.extend_from_slice(&self.amount.to_be_bytes());
        bytes.extend_from_slice(&self.nonce.to_be_bytes());

        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_transaction(amount: u128) -> Result<Transaction, TransactionError> {
        Transaction::new(
            1,
            NetworkId::Development,
            ChainId::new([4u8; 32]),
            AccountId::new([1u8; 32]),
            AccountId::new([2u8; 32]),
            AssetId::new([3u8; 32]),
            amount,
            0,
        )
    }

    #[test]
    fn transaction_is_created_with_valid_amount() {
        let transaction = sample_transaction(100).unwrap();

        assert_eq!(transaction.amount, 100);
        assert_eq!(transaction.version, 1);
        assert_eq!(transaction.nonce, 0);
    }

    #[test]
    fn zero_amount_is_rejected() {
        let result = sample_transaction(0);

        assert_eq!(result, Err(TransactionError::ZeroAmount));
    }

    #[test]
    fn transaction_preserves_asset_identifier() {
        let transaction = sample_transaction(500).unwrap();

        assert_eq!(transaction.asset_id, AssetId::new([3u8; 32]));
    }

    #[test]
    fn canonical_encoding_is_deterministic() {
        let transaction = sample_transaction(100).unwrap();

        assert_eq!(transaction.encode(), transaction.encode());
    }

    #[test]
    fn canonical_encoding_contains_domain_separator() {
        let transaction = sample_transaction(100).unwrap();

        assert!(transaction.encode().starts_with(TRANSACTION_DOMAIN));
    }

    #[test]
    fn canonical_encoding_contains_chain_identifier() {
        let transaction = sample_transaction(100).unwrap();
        let encoded = transaction.encode();

        let chain_id_start = TRANSACTION_DOMAIN.len() + 2 + 1;

        assert_eq!(&encoded[chain_id_start..chain_id_start + 32], &[4u8; 32]);
    }
}
