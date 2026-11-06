//! NOVARA transaction execution engine.

use crate::transaction::{
    Transaction,
    signing::TransactionSignature,
    validation::{ValidationContext, ValidationError, validate_transaction},
};

use super::State;

/// Errors returned when executing a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionError {
    InvalidTransaction(ValidationError),
    AccountNotFound,
    NonceMismatch,
    InsufficientBalance,
    BalanceOverflow,
    NonceOverflow,
}

/// Executes a validated transaction and updates application state.
impl State {
    pub fn execute_transaction(
        &mut self,
        transaction: &Transaction,
        signature: &TransactionSignature,
        context: &ValidationContext,
    ) -> Result<(), ExecutionError> {
        validate_transaction(transaction, signature, context)
            .map_err(ExecutionError::InvalidTransaction)?;

        let sender_key = *transaction.sender.as_bytes();
        let recipient_key = *transaction.recipient.as_bytes();
        let asset_key = *transaction.asset_id.as_bytes();

        let sender = self
            .accounts
            .get(&sender_key)
            .ok_or(ExecutionError::AccountNotFound)?;

        if sender.nonce != transaction.nonce {
            return Err(ExecutionError::NonceMismatch);
        }

        let sender_balance = sender.balances.get(&asset_key).copied().unwrap_or(0);

        if sender_balance < transaction.amount {
            return Err(ExecutionError::InsufficientBalance);
        }

        let next_nonce = sender
            .nonce
            .checked_add(1)
            .ok_or(ExecutionError::NonceOverflow)?;

        let recipient_balance = if sender_key == recipient_key {
            sender_balance
        } else {
            let recipient = self
                .accounts
                .get(&recipient_key)
                .ok_or(ExecutionError::AccountNotFound)?;

            recipient
                .balances
                .get(&asset_key)
                .copied()
                .unwrap_or(0)
                .checked_add(transaction.amount)
                .ok_or(ExecutionError::BalanceOverflow)?
        };

        if sender_key == recipient_key {
            let sender = self
                .accounts
                .get_mut(&sender_key)
                .ok_or(ExecutionError::AccountNotFound)?;

            sender.nonce = next_nonce;

            return Ok(());
        }

        let updated_sender_balance = sender_balance - transaction.amount;

        let sender = self
            .accounts
            .get_mut(&sender_key)
            .ok_or(ExecutionError::AccountNotFound)?;

        sender.balances.insert(asset_key, updated_sender_balance);
        sender.nonce = next_nonce;

        let recipient = self
            .accounts
            .get_mut(&recipient_key)
            .ok_or(ExecutionError::AccountNotFound)?;

        recipient.balances.insert(asset_key, recipient_balance);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::account::{address::AccountId, network::NetworkId};
    use crate::crypto::ed25519::Ed25519Identity;
    use crate::transaction::{AssetId, ChainId, Transaction, signing::sign_transaction};

    fn setup(
        amount: u128,
        nonce: u64,
    ) -> (State, Transaction, TransactionSignature, ValidationContext) {
        let identity = Ed25519Identity::generate();

        let sender =
            AccountId::try_from_ed25519_public_key(NetworkId::Development, &identity.public_key())
                .unwrap();

        let recipient = AccountId::new([2u8; 32]);
        let asset = AssetId::new([3u8; 32]);
        let chain_id = ChainId::new([4u8; 32]);

        let transaction = Transaction::new(
            1,
            NetworkId::Development,
            chain_id,
            sender,
            recipient,
            asset,
            amount,
            nonce,
        )
        .unwrap();

        let signature = sign_transaction(&transaction, &identity);

        let context = ValidationContext {
            network: NetworkId::Development,
            chain_id,
        };

        let mut state = State::new();

        state.create_account(sender).unwrap();
        state.create_account(recipient).unwrap();

        state
            .accounts
            .get_mut(sender.as_bytes())
            .unwrap()
            .balances
            .insert(*asset.as_bytes(), 1_000);

        (state, transaction, signature, context)
    }

    #[test]
    fn valid_transaction_transfers_balance() {
        let (mut state, transaction, signature, context) = setup(100, 0);

        assert_eq!(
            state.execute_transaction(&transaction, &signature, &context),
            Ok(())
        );

        assert_eq!(
            state.balance_of(&transaction.sender, &transaction.asset_id),
            Ok(900)
        );

        assert_eq!(
            state.balance_of(&transaction.recipient, &transaction.asset_id),
            Ok(100)
        );

        assert_eq!(state.nonce_of(&transaction.sender), Ok(1));
    }

    #[test]
    fn insufficient_balance_is_rejected_without_state_change() {
        let (mut state, transaction, signature, context) = setup(1_001, 0);

        let before = state.clone();

        assert_eq!(
            state.execute_transaction(&transaction, &signature, &context),
            Err(ExecutionError::InsufficientBalance)
        );

        assert_eq!(state, before);
    }

    #[test]
    fn invalid_signature_is_rejected_without_state_change() {
        let (mut state, transaction, mut signature, context) = setup(100, 0);

        signature.signature = [0u8; 64];

        let before = state.clone();

        assert_eq!(
            state.execute_transaction(&transaction, &signature, &context),
            Err(ExecutionError::InvalidTransaction(
                ValidationError::InvalidSignature
            ))
        );

        assert_eq!(state, before);
    }

    #[test]
    fn incorrect_nonce_is_rejected_without_state_change() {
        let (mut state, transaction, signature, context) = setup(100, 1);

        let before = state.clone();

        assert_eq!(
            state.execute_transaction(&transaction, &signature, &context),
            Err(ExecutionError::NonceMismatch)
        );

        assert_eq!(state, before);
    }

    #[test]
    fn wrong_network_is_rejected() {
        let (mut state, transaction, signature, mut context) = setup(100, 0);

        context.network = NetworkId::Testnet;

        assert_eq!(
            state.execute_transaction(&transaction, &signature, &context),
            Err(ExecutionError::InvalidTransaction(
                ValidationError::WrongNetwork
            ))
        );
    }

    #[test]
    fn wrong_chain_is_rejected() {
        let (mut state, transaction, signature, mut context) = setup(100, 0);

        context.chain_id = ChainId::new([9u8; 32]);

        assert_eq!(
            state.execute_transaction(&transaction, &signature, &context),
            Err(ExecutionError::InvalidTransaction(
                ValidationError::WrongChain
            ))
        );
    }

    #[test]
    fn recipient_balance_overflow_is_rejected_without_state_change() {
        let (mut state, transaction, signature, context) = setup(100, 0);

        state
            .accounts
            .get_mut(transaction.recipient.as_bytes())
            .unwrap()
            .balances
            .insert(*transaction.asset_id.as_bytes(), u128::MAX);

        let before = state.clone();

        assert_eq!(
            state.execute_transaction(&transaction, &signature, &context),
            Err(ExecutionError::BalanceOverflow)
        );

        assert_eq!(state, before);
    }

    #[test]
    fn nonce_overflow_is_rejected_without_state_change() {
        let (mut state, transaction, signature, context) = setup(100, u64::MAX);

        state
            .accounts
            .get_mut(transaction.sender.as_bytes())
            .unwrap()
            .nonce = u64::MAX;

        let before = state.clone();

        assert_eq!(
            state.execute_transaction(&transaction, &signature, &context),
            Err(ExecutionError::NonceOverflow)
        );

        assert_eq!(state, before);
    }

    #[test]
    fn self_transfer_preserves_balance_and_increments_nonce() {
        let identity = Ed25519Identity::generate();

        let sender = AccountId::try_from_ed25519_public_key(
            NetworkId::Development,
            &identity.public_key(),
        )
        .unwrap();

        let asset = AssetId::new([3u8; 32]);
        let chain_id = ChainId::new([4u8; 32]);

        let transaction = Transaction::new(
            1,
            NetworkId::Development,
            chain_id,
            sender,
            sender,
            asset,
            100,
            0,
        )
        .unwrap();

        let signature = sign_transaction(&transaction, &identity);

        let context = ValidationContext {
            network: NetworkId::Development,
            chain_id,
        };

        let mut state = State::new();
        state.create_account(sender).unwrap();

        state
            .accounts
            .get_mut(sender.as_bytes())
            .unwrap()
            .balances
            .insert(*asset.as_bytes(), 1_000);

        assert_eq!(
            state.execute_transaction(&transaction, &signature, &context),
            Ok(())
        );

        assert_eq!(state.balance_of(&sender, &asset), Ok(1_000));
        assert_eq!(state.nonce_of(&sender), Ok(1));
    }
}
