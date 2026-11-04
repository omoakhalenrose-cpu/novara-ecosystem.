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
