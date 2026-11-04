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
