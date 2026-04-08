//! Database layer - Connection management and schema
//!
//! This module provides a unified database abstraction layer supporting both
//! Turso (remote SQLite) and local SQLite backends.
//!
//! Each backend receives its own set of connection optimizations on startup:
//! - **Local SQLite**: WAL journal, NORMAL sync, 64 MiB page cache, 128 MiB mmap,
//!   in-memory temp store, 5 s busy timeout, and FK enforcement — tuned for HDD.
//! - **Turso (remote)**: FK enforcement, 16 MiB client-side cache, and 5 s busy
//!   timeout. Journaling and sync are managed server-side by Turso.

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
        let instance = Self { conn };

        // Apply backend-specific optimizations immediately after connecting.
        match config {
            DatabaseConfig::Sqlite { .. } => instance.apply_local_pragmas().await?,
            DatabaseConfig::Turso { .. } => instance.apply_turso_pragmas().await?,
        }

        Ok(instance)
    }

    /// Get a reference to the connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Apply SQLite PRAGMAs tuned for **local HDD** deployments.
    ///
    /// Settings persist for the lifetime of this connection. WAL mode
    /// persists in the DB file after being set for the first time.
    async fn apply_local_pragmas(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let pragmas = [
            // WAL mode: readers never block writers, writers never block readers.
            "PRAGMA journal_mode = WAL",
            // NORMAL sync is safe under WAL — only the last commit is at risk
            // after a power failure, never the database file integrity.
            "PRAGMA synchronous = NORMAL",
            // -65536 KiB = 64 MiB page cache; reduces random disk seeks on HDD.
            "PRAGMA cache_size = -65536",
            // Memory-map up to 128 MiB for fast sequential I/O on spinning disk.
            "PRAGMA mmap_size = 134217728",
            // Store internal temp tables (sorts, subqueries) in RAM.
            "PRAGMA temp_store = MEMORY",
            // Wait up to 5 s before returning SQLITE_BUSY under write contention.
            "PRAGMA busy_timeout = 5000",
            // Enforce FK constraints (SQLite disables this by default).
            "PRAGMA foreign_keys = ON",
        ];

        for pragma in pragmas {
            self.conn.execute(pragma, ()).await?;
        }

        Ok(())
    }

    /// Apply optimizations suitable for **Turso (remote libSQL)** connections.
    ///
    /// Turso manages journaling and sync server-side, so we only tune settings
    /// that are meaningful for a remote connection: FK enforcement, timeout,
    /// and a modest client-side page cache to reduce round-trips.
    async fn apply_turso_pragmas(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let pragmas = [
            // WAL mode: readers never block writers, writers never block readers.
            "PRAGMA journal_mode = WAL",
            // Enforce FK constraints for consistency with local mode.
            "PRAGMA foreign_keys = ON",
            // Keep a 16 MiB client-side cache to reduce remote round-trips
            // for repeated reads of hot pages.
            "PRAGMA cache_size = -16384",
            // Return immediately on lock contention — Turso uses server-side
            // serialisation so this primarily guards against client-side races.
            "PRAGMA busy_timeout = 5000",
        ];

        for pragma in pragmas {
            // Turso may silently ignore unsupported PRAGMAs — log but don't fail.
            if let Err(e) = self.conn.execute(pragma, ()).await {
                tracing::warn!(pragma, error = %e, "Turso PRAGMA ignored (unsupported server-side)");
            }
        }

        Ok(())
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
