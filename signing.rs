//! NOVARA transaction signing and signature verification.

use crate::crypto::ed25519::Ed25519Identity;

use super::Transaction;

/// A digital signature attached to a NOVARA transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionSignature {
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
}

/// Errors returned during transaction signature verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureError {
    InvalidSignature,
}

/// Signs a transaction using an Ed25519 identity.
pub fn sign_transaction(
    transaction: &Transaction,
    identity: &Ed25519Identity,
) -> TransactionSignature {
    let message = transaction.encode();

    TransactionSignature {
        public_key: identity.public_key(),
        signature: identity.sign(&message),
    }
}

/// Verifies a transaction's digital signature.
pub fn verify_transaction_signature(
    transaction: &Transaction,
    signed: &TransactionSignature,
) -> Result<(), SignatureError> {
    let message = transaction.encode();

    if Ed25519Identity::verify(
        &signed.public_key,
        &message,
        &signed.signature,
    ) {
        Ok(())
    } else {
        Err(SignatureError::InvalidSignature)
    }
}
