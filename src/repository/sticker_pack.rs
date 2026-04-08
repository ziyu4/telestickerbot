//! Sticker pack repository trait and SQLite implementation

use async_trait::async_trait;
use libsql::Connection;
use std::sync::Arc;
use crate::db::schema::{NewStickerPack, StickerPack};
use super::RepositoryError;

#[async_trait]
pub trait StickerPackRepository: Send + Sync {
    async fn get_active_pack(&self, user_id: i64) -> Result<Option<Arc<StickerPack>>, RepositoryError>;
    async fn create(&self, pack: NewStickerPack) -> Result<Arc<StickerPack>, RepositoryError>;
    async fn increment_sticker_count(&self, pack_id: i64) -> Result<(), RepositoryError>;
    async fn set_active_pack(&self, user_id: i64, pack_id: i64) -> Result<(), RepositoryError>;
    async fn get_by_id(&self, pack_id: i64) -> Result<Option<Arc<StickerPack>>, RepositoryError>;
    async fn get_all_by_user(&self, user_id: i64) -> Result<Vec<Arc<StickerPack>>, RepositoryError>;
    async fn update_sticker_count(&self, pack_id: i64, count: i32) -> Result<(), RepositoryError>;
    async fn update_last_synced(&self, pack_id: i64) -> Result<(), RepositoryError>;
    async fn is_pack_full(&self, pack_id: i64) -> Result<bool, RepositoryError>;
    async fn delete(&self, pack_id: i64) -> Result<(), RepositoryError>;
    async fn get_by_pack_link(&self, link: &str) -> Result<Option<Arc<StickerPack>>, RepositoryError>;
    async fn insert_recovered_pack(&self, pack: StickerPack) -> Result<Arc<StickerPack>, RepositoryError>;
}

/// SQLite/libSQL implementation of the StickerPackRepository trait.
pub struct SqliteStickerPackRepository {
    conn: Connection,
}

impl SqliteStickerPackRepository {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn,
        }
    }

    fn map_sticker_pack(row: &libsql::Row) -> Result<StickerPack, libsql::Error> {
        Ok(StickerPack {
            id: row.get(0)?,
            user_id: row.get(1)?,
            pack_name: row.get(2)?,
            pack_link: row.get(3)?,
            version: row.get(4)?,
            sticker_count: row.get(5)?,
            is_active: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
            last_synced_at: row.get(9)?,
        })
    }
}

#[async_trait]
impl StickerPackRepository for SqliteStickerPackRepository {
    async fn get_active_pack(&self, user_id: i64) -> Result<Option<Arc<StickerPack>>, RepositoryError> {
        let mut rows = self.conn.query(
            "SELECT id, user_id, pack_name, pack_link, version, sticker_count, is_active, created_at, updated_at, last_synced_at FROM sticker_packs WHERE user_id = ? AND is_active = 1 LIMIT 1",
            [user_id]
        ).await?;

        if let Some(row) = rows.next().await? {
            Ok(Some(Arc::new(Self::map_sticker_pack(&row)?)))
        } else {
            Ok(None)
        }
    }

    async fn create(&self, pack: NewStickerPack) -> Result<Arc<StickerPack>, RepositoryError> {
        self.conn.execute(
            "UPDATE sticker_packs SET is_active = 0, updated_at = unixepoch() WHERE user_id = ? AND is_active = 1",
            [pack.user_id]
        ).await?;

        let mut rows = self.conn.query(
            "INSERT INTO sticker_packs (user_id, pack_name, pack_link, version, sticker_count, is_active) VALUES (?, ?, ?, ?, 0, 1) RETURNING id, user_id, pack_name, pack_link, version, sticker_count, is_active, created_at, updated_at, last_synced_at",
            libsql::params![pack.user_id, pack.pack_name, pack.pack_link, pack.version]
        ).await?;

        if let Some(row) = rows.next().await? {
            Ok(Arc::new(Self::map_sticker_pack(&row)?))
        } else {
            Err(RepositoryError::DuplicateEntry)
        }
    }

    async fn increment_sticker_count(&self, pack_id: i64) -> Result<(), RepositoryError> {
        let rows_affected = self.conn.execute(
            "UPDATE sticker_packs SET sticker_count = sticker_count + 1, updated_at = unixepoch(), last_synced_at = unixepoch() WHERE id = ?",
            [pack_id]
        ).await?;

        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }

        Ok(())
    }

    async fn set_active_pack(&self, user_id: i64, pack_id: i64) -> Result<(), RepositoryError> {
        let mut rows = self.conn.query("SELECT user_id FROM sticker_packs WHERE id = ?", [pack_id]).await?;
        if let Some(row) = rows.next().await? {
            let owner_id: i64 = row.get(0)?;
            if owner_id == user_id {
                self.conn.execute("UPDATE sticker_packs SET is_active = 0, updated_at = unixepoch() WHERE user_id = ?", [user_id]).await?;
                self.conn.execute("UPDATE sticker_packs SET is_active = 1, updated_at = unixepoch() WHERE id = ?", [pack_id]).await?;
                Ok(())
            } else {
                Err(RepositoryError::NotFound)
            }
        } else {
            Err(RepositoryError::NotFound)
        }
    }

    async fn get_by_id(&self, pack_id: i64) -> Result<Option<Arc<StickerPack>>, RepositoryError> {
        let mut rows = self.conn.query(
            "SELECT id, user_id, pack_name, pack_link, version, sticker_count, is_active, created_at, updated_at, last_synced_at FROM sticker_packs WHERE id = ?",
            [pack_id]
        ).await?;

        if let Some(row) = rows.next().await? {
            let pack = Arc::new(Self::map_sticker_pack(&row)?);
            Ok(Some(pack))
        } else {
            Ok(None)
        }
    }

    async fn get_all_by_user(&self, user_id: i64) -> Result<Vec<Arc<StickerPack>>, RepositoryError> {
        let mut rows = self.conn.query(
            "SELECT id, user_id, pack_name, pack_link, version, sticker_count, is_active, created_at, updated_at, last_synced_at FROM sticker_packs WHERE user_id = ? ORDER BY created_at DESC",
            [user_id]
        ).await?;

        let mut packs = Vec::new();
        while let Some(row) = rows.next().await? {
            packs.push(Arc::new(Self::map_sticker_pack(&row)?));
        }
        Ok(packs)
    }

    async fn update_sticker_count(&self, pack_id: i64, count: i32) -> Result<(), RepositoryError> {
        let rows_affected = self.conn.execute(
            "UPDATE sticker_packs SET sticker_count = ?, updated_at = unixepoch() WHERE id = ?",
            libsql::params![count, pack_id]
        ).await?;

        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }

        Ok(())
    }

    async fn update_last_synced(&self, pack_id: i64) -> Result<(), RepositoryError> {
        let rows_affected = self.conn.execute(
            "UPDATE sticker_packs SET last_synced_at = unixepoch(), updated_at = unixepoch() WHERE id = ?",
            [pack_id]
        ).await?;

        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }

        Ok(())
    }

    async fn is_pack_full(&self, pack_id: i64) -> Result<bool, RepositoryError> {
        let mut rows = self.conn.query("SELECT sticker_count FROM sticker_packs WHERE id = ?", [pack_id]).await?;

        if let Some(row) = rows.next().await? {
            let count: i32 = row.get(0)?;
            Ok(count >= 120)
        } else {
            Err(RepositoryError::NotFound)
        }
    }

    async fn delete(&self, pack_id: i64) -> Result<(), RepositoryError> {
        let rows_affected = self.conn.execute("DELETE FROM sticker_packs WHERE id = ?", [pack_id]).await?;
            
        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }

        Ok(())
    }

    async fn get_by_pack_link(&self, link: &str) -> Result<Option<Arc<StickerPack>>, RepositoryError> {
        let mut rows = self.conn.query(
            "SELECT id, user_id, pack_name, pack_link, version, sticker_count, is_active, created_at, updated_at, last_synced_at FROM sticker_packs WHERE pack_link = ? LIMIT 1",
            [link.to_string()]
        ).await?;

        if let Some(row) = rows.next().await? {
            Ok(Some(Arc::new(Self::map_sticker_pack(&row)?)))
        } else {
            Ok(None)
        }
    }

    async fn insert_recovered_pack(&self, pack: StickerPack) -> Result<Arc<StickerPack>, RepositoryError> {
        self.conn.execute(
            "UPDATE sticker_packs SET is_active = 0, updated_at = unixepoch() WHERE user_id = ? AND is_active = 1",
            [pack.user_id]
        ).await?;

        let mut rows = self.conn.query(
            "INSERT INTO sticker_packs (user_id, pack_name, pack_link, version, sticker_count, is_active, last_synced_at) VALUES (?, ?, ?, ?, ?, 1, unixepoch()) RETURNING id, user_id, pack_name, pack_link, version, sticker_count, is_active, created_at, updated_at, last_synced_at",
            libsql::params![pack.user_id, pack.pack_name, pack.pack_link, pack.version, pack.sticker_count]
        ).await?;

        if let Some(row) = rows.next().await? {
            Ok(Arc::new(Self::map_sticker_pack(&row)?))
        } else {
            Err(RepositoryError::DuplicateEntry)
        }
    }
}