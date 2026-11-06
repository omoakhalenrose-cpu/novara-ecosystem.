//! NOVARA application state.
//!
//! Stores account balances and transaction nonces.
//! Supports multiple digital assets per account.
//! Provides deterministic cryptographic state roots.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::account::address::AccountId;
use crate::genesis::InitializedGenesis;
use crate::transaction::AssetId;

pub mod execution;

/// Domain separator for NOVARA application state hashes.
const STATE_ROOT_DOMAIN: &[u8] = b"NOVARA_STATE_ROOT_V1";

/// Stores the state belonging to one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountState {
    balances: BTreeMap<[u8; 32], u128>,
    nonce: u64,
}

impl AccountState {
    fn new() -> Self {
        Self {
            balances: BTreeMap::new(),
            nonce: 0,
        }
    }

    /// Returns the account's current transaction nonce.
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Returns the balance of a particular asset.
    pub fn balance(&self, asset_id: &AssetId) -> u128 {
        self.balances
            .get(asset_id.as_bytes())
            .copied()
            .unwrap_or(0)
    }
}

/// Errors returned by application state operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    AccountAlreadyExists,
    AccountNotFound,
}

/// Errors returned during genesis state initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenesisInitializationError {
    BalanceOverflow,
}

/// Stores NOVARA's application state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct State {
    accounts: BTreeMap<[u8; 32], AccountState>,
}

impl State {
    /// Creates an empty application state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initializes application state from a validated genesis.
    pub fn from_genesis(
        genesis: &InitializedGenesis,
    ) -> Result<Self, GenesisInitializationError> {
        let config = genesis.config();
        let mut state = Self::new();

        for allocation in &config.allocations {
            let account_key = *allocation.account_id.as_bytes();
            let asset_key = *allocation.asset_id.as_bytes();

            let account = state
                .accounts
                .entry(account_key)
                .or_insert_with(AccountState::new);

            let current_balance = account.balances.get(&asset_key).copied().unwrap_or(0);

            let new_balance = current_balance
                .checked_add(allocation.amount)
                .ok_or(GenesisInitializationError::BalanceOverflow)?;

            account.balances.insert(asset_key, new_balance);
        }

        Ok(state)
    }

    /// Creates a new account in application state.
    pub fn create_account(&mut self, account_id: AccountId) -> Result<(), StateError> {
        let key = *account_id.as_bytes();

        if self.accounts.contains_key(&key) {
            return Err(StateError::AccountAlreadyExists);
        }

        self.accounts.insert(key, AccountState::new());

        Ok(())
    }

    /// Checks whether an account exists.
    pub fn account_exists(&self, account_id: &AccountId) -> bool {
        self.accounts.contains_key(account_id.as_bytes())
    }

    /// Returns an account's current balance for an asset.
    pub fn balance_of(
        &self,
        account_id: &AccountId,
        asset_id: &AssetId,
    ) -> Result<u128, StateError> {
        let account = self
            .accounts
            .get(account_id.as_bytes())
            .ok_or(StateError::AccountNotFound)?;

        Ok(account.balance(asset_id))
    }

    /// Returns an account's current transaction nonce.
    pub fn nonce_of(&self, account_id: &AccountId) -> Result<u64, StateError> {
        let account = self
            .accounts
            .get(account_id.as_bytes())
            .ok_or(StateError::AccountNotFound)?;

        Ok(account.nonce())
    }

    /// Returns the number of accounts stored in the state.
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Calculates a deterministic SHA-256 root of the entire application state.
    ///
    /// Accounts and assets are hashed in sorted order because they are stored
    /// in BTreeMaps. Identical application states therefore produce identical
    /// state roots, regardless of insertion order.
    pub fn state_root(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();

        hasher.update(STATE_ROOT_DOMAIN);
        hasher.update((self.accounts.len() as u64).to_le_bytes());

        for (account_id, account_state) in &self.accounts {
            hasher.update(account_id);
            hasher.update(account_state.nonce.to_le_bytes());

            hasher.update((account_state.balances.len() as u64).to_le_bytes());

            for (asset_id, balance) in &account_state.balances {
                hasher.update(asset_id);
                hasher.update(balance.to_le_bytes());
            }
        }

        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::network::NetworkId;
    use crate::genesis::{GenesisAllocation, GenesisConfig};

    fn sample_account(value: u8) -> AccountId {
        AccountId::new([value; 32])
    }

    fn sample_allocation(
        account_id: AccountId,
        asset_id: AssetId,
        amount: u128,
    ) -> GenesisAllocation {
        GenesisAllocation {
            account_id,
            asset_id,
            amount,
        }
    }

    fn initialize_genesis(config: GenesisConfig) -> InitializedGenesis {
        InitializedGenesis::new(config).unwrap()
    }

    #[test]
    fn new_state_contains_no_accounts() {
        let state = State::new();

        assert_eq!(state.account_count(), 0);
    }

    #[test]
    fn account_can_be_created() {
        let mut state = State::new();
        let account = sample_account(1);

        assert_eq!(state.create_account(account), Ok(()));
        assert!(state.account_exists(&account));
    }

    #[test]
    fn duplicate_account_creation_is_rejected() {
        let mut state = State::new();
        let account = sample_account(1);

        assert_eq!(state.create_account(account), Ok(()));

        assert_eq!(
            state.create_account(account),
            Err(StateError::AccountAlreadyExists)
        );
    }

    #[test]
    fn new_account_starts_with_zero_nonce() {
        let mut state = State::new();
        let account = sample_account(1);

        state.create_account(account).unwrap();

        assert_eq!(state.nonce_of(&account), Ok(0));
    }

    #[test]
    fn new_account_has_zero_asset_balance() {
        let mut state = State::new();
        let account = sample_account(1);
        let asset = AssetId::new([5u8; 32]);

        state.create_account(account).unwrap();

        assert_eq!(state.balance_of(&account, &asset), Ok(0));
    }

    #[test]
    fn missing_account_returns_error() {
        let state = State::new();
        let account = sample_account(1);

        assert_eq!(state.nonce_of(&account), Err(StateError::AccountNotFound));
    }

    #[test]
    fn genesis_initializes_account_balance() {
        let account = sample_account(1);
        let asset = AssetId::new([5u8; 32]);

        let config = GenesisConfig::new(
            NetworkId::Development,
            vec![sample_allocation(account, asset, 1_000)],
        );

        let genesis = initialize_genesis(config);
        let state = State::from_genesis(&genesis).unwrap();

        assert_eq!(state.account_count(), 1);
        assert_eq!(state.balance_of(&account, &asset), Ok(1_000));
        assert_eq!(state.nonce_of(&account), Ok(0));
    }

    #[test]
    fn genesis_can_allocate_multiple_assets_to_one_account() {
        let account = sample_account(1);
        let first_asset = AssetId::new([5u8; 32]);
        let second_asset = AssetId::new([6u8; 32]);

        let config = GenesisConfig::new(
            NetworkId::Development,
            vec![
                sample_allocation(account, first_asset, 500),
                sample_allocation(account, second_asset, 750),
            ],
        );

        let genesis = initialize_genesis(config);
        let state = State::from_genesis(&genesis).unwrap();

        assert_eq!(state.account_count(), 1);
        assert_eq!(state.balance_of(&account, &first_asset), Ok(500));
        assert_eq!(state.balance_of(&account, &second_asset), Ok(750));
    }

    #[test]
    fn genesis_can_allocate_to_multiple_accounts() {
        let first_account = sample_account(1);
        let second_account = sample_account(2);
        let asset = AssetId::new([5u8; 32]);

        let config = GenesisConfig::new(
            NetworkId::Development,
            vec![
                sample_allocation(first_account, asset, 1_000),
                sample_allocation(second_account, asset, 2_000),
            ],
        );

        let genesis = initialize_genesis(config);
        let state = State::from_genesis(&genesis).unwrap();

        assert_eq!(state.account_count(), 2);
        assert_eq!(state.balance_of(&first_account, &asset), Ok(1_000));
        assert_eq!(state.balance_of(&second_account, &asset), Ok(2_000));
    }

    #[test]
    fn initialized_genesis_preserves_account_balances() {
        let account = sample_account(1);
        let asset = AssetId::new([5u8; 32]);

        let config = GenesisConfig::new(
            NetworkId::Development,
            vec![sample_allocation(account, asset, 1_000)],
        );

        let genesis = initialize_genesis(config);

        let state = State::from_genesis(&genesis).unwrap();

        assert!(state.account_exists(&account));
        assert_eq!(state.balance_of(&account, &asset), Ok(1_000));
    }

    #[test]
    fn identical_states_produce_identical_state_roots() {
        let account = sample_account(1);
        let asset = AssetId::new([5u8; 32]);

        let config = GenesisConfig::new(
            NetworkId::Development,
            vec![sample_allocation(account, asset, 1_000)],
        );

        let genesis = initialize_genesis(config);
        let first_state = State::from_genesis(&genesis).unwrap();
        let second_state = State::from_genesis(&genesis).unwrap();

        assert_eq!(first_state.state_root(), second_state.state_root());
    }

    #[test]
    fn different_balances_produce_different_state_roots() {
        let account = sample_account(1);
        let asset = AssetId::new([5u8; 32]);

        let first_config = GenesisConfig::new(
            NetworkId::Development,
            vec![sample_allocation(account, asset, 1_000)],
        );

        let second_config = GenesisConfig::new(
            NetworkId::Development,
            vec![sample_allocation(account, asset, 2_000)],
        );

        let first_state =
            State::from_genesis(&initialize_genesis(first_config)).unwrap();

        let second_state =
            State::from_genesis(&initialize_genesis(second_config)).unwrap();

        assert_ne!(first_state.state_root(), second_state.state_root());
    }

    #[test]
    fn account_insertion_order_does_not_change_state_root() {
        let first_account = sample_account(1);
        let second_account = sample_account(2);

        let mut first_state = State::new();
        first_state.create_account(first_account).unwrap();
        first_state.create_account(second_account).unwrap();

        let mut second_state = State::new();
        second_state.create_account(second_account).unwrap();
        second_state.create_account(first_account).unwrap();

        assert_eq!(first_state.state_root(), second_state.state_root());
    }
}
