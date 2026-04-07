//! Sticker pack repository trait and SQLite implementation

use async_trait::async_trait;
use sqlx::SqlitePool;
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

/// SQLite implementation of the StickerPackRepository trait.
pub struct SqliteStickerPackRepository {
    pool: SqlitePool,
    cache: Arc<crate::cache::CacheLayer<i64, StickerPack>>,
}

impl SqliteStickerPackRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            cache: Arc::new(crate::cache::CacheLayer::None),
        }
    }

    pub fn with_cache(mut self, cache: Arc<crate::cache::CacheLayer<i64, StickerPack>>) -> Self {
        self.cache = cache;
        self
    }
}

#[async_trait]
impl StickerPackRepository for SqliteStickerPackRepository {
    /// Fetch the active sticker pack for a user.
    ///
    /// Returns `Ok(Some(pack))` if an active pack exists, `Ok(None)` if not found.
    ///
    /// active pack per user at any given time.
    async fn get_active_pack(&self, user_id: i64) -> Result<Option<Arc<StickerPack>>, RepositoryError> {
        let pack = sqlx::query_as::<_, StickerPack>(
            r#"
            SELECT id, user_id, pack_name, pack_link, version, sticker_count, 
                   is_active, created_at, updated_at, last_synced_at
            FROM sticker_packs
            WHERE user_id = ? AND is_active = 1
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(pack.map(Arc::new))
    }

    /// Create a new sticker pack.
    ///
    /// The new pack is created with sticker_count = 0 and is_active = true.
    /// If the user already has an active pack, it will be deactivated first
    /// to maintain Property 4: Active Pack Uniqueness.
    async fn create(&self, pack: NewStickerPack) -> Result<Arc<StickerPack>, RepositoryError> {
        // Deactivate any existing active packs for this user to maintain
        // Property 4: Active Pack Uniqueness
        sqlx::query(
            "UPDATE sticker_packs SET is_active = 0, updated_at = unixepoch() WHERE user_id = ? AND is_active = 1",
        )
        .bind(pack.user_id)
        .execute(&self.pool)
        .await?;

        // Insert the new pack
        let result = sqlx::query_as::<_, StickerPack>(
            r#"
            INSERT INTO sticker_packs (user_id, pack_name, pack_link, version, sticker_count, is_active)
            VALUES (?, ?, ?, ?, 0, 1)
            RETURNING id, user_id, pack_name, pack_link, version, sticker_count, 
                      is_active, created_at, updated_at, last_synced_at
            "#,
        )
        .bind(pack.user_id)
        .bind(&pack.pack_name)
        .bind(&pack.pack_link)
        .bind(&pack.version)
        .fetch_one(&self.pool)
        .await?;

        Ok(Arc::new(result))
    }

    /// Increment the sticker count for a pack.
    ///
    /// adding a sticker, the sticker_count becomes N+1.
    async fn increment_sticker_count(&self, pack_id: i64) -> Result<(), RepositoryError> {
        let rows_affected = sqlx::query(
            r#"
            UPDATE sticker_packs
            SET sticker_count = sticker_count + 1, 
                updated_at = unixepoch(),
                last_synced_at = unixepoch()
            WHERE id = ?
            "#,
        )
        .bind(pack_id)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }

        self.cache.invalidate(&pack_id).await;

        Ok(())
    }

    /// Set a specific pack as the active pack for a user.
    ///
    /// Deactivates any currently active pack for the user and activates
    /// the specified pack. This maintains Property 4: Active Pack Uniqueness.
    async fn set_active_pack(&self, user_id: i64, pack_id: i64) -> Result<(), RepositoryError> {
        // First, verify the pack belongs to the user
        let pack = sqlx::query_as::<_, StickerPack>(
            "SELECT id, user_id, pack_name, pack_link, version, sticker_count, is_active, created_at, updated_at, last_synced_at FROM sticker_packs WHERE id = ?",
        )
        .bind(pack_id)
        .fetch_optional(&self.pool)
        .await?;

        match pack {
            Some(p) if p.user_id == user_id => {
                // Deactivate all packs for this user
                sqlx::query(
                    "UPDATE sticker_packs SET is_active = 0, updated_at = unixepoch() WHERE user_id = ?",
                )
                .bind(user_id)
                .execute(&self.pool)
                .await?;

                // Activate the specified pack
                sqlx::query(
                    "UPDATE sticker_packs SET is_active = 1, updated_at = unixepoch() WHERE id = ?",
                )
                .bind(pack_id)
                .execute(&self.pool)
                .await?;

                Ok(())
            }
            Some(_) => Err(RepositoryError::NotFound), // Pack belongs to different user
            None => Err(RepositoryError::NotFound),    // Pack doesn't exist
        }
    }

    /// Get a sticker pack by its ID.
    ///
    /// Returns `Ok(Some(pack))` if the pack exists, `Ok(None)` if not found.
    async fn get_by_id(&self, pack_id: i64) -> Result<Option<Arc<StickerPack>>, RepositoryError> {
        // Check cache first
        if let Some(cached_pack) = self.cache.get(&pack_id).await {
            return Ok(Some(cached_pack));
        }

        // Query database
        let pack = sqlx::query_as::<_, StickerPack>(
            "SELECT id, user_id, pack_name, pack_link, version, sticker_count, is_active, created_at, updated_at, last_synced_at FROM sticker_packs WHERE id = ?"
        )
        .bind(pack_id)
        .fetch_optional(&self.pool)
        .await?;

        // Cache search results if found
        if let Some(ref p) = pack {
            self.cache.insert(pack_id, Arc::new(p.clone())).await;
        }

        Ok(pack.map(Arc::new))
    }

    /// Get all sticker packs for a user.
    ///
    /// Returns a vector of all packs owned by the user, ordered by creation date
    /// (newest first).
    async fn get_all_by_user(&self, user_id: i64) -> Result<Vec<Arc<StickerPack>>, RepositoryError> {
        let packs = sqlx::query_as::<_, StickerPack>(
            r#"
            SELECT id, user_id, pack_name, pack_link, version, sticker_count, 
                   is_active, created_at, updated_at, last_synced_at
            FROM sticker_packs
            WHERE user_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(packs.into_iter().map(Arc::new).collect())
    }

    /// Update the sticker count for a pack.
    ///
    /// This method sets the sticker count to an exact value, typically used
    /// during synchronization with Telegram API.
    async fn update_sticker_count(&self, pack_id: i64, count: i32) -> Result<(), RepositoryError> {
        let rows_affected = sqlx::query(
            r#"
            UPDATE sticker_packs
            SET sticker_count = ?, 
                updated_at = unixepoch()
            WHERE id = ?
            "#,
        )
        .bind(count)
        .bind(pack_id)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }

        self.cache.invalidate(&pack_id).await;

        Ok(())
    }

    /// Update the last_synced_at timestamp for a pack.
    ///
    /// This method is called after successfully synchronizing pack data
    /// with Telegram API.
    async fn update_last_synced(&self, pack_id: i64) -> Result<(), RepositoryError> {
        let rows_affected = sqlx::query(
            r#"
            UPDATE sticker_packs
            SET last_synced_at = unixepoch(),
                updated_at = unixepoch()
            WHERE id = ?
            "#,
        )
        .bind(pack_id)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }

        // Invalidate cache to force refresh of timestamps on next get
        self.cache.invalidate(&pack_id).await;

        Ok(())
    }

    /// Check if a pack has reached the 120 sticker capacity limit.
    ///
    /// Returns `Ok(true)` if the pack has 120 or more stickers,
    /// `Ok(false)` if it has fewer than 120 stickers.
    ///
    /// when sticker_count >= 120.
    async fn is_pack_full(&self, pack_id: i64) -> Result<bool, RepositoryError> {
        let pack = sqlx::query_as::<_, StickerPack>(
            r#"
            SELECT id, user_id, pack_name, pack_link, version, sticker_count, 
                   is_active, created_at, updated_at, last_synced_at
            FROM sticker_packs
            WHERE id = ?
            "#,
        )
        .bind(pack_id)
        .fetch_optional(&self.pool)
        .await?;

        match pack {
            Some(p) => Ok(p.sticker_count >= 120),
            None => Err(RepositoryError::NotFound),
        }
    }

    /// Delete a sticker pack.
    async fn delete(&self, pack_id: i64) -> Result<(), RepositoryError> {
        let rows_affected = sqlx::query("DELETE FROM sticker_packs WHERE id = ?")
            .bind(pack_id)
            .execute(&self.pool)
            .await?
            .rows_affected();
            
        if rows_affected == 0 {
            return Err(RepositoryError::NotFound);
        }

        self.cache.invalidate(&pack_id).await;
        
        Ok(())
    }

    /// Get a sticker pack by its link.
    async fn get_by_pack_link(&self, link: &str) -> Result<Option<Arc<StickerPack>>, RepositoryError> {
        let pack = sqlx::query_as::<_, StickerPack>(
            r#"
            SELECT id, user_id, pack_name, pack_link, version, sticker_count, 
                   is_active, created_at, updated_at, last_synced_at
            FROM sticker_packs
            WHERE pack_link = ?
            LIMIT 1
            "#,
        )
        .bind(link)
        .fetch_optional(&self.pool)
        .await?;

        Ok(pack.map(Arc::new))
    }

    /// Insert a recovered pack (one that already exists on Telegram).
    /// If the user already has an active pack, it will be deactivated first.
    async fn insert_recovered_pack(&self, pack: StickerPack) -> Result<Arc<StickerPack>, RepositoryError> {
        // Deactivate any existing active packs for this user
        sqlx::query(
            "UPDATE sticker_packs SET is_active = 0, updated_at = unixepoch() WHERE user_id = ? AND is_active = 1",
        )
        .bind(pack.user_id)
        .execute(&self.pool)
        .await?;

        // Insert the recovered pack
        let result = sqlx::query_as::<_, StickerPack>(
            r#"
            INSERT INTO sticker_packs (user_id, pack_name, pack_link, version, sticker_count, is_active, last_synced_at)
            VALUES (?, ?, ?, ?, ?, 1, unixepoch())
            RETURNING id, user_id, pack_name, pack_link, version, sticker_count, 
                      is_active, created_at, updated_at, last_synced_at
            "#,
        )
        .bind(pack.user_id)
        .bind(&pack.pack_name)
        .bind(&pack.pack_link)
        .bind(&pack.version)
        .bind(pack.sticker_count)
        .fetch_one(&self.pool)
        .await?;

        Ok(Arc::new(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{SqlitePool, Row};
    use crate::db::schema::SCHEMA_MIGRATIONS;

    async fn setup_test_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        
        // Run migrations, handling duplicate column errors for idempotency
        for migration in SCHEMA_MIGRATIONS {
            let result = sqlx::query(migration).execute(&pool).await;
            
            // Allow duplicate column errors for idempotency
            if let Err(e) = result {
                let error_msg = e.to_string();
                if !error_msg.contains("duplicate column name") && !error_msg.contains("no such column") {
                    panic!("Migration failed: {}", e);
                }
                // Ignore "no such column" errors during UPDATE migrations when column doesn't exist yet
            }
        }
        
        pool
    }

    async fn create_test_user(pool: &SqlitePool, telegram_id: i64) -> i64 {
        let result = sqlx::query(
            "INSERT INTO users (telegram_id, username) VALUES (?, ?) RETURNING id"
        )
        .bind(telegram_id)
        .bind(format!("user_{}", telegram_id))
        .fetch_one(pool)
        .await
        .unwrap();
        
        result.get(0)
    }

    #[tokio::test]
    async fn test_get_by_id_existing_pack() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user and pack
        let user_id = create_test_user(&pool, 123456).await;
        let pack = repo.create(NewStickerPack {
            user_id,
            pack_name: "Test Pack".to_string(),
            pack_link: "test_pack_by_bot".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        // Test get_by_id
        let result = repo.get_by_id(pack.id).await.unwrap();
        assert!(result.is_some());
        let retrieved_pack = result.unwrap();
        assert_eq!(retrieved_pack.id, pack.id);
        assert_eq!(retrieved_pack.pack_name, "Test Pack");
    }

    #[tokio::test]
    async fn test_get_by_id_nonexistent_pack() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool);
        
        // Test get_by_id with non-existent ID
        let result = repo.get_by_id(999).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_all_by_user() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user
        let user_id = create_test_user(&pool, 123456).await;
        
        // Create multiple packs
        let pack1 = repo.create(NewStickerPack {
            user_id,
            pack_name: "Pack 1".to_string(),
            pack_link: "pack1_by_bot".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        let pack2 = repo.create(NewStickerPack {
            user_id,
            pack_name: "Pack 2".to_string(),
            pack_link: "pack2_by_bot".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        // Test get_all_by_user
        let packs = repo.get_all_by_user(user_id).await.unwrap();
        assert_eq!(packs.len(), 2);
        
        // Verify both packs are present (order may vary due to timing)
        let pack_names: Vec<&str> = packs.iter().map(|p| p.pack_name.as_str()).collect();
        assert!(pack_names.contains(&"Pack 1"));
        assert!(pack_names.contains(&"Pack 2"));
        
        // Verify IDs match
        let pack_ids: Vec<i64> = packs.iter().map(|p| p.id).collect();
        assert!(pack_ids.contains(&pack1.id));
        assert!(pack_ids.contains(&pack2.id));
    }

    #[tokio::test]
    async fn test_get_all_by_user_empty() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user with no packs
        let user_id = create_test_user(&pool, 123456).await;
        
        // Test get_all_by_user
        let packs = repo.get_all_by_user(user_id).await.unwrap();
        assert_eq!(packs.len(), 0);
    }

    #[tokio::test]
    async fn test_update_sticker_count() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user and pack
        let user_id = create_test_user(&pool, 123456).await;
        let pack = repo.create(NewStickerPack {
            user_id,
            pack_name: "Test Pack".to_string(),
            pack_link: "test_pack_by_bot".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        assert_eq!(pack.sticker_count, 0);
        
        // Update sticker count
        repo.update_sticker_count(pack.id, 50).await.unwrap();
        
        // Verify update
        let updated_pack = repo.get_by_id(pack.id).await.unwrap().unwrap();
        assert_eq!(updated_pack.sticker_count, 50);
    }

    #[tokio::test]
    async fn test_update_sticker_count_nonexistent() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool);
        
        // Test update on non-existent pack
        let result = repo.update_sticker_count(999, 50).await;
        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }

    #[tokio::test]
    async fn test_update_last_synced() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user and pack
        let user_id = create_test_user(&pool, 123456).await;
        let pack = repo.create(NewStickerPack {
            user_id,
            pack_name: "Test Pack".to_string(),
            pack_link: "test_pack_by_bot".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        assert!(pack.last_synced_at.is_none());
        
        // Update last_synced_at
        repo.update_last_synced(pack.id).await.unwrap();
        
        // Verify update
        let updated_pack = repo.get_by_id(pack.id).await.unwrap().unwrap();
        assert!(updated_pack.last_synced_at.is_some());
    }

    #[tokio::test]
    async fn test_update_last_synced_nonexistent() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool);
        
        // Test update on non-existent pack
        let result = repo.update_last_synced(999).await;
        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }

    #[tokio::test]
    async fn test_is_pack_full_empty_pack() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user and pack
        let user_id = create_test_user(&pool, 123456).await;
        let pack = repo.create(NewStickerPack {
            user_id,
            pack_name: "Test Pack".to_string(),
            pack_link: "test_pack_by_bot".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        // Test is_pack_full on empty pack
        let is_full = repo.is_pack_full(pack.id).await.unwrap();
        assert!(!is_full);
    }

    #[tokio::test]
    async fn test_is_pack_full_partial_pack() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user and pack
        let user_id = create_test_user(&pool, 123456).await;
        let pack = repo.create(NewStickerPack {
            user_id,
            pack_name: "Test Pack".to_string(),
            pack_link: "test_pack_by_bot".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        // Update to 50 stickers
        repo.update_sticker_count(pack.id, 50).await.unwrap();
        
        // Test is_pack_full
        let is_full = repo.is_pack_full(pack.id).await.unwrap();
        assert!(!is_full);
    }

    #[tokio::test]
    async fn test_is_pack_full_at_capacity() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user and pack
        let user_id = create_test_user(&pool, 123456).await;
        let pack = repo.create(NewStickerPack {
            user_id,
            pack_name: "Test Pack".to_string(),
            pack_link: "test_pack_by_bot".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        // Update to 120 stickers (capacity limit)
        repo.update_sticker_count(pack.id, 120).await.unwrap();
        
        // Test is_pack_full
        let is_full = repo.is_pack_full(pack.id).await.unwrap();
        assert!(is_full);
    }

    #[tokio::test]
    async fn test_is_pack_full_over_capacity() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user and pack
        let user_id = create_test_user(&pool, 123456).await;
        let pack = repo.create(NewStickerPack {
            user_id,
            pack_name: "Test Pack".to_string(),
            pack_link: "test_pack_by_bot".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        // Update to 125 stickers (over capacity)
        repo.update_sticker_count(pack.id, 125).await.unwrap();
        
        // Test is_pack_full
        let is_full = repo.is_pack_full(pack.id).await.unwrap();
        assert!(is_full);
    }

    #[tokio::test]
    async fn test_is_pack_full_nonexistent() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool);
        
        // Test on non-existent pack
        let result = repo.is_pack_full(999).await;
        assert!(matches!(result, Err(RepositoryError::NotFound)));
    }

    #[tokio::test]
    async fn test_cache_hot_reload_cow() {
        let pool = setup_test_db().await;
        let repo = SqliteStickerPackRepository::new(pool.clone());
        
        // Create user and pack
        let user_id = create_test_user(&pool, 123456).await;
        let pack = repo.create(NewStickerPack {
            user_id,
            pack_name: "COW Test".to_string(),
            pack_link: "cow_test".to_string(),
            version: "v1".to_string(),
        }).await.unwrap();
        
        // 1. Initial fetch to populate cache
        let pack_v1 = repo.get_by_id(pack.id).await.unwrap().unwrap();
        assert_eq!(pack_v1.sticker_count, 0);
        
        // 2. Increment count via repository (this should trigger COW update in cache)
        repo.increment_sticker_count(pack.id).await.unwrap();
        
        // 3. Fetch again. If hot reload works, this should return 1.
        let pack_v2 = repo.get_by_id(pack.id).await.unwrap().unwrap();
        assert_eq!(pack_v2.sticker_count, 1);

        // 4. Update count to specific value
        repo.update_sticker_count(pack.id, 50).await.unwrap();
        
        // 5. Fetch again. Should be 50.
        let pack_v3 = repo.get_by_id(pack.id).await.unwrap().unwrap();
        assert_eq!(pack_v3.sticker_count, 50);
    }
}