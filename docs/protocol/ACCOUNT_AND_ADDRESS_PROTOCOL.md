NOVARA Account and Address Protocol

Version: 0.1.0
Status: Initial design proposal
Project: NOVARA Independent Layer-1 Blockchain

1. Purpose

The NOVARA Account and Address Protocol defines how accounts are identified, controlled and represented on the NOVARA network.

It must support individuals, businesses and smart contracts without requiring three unrelated address systems.

2. Account Categories

NOVARA will support three account categories.

2.1 Individual Accounts

Individual accounts represent people using NOVARA.

- Controlled by cryptographic signing keys.
- Each account has a unique NOVARA address.
- Additional recovery and authorisation mechanisms may be introduced through a future protocol upgrade.

2.2 Business Accounts

Business accounts represent companies and organisations.

They must support multiple authorised signers and configurable permissions.

Proposed roles:

- Owner: manages account administration and permissions.
- Administrator: manages approved account operations.
- Approver: authorises transactions.
- Viewer: can inspect account activity without authorising transactions.

The protocol must support configurable multi-signature approval requirements.

2.3 Smart-Contract Accounts

Smart-contract accounts represent deployed programs on NOVARA.

- Controlled by contract execution rules.
- Have unique addresses.
- Their address derivation must be deterministic.
- Their permissions and asset operations are governed by the contract's code and protocol rules.

3. Address and Identity Separation

An account address is not the same thing as a public key.

The system distinguishes:

- Private key: secret signing authority.
- Public key: verifies digital signatures.
- Account address: identifies an account on NOVARA.
- Account type: identifies whether an account is individual, business or smart contract.
- Account state: stores the account's data and permissions.

An address must not be treated as proof of account ownership.

4. Cryptographic Foundation

The initial cryptographic components are:

- SHA-256 for hashing.
- Ed25519 for digital signatures.

Each key-controlled account must have its own cryptographic keys.

Private keys must never be stored in public blockchain records.

Business accounts must support multiple authorised keys through an explicit authorisation policy.

Smart-contract accounts must use contract-defined authorisation rules.

5. Address Requirements

Every NOVARA address must:

1. Be unique within its defined network.
2. Have a deterministic derivation process.
3. Include a versioning mechanism.
4. Include network identification.
5. Include checksum protection against common transcription errors.
6. Be unambiguous when encoded and decoded.
7. Support future protocol upgrades.

The exact binary layout, encoding, checksum construction and derivation rules must be specified and tested before implementation.

6. Network Separation

NOVARA must distinguish addresses and signed operations belonging to different networks.

For example:

- Development network.
- Test network.
- Main network.

A transaction intended for one network must not be accepted as a valid transaction on another network merely because its signature is valid.

The final protocol must define network and chain identifiers and include them in the relevant cryptographic signing domains.

7. Business Authorisation

Business accounts must support:

- Multiple authorised signers.
- Role-based permissions.
- Configurable approval thresholds.
- Authorisation changes.
- Protection against unauthorised permission changes.

The protocol must define how signer changes are approved and how conflicting or revoked permissions are handled.

8. Smart-Contract Identity

Smart-contract addresses must be deterministically derived from deployment information.

The final derivation rule must define the creator identity, deployment nonce or equivalent deployment identifier, and network separation.

Two different deployments must not accidentally receive the same address.

9. Security Requirements

The implementation must consider:

- Private-key protection.
- Signature verification.
- Replay protection.
- Address collision resistance.
- Network separation.
- Secure account recovery.
- Business signer revocation.
- Contract authorisation vulnerabilities.

Security claims must be supported by implementation and testing.

10. Implementation Plan

The proposed implementation sequence is:

1. Define the account data model.
2. Define the NOVARA address format.
3. Implement key generation and public-key handling.
4. Implement individual account address derivation.
5. Implement business account authorisation policies.
6. Implement smart-contract address derivation.
7. Implement address encoding and checksum validation.
8. Write unit and integration tests.
9. Document the protocol's final binary formats and compatibility rules.

11. Design Status

This document is an initial protocol proposal.

It does not claim that NOVARA account creation, business authorisation, smart-contract deployment or address generation has already been implemented.

All final protocol rules must be documented, reviewed and tested before being treated as stable.
