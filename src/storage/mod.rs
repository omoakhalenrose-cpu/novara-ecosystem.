//! NOVARA persistent storage.
//!
//! Provides SQLite database initialization and block persistence.

use rusqlite::{Connection, OptionalExtension, Result as SqliteResult};

use crate::block::{Block, BlockDecodeError, BlockHash};
use crate::genesis::GenesisHash;

/// Current NOVARA storage schema version.
const SCHEMA_VERSION: i64 = 1;

/// Errors returned by NOVARA storage operations.
#[derive(Debug)]
pub enum StorageError {
    Database(rusqlite::Error),
    BlockDecode(BlockDecodeError),
    InvalidStoredHash,
    HeightOutOfRange,
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

/// Opens or creates the NOVARA database and manages persistent blocks.
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

    /// Saves a complete block to SQLite.
    ///
    /// Existing block heights cannot be overwritten.
    pub fn save_block(&self, block: &Block) -> Result<(), StorageError> {
        let height =
            i64::try_from(block.header.height).map_err(|_| StorageError::HeightOutOfRange)?;

        let block_hash = block.hash();
        let encoded_block = block.encode();

        self.connection.execute(
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

        Ok(())
    }

    /// Loads a block by height and verifies its stored hash.
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

        let stored_hash: [u8; 32] = stored_hash
            .try_into()
            .map_err(|_| StorageError::InvalidStoredHash)?;

        let stored_previous_hash: [u8; 32] = stored_previous_hash
            .try_into()
            .map_err(|_| StorageError::InvalidStoredHash)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::network::NetworkId;
    use crate::block::BlockHash;
    use crate::genesis::{GenesisConfig, InitializedGenesis};

    fn test_genesis() -> InitializedGenesis {
        InitializedGenesis::new(GenesisConfig::new(NetworkId::Development, Vec::new())).unwrap()
    }

    fn test_block(genesis: &InitializedGenesis) -> Block {
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
    fn saves_and_loads_block() {
        let storage = Storage::open_in_memory().unwrap();
        let genesis = test_genesis();
        let block = test_block(&genesis);

        storage.save_block(&block).unwrap();

        let loaded = storage
            .load_block(1, genesis.genesis_hash())
            .unwrap()
            .unwrap();

        assert_eq!(loaded, block);
    }

    #[test]
    fn loaded_block_preserves_original_hash() {
        let storage = Storage::open_in_memory().unwrap();
        let genesis = test_genesis();
        let block = test_block(&genesis);

        let original_hash = block.hash();

        storage.save_block(&block).unwrap();

        let loaded = storage
            .load_block(1, genesis.genesis_hash())
            .unwrap()
            .unwrap();

        assert_eq!(loaded.hash(), original_hash);
    }

    #[test]
    fn returns_none_for_missing_block() {
        let storage = Storage::open_in_memory().unwrap();
        let genesis = test_genesis();

        let loaded = storage.load_block(1, genesis.genesis_hash()).unwrap();

        assert!(loaded.is_none());
    }
}
