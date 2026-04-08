//! User repository trait and SQLite implementation

use async_trait::async_trait;
use libsql::Connection;
use std::sync::Arc;
use crate::db::schema::{NewUser, User};
use super::RepositoryError;

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_by_telegram_id(&self, telegram_id: i64) -> Result<Option<Arc<User>>, RepositoryError>;
    async fn create(&self, user: NewUser) -> Result<Arc<User>, RepositoryError>;
    async fn update(&self, user: &User) -> Result<(), RepositoryError>;
    
    /// Set the default pack for a user.
    async fn set_default_pack(&self, user_id: i64, pack_id: Option<i64>) -> Result<(), RepositoryError>;
    
    /// Get the default pack ID for a user.
    async fn get_default_pack_id(&self, user_id: i64) -> Result<Option<i64>, RepositoryError>;
}

/// SQLite/libSQL implementation of the UserRepository trait.
pub struct SqliteUserRepository {
    conn: Connection,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::SCHEMA_MIGRATIONS;

    async fn setup_test_db() -> Connection {
        let db = libsql::Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        
        for migration in SCHEMA_MIGRATIONS {
            let _ = conn.execute(migration, ()).await;
        }
        
        conn
    }

    #[tokio::test]
    async fn test_user_creation_and_retrieval() {
        let conn = setup_test_db().await;
        let repo = SqliteUserRepository::new(conn);
        
        let new_user = NewUser {
            telegram_id: 12345,
            username: Some("testuser".to_string()),
        };
        
        let created = repo.create(new_user).await.unwrap();
        assert_eq!(created.telegram_id, 12345);
        assert_eq!(created.username, Some("testuser".to_string()));
        
        let retrieved = repo.get_by_telegram_id(12345).await.unwrap().unwrap();
        assert_eq!(retrieved.id, created.id);
        assert_eq!(retrieved.username, created.username);
    }
}

impl SqliteUserRepository {
    /// Create a new SqliteUserRepository with the given connection.
    pub fn new(conn: Connection) -> Self {
        Self { 
            conn,
        }
    }

    fn map_user(row: &libsql::Row) -> Result<User, libsql::Error> {
        Ok(User {
            id: row.get(0)?,
            telegram_id: row.get(1)?,
            username: row.get(2)?,
            default_pack_id: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    }
}

#[async_trait]
impl UserRepository for SqliteUserRepository {
    async fn get_by_telegram_id(&self, telegram_id: i64) -> Result<Option<Arc<User>>, RepositoryError> {
        let mut rows = self.conn.query(
            "SELECT id, telegram_id, username, default_pack_id, created_at, updated_at FROM users WHERE telegram_id = ?",
            [telegram_id]
        ).await?;

        if let Some(row) = rows.next().await? {
            let user = Arc::new(Self::map_user(&row)?);
            Ok(Some(user))
        } else {
            Ok(None)
        }
    }

    async fn create(&self, user: NewUser) -> Result<Arc<User>, RepositoryError> {
        if let Some(existing) = self.get_by_telegram_id(user.telegram_id).await? {
            return Ok(existing);
        }

        let mut rows = self.conn.query(
            "INSERT INTO users (telegram_id, username) VALUES (?, ?) RETURNING id, telegram_id, username, default_pack_id, created_at, updated_at",
            libsql::params![user.telegram_id, user.username]
        ).await?;

        if let Some(row) = rows.next().await? {
            Ok(Arc::new(Self::map_user(&row)?))
        } else {
            // This shouldn't happen with RETURNING
            Err(RepositoryError::DuplicateEntry)
        }
    }

    async fn update(&self, user: &User) -> Result<(), RepositoryError> {
        let rows_affected = self.conn.execute(
            "UPDATE users SET username = ?, updated_at = unixepoch() WHERE id = ?",
            libsql::params![user.username.clone(), user.id]
        ).await?;

        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }

        Ok(())
    }

    async fn set_default_pack(&self, user_id: i64, pack_id: Option<i64>) -> Result<(), RepositoryError> {
        let rows_affected = self.conn.execute(
            "UPDATE users SET default_pack_id = ?, updated_at = unixepoch() WHERE id = ?",
            libsql::params![pack_id, user_id]
        ).await?;

        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }
        
        Ok(())
    }

    async fn get_default_pack_id(&self, user_id: i64) -> Result<Option<i64>, RepositoryError> {
        let mut rows = self.conn.query(
            "SELECT default_pack_id FROM users WHERE id = ?",
            [user_id]
        ).await?;

        if let Some(row) = rows.next().await? {
            Ok(row.get(0)?)
        } else {
            Err(RepositoryError::NotFound)
        }
    }
}