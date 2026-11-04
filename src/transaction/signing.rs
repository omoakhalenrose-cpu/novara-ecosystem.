//! NOVARA transaction signing and sender identity verification.

use crate::account::address::AccountId;
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
    SenderMismatch,
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

/// Verifies the signature and confirms that the public key
/// derives the sender's account identifier.
pub fn verify_transaction_signature(
    transaction: &Transaction,
    signed: &TransactionSignature,
) -> Result<(), SignatureError> {
    if !Ed25519Identity::verify(&signed.public_key, &transaction.encode(), &signed.signature) {
        return Err(SignatureError::InvalidSignature);
    }

    let derived_account =
        AccountId::try_from_ed25519_public_key(transaction.network, &signed.public_key)
            .map_err(|_| SignatureError::InvalidSignature)?;

    if derived_account != transaction.sender {
        return Err(SignatureError::SenderMismatch);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::address::AccountId;
    use crate::account::network::NetworkId;
    use crate::transaction::{AssetId, ChainId, Transaction};

    fn create_transaction(identity: &Ed25519Identity) -> Transaction {
        let sender =
            AccountId::try_from_ed25519_public_key(NetworkId::Development, &identity.public_key())
                .unwrap();

        Transaction::new(
            1,
            NetworkId::Development,
            ChainId::new([4u8; 32]),
            sender,
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
        let transaction = create_transaction(&identity);
        let signed = sign_transaction(&transaction, &identity);

        assert_eq!(verify_transaction_signature(&transaction, &signed), Ok(()));
    }

    #[test]
    fn modified_transaction_signature_is_rejected() {
        let identity = Ed25519Identity::generate();
        let transaction = create_transaction(&identity);
        let signed = sign_transaction(&transaction, &identity);

        let mut modified = transaction.clone();
        modified.amount = 200;

        assert_eq!(
            verify_transaction_signature(&modified, &signed),
            Err(SignatureError::InvalidSignature)
        );
    }

    #[test]
    fn signature_from_another_identity_is_rejected() {
        let first = Ed25519Identity::generate();
        let second = Ed25519Identity::generate();
        let transaction = create_transaction(&first);

        let mut signed = sign_transaction(&transaction, &first);
        signed.public_key = second.public_key();

        assert_eq!(
            verify_transaction_signature(&transaction, &signed),
            Err(SignatureError::InvalidSignature)
        );
    }

    #[test]
    fn valid_signature_from_wrong_sender_is_rejected() {
        let first = Ed25519Identity::generate();
        let second = Ed25519Identity::generate();
        let transaction = create_transaction(&second);
        let signed = sign_transaction(&transaction, &first);

        assert_eq!(
            verify_transaction_signature(&transaction, &signed),
            Err(SignatureError::SenderMismatch)
        );
    }
}
