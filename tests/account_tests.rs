use novara::account::address::AccountId;
use novara::account::network::NetworkId;
use novara::crypto::ed25519::Ed25519Identity;

#[test]
fn individual_identity_produces_account_id() {
    let identity = Ed25519Identity::generate();
    let public_key = identity.public_key();

    let account_id =
        AccountId::from_ed25519_public_key(NetworkId::Testnet, &public_key);

    assert_eq!(account_id.as_bytes().len(), 32);
}

#[test]
fn different_users_receive_different_account_ids() {
    let alice = Ed25519Identity::generate();
    let bob = Ed25519Identity::generate();

    let alice_id = AccountId::from_ed25519_public_key(
        NetworkId::Testnet,
        &alice.public_key(),
    );

    let bob_id = AccountId::from_ed25519_public_key(
        NetworkId::Testnet,
        &bob.public_key(),
    );

    assert_ne!(alice_id, bob_id);
}

#[test]
fn same_identity_has_different_ids_on_different_networks() {
    let identity = Ed25519Identity::generate();
    let public_key = identity.public_key();

    let testnet_id =
        AccountId::from_ed25519_public_key(NetworkId::Testnet, &public_key);

    let mainnet_id =
        AccountId::from_ed25519_public_key(NetworkId::Mainnet, &public_key);

    assert_ne!(testnet_id, mainnet_id);
}
