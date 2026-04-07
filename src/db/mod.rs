//! Database layer - Connection management and schema
//!
//! This module provides a unified database abstraction layer supporting both
//! Turso (remote SQLite) and local SQLite backends.

pub mod schema;

use sqlx::SqlitePool;
use crate::config::DatabaseConfig;

/// Database connection manager providing a unified interface for both
/// Turso (remote) and SQLite (local) backends.
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Create a new database connection pool from the given configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - The database configuration (Turso or SQLite)
    ///
    /// # Returns
    ///
    /// Returns a `Database` instance with an initialized connection pool.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection pool cannot be created.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use stickerbot::config::DatabaseConfig;
    /// use stickerbot::db::Database;
    ///
    /// let config = DatabaseConfig::Sqlite { path: "stickerbot.db".to_string() };
    /// let db = Database::new(&config).await?;
    /// ```
    pub async fn new(config: &DatabaseConfig) -> Result<Self, sqlx::Error> {
        let database_url = Self::build_connection_string(config);
        let pool = SqlitePool::connect(&database_url).await?;
        Ok(Self { pool })
    }

    /// Build a connection string from the database configuration.
    ///
    /// For Turso, the URL is used as-is (libsql:// or http(s):// URLs).
    /// For SQLite, a local file path is converted to a SQLite connection string.
    fn build_connection_string(config: &DatabaseConfig) -> String {
        match config {
            DatabaseConfig::Turso { url } => url.clone(),
            DatabaseConfig::Sqlite { path } => format!("sqlite:{}?mode=rwc", path),
        }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Run all schema migrations.
    ///
    /// This creates the users and sticker_packs tables with their indexes
    /// if they don't already exist.
    ///
    /// # Errors
    ///
    /// Returns an error if any migration fails to execute.
    pub async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        for migration in schema::SCHEMA_MIGRATIONS {
            // Execute migration, ignoring "duplicate column" errors for ALTER TABLE
            let result = sqlx::query(migration).execute(&self.pool).await;
            
            // Check if error is due to duplicate column (which is fine for idempotency)
            if let Err(ref e) = result {
                let error_msg = e.to_string();
                if error_msg.contains("duplicate column name") {
                    // Column already exists, skip this migration
                    tracing::debug!("Skipping migration (column already exists): {}", migration);
                    continue;
                }
                // For other errors, propagate them
                return result.map(|_| ());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_connection_string_sqlite() {
        let config = DatabaseConfig::Sqlite {
            path: "test.db".to_string(),
        };
        let conn_str = Database::build_connection_string(&config);
        assert_eq!(conn_str, "sqlite:test.db?mode=rwc");
    }

    #[test]
    fn test_build_connection_string_sqlite_with_path() {
        let config = DatabaseConfig::Sqlite {
            path: "/path/to/database.db".to_string(),
        };
        let conn_str = Database::build_connection_string(&config);
        assert_eq!(conn_str, "sqlite:/path/to/database.db?mode=rwc");
    }

    #[test]
    fn test_build_connection_string_turso_libsql() {
        let config = DatabaseConfig::Turso {
            url: "libsql://example.turso.io".to_string(),
        };
        let conn_str = Database::build_connection_string(&config);
        assert_eq!(conn_str, "libsql://example.turso.io");
    }

    #[test]
    fn test_build_connection_string_turso_https() {
        let config = DatabaseConfig::Turso {
            url: "https://example.turso.io".to_string(),
        };
        let conn_str = Database::build_connection_string(&config);
        assert_eq!(conn_str, "https://example.turso.io");
    }
}