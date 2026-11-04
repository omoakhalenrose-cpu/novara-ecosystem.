//! NOVARA Ed25519 cryptographic identity.

use ed25519_dalek::{SigningKey, VerifyingKey};
use rand_core::{OsRng, RngCore};

/// Represents an independently generated Ed25519 identity.
pub struct Ed25519Identity {
    signing_key: SigningKey,
}

impl Ed25519Identity {
    /// Generates a new identity using secure operating-system randomness.
    pub fn generate() -> Self {
        let mut secret_key = [0u8; 32];
        OsRng.fill_bytes(&mut secret_key);

        let signing_key = SigningKey::from_bytes(&secret_key);

        secret_key.fill(0);

        Self { signing_key }
    }

    /// Returns the public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Signs a message using this identity's private key.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;

        self.signing_key.sign(message).to_bytes()
    }

    /// Verifies a signature against a public key.
    pub fn verify(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
        use ed25519_dalek::Verifier;

        let verifying_key = match VerifyingKey::from_bytes(public_key) {
            Ok(key) => key,
            Err(_) => return false,
        };

        let signature = ed25519_dalek::Signature::from_bytes(signature);

        verifying_key.verify(message, &signature).is_ok()
    }
}
