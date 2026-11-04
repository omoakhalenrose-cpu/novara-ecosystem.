//! NOVARA transaction data model.

use crate::account::{address::AccountId, network::NetworkId};

/// Identifies a digital asset supported by NOVARA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetId(pub [u8; 32]);

impl AssetId {
    /// Creates an asset identifier from 32 bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the identifier's underlying bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Represents a NOVARA transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub version: u16,
    pub network: NetworkId,
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
            sender,
            recipient,
            asset_id,
            amount,
            nonce,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_transaction(amount: u128) -> Result<Transaction, TransactionError> {
        Transaction::new(
            1,
            NetworkId::Development,
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
}
