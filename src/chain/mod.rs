//! NOVARA blockchain chain management.
//!
//! Maintains the ordered block sequence and application state.
//! Blocks are accepted only after validation, successful execution,
//! and State Root verification.

use crate::account::network::NetworkId;
use crate::block::validation::{validate_block, BlockValidationContext, BlockValidationError};
use crate::block::{Block, BlockHash};
use crate::genesis::{GenesisHash, InitializedGenesis};
use crate::state::execution::ExecutionError;
use crate::state::{GenesisInitializationError, State};
use crate::transaction::ChainId;
use crate::transaction::validation::ValidationContext;

/// Errors returned by chain initialization and block application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainError {
    GenesisInitialization(GenesisInitializationError),
    InvalidBlock(BlockValidationError),
    TransactionExecution(ExecutionError),
    StateRootMismatch,
}

/// Maintains NOVARA's ordered blockchain and application state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    network: NetworkId,
    chain_id: ChainId,
    genesis_hash: GenesisHash,
    blocks: Vec<Block>,
    state: State,
}

impl Chain {
    /// Initializes a new chain from validated genesis.
    ///
    /// Genesis represents height zero. The first accepted block has height one.
    pub fn new(genesis: &InitializedGenesis) -> Result<Self, ChainError> {
        let state = State::from_genesis(genesis).map_err(ChainError::GenesisInitialization)?;

        Ok(Self {
            network: genesis.config().network,
            chain_id: genesis.chain_id(),
            genesis_hash: genesis.genesis_hash(),
            blocks: Vec::new(),
            state,
        })
    }

    /// Returns the chain's network.
    pub const fn network(&self) -> NetworkId {
        self.network
    }

    /// Returns the chain's identifier.
    pub const fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    /// Returns the original genesis hash.
    pub const fn genesis_hash(&self) -> GenesisHash {
        self.genesis_hash
    }

    /// Returns the current chain height.
    ///
    /// An initialized chain with no blocks has height zero.
    pub fn height(&self) -> u64 {
        self.blocks.len() as u64
    }

    /// Returns the number of accepted blocks after genesis.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Returns the latest block hash.
    ///
    /// Before the first block, the genesis hash is used as the chain anchor.
    pub fn latest_hash(&self) -> BlockHash {
        self.blocks
            .last()
            .map(Block::hash)
            .unwrap_or_else(|| BlockHash::new(*self.genesis_hash.as_bytes()))
    }

    /// Returns a reference to the application state.
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// Returns a block by its height.
    ///
    /// Height zero represents genesis and has no stored Block.
    pub fn block_at_height(&self, height: u64) -> Option<&Block> {
        if height == 0 {
            return None;
        }

        self.blocks.get((height - 1) as usize)
    }

    /// Validates and applies a new block atomically.
    ///
    /// Transactions execute against a cloned state.
    /// The resulting State Root must match the root committed
    /// in the block header before the chain can be updated.
    pub fn apply_block(&mut self, block: Block) -> Result<(), ChainError> {
        let expected_height = self
            .height()
            .checked_add(1)
            .ok_or(ChainError::InvalidBlock(BlockValidationError::WrongHeight))?;

        let expected_previous_hash = self.latest_hash();

        let block_context = BlockValidationContext {
            network: self.network,
            chain_id: self.chain_id,
            genesis_hash: self.genesis_hash,
            expected_height,
            expected_previous_hash,
        };

        validate_block(&block, &block_context).map_err(ChainError::InvalidBlock)?;

        let transaction_context = ValidationContext {
            network: self.network,
            chain_id: self.chain_id,
        };

        // Execute transactions against temporary state.
        let mut candidate_state = self.state.clone();

        for signed_transaction in &block.transactions {
            candidate_state
                .execute_transaction(
                    &signed_transaction.transaction,
                    &signed_transaction.signature,
                    &transaction_context,
                )
                .map_err(ChainError::TransactionExecution)?;
        }

        // Verify that the executed state matches the block commitment.
        let calculated_state_root = candidate_state.state_root();

        if calculated_state_root != block.header.state_root {
            return Err(ChainError::StateRootMismatch);
        }

        // Commit only after every validation succeeds.
        self.state = candidate_state;
        self.blocks.push(block);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::address::AccountId;
    use crate::crypto::ed25519::Ed25519Identity;
    use crate::genesis::{GenesisAllocation, GenesisConfig};
    use crate::transaction::signing::sign_transaction;
    use crate::transaction::{AssetId, Transaction};

    fn test_genesis() -> (InitializedGenesis, Ed25519Identity, AccountId) {
        let identity = Ed25519Identity::generate();

        let sender =
            AccountId::try_from_ed25519_public_key(NetworkId::Development, &identity.public_key())
                .unwrap();

        let recipient = AccountId::new([8u8; 32]);
        let asset_id = AssetId::new([4u8; 32]);

        let genesis = InitializedGenesis::new(GenesisConfig::new(
            NetworkId::Development,
            vec![
                GenesisAllocation {
                    account_id: sender,
                    asset_id,
                    amount: 1_000,
                },
                GenesisAllocation {
                    account_id: recipient,
                    asset_id,
                    amount: 1,
                },
            ],
        ))
        .unwrap();

        (genesis, identity, recipient)
    }

    fn create_valid_block(
        chain: &Chain,
        genesis: &InitializedGenesis,
        identity: &Ed25519Identity,
        recipient: AccountId,
        amount: u128,
    ) -> Block {
        let sender =
            AccountId::try_from_ed25519_public_key(NetworkId::Development, &identity.public_key())
                .unwrap();

        let transaction = Transaction::new(
            1,
            NetworkId::Development,
            genesis.chain_id(),
            sender,
            recipient,
            AssetId::new([4u8; 32]),
            amount,
            0,
        )
        .unwrap();

        let signature = sign_transaction(&transaction, identity);

        let signed_transaction = crate::block::SignedTransaction {
            transaction,
            signature,
        };

        // Calculate the expected state after executing this transaction.
        let transaction_context = ValidationContext {
            network: NetworkId::Development,
            chain_id: genesis.chain_id(),
        };

        let mut candidate_state = chain.state.clone();

        candidate_state
            .execute_transaction(
                &signed_transaction.transaction,
                &signed_transaction.signature,
                &transaction_context,
            )
            .unwrap();

        let expected_state_root = candidate_state.state_root();

        Block::new(
            NetworkId::Development,
            genesis.chain_id(),
            genesis.genesis_hash(),
            chain.height() + 1,
            1_000,
            chain.latest_hash(),
            expected_state_root,
            vec![signed_transaction],
        )
    }

    #[test]
    fn new_chain_starts_at_genesis_height() {
        let (genesis, _, _) = test_genesis();
        let chain = Chain::new(&genesis).unwrap();

        assert_eq!(chain.height(), 0);
        assert_eq!(chain.block_count(), 0);
        assert_eq!(
            chain.latest_hash().as_bytes(),
            genesis.genesis_hash().as_bytes()
        );
    }

    #[test]
    fn valid_block_is_accepted() {
        let (genesis, identity, recipient) = test_genesis();
        let mut chain = Chain::new(&genesis).unwrap();

        let block = create_valid_block(&chain, &genesis, &identity, recipient, 100);

        assert_eq!(chain.apply_block(block), Ok(()));
        assert_eq!(chain.height(), 1);
        assert_eq!(chain.block_count(), 1);
    }

    #[test]
    fn incorrect_state_root_is_rejected_atomically() {
        let (genesis, identity, recipient) = test_genesis();
        let mut chain = Chain::new(&genesis).unwrap();

        let mut block = create_valid_block(&chain, &genesis, &identity, recipient, 100);

        // Deliberately corrupt the State Root commitment.
        block.header.state_root = [9u8; 32];

        let previous_state = chain.state.clone();
        let previous_hash = chain.latest_hash();

        assert_eq!(chain.apply_block(block), Err(ChainError::StateRootMismatch));

        assert_eq!(chain.height(), 0);
        assert_eq!(chain.block_count(), 0);
        assert_eq!(chain.state, previous_state);
        assert_eq!(chain.latest_hash(), previous_hash);
    }

    #[test]
    fn invalid_block_does_not_change_chain() {
        let (genesis, identity, recipient) = test_genesis();
        let mut chain = Chain::new(&genesis).unwrap();

        let mut block = create_valid_block(&chain, &genesis, &identity, recipient, 100);

        block.header.height = 5;

        let previous_state = chain.state.clone();
        let previous_hash = chain.latest_hash();

        assert_eq!(
            chain.apply_block(block),
            Err(ChainError::InvalidBlock(BlockValidationError::WrongHeight))
        );

        assert_eq!(chain.height(), 0);
        assert_eq!(chain.state, previous_state);
        assert_eq!(chain.latest_hash(), previous_hash);
    }

    #[test]
    fn failed_transaction_does_not_partially_update_chain_state() {
        let (genesis, identity, recipient) = test_genesis();
        let mut chain = Chain::new(&genesis).unwrap();

        let sender =
            AccountId::try_from_ed25519_public_key(NetworkId::Development, &identity.public_key())
                .unwrap();

        // This signed transaction exceeds the sender's genesis balance.
        // Build the block directly so the test exercises apply_block's
        // rejection path rather than panicking inside create_valid_block.
        let transaction = Transaction::new(
            1,
            NetworkId::Development,
            genesis.chain_id(),
            sender,
            recipient,
            AssetId::new([4u8; 32]),
            2_000,
            0,
        )
        .unwrap();

        let signature = sign_transaction(&transaction, &identity);

        let block = Block::new(
            NetworkId::Development,
            genesis.chain_id(),
            genesis.genesis_hash(),
            chain.height() + 1,
            1_000,
            chain.latest_hash(),
            chain.state.state_root(),
            vec![crate::block::SignedTransaction {
                transaction,
                signature,
            }],
        );

        let previous_state = chain.state.clone();
        let previous_hash = chain.latest_hash();

        assert_eq!(
            chain.apply_block(block),
            Err(ChainError::TransactionExecution(
                ExecutionError::InsufficientBalance
            ))
        );

        assert_eq!(chain.height(), 0);
        assert_eq!(chain.block_count(), 0);
        assert_eq!(chain.state, previous_state);
        assert_eq!(chain.latest_hash(), previous_hash);
    }
}
