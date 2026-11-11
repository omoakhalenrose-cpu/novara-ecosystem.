//! NOVARA block validation.

use std::collections::HashSet;

use crate::account::network::NetworkId;
use crate::transaction::validation::{validate_transaction, ValidationContext, ValidationError};

use super::{
    Block, BlockHash, MAX_BLOCK_SIZE, MAX_TRANSACTION_SIZE, MAX_TRANSACTIONS_PER_BLOCK,
};

/// Rules describing the expected position and identity of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockValidationContext {
    pub network: NetworkId,
    pub chain_id: crate::transaction::ChainId,
    pub genesis_hash: crate::genesis::GenesisHash,
    pub expected_height: u64,
    pub expected_previous_hash: BlockHash,
}

/// Errors returned during block validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockValidationError {
    WrongNetwork,
    WrongChain,
    WrongGenesis,
    WrongHeight,
    WrongPreviousHash,
    OversizedBlock,
    TooManyTransactions,
    OversizedTransaction,
    DuplicateTransaction,
    InvalidTransaction(ValidationError),
}

/// Validates a block against the expected chain context.
pub fn validate_block(
    block: &Block,
    context: &BlockValidationContext,
) -> Result<(), BlockValidationError> {
    if block.header.network != context.network {
        return Err(BlockValidationError::WrongNetwork);
    }

    if block.header.chain_id != context.chain_id {
        return Err(BlockValidationError::WrongChain);
    }

    if block.header.genesis_hash != context.genesis_hash {
        return Err(BlockValidationError::WrongGenesis);
    }

    if block.header.height != context.expected_height {
        return Err(BlockValidationError::WrongHeight);
    }

    if block.header.previous_hash != context.expected_previous_hash {
        return Err(BlockValidationError::WrongPreviousHash);
    }

    if block.transactions.len() > MAX_TRANSACTIONS_PER_BLOCK {
        return Err(BlockValidationError::TooManyTransactions);
    }

    // Calculate the encoded block size without allocating a complete
    // additional encoded copy of the block.
    let mut encoded_size = b"NOVARA_BLOCK_ENCODING_V2".len()
        + 1
        + 32
        + 32
        + 8
        + 8
        + 32
        + 32
        + 8;

    // Track canonical transaction bytes to reject duplicates.
    let mut seen_transactions = HashSet::new();

    for signed_transaction in &block.transactions {
        let transaction_bytes = signed_transaction.transaction.encode();

        if transaction_bytes.len() > MAX_TRANSACTION_SIZE {
            return Err(BlockValidationError::OversizedTransaction);
        }

        if !seen_transactions.insert(transaction_bytes.clone()) {
            return Err(BlockValidationError::DuplicateTransaction);
        }

        // Each transaction includes an 8-byte length prefix,
        // a 32-byte public key and a 64-byte signature.
        encoded_size = encoded_size
            .checked_add(8)
            .and_then(|size| size.checked_add(transaction_bytes.len()))
            .and_then(|size| size.checked_add(32 + 64))
            .ok_or(BlockValidationError::OversizedBlock)?;

        if encoded_size > MAX_BLOCK_SIZE {
            return Err(BlockValidationError::OversizedBlock);
        }
    }

    let transaction_context = ValidationContext {
        network: context.network,
        chain_id: context.chain_id,
    };

    for signed_transaction in &block.transactions {
        validate_transaction(
            &signed_transaction.transaction,
            &signed_transaction.signature,
            &transaction_context,
        )
        .map_err(BlockValidationError::InvalidTransaction)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::account::address::AccountId;
    use crate::crypto::ed25519::Ed25519Identity;
    use crate::genesis::{GenesisConfig, InitializedGenesis};
    use crate::transaction::signing::sign_transaction;
    use crate::transaction::{AssetId, ChainId, Transaction};

    fn setup() -> (Block, BlockValidationContext) {
        let identity = Ed25519Identity::generate();
        let network = NetworkId::Development;

        let genesis =
            InitializedGenesis::new(GenesisConfig::new(network, Vec::new())).unwrap();

        let chain_id = genesis.chain_id();
        let genesis_hash = genesis.genesis_hash();
        let previous_hash = BlockHash::new([2u8; 32]);

        let sender =
            AccountId::try_from_ed25519_public_key(network, &identity.public_key()).unwrap();

        let transaction = Transaction::new(
            1,
            network,
            chain_id,
            sender,
            AccountId::new([3u8; 32]),
            AssetId::new([4u8; 32]),
            100,
            0,
        )
        .unwrap();

        let signature = sign_transaction(&transaction, &identity);

        let signed_transaction = super::super::SignedTransaction {
            transaction,
            signature,
        };

        let block = Block::new(
            network,
            chain_id,
            genesis_hash,
            1,
            1_000,
            previous_hash,
            [0u8; 32],
            vec![signed_transaction],
        );

        let context = BlockValidationContext {
            network,
            chain_id,
            genesis_hash,
            expected_height: 1,
            expected_previous_hash: previous_hash,
        };

        (block, context)
    }

    #[test]
    fn valid_block_is_accepted() {
        let (block, context) = setup();

        assert_eq!(validate_block(&block, &context), Ok(()));
    }

    #[test]
    fn wrong_network_is_rejected() {
        let (block, mut context) = setup();
        context.network = NetworkId::Testnet;

        assert_eq!(
            validate_block(&block, &context),
            Err(BlockValidationError::WrongNetwork)
        );
    }

    #[test]
    fn wrong_chain_is_rejected() {
        let (block, mut context) = setup();
        context.chain_id = ChainId::new([9u8; 32]);

        assert_eq!(
            validate_block(&block, &context),
            Err(BlockValidationError::WrongChain)
        );
    }

    #[test]
    fn wrong_genesis_is_rejected() {
        let (block, mut context) = setup();

        let different_genesis = InitializedGenesis::new(GenesisConfig::new(
            NetworkId::Development,
            vec![crate::genesis::GenesisAllocation {
                account_id: AccountId::new([5u8; 32]),
                asset_id: AssetId::new([6u8; 32]),
                amount: 100,
            }],
        ))
        .unwrap();

        context.genesis_hash = different_genesis.genesis_hash();

        assert_eq!(
            validate_block(&block, &context),
            Err(BlockValidationError::WrongGenesis)
        );
    }

    #[test]
    fn wrong_height_is_rejected() {
        let (block, mut context) = setup();
        context.expected_height = 2;

        assert_eq!(
            validate_block(&block, &context),
            Err(BlockValidationError::WrongHeight)
        );
    }

    #[test]
    fn wrong_previous_hash_is_rejected() {
        let (block, mut context) = setup();
        context.expected_previous_hash = BlockHash::new([9u8; 32]);

        assert_eq!(
            validate_block(&block, &context),
            Err(BlockValidationError::WrongPreviousHash)
        );
    }

    #[test]
    fn invalid_transaction_signature_is_rejected() {
        let (mut block, context) = setup();

        block.transactions[0].transaction.amount = 200;

        assert_eq!(
            validate_block(&block, &context),
            Err(BlockValidationError::InvalidTransaction(
                ValidationError::InvalidSignature
            ))
        );
    }

    #[test]
    fn duplicate_transaction_is_rejected() {
        let (mut block, context) = setup();

        let duplicate = block.transactions[0].clone();
        block.transactions.push(duplicate);

        assert_eq!(
            validate_block(&block, &context),
            Err(BlockValidationError::DuplicateTransaction)
        );
    }

    #[test]
    fn too_many_transactions_are_rejected() {
        let (mut block, context) = setup();

        block.transactions = vec![block.transactions[0].clone(); MAX_TRANSACTIONS_PER_BLOCK + 1];

        assert_eq!(
            validate_block(&block, &context),
            Err(BlockValidationError::TooManyTransactions)
        );
    }
}
