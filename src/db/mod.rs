//! Database layer - Connection management and schema
//!
//! This module provides a unified database abstraction layer supporting both
//! Turso (remote SQLite) and local SQLite backends.

pub mod schema;

use libsql::Connection;
use crate::config::DatabaseConfig;

/// Database connection manager providing a unified interface for both
/// Turso (remote) and SQLite (local) backends.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Create a new database connection from the given configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - The database configuration (Turso or SQLite)
    ///
    /// # Returns
    ///
    /// Returns a `Database` instance with an initialized connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection cannot be created.
    pub async fn new(config: &DatabaseConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let db = match config {
            DatabaseConfig::Turso { url, auth_token } => {
                libsql::Builder::new_remote(url.clone(), auth_token.clone())
                    .build()
                    .await?
            }
            DatabaseConfig::Sqlite { path } => {
                libsql::Builder::new_local(path)
                    .build()
                    .await?
            }
        };

        let conn = db.connect()?;
        Ok(Self { conn })
    }

    /// Get a reference to the connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Run all schema migrations.
    ///
    /// This creates the users and sticker_packs tables with their indexes
    /// if they don't already exist.
    ///
    /// # Errors
    ///
    /// Returns an error if any migration fails to execute.
    pub async fn run_migrations(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for migration in schema::SCHEMA_MIGRATIONS {
            // Execute migration, ignoring "duplicate column" errors for ALTER TABLE
            let result = self.conn.execute(migration, ()).await;
            
            // Check if error is due to duplicate column (which is fine for idempotency)
            if let Err(ref e) = result {
                let error_msg = e.to_string();
                if error_msg.contains("duplicate column name") {
                    // Column already exists, skip this migration
                    tracing::debug!("Skipping migration (column already exists): {}", migration);
                    continue;
                }
                // For other errors, propagate them
                return Err(Box::new(result.unwrap_err()));
            }
        }
        Ok(())
    }
}
