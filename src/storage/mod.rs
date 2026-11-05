//! NOVARA persistent storage.
//!
//! Provides local SQLite database initialization and schema management.

use rusqlite::{Connection, Result};

/// Current NOVARA storage schema version.
const SCHEMA_VERSION: i64 = 1;

/// Opens or creates the NOVARA database and initializes its schema.
pub struct Storage {
    connection: Connection,
}

impl Storage {
    /// Opens a database at the supplied filesystem path.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let connection = Connection::open(path)?;

        let storage = Self { connection };
        storage.initialize_schema()?;

        Ok(storage)
    }

    /// Opens an in-memory database, primarily useful for tests.
    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;

        let storage = Self { connection };
        storage.initialize_schema()?;

        Ok(storage)
    }

    /// Initializes the database schema.
    fn initialize_schema(&self) -> Result<()> {
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

    /// Returns the underlying SQLite connection.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
