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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{
        address::AccountId,
        network::NetworkId,
    };
    use super::super::{AssetId, ChainId};

    fn create_transaction() -> Transaction {
        Transaction::new(
            1,
            NetworkId::Development,
            ChainId::new([4u8; 32]),
            AccountId::new([1u8; 32]),
            AccountId::new([2u8; 32]),
            AssetId::new([3u8; 32]),
            100,
            0,
        )
        .unwrap()
    }

    #[test]
    fn valid_transaction_signature_is_accepted() {
        let identity = Ed25519Identity::generate();
        let transaction = create_transaction();
        let signed = sign_transaction(&transaction, &identity);

        assert_eq!(
            verify_transaction_signature(&transaction, &signed),
            Ok(())
        );
    }

    #[test]
    fn modified_transaction_signature_is_rejected() {
        let identity = Ed25519Identity::generate();
        let transaction = create_transaction();
        let signed = sign_transaction(&transaction, &identity);

        let mut modified_transaction = transaction.clone();
        modified_transaction.amount = 200;

        assert_eq!(
            verify_transaction_signature(&modified_transaction, &signed),
            Err(SignatureError::InvalidSignature)
        );
    }

    #[test]
    fn signature_from_another_identity_is_rejected() {
        let first_identity = Ed25519Identity::generate();
        let second_identity = Ed25519Identity::generate();
        let transaction = create_transaction();

        let mut signed = sign_transaction(&transaction, &first_identity);
        signed.public_key = second_identity.public_key();

        assert_eq!(
            verify_transaction_signature(&transaction, &signed),
            Err(SignatureError::InvalidSignature)
        );
    }
}
