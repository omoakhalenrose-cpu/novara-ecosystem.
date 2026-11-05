//! NOVARA block structure and deterministic hashing.

use sha2::{Digest, Sha256};

use crate::account::network::NetworkId;
use crate::transaction::
{ChainId, Transaction,
signing::TransactionSignature}
;

/// Domain separator for NOVARA block hashing version 1.
const BLOCK_DOMAIN: &[u8] = b"NOVARA_BLOCK_V1";

/// Identifies a block using its SHA-256 hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockHash([u8; 32]);

impl BlockHash {
    /// Creates a block hash from 32 bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the hash bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A transaction together with its digital signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTransaction {
    pub transaction: Transaction,
    pub signature: TransactionSignature,
}

/// Header containing the identifying information of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHeader {
    pub network: NetworkId,
    pub chain_id: ChainId,
    pub height: u64,
    pub timestamp: u64,
    pub previous_hash: BlockHash,
}

/// A NOVARA block containing signed transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<SignedTransaction>,
}

impl Block {
    /// Creates a block with its header and signed transactions.
    pub fn new(
        network: NetworkId,
        chain_id: ChainId,
        height: u64,
        timestamp: u64,
        previous_hash: BlockHash,
        transactions: Vec<SignedTransaction>,
    ) -> Self {
        Self {
            header: BlockHeader {
                network,
                chain_id,
                height,
                timestamp,
                previous_hash,
            },
            transactions,
        }
    }

    /// Calculates the deterministic SHA-256 hash of the block.
    ///
    /// Transaction order is preserved and signatures are included.
    pub fn hash(&self) -> BlockHash {
        let mut hasher = Sha256::new();

        hasher.update(BLOCK_DOMAIN);
        hasher.update([self.header.network.as_byte()]);
        hasher.update(self.header.chain_id.as_bytes());
        hasher.update(self.header.height.to_be_bytes());
        hasher.update(self.header.timestamp.to_be_bytes());
        hasher.update(self.header.previous_hash.as_bytes());

        hasher.update((self.transactions.len() as u64).to_be_bytes());

        for signed_transaction in &self.transactions {
            let encoded = signed_transaction.transaction.encode();

            hasher.update((encoded.len() as u64).to_be_bytes());
            hasher.update(&encoded);
            hasher.update(signed_transaction.signature.public_key);
            hasher.update(signed_transaction.signature.signature);
        }

        BlockHash::new(hasher.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_hash_is_deterministic() {
        let block = Block::new(
            NetworkId::Development,
            ChainId::new([1u8; 32]),
            1,
            1_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        );

        assert_eq!(block.hash(), block.hash());
    }

    #[test]
    fn different_block_heights_produce_different_hashes() {
        let first = Block::new(
            NetworkId::Development,
            ChainId::new([1u8; 32]),
            1,
            1_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        );

        let second = Block::new(
            NetworkId::Development,
            ChainId::new([1u8; 32]),
            2,
            1_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        );

        assert_ne!(first.hash(), second.hash());
    }

    #[test]
    fn different_previous_hashes_produce_different_block_hashes() {
        let first = Block::new(
            NetworkId::Development,
            ChainId::new([1u8; 32]),
            1,
            1_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        );

        let second = Block::new(
            NetworkId::Development,
            ChainId::new([1u8; 32]),
            1,
            1_000,
            BlockHash::new([2u8; 32]),
            Vec::new(),
        );

        assert_ne!(first.hash(), second.hash());
    }

    #[test]
    fn different_timestamps_produce_different_hashes() {
        let first = Block::new(
            NetworkId::Development,
            ChainId::new([1u8; 32]),
            1,
            1_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        );

        let second = Block::new(
            NetworkId::Development,
            ChainId::new([1u8; 32]),
            1,
            2_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        );

        assert_ne!(first.hash(), second.hash());
    }
}
