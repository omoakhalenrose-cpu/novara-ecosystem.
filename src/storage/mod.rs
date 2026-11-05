//! NOVARA persistent storage.
//!
//! Provides SQLite database initialization, chain metadata management,
//! sequential block persistence, and chain recovery.

use rusqlite::{Connection, OptionalExtension, Result as SqliteResult};

use crate::account::network::NetworkId;
use crate::block::{Block, BlockDecodeError, BlockHash};
use crate::chain::{Chain, ChainError};
use crate::genesis::{GenesisHash, InitializedGenesis};

/// Current NOVARA storage schema version.
const SCHEMA_VERSION: i64 = 1;

/// SQLite row representation of persisted chain metadata.
type StoredChainMetadataRow = (i64, Vec<u8>, Vec<u8>, i64, Vec<u8>);

/// Errors returned by NOVARA storage operations.
#[derive(Debug)]
pub enum StorageError {
    Database(rusqlite::Error),
    BlockDecode(BlockDecodeError),
    ChainRecovery(ChainError),
    ChainNotInitialized,
    ChainAlreadyInitialized,
    WrongNetwork,
    WrongChain,
    WrongGenesis,
    WrongHeight,
    WrongPreviousHash,
    InvalidStoredHash,
    InvalidStoredChain,
    MissingStoredBlock,
    HeightOutOfRange,
}

/// Persisted identity and tip information for a NOVARA chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainMetadata {
    pub network: NetworkId,
    pub chain_id: [u8; 32],
    pub genesis_hash: [u8; 32],
    pub height: u64,
    pub latest_block_hash: [u8; 32],
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<BlockDecodeError> for StorageError {
    fn from(error: BlockDecodeError) -> Self {
        Self::BlockDecode(error)
    }
}

/// Opens or creates the NOVARA database and manages persistent chain data.
pub struct Storage {
    connection: Connection,
}

impl Storage {
    /// Opens a database at the supplied filesystem path.
    pub fn open(path: impl AsRef<std::path::Path>) -> SqliteResult<Self> {
        let connection = Connection::open(path)?;

        let storage = Self { connection };
        storage.initialize_schema()?;

        Ok(storage)
    }

    /// Opens an in-memory database, primarily useful for tests.
    pub fn open_in_memory() -> SqliteResult<Self> {
        let connection = Connection::open_in_memory()?;

        let storage = Self { connection };
        storage.initialize_schema()?;

        Ok(storage)
    }

    /// Initializes the database schema.
    fn initialize_schema(&self) -> SqliteResult<()> {
        self.connection.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS storage_metadata (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS chain_metadata (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                network_id INTEGER NOT NULL,
                chain_id BLOB NOT NULL CHECK (length(chain_id) = 32),
                genesis_hash BLOB NOT NULL CHECK (length(genesis_hash) = 32),
                height INTEGER NOT NULL CHECK (height >= 0),
                latest_block_hash BLOB NOT NULL CHECK (length(latest_block_hash) = 32)
            );

            CREATE TABLE IF NOT EXISTS blocks (
                height INTEGER PRIMARY KEY CHECK (height > 0),
                block_hash BLOB NOT NULL UNIQUE CHECK (length(block_hash) = 32),
                previous_hash BLOB NOT NULL CHECK (length(previous_hash) = 32),
                block_data BLOB NOT NULL
            );
            ",
        )?;

        self.connection.execute(
            "
            INSERT OR IGNORE INTO storage_metadata (key, value)
            VALUES ('schema_version', ?1)
            ",
            [SCHEMA_VERSION.to_string()],
        )?;

        let stored_version: String = self.connection.query_row(
            "SELECT value FROM storage_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )?;

        if stored_version != SCHEMA_VERSION.to_string() {
            return Err(rusqlite::Error::InvalidQuery);
        }

        Ok(())
    }

    /// Initializes chain metadata from validated genesis.
    ///
    /// The genesis hash is the initial chain-tip hash at height zero.
    /// Reinitialization is rejected.
    pub fn initialize_chain(&self, genesis: &InitializedGenesis) -> Result<(), StorageError> {
        let existing: Option<i64> = self
            .connection
            .query_row("SELECT id FROM chain_metadata WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;

        if existing.is_some() {
            return Err(StorageError::ChainAlreadyInitialized);
        }

        self.connection.execute(
            "
            INSERT INTO chain_metadata (
                id,
                network_id,
                chain_id,
                genesis_hash,
                height,
                latest_block_hash
            )
            VALUES (1, ?1, ?2, ?3, 0, ?3)
            ",
            rusqlite::params![
                genesis.config().network.as_byte() as i64,
                genesis.chain_id().as_bytes().as_slice(),
                genesis.genesis_hash().as_bytes().as_slice(),
            ],
        )?;

        Ok(())
    }

    /// Loads the persisted chain identity and latest chain tip.
    pub fn load_chain_metadata(&self) -> Result<Option<ChainMetadata>, StorageError> {
        let stored: Option<StoredChainMetadataRow> = self
            .connection
            .query_row(
                "
                SELECT network_id, chain_id, genesis_hash, height, latest_block_hash
                FROM chain_metadata
                WHERE id = 1
                ",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;

        let Some((network_id, chain_id, genesis_hash, height, latest_block_hash)) = stored else {
            return Ok(None);
        };

        let network = match network_id {
            0x01 => NetworkId::Development,
            0x02 => NetworkId::Testnet,
            0x03 => NetworkId::Mainnet,
            _ => return Err(StorageError::WrongNetwork),
        };

        let chain_id = decode_hash_bytes(chain_id)?;
        let genesis_hash = decode_hash_bytes(genesis_hash)?;
        let latest_block_hash = decode_hash_bytes(latest_block_hash)?;

        let height = u64::try_from(height).map_err(|_| StorageError::HeightOutOfRange)?;

        Ok(Some(ChainMetadata {
            network,
            chain_id,
            genesis_hash,
            height,
            latest_block_hash,
        }))
    }

    /// Recovers a chain from persisted metadata and blocks.
    ///
    /// The original validated genesis configuration is required to rebuild
    /// the initial application state. Every stored block is decoded,
    /// validated, and replayed in ascending height order.
    pub fn recover_chain(
        &self,
        genesis: &InitializedGenesis,
    ) -> Result<Chain, StorageError> {
        let metadata = self
            .load_chain_metadata()?
            .ok_or(StorageError::ChainNotInitialized)?;

        if metadata.network != genesis.config().network {
            return Err(StorageError::WrongNetwork);
        }

        if metadata.chain_id != *genesis.chain_id().as_bytes() {
            return Err(StorageError::WrongChain);
        }

        if metadata.genesis_hash != *genesis.genesis_hash().as_bytes() {
            return Err(StorageError::WrongGenesis);
        }

        let (block_count, maximum_height): (i64, Option<i64>) = self
            .connection
            .query_row(
                "SELECT COUNT(*), MAX(height) FROM blocks",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;

        let stored_block_count =
            u64::try_from(block_count).map_err(|_| StorageError::InvalidStoredChain)?;

        if stored_block_count != metadata.height {
            return Err(StorageError::InvalidStoredChain);
        }

        match metadata.height {
            0 => {
                if maximum_height.is_some()
                    || metadata.latest_block_hash != metadata.genesis_hash
                {
                    return Err(StorageError::InvalidStoredChain);
                }
            }
            height => {
                if maximum_height
                    != Some(i64::try_from(height).map_err(|_| StorageError::HeightOutOfRange)?)
                {
                    return Err(StorageError::InvalidStoredChain);
                }
            }
        }

        let mut chain = Chain::new(genesis).map_err(StorageError::ChainRecovery)?;

        for height in 1..=metadata.height {
            let block = self
                .load_block(height, genesis.genesis_hash())?
                .ok_or(StorageError::MissingStoredBlock)?;

            chain
                .apply_block(block)
                .map_err(StorageError::ChainRecovery)?;
        }

        if chain.height() != metadata.height {
            return Err(StorageError::InvalidStoredChain);
        }

        if chain.latest_hash().as_bytes() != &metadata.latest_block_hash {
            return Err(StorageError::InvalidStoredChain);
        }

        Ok(chain)
            }
            /// Saves a block only when it extends the persisted chain tip.
    ///
    /// Block insertion and chain metadata updates happen atomically.
    pub fn save_block(&mut self, block: &Block) -> Result<(), StorageError> {
        let height =
            i64::try_from(block.header.height).map_err(|_| StorageError::HeightOutOfRange)?;

        let transaction = self.connection.transaction()?;

        let metadata: StoredChainMetadataRow = transaction
            .query_row(
                "
                SELECT network_id, chain_id, genesis_hash, height, latest_block_hash
                FROM chain_metadata
                WHERE id = 1
                ",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StorageError::ChainNotInitialized)?;

        let expected_network = match metadata.0 {
            0x01 => NetworkId::Development,
            0x02 => NetworkId::Testnet,
            0x03 => NetworkId::Mainnet,
            _ => return Err(StorageError::WrongNetwork),
        };

        if block.header.network != expected_network {
            return Err(StorageError::WrongNetwork);
        }

        let expected_chain_id = decode_hash_bytes(metadata.1)?;
        if block.header.chain_id.as_bytes() != &expected_chain_id {
            return Err(StorageError::WrongChain);
        }

        let expected_genesis_hash = decode_hash_bytes(metadata.2)?;
        if block.header.genesis_hash.as_bytes() != &expected_genesis_hash {
            return Err(StorageError::WrongGenesis);
        }

        let previous_height =
            u64::try_from(metadata.3).map_err(|_| StorageError::HeightOutOfRange)?;

        let expected_height = previous_height
            .checked_add(1)
            .ok_or(StorageError::HeightOutOfRange)?;

        if block.header.height != expected_height || height <= 0 {
            return Err(StorageError::WrongHeight);
        }

        let latest_hash = decode_hash_bytes(metadata.4)?;

        if block.header.previous_hash.as_bytes() != &latest_hash {
            return Err(StorageError::WrongPreviousHash);
        }

        let block_hash = block.hash();
        let encoded_block = block.encode();

        transaction.execute(
            "
            INSERT INTO blocks (
                height,
                block_hash,
                previous_hash,
                block_data
            )
            VALUES (?1, ?2, ?3, ?4)
            ",
            rusqlite::params![
                height,
                block_hash.as_bytes().as_slice(),
                block.header.previous_hash.as_bytes().as_slice(),
                encoded_block,
            ],
        )?;

        transaction.execute(
            "
            UPDATE chain_metadata
            SET height = ?1, latest_block_hash = ?2
            WHERE id = 1
            ",
            rusqlite::params![height, block_hash.as_bytes().as_slice()],
        )?;

        transaction.commit()?;

        Ok(())
    }

    /// Loads a block by height and verifies its stored hashes.
    pub fn load_block(
        &self,
        height: u64,
        expected_genesis_hash: GenesisHash,
    ) -> Result<Option<Block>, StorageError> {
        let database_height = i64::try_from(height).map_err(|_| StorageError::HeightOutOfRange)?;

        let stored: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = self
            .connection
            .query_row(
                "
                SELECT block_hash, previous_hash, block_data
                FROM blocks
                WHERE height = ?1
                ",
                [database_height],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        let Some((stored_hash, stored_previous_hash, block_data)) = stored else {
            return Ok(None);
        };

        let stored_hash = decode_hash_bytes(stored_hash)?;
        let stored_previous_hash = decode_hash_bytes(stored_previous_hash)?;

        let block = Block::decode(&block_data, expected_genesis_hash)?;

        if block.header.height != height
            || block.hash() != BlockHash::new(stored_hash)
            || block.header.previous_hash != BlockHash::new(stored_previous_hash)
        {
            return Err(StorageError::InvalidStoredHash);
        }

        Ok(Some(block))
    }

    /// Returns the underlying SQLite connection.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
}

/// Converts a SQLite BLOB into a fixed-size 32-byte value.
fn decode_hash_bytes(bytes: Vec<u8>) -> Result<[u8; 32], StorageError> {
    bytes
        .try_into()
        .map_err(|_| StorageError::InvalidStoredHash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::address::AccountId;
    use crate::crypto::ed25519::Ed25519Identity;
    use crate::genesis::{GenesisAllocation, GenesisConfig, InitializedGenesis};
    use crate::transaction::signing::sign_transaction;
    use crate::transaction::{AssetId, Transaction};

    fn test_genesis() -> (InitializedGenesis, Ed25519Identity, AccountId) {
        let identity = Ed25519Identity::generate();

        let sender =
            AccountId::try_from_ed25519_public_key(NetworkId::Development, &identity.public_key())
                .unwrap();

        let recipient = AccountId::new([8u8; 32]);
        let asset_id = AssetId::new([4u8; 32]);

        let genesis = InitializedGenesis::new(GenesisConfig::new(
            NetworkId::Development,
            vec![
                GenesisAllocation {
                    account_id: sender,
                    asset_id,
                    amount: 1_000,
                },
                GenesisAllocation {
                    account_id: recipient,
                    asset_id,
                    amount: 1,
                },
            ],
        ))
        .unwrap();

        (genesis, identity, recipient)
    }

    fn test_block(
        genesis: &InitializedGenesis,
        identity: &Ed25519Identity,
        recipient: AccountId,
        height: u64,
        previous_hash: BlockHash,
        amount: u128,
    ) -> Block {
        let sender =
            AccountId::try_from_ed25519_public_key(NetworkId::Development, &identity.public_key())
                .unwrap();

        let transaction = Transaction::new(
            1,
            NetworkId::Development,
            genesis.chain_id(),
            sender,
            recipient,
            AssetId::new([4u8; 32]),
            amount,
            height - 1,
        )
        .unwrap();

        let signature = sign_transaction(&transaction, identity);

        Block::new(
            NetworkId::Development,
            genesis.chain_id(),
            genesis.genesis_hash(),
            height,
            1_000 + height,
            previous_hash,
            vec![crate::block::SignedTransaction {
                transaction,
                signature,
            }],
        )
    }

    #[test]
    fn creates_storage_schema() {
        let storage = Storage::open_in_memory().unwrap();

        let table_count: i64 = storage
            .connection()
            .query_row(
                "
                SELECT COUNT(*)
                FROM sqlite_master
                WHERE type = 'table'
                AND name IN ('storage_metadata', 'chain_metadata', 'blocks')
                ",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(table_count, 3);
    }

    #[test]
    fn stores_schema_version() {
        let storage = Storage::open_in_memory().unwrap();

        let version: String = storage
            .connection()
            .query_row(
                "SELECT value FROM storage_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(version, SCHEMA_VERSION.to_string());
    }

    #[test]
    fn initializes_and_loads_chain_metadata() {
        let storage = Storage::open_in_memory().unwrap();
        let (genesis, _, _) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        let metadata = storage.load_chain_metadata().unwrap().unwrap();

        assert_eq!(metadata.network, NetworkId::Development);
        assert_eq!(metadata.chain_id, *genesis.chain_id().as_bytes());
        assert_eq!(metadata.genesis_hash, *genesis.genesis_hash().as_bytes());
        assert_eq!(metadata.height, 0);
        assert_eq!(
            metadata.latest_block_hash,
            *genesis.genesis_hash().as_bytes()
        );
    }

    #[test]
    fn rejects_chain_reinitialization() {
        let storage = Storage::open_in_memory().unwrap();
        let (genesis, _, _) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        assert!(matches!(
            storage.initialize_chain(&genesis),
            Err(StorageError::ChainAlreadyInitialized)
        ));
    }

    #[test]
    fn saves_block_and_updates_chain_metadata() {
        let mut storage = Storage::open_in_memory().unwrap();
        let (genesis, identity, recipient) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        let previous_hash = BlockHash::new(*genesis.genesis_hash().as_bytes());
        let block = test_block(&genesis, &identity, recipient, 1, previous_hash, 100);
        let expected_hash = block.hash();

        storage.save_block(&block).unwrap();

        let metadata = storage.load_chain_metadata().unwrap().unwrap();

        assert_eq!(metadata.height, 1);
        assert_eq!(metadata.latest_block_hash, *expected_hash.as_bytes());
    }

    #[test]
    fn saves_and_loads_block() {
        let mut storage = Storage::open_in_memory().unwrap();
        let (genesis, identity, recipient) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        let previous_hash = BlockHash::new(*genesis.genesis_hash().as_bytes());
        let block = test_block(&genesis, &identity, recipient, 1, previous_hash, 100);

        storage.save_block(&block).unwrap();

        let loaded = storage
            .load_block(1, genesis.genesis_hash())
            .unwrap()
            .unwrap();

        assert_eq!(loaded, block);
    }

    #[test]
    fn loaded_block_preserves_original_hash() {
        let mut storage = Storage::open_in_memory().unwrap();
        let (genesis, identity, recipient) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        let previous_hash = BlockHash::new(*genesis.genesis_hash().as_bytes());
        let block = test_block(&genesis, &identity, recipient, 1, previous_hash, 100);
        let original_hash = block.hash();

        storage.save_block(&block).unwrap();

        let loaded = storage
            .load_block(1, genesis.genesis_hash())
            .unwrap()
            .unwrap();

        assert_eq!(loaded.hash(), original_hash);
    }

    #[test]
    fn rejects_block_with_wrong_height() {
        let mut storage = Storage::open_in_memory().unwrap();
        let (genesis, identity, recipient) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        let previous_hash = BlockHash::new(*genesis.genesis_hash().as_bytes());
        let block = test_block(&genesis, &identity, recipient, 2, previous_hash, 100);

        assert!(matches!(
            storage.save_block(&block),
            Err(StorageError::WrongHeight)
        ));
    }

    #[test]
    fn rejects_block_with_wrong_previous_hash() {
        let mut storage = Storage::open_in_memory().unwrap();
        let (genesis, identity, recipient) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        let block = test_block(
            &genesis,
            &identity,
            recipient,
            1,
            BlockHash::new([9u8; 32]),
            100,
        );

        assert!(matches!(
            storage.save_block(&block),
            Err(StorageError::WrongPreviousHash)
        ));
    }

    #[test]
    fn recovers_empty_chain() {
        let storage = Storage::open_in_memory().unwrap();
        let (genesis, _, _) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        let recovered = storage.recover_chain(&genesis).unwrap();

        assert_eq!(recovered.height(), 0);
        assert_eq!(
            recovered.latest_hash().as_bytes(),
            genesis.genesis_hash().as_bytes()
        );
    }

    #[test]
    fn recovers_chain_with_persisted_block() {
        let mut storage = Storage::open_in_memory().unwrap();
        let (genesis, identity, recipient) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        let previous_hash = BlockHash::new(*genesis.genesis_hash().as_bytes());
        let block = test_block(&genesis, &identity, recipient, 1, previous_hash, 100);
        let expected_hash = block.hash();

        storage.save_block(&block).unwrap();

        let recovered = storage.recover_chain(&genesis).unwrap();

        assert_eq!(recovered.height(), 1);
        assert_eq!(recovered.block_count(), 1);
        assert_eq!(recovered.latest_hash(), expected_hash);
    }

    #[test]
    fn rejects_recovery_with_wrong_genesis() {
        let storage = Storage::open_in_memory().unwrap();
        let (genesis, _, _) = test_genesis();
        let (different_genesis, _, _) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        assert!(matches!(
            storage.recover_chain(&different_genesis),
            Err(StorageError::WrongChain) | Err(StorageError::WrongGenesis)
        ));
    }

    #[test]
    fn rejects_recovery_when_stored_block_is_missing() {
        let storage = Storage::open_in_memory().unwrap();
        let (genesis, _, _) = test_genesis();

        storage.initialize_chain(&genesis).unwrap();

        storage
            .connection()
            .execute(
                "
                UPDATE chain_metadata
                SET height = 1, latest_block_hash = genesis_hash
                WHERE id = 1
                ",
                [],
            )
            .unwrap();

        assert!(matches!(
            storage.recover_chain(&genesis),
            Err(StorageError::InvalidStoredChain)
        ));
    }

    #[test]
    fn returns_none_for_missing_block() {
        let storage = Storage::open_in_memory().unwrap();
        let (genesis, _, _) = test_genesis();

        let loaded = storage.load_block(1, genesis.genesis_hash()).unwrap();

        assert!(loaded.is_none());
    }
    
}
