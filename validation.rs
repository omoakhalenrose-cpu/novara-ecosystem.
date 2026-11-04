//! NOVARA transaction validation.

use crate::account::network::NetworkId;

use super::signing::{
    verify_transaction_signature,
    SignatureError,
    TransactionSignature,
};
use super::Transaction;

/// The currently supported transaction protocol version.
pub const SUPPORTED_TRANSACTION_VERSION: u16 = 1;

/// Rules describing the network and chain that accept transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationContext {
    pub network: NetworkId,
    pub chain_id: super::ChainId,
}

/// Errors returned during transaction validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    UnsupportedVersion,
    WrongNetwork,
    WrongChain,
    ZeroAmount,
    InvalidSignature,
    SenderMismatch,
}

/// Validates a transaction against the supplied network and chain context.
pub fn validate_transaction(
    transaction: &Transaction,
    signature: &TransactionSignature,
    context: &ValidationContext,
) -> Result<(), ValidationError> {
    if transaction.version != SUPPORTED_TRANSACTION_VERSION {
        return Err(ValidationError::UnsupportedVersion);
    }

    if transaction.network != context.network {
        return Err(ValidationError::WrongNetwork);
    }

    if transaction.chain_id != context.chain_id {
        return Err(ValidationError::WrongChain);
    }

    if transaction.amount == 0 {
        return Err(ValidationError::ZeroAmount);
    }

    verify_transaction_signature(transaction, signature).map_err(|error| match error {
        SignatureError::InvalidSignature => ValidationError::InvalidSignature,
        SignatureError::SenderMismatch => ValidationError::SenderMismatch,
    })?;

    Ok(())
}
