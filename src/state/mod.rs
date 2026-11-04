//! NOVARA application state foundation.
//!
//! This module stores account balances and transaction nonces.
//! It supports multiple digital assets per account.

use std::collections::BTreeMap;

use crate::account::address::AccountId;
use crate::transaction::AssetId;

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
        self.balances.get(asset_id.as_bytes()).copied().unwrap_or(0)
    }
}

/// Errors returned by application state operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    AccountAlreadyExists,
    AccountNotFound,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_account(value: u8) -> AccountId {
        AccountId::new([value; 32])
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
}
