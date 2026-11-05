//! NOVARA block structure, deterministic hashing, and serialization.

use sha2::{Digest, Sha256};

use crate::account::network::NetworkId;
use crate::genesis::GenesisHash;
use crate::transaction::{
    AssetId, ChainId, Transaction, TransactionSignature,
};

pub mod validation;

/// Domain separator for NOVARA block hashing version 1.
const BLOCK_DOMAIN: &[u8] = b"NOVARA_BLOCK_V1";

/// Domain separator for serialized NOVARA blocks.
const BLOCK_ENCODING_DOMAIN: &[u8] = b"NOVARA_BLOCK_ENCODING_V1";

/// Domain separator for serialized NOVARA transactions.
const TRANSACTION_DOMAIN: &[u8] = b"NOVARA_TRANSACTION_V1";

/// Identifies a block using its SHA-256 hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockHash([u8; 32]);

impl BlockHash {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A transaction together with its digital signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTransaction {
    pub transaction: Transaction,
    pub signature: TransactionSignature,
}

/// Header containing the identifying information of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHeader {
    pub network: NetworkId,
    pub chain_id: ChainId,
    pub genesis_hash: GenesisHash,
    pub height: u64,
    pub timestamp: u64,
    pub previous_hash: BlockHash,
}

/// A NOVARA block containing signed transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<SignedTransaction>,
}

/// Errors returned when decoding a serialized block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockDecodeError {
    InvalidFormat,
    UnsupportedVersion,
    InvalidNetwork,
    InvalidTransaction,
    WrongGenesis,
    TrailingBytes,
}

impl Block {
    pub fn new(
        network: NetworkId,
        chain_id: ChainId,
        genesis_hash: GenesisHash,
        height: u64,
        timestamp: u64,
        previous_hash: BlockHash,
        transactions: Vec<SignedTransaction>,
    ) -> Self {
        Self {
            header: BlockHeader {
                network,
                chain_id,
                genesis_hash,
                height,
                timestamp,
                previous_hash,
            },
            transactions,
        }
    }

    /// Calculates the deterministic SHA-256 block hash.
    ///
    /// This preserves the original NOVARA_BLOCK_V1 hashing format.
    pub fn hash(&self) -> BlockHash {
        let mut hasher = Sha256::new();

        hasher.update(BLOCK_DOMAIN);
        hasher.update([self.header.network.as_byte()]);
        hasher.update(self.header.chain_id.as_bytes());
        hasher.update(self.header.genesis_hash.as_bytes());
        hasher.update(self.header.height.to_be_bytes());
        hasher.update(self.header.timestamp.to_be_bytes());
        hasher.update(self.header.previous_hash.as_bytes());

        hasher.update((self.transactions.len() as u64).to_be_bytes());

        for signed_transaction in &self.transactions {
            let encoded = signed_transaction.transaction.encode();

            hasher.update((encoded.len() as u64).to_be_bytes());
            hasher.update(&encoded);
            hasher.update(signed_transaction.signature.public_key);
            hasher.update(signed_transaction.signature.signature);
        }

        BlockHash::new(hasher.finalize().into())
    }

    /// Serializes the complete block into a versioned binary format.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        bytes.extend_from_slice(BLOCK_ENCODING_DOMAIN);
        bytes.push(self.header.network.as_byte());
        bytes.extend_from_slice(self.header.chain_id.as_bytes());
        bytes.extend_from_slice(self.header.genesis_hash.as_bytes());
        bytes.extend_from_slice(&self.header.height.to_be_bytes());
        bytes.extend_from_slice(&self.header.timestamp.to_be_bytes());
        bytes.extend_from_slice(self.header.previous_hash.as_bytes());

        bytes.extend_from_slice(&(self.transactions.len() as u64).to_be_bytes());

        for signed_transaction in &self.transactions {
            let transaction_bytes = signed_transaction.transaction.encode();

            bytes.extend_from_slice(&(transaction_bytes.len() as u64).to_be_bytes());
            bytes.extend_from_slice(&transaction_bytes);
            bytes.extend_from_slice(&signed_transaction.signature.public_key);
            bytes.extend_from_slice(&signed_transaction.signature.signature);
        }

        bytes
    }

    /// Reconstructs a block from bytes and checks its expected genesis.
    pub fn decode(
        bytes: &[u8],
        expected_genesis_hash: GenesisHash,
    ) -> Result<Self, BlockDecodeError> {
        let mut reader = ByteReader::new(bytes);

        if reader.read_exact(BLOCK_ENCODING_DOMAIN.len())? != BLOCK_ENCODING_DOMAIN {
            return Err(BlockDecodeError::UnsupportedVersion);
        }

        let network = decode_network(reader.read_u8()?)?;

        let chain_id = ChainId::new(reader.read_array::<32>()?);
        let genesis_hash = expected_genesis_hash;

        if reader.read_array::<32>()? != *genesis_hash.as_bytes() {
            return Err(BlockDecodeError::WrongGenesis);
        }

        let height = reader.read_u64()?;
        let timestamp = reader.read_u64()?;
        let previous_hash = BlockHash::new(reader.read_array::<32>()?);

        let transaction_count = reader.read_u64()?;

        let remaining = reader.remaining();

        if transaction_count > (remaining / 112) as u64 {
            return Err(BlockDecodeError::InvalidFormat);
        }

        let mut transactions = Vec::new();

        for _ in 0..transaction_count {
            let transaction_length = reader.read_u64()?;

            let transaction_length = usize::try_from(transaction_length)
                .map_err(|_| BlockDecodeError::InvalidFormat)?;

            let transaction_bytes = reader.read_exact(transaction_length)?;

            let transaction = decode_transaction(transaction_bytes)?;

            let public_key = reader.read_array::<32>()?;
            let signature = reader.read_array::<64>()?;

            transactions.push(SignedTransaction {
                transaction,
                signature: TransactionSignature {
                    public_key,
                    signature,
                },
            });
        }

        if reader.remaining() != 0 {
            return Err(BlockDecodeError::TrailingBytes);
        }

        Ok(Self::new(
            network,
            chain_id,
            genesis_hash,
            height,
            timestamp,
            previous_hash,
            transactions,
        ))
    }
}

fn decode_network(value: u8) -> Result<NetworkId, BlockDecodeError> {
    match value {
        0x01 => Ok(NetworkId::Development),
        0x02 => Ok(NetworkId::Testnet),
        0x03 => Ok(NetworkId::Mainnet),
        _ => Err(BlockDecodeError::InvalidNetwork),
    }
}

fn decode_transaction(bytes: &[u8]) -> Result<Transaction, BlockDecodeError> {
    let mut reader = ByteReader::new(bytes);

    if reader.read_exact(TRANSACTION_DOMAIN.len())? != TRANSACTION_DOMAIN {
        return Err(BlockDecodeError::InvalidTransaction);
    }

    let version = reader.read_u16()?;
    let network = decode_network(reader.read_u8()?)?;

    let chain_id = ChainId::new(reader.read_array::<32>()?);
    let sender = crate::account::address::AccountId::new(reader.read_array::<32>()?);
    let recipient = crate::account::address::AccountId::new(reader.read_array::<32>()?);
    let asset_id = AssetId::new(reader.read_array::<32>()?);

    let amount = reader.read_u128()?;
    let nonce = reader.read_u64()?;

    if reader.remaining() != 0 {
        return Err(BlockDecodeError::InvalidTransaction);
    }

    Transaction::new(
        version,
        network,
        chain_id,
        sender,
        recipient,
        asset_id,
        amount,
        nonce,
    )
    .map_err(|_| BlockDecodeError::InvalidTransaction)
}

/// Reads fixed-size values safely from a byte slice.
struct ByteReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
        }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], BlockDecodeError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(BlockDecodeError::InvalidFormat)?;

        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(BlockDecodeError::InvalidFormat)?;

        self.position = end;

        Ok(value)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], BlockDecodeError> {
        self.read_exact(N)?
            .try_into()
            .map_err(|_| BlockDecodeError::InvalidFormat)
    }

    fn read_u8(&mut self) -> Result<u8, BlockDecodeError> {
        Ok(self.read_array::<1>()?[0])
    }

    fn read_u16(&mut self) -> Result<u16, BlockDecodeError> {
        Ok(u16::from_be_bytes(self.read_array::<2>()?))
    }

    fn read_u64(&mut self) -> Result<u64, BlockDecodeError> {
        Ok(u64::from_be_bytes(self.read_array::<8>()?))
    }

    fn read_u128(&mut self) -> Result<u128, BlockDecodeError> {
        Ok(u128::from_be_bytes(self.read_array::<16>()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genesis::{
        GenesisAllocation, GenesisConfig, InitializedGenesis,
    };

    fn test_genesis() -> InitializedGenesis {
        InitializedGenesis::new(GenesisConfig::new(
            NetworkId::Development,
            Vec::new(),
        ))
        .unwrap()
    }

    fn empty_block(genesis: &InitializedGenesis) -> Block {
        Block::new(
            NetworkId::Development,
            genesis.chain_id(),
            genesis.genesis_hash(),
            1,
            1_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        )
    }

    #[test]
    fn block_hash_is_deterministic() {
        let genesis = test_genesis();
        let block = empty_block(&genesis);

        assert_eq!(block.hash(), block.hash());
    }

    #[test]
    fn different_block_heights_produce_different_hashes() {
        let genesis = test_genesis();
        let first = empty_block(&genesis);

        let second = Block::new(
            NetworkId::Development,
            genesis.chain_id(),
            genesis.genesis_hash(),
            2,
            1_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        );

        assert_ne!(first.hash(), second.hash());
    }

    #[test]
    fn different_previous_hashes_produce_different_block_hashes() {
        let genesis = test_genesis();
        let first = empty_block(&genesis);

        let second = Block::new(
            NetworkId::Development,
            genesis.chain_id(),
            genesis.genesis_hash(),
            1,
            1_000,
            BlockHash::new([2u8; 32]),
            Vec::new(),
        );

        assert_ne!(first.hash(), second.hash());
    }

    #[test]
    fn different_timestamps_produce_different_block_hashes() {
        let genesis = test_genesis();
        let first = empty_block(&genesis);

        let second = Block::new(
            NetworkId::Development,
            genesis.chain_id(),
            genesis.genesis_hash(),
            1,
            2_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        );

        assert_ne!(first.hash(), second.hash());
    }

    #[test]
    fn different_genesis_hashes_produce_different_block_hashes() {
        let genesis = test_genesis();

        let different_genesis = InitializedGenesis::new(GenesisConfig::new(
            NetworkId::Development,
            vec![GenesisAllocation {
                account_id: crate::account::address::AccountId::new([5u8; 32]),
                asset_id: AssetId::new([6u8; 32]),
                amount: 100,
            }],
        ))
        .unwrap();

        let first = empty_block(&genesis);

        let second = Block::new(
            NetworkId::Development,
            different_genesis.chain_id(),
            different_genesis.genesis_hash(),
            1,
            1_000,
            BlockHash::new([0u8; 32]),
            Vec::new(),
        );

        assert_ne!(first.hash(), second.hash());
    }

    #[test]
    fn block_encoding_round_trip_preserves_hash() {
        let genesis = test_genesis();
        let block = empty_block(&genesis);

        let encoded = block.encode();
        let decoded = Block::decode(&encoded, genesis.genesis_hash()).unwrap();

        assert_eq!(decoded, block);
        assert_eq!(decoded.hash(), block.hash());
    }

    #[test]
    fn block_decoder_rejects_wrong_genesis() {
        let genesis = test_genesis();
        let block = empty_block(&genesis);

        let other_genesis = InitializedGenesis::new(GenesisConfig::new(
            NetworkId::Development,
            vec![GenesisAllocation {
                account_id: crate::account::address::AccountId::new([5u8; 32]),
                asset_id: AssetId::new([6u8; 32]),
                amount: 100,
            }],
        ))
        .unwrap();

        assert_eq!(
            Block::decode(&block.encode(), other_genesis.genesis_hash()),
            Err(BlockDecodeError::WrongGenesis)
        );
    }

    #[test]
    fn block_decoder_rejects_trailing_bytes() {
        let genesis = test_genesis();
        let block = empty_block(&genesis);

        let mut encoded = block.encode();
        encoded.push(0);

        assert_eq!(
            Block::decode(&encoded, genesis.genesis_hash()),
            Err(BlockDecodeError::TrailingBytes)
        );
    }
}
