//! NOVARA transaction validation.

use crate::account::network::NetworkId;

use super::Transaction;
use super::signing::{SignatureError, TransactionSignature, verify_transaction_signature};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::address::AccountId;
    use crate::crypto::ed25519::Ed25519Identity;
    use crate::transaction::{AssetId, ChainId, signing::sign_transaction};

    fn setup() -> (Transaction, TransactionSignature, ValidationContext) {
        let identity = Ed25519Identity::generate();

        let sender =
            AccountId::try_from_ed25519_public_key(NetworkId::Development, &identity.public_key())
                .unwrap();

        let chain_id = ChainId::new([4u8; 32]);

        let transaction = Transaction::new(
            1,
            NetworkId::Development,
            chain_id,
            sender,
            AccountId::new([2u8; 32]),
            AssetId::new([3u8; 32]),
            100,
            0,
        )
        .unwrap();

        let signature = sign_transaction(&transaction, &identity);

        let context = ValidationContext {
            network: NetworkId::Development,
            chain_id,
        };

        (transaction, signature, context)
    }

    #[test]
    fn valid_transaction_is_accepted() {
        let (transaction, signature, context) = setup();

        assert_eq!(
            validate_transaction(&transaction, &signature, &context),
            Ok(())
        );
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let (mut transaction, signature, context) = setup();
        transaction.version = 2;

        assert_eq!(
            validate_transaction(&transaction, &signature, &context),
            Err(ValidationError::UnsupportedVersion)
        );
    }

    #[test]
    fn wrong_network_is_rejected() {
        let (transaction, signature, mut context) = setup();
        context.network = NetworkId::Testnet;

        assert_eq!(
            validate_transaction(&transaction, &signature, &context),
            Err(ValidationError::WrongNetwork)
        );
    }

    #[test]
    fn wrong_chain_is_rejected() {
        let (transaction, signature, mut context) = setup();
        context.chain_id = ChainId::new([9u8; 32]);

        assert_eq!(
            validate_transaction(&transaction, &signature, &context),
            Err(ValidationError::WrongChain)
        );
    }

    #[test]
    fn zero_amount_is_rejected() {
        let (mut transaction, signature, context) = setup();
        transaction.amount = 0;

        assert_eq!(
            validate_transaction(&transaction, &signature, &context),
            Err(ValidationError::ZeroAmount)
        );
    }

    #[test]
    fn modified_transaction_is_rejected() {
        let (mut transaction, signature, context) = setup();
        transaction.amount = 200;

        assert_eq!(
            validate_transaction(&transaction, &signature, &context),
            Err(ValidationError::InvalidSignature)
        );
    }
}
