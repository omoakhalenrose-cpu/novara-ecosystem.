use novara::crypto::ed25519::Ed25519Identity;
use novara::crypto::{hash_to_hex, sha256};

#[test]
fn sha256_matches_standard_test_vector() {
    let hash = sha256(b"abc");

    assert_eq!(
        hash_to_hex(&hash),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn sha256_returns_32_bytes() {
    let hash = sha256(b"NOVARA");

    assert_eq!(hash.len(), 32);
}

#[test]
fn sha256_is_deterministic() {
    let first = sha256(b"NOVARA");
    let second = sha256(b"NOVARA");

    assert_eq!(first, second);
}

#[test]
fn generated_users_receive_different_public_keys() {
    let alice = Ed25519Identity::generate();
    let bob = Ed25519Identity::generate();

    assert_ne!(alice.public_key(), bob.public_key());
}

#[test]
fn ed25519_signature_verifies() {
    let alice = Ed25519Identity::generate();
    let message = b"NOVARA transaction";

    let public_key = alice.public_key();
    let signature = alice.sign(message);

    assert!(Ed25519Identity::verify(&public_key, message, &signature));
}

#[test]
fn one_user_cannot_verify_another_users_signature() {
    let alice = Ed25519Identity::generate();
    let bob = Ed25519Identity::generate();

    let message = b"NOVARA transaction";
    let signature = alice.sign(message);

    let bobs_public_key = bob.public_key();

    assert!(!Ed25519Identity::verify(
        &bobs_public_key,
        message,
        &signature
    ));
}
