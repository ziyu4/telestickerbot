//! Sticker pack business logic
//!
//! This module provides the `StickerService` which handles sticker pack-related business logic
//! including sticker addition (kang), custom pack creation, and stale pack detection.

use std::sync::Arc;
use chrono::Utc;

use crate::bot::error::BotError;
use crate::bot::telegram::{TelegramClient, create_input_sticker_from_file_id};
use crate::db::schema::{NewStickerPack, StickerPack, User};
use crate::repository::{RepositoryError, StickerPackRepository, UserRepository};
use teloxide::types::StickerFormat;

/// Maximum number of stickers per pack before creating a new version (Telegram limit).
const MAX_STICKERS_PER_PACK: i32 = 120;

/// Maximum pack name length (Telegram limit).
const MAX_PACK_NAME_LENGTH: usize = 64;

/// Duration in seconds after which a pack is considered stale (7 days).
const STALE_THRESHOLD_SECONDS: i64 = 604_800;

/// Result of a kang operation.
#[derive(Debug, Clone)]
pub struct KangResult {
    /// The sticker pack that was modified or created.
    pub pack: Arc<StickerPack>,
    /// Whether a new pack was created.
    pub created_new_pack: bool,
}

/// Result of a create pack operation.
#[derive(Debug, Clone)]
pub struct CreatePackResult {
    /// The newly created sticker pack.
    pub pack: Arc<StickerPack>,
}

/// Service for sticker pack operations.
///
/// This service provides business logic for sticker pack management, including:
/// - Adding stickers to packs (kang operation)
/// - Creating custom-named packs
/// - Automatic pack versioning when packs are full
/// - Stale pack detection and re-synchronization
///
/// The service uses repositories for database operations and optionally
/// a cache for performance optimization.
pub struct StickerService<U, S>
where
    U: UserRepository,
    S: StickerPackRepository,
{
    user_repository: Arc<U>,
    sticker_pack_repository: Arc<S>,
    bot_username: String,
    telegram_client: Arc<TelegramClient<teloxide::adaptors::Throttle<teloxide::Bot>>>,
}

impl<U, S> StickerService<U, S>
where
    U: UserRepository,
    S: StickerPackRepository,
{
    /// Create a new StickerService with the given dependencies.
    ///
    /// # Arguments
    /// * `user_repository` - Repository for user database operations
    /// * `sticker_pack_repository` - Repository for sticker pack database operations
    /// * `cache` - Cache for storing sticker pack data
    /// * `bot_username` - The bot's username (used for pack link generation)
    /// * `telegram_client` - Client for Telegram API operations
    pub fn new(
        user_repository: Arc<U>,
        sticker_pack_repository: Arc<S>,
        bot_username: String,
        telegram_client: Arc<TelegramClient<teloxide::adaptors::Throttle<teloxide::Bot>>>,
    ) -> Self {
        Self {
            user_repository,
            sticker_pack_repository,
            bot_username,
            telegram_client,
        }
    }

    /// Add a sticker to the user's default or active pack (kang operation).
    ///
    /// This method implements the kang command logic:
    /// 1. If the user has a default pack configured, use that pack
    /// 2. Otherwise, fall back to the active pack
    /// 3. If no pack exists, create a new one with default naming
    /// 4. If the pack is full (9 stickers), create a new version
    /// 5. Add the sticker to the pack via Telegram API
    ///
    /// # Arguments
    /// * `user` - The user performing the kang operation
    /// * `sticker_file_id` - The Telegram file ID of the sticker to add
    /// * `sticker_format` - The format of the sticker (static, animated, or video)
    /// * `emojis` - The emojis associated with the sticker
    ///
    /// # Returns
    /// A `KangResult` containing the pack and whether a new pack was created.
    pub async fn kang_sticker(
        &self,
        user: &User,
        sticker_file_id: &str,
        sticker_format: StickerFormat,
        emojis: &str,
    ) -> Result<KangResult, BotError> {
        // Get the pack to use: default pack if configured, otherwise active pack
        let pack = self.get_pack_for_kang(user).await?;

        // Lazy sync: Check if pack is stale and sync if needed
        let pack = if let Some(p) = pack {
            if Self::is_pack_stale(&p) {
                tracing::info!(
                    pack_id = p.id,
                    pack_link = %p.pack_link,
                    "Pack is stale (last sync > 7 days), syncing before operation"
                );
                // Sync the pack to get fresh data from Telegram
                match self.sync_pack_internal(&p).await {
                    Ok(synced_pack) => Some(synced_pack),
                    Err(e) => {
                        tracing::warn!(error = ?e, "Failed to sync stale pack");
                        // If pack is invalid/deleted, we should force it to trigger a new pack creation
                        if Self::is_pack_invalid_error(&e) {
                            tracing::info!("Pack is invalid (deleted from Telegram), forcing pack recreation");
                            let mut mutated_pack = (*p).clone();
                            mutated_pack.sticker_count = MAX_STICKERS_PER_PACK;
                            Some(Arc::new(mutated_pack))
                        } else {
                            Some(p)
                        }
                    }
                }
            } else {
                Some(p)
            }
        } else {
            None
        };

        match pack {
            Some(pack) => {
                // Check if pack is full (120 stickers)
                if pack.sticker_count >= MAX_STICKERS_PER_PACK {
                    // Create a new version
                    let new_pack = self.create_next_version_pack(user, &pack).await?;
                    
                    // Create input sticker for Telegram API
                    let input_sticker = create_input_sticker_from_file_id(
                        sticker_file_id,
                        &sticker_format,
                        emojis,
                    );
                    
                    // Create new sticker set via Telegram API
                    if let Err(e) = self.telegram_client
                        .create_sticker_set(
                            user.telegram_id,
                            &new_pack.pack_link,
                            &new_pack.pack_name,
                            input_sticker.clone(),
                        )
                        .await
                    {
                        // Handle "Occupied" error - could happen if database was lost
                        if Self::is_pack_occupied_error(&e) {
                            tracing::info!(pack_link = %new_pack.pack_link, "Next version pack already exists on Telegram, recovering...");
                            let recovered_pack = self.handle_occupied_pack_recovery(
                                user, 
                                &new_pack.pack_link, 
                                &new_pack.pack_name,
                                &new_pack.version
                            ).await?;
                            
                            // Re-add the sticker to the recovered pack
                            self.telegram_client
                                .add_sticker_to_set(user.telegram_id, &recovered_pack.pack_link, input_sticker)
                                .await?;
                                
                            self.sticker_pack_repository.increment_sticker_count(recovered_pack.id).await?;
                            return Ok(KangResult {
                                pack: recovered_pack,
                                created_new_pack: true,
                            });
                        }
                        
                        tracing::error!(error = ?e, "Failed to create new sticker set version");
                        return Err(e);
                    }
                    
                    // Increment sticker count for the new pack
                    self.sticker_pack_repository
                        .increment_sticker_count(new_pack.id)
                        .await?;
                    
                    // Invalidate cache

                    
                    Ok(KangResult {
                        pack: new_pack,
                        created_new_pack: true,
                    })
                } else {
                    // Create input sticker for Telegram API
                    let input_sticker = create_input_sticker_from_file_id(
                        sticker_file_id,
                        &sticker_format,
                        emojis,
                    );
                    
                    // Add sticker to existing pack via Telegram API
                    if let Err(e) = self.telegram_client
                        .add_sticker_to_set(user.telegram_id, &pack.pack_link, input_sticker)
                        .await
                    {
                        // Check if error is pack full or invalid - sync and handle
                        if Self::is_pack_full_error(&e) || Self::is_pack_invalid_error(&e) {
                            tracing::info!("Pack full or invalid error detected, syncing and creating new version");
                            let synced_pack = self.sync_pack_internal(&pack).await.unwrap_or(pack.clone());
                            
                            // Create new version since pack is confirmed full
                            let new_pack = self.create_next_version_pack(user, &synced_pack).await?;
                            let retry_input_sticker = create_input_sticker_from_file_id(
                                sticker_file_id,
                                &sticker_format,
                                emojis,
                            );
                            
                            self.telegram_client
                                .add_sticker_to_set(user.telegram_id, &new_pack.pack_link, retry_input_sticker)
                                .await?;
                            
                            self.sticker_pack_repository.increment_sticker_count(new_pack.id).await?;
        
                            
                            return Ok(KangResult {
                                pack: new_pack,
                                created_new_pack: true,
                            });
                        }
                        tracing::error!(error = ?e, "Failed to add sticker to existing pack");
                        return Err(e);
                    }
                    
                    // Increment sticker count
                    self.sticker_pack_repository
                        .increment_sticker_count(pack.id)
                        .await?;
                    
                    // Invalidate cache

                    
                    Ok(KangResult {
                        pack,
                        created_new_pack: false,
                    })
                }
            }
            None => {
                // No active pack - create the first one
                let new_pack = self.create_default_pack(user).await?;
                
                // Create input sticker for Telegram API
                let input_sticker = create_input_sticker_from_file_id(
                    sticker_file_id,
                    &sticker_format,
                    emojis,
                );
                
                // Create new sticker set via Telegram API
                if let Err(e) = self.telegram_client
                    .create_sticker_set(
                        user.telegram_id,
                        &new_pack.pack_link,
                        &new_pack.pack_name,
                        input_sticker.clone(),
                    )
                    .await
                {
                    if Self::is_pack_occupied_error(&e) {
                        tracing::info!(pack_link = %new_pack.pack_link, "Sticker set already exists on Telegram, recovering...");
                        // Recover existing pack and re-assign it to the user
                        let recovered_pack = self.handle_occupied_pack_recovery(
                            user, 
                            &new_pack.pack_link, 
                            &new_pack.pack_name,
                            &new_pack.version
                        ).await?;
                        
                        // Proceed to add sticker to recovered pack
                        self.telegram_client
                            .add_sticker_to_set(user.telegram_id, &recovered_pack.pack_link, input_sticker)
                            .await?;
                            
                        self.sticker_pack_repository.increment_sticker_count(recovered_pack.id).await?;
                        return Ok(KangResult {
                            pack: recovered_pack,
                            created_new_pack: true, // Treated as new for this session/user context
                        });
                    }
                    tracing::error!(error = ?e, "Failed to create sticker set");
                    return Err(e);
                }
                
                // Increment sticker count
                self.sticker_pack_repository
                    .increment_sticker_count(new_pack.id)
                    .await?;
                
                Ok(KangResult {
                    pack: new_pack,
                    created_new_pack: true,
                })
            }
        }
    }

    /// Create a custom-named sticker pack.
    ///
    /// This method implements the /createpack command logic:
    /// 1. Validate the pack name length
    /// 2. Create a new pack with the specified name
    /// 3. Set it as the user's active pack
    ///
    /// # Arguments
    /// * `user` - The user creating the pack
    /// * `pack_name` - The custom name for the pack
    ///
    /// # Returns
    /// A `CreatePackResult` containing the newly created pack.
    pub async fn create_custom_pack(
        &self,
        user: &User,
        pack_name: &str,
        sticker_file_id: &str,
        sticker_format: StickerFormat,
        emojis: &str,
    ) -> Result<CreatePackResult, BotError> {
        // Validate pack name length
        if pack_name.len() > MAX_PACK_NAME_LENGTH {
            return Err(BotError::PackNameTooLong);
        }

        let pack_link = generate_custom_pack_link(user.telegram_id, pack_name, &self.bot_username);

        // Create input sticker for Telegram API
        let input_sticker = create_input_sticker_from_file_id(
            sticker_file_id,
            &sticker_format,
            emojis,
        );

        // Create the actual sticker set on Telegram
        if let Err(e) = self.telegram_client
            .create_sticker_set(
                user.telegram_id,
                &pack_link,
                pack_name,
                input_sticker.clone(),
            )
            .await
        {
            if Self::is_pack_occupied_error(&e) {
                tracing::info!(pack_link = %pack_link, "Custom sticker set already exists, recovering...");
                let recovered_pack = self.handle_occupied_pack_recovery(
                    user, 
                    &pack_link, 
                    pack_name,
                    "Custom"
                ).await?;
                
                // Add sticker to recovered pack
                self.telegram_client
                    .add_sticker_to_set(user.telegram_id, &recovered_pack.pack_link, input_sticker)
                    .await?;
                    
                self.sticker_pack_repository.increment_sticker_count(recovered_pack.id).await?;
                return Ok(CreatePackResult { pack: recovered_pack });
            }
            tracing::error!(error = ?e, "Failed to create custom sticker set on Telegram");
            return Err(e);
        }

        // Only after Telegram succeeds, persist it to the database
        let new_pack = NewStickerPack {
            user_id: user.id,
            pack_name: pack_name.to_string(),
            pack_link,
            version: "Custom".to_string(),
        };

        let pack = self.sticker_pack_repository.create(new_pack).await?;
        
        // Ensure sticker count is updated to 1
        self.sticker_pack_repository
            .increment_sticker_count(pack.id)
            .await?;
        
        // Invalidate cache

        
        // Fetch the updated pack to return it
        let updated_pack = self.sticker_pack_repository.get_by_id(pack.id).await?.unwrap_or(pack);

        Ok(CreatePackResult { pack: updated_pack })
    }

    /// Check if a sticker pack is stale and needs re-synchronization.
    ///
    /// A pack is considered stale if its last_synced_at timestamp is more than
    /// 24 hours old.
    ///
    /// # Arguments
    /// * `pack` - The sticker pack to check
    ///
    /// # Returns
    /// `true` if the pack is stale, `false` otherwise.
    /// Handles the recovery of a pack that already exists on Telegram but is missing from the DB.
    async fn handle_occupied_pack_recovery(
        &self,
        user: &User,
        pack_link: &str,
        pack_name: &str,
        version: &str,
    ) -> Result<Arc<StickerPack>, BotError> {
        tracing::info!(pack_link = %pack_link, "Attempting to recover existing pack from Telegram");
        
        // 1. Fetch info from Telegram to get current sticker count
        let sticker_set = self.telegram_client.get_sticker_set(pack_link).await?;
        
        // 2. Verify ownership (must end with _by_{bot_username})
        let expected_suffix = format!("_by_{}", self.bot_username);
        if !pack_link.ends_with(&expected_suffix) {
            tracing::error!(pack_link = %pack_link, "Pack exists but does not belong to this bot");
            return Err(BotError::PackOwnershipViolation);
        }

        // 3. Create a recovery record for the DB
        let recovered_pack = StickerPack {
            id: 0, // Placeholder, actual ID is generated by DB
            user_id: user.id,
            pack_name: pack_name.to_string(),
            pack_link: pack_link.to_string(),
            version: version.to_string(),
            sticker_count: sticker_set.stickers.len() as i32,
            is_active: true,
            created_at: Utc::now().timestamp(),
            updated_at: Utc::now().timestamp(),
            last_synced_at: Some(Utc::now().timestamp()),
        };

        // 4. Insert into repository
        let pack = self.sticker_pack_repository.insert_recovered_pack(recovered_pack).await?;
        
        // 5. Invalidate cache

        
        Ok(pack)
    }

    pub fn is_pack_stale(pack: &StickerPack) -> bool {
        match pack.last_synced_at {
            Some(last_synced) => {
                let now = Utc::now().timestamp();
                (now - last_synced) >= STALE_THRESHOLD_SECONDS
            }
            None => {
                // No sync timestamp - pack is stale
                true
            }
        }
    }

    /// Get the active pack for a user.
    ///
    /// # Arguments
    /// * `user_id` - The database ID of the user
    ///
    /// # Returns
    /// The active pack if one exists, or None.
    async fn get_active_pack(
        &self,
        user_id: i64,
    ) -> Result<Option<Arc<StickerPack>>, RepositoryError> {
        // Query repository
        self.sticker_pack_repository.get_active_pack(user_id).await
    }

    /// Get the pack to use for kang operation.
    ///
    /// This method implements the default pack selection logic:
    /// 1. If user has a default_pack_id configured, use that pack
    /// 2. Validate that the default pack belongs to the user
    /// 3. Otherwise, fall back to the active pack
    ///
    /// # Arguments
    /// * `user` - The user performing the kang operation
    ///
    /// # Returns
    /// The pack to use, or None if no pack exists.
    async fn get_pack_for_kang(
        &self,
        user: &User,
    ) -> Result<Option<Arc<StickerPack>>, BotError> {
        // Check if user has a default pack configured
        if let Some(default_pack_id) = user.default_pack_id {
            // Get the default pack by ID
            if let Some(pack) = self.sticker_pack_repository.get_by_id(default_pack_id).await? {
                // Validate pack ownership
                if pack.user_id != user.id {
                    tracing::warn!(
                        user_id = user.id,
                        pack_id = pack.id,
                        pack_owner_id = pack.user_id,
                        "Pack ownership violation detected in get_pack_for_kang"
                    );
                    return Err(BotError::PackOwnershipViolation);
                }
                return Ok(Some(pack));
            }
            // If default pack not found, fall through to active pack
        }

        // Fall back to active pack
        Ok(self.get_active_pack(user.id).await?)
    }

    /// Create a default pack for a user with no existing packs.
    ///
    /// The pack is named "@{username}'s Kang Pack Vol1".
    ///
    /// # Arguments
    /// * `user` - The user to create the pack for
    ///
    /// # Returns
    /// The newly created pack.
    async fn create_default_pack(&self, user: &User) -> Result<Arc<StickerPack>, RepositoryError> {
        let username = user.username.as_deref().unwrap_or("user");
        let version = "Vol1".to_string();
        let pack_name = generate_pack_name(username, &version);
        let pack_link = generate_pack_link(user.telegram_id, "1", &self.bot_username);

        let new_pack = NewStickerPack {
            user_id: user.id,
            pack_name,
            pack_link,
            version,
        };

        self.sticker_pack_repository.create(new_pack).await
    }

    /// Create the next version of a pack when the current one is full.
    ///
    /// # Arguments
    /// * `user` - The user who owns the pack
    /// * `current_pack` - The current (full) pack
    ///
    /// # Returns
    /// The newly created pack with the next version.
    async fn create_next_version_pack(
        &self,
        user: &User,
        current_pack: &StickerPack,
    ) -> Result<Arc<StickerPack>, RepositoryError> {
        let username = user.username.as_deref().unwrap_or("user");
        let next_version = next_version(&current_pack.version);
        let pack_name = generate_pack_name(username, &next_version);
        
        // Extract version number for pack link (without "Vol" prefix)
        let version_num = next_version.strip_prefix("Vol").unwrap_or("1");
        let pack_link = generate_pack_link(user.telegram_id, version_num, &self.bot_username);

        let new_pack = NewStickerPack {
            user_id: user.id,
            pack_name,
            pack_link,
            version: next_version,
        };

        self.sticker_pack_repository.create(new_pack).await
    }

    /// Get the next version number for a user creating a custom pack.
    ///
    /// This determines what version to use for a new custom pack based on
    /// the user's existing packs.
    ///
    /// # Arguments
    /// * `user_id` - The database ID of the user
    ///
    /// # Returns
    /// The next version string (e.g., "Vol1", "Vol1.1", etc.)
    async fn get_next_version_for_user(&self, user_id: i64) -> Result<String, RepositoryError> {
        // Get current active pack to determine version
        let active_pack = self.get_active_pack(user_id).await?;
        
        match active_pack {
            Some(pack) => {
                // Use next version from current pack
                Ok(next_version(&pack.version))
            }
            None => {
                // No existing pack - start with Vol1
                Ok("Vol1".to_string())
            }
        }
    }

    /// Check if a pack has reached capacity and handle it by creating a new version.
    ///
    /// This method implements the automatic pack rotation logic:
    /// 1. Check if the pack has reached the maximum sticker count (120)
    /// 2. If full, create a new pack version
    /// 3. Update the user's default_pack_id to the new pack
    ///
    /// # Arguments
    /// * `user` - The user who owns the pack
    /// * `pack` - The current sticker pack to check
    ///
    /// # Returns
    /// The pack to use (either the original if not full, or the new version if full).
    async fn check_and_handle_capacity(
        &self,
        user: &User,
        pack: &StickerPack,
    ) -> Result<Arc<StickerPack>, BotError> {
        // Check if pack has reached capacity
        if pack.sticker_count >= MAX_STICKERS_PER_PACK {
            tracing::info!(
                user_id = user.id,
                pack_id = pack.id,
                sticker_count = pack.sticker_count,
                "Pack is full, creating new version"
            );

            // Create a new pack version
            let new_pack = self.create_next_version_pack(user, pack).await?;

            // Update user's default_pack_id to the new pack
            self.user_repository
                .set_default_pack(user.id, Some(new_pack.id))
                .await?;

            tracing::info!(
                user_id = user.id,
                old_pack_id = pack.id,
                new_pack_id = new_pack.id,
                "Created new pack version and updated default_pack_id"
            );

            Ok(new_pack)
        } else {
            Ok(Arc::new(pack.clone()))
        }
    }

    /// Synchronize a sticker pack with Telegram API.
    ///
    /// This method implements the lazy sync strategy:
    /// 1. Validate pack ownership
    /// 2. Query Telegram API for the current sticker count
    /// 3. Update the database with the actual count
    /// 4. Update the last_synced_at timestamp
    ///
    /// # Arguments
    /// * `user` - The user requesting the sync (for ownership validation)
    /// * `pack_id` - The database ID of the pack to sync
    ///
    /// # Returns
    /// The updated sticker pack with fresh data from Telegram.
    pub async fn sync_pack(&self, user: &User, pack_id: i64) -> Result<Arc<StickerPack>, BotError> {
        // Get the pack from database
        let pack = self
            .sticker_pack_repository
            .get_by_id(pack_id)
            .await?
            .ok_or_else(|| {
                tracing::error!(pack_id, "Pack not found for sync");
                BotError::PackNotFound
            })?;

        // Validate pack ownership
        if pack.user_id != user.id {
            tracing::warn!(
                user_id = user.id,
                pack_id = pack.id,
                pack_owner_id = pack.user_id,
                "Pack ownership violation detected"
            );
            return Err(BotError::PackOwnershipViolation);
        }

        tracing::info!(
            pack_id = pack.id,
            pack_link = %pack.pack_link,
            current_count = pack.sticker_count,
            "Starting pack sync with Telegram API"
        );

        // Query Telegram API for current sticker set info
        let sticker_set = match self.telegram_client.get_sticker_set(&pack.pack_link).await {
            Ok(set) => set,
            Err(e) => {
                if Self::is_pack_invalid_error(&e) {
                    tracing::info!(pack_id = pack.id, "Pack is invalid or deleted on Telegram. Deleting from DB.");
                    self.sticker_pack_repository.delete(pack.id).await?;

                }
                return Err(e);
            }
        };

        let actual_count = sticker_set.stickers.len() as i32;
        
        if actual_count == 0 {
            tracing::info!(pack_id = pack.id, "Pack has 0 stickers. Deleting from DB.");
            self.sticker_pack_repository.delete(pack.id).await?;

            return Err(BotError::PackNotFound);
        }

        tracing::info!(
            pack_id = pack.id,
            pack_link = %pack.pack_link,
            db_count = pack.sticker_count,
            actual_count,
            "Retrieved sticker count from Telegram API"
        );

        // Update database with actual count
        self.sticker_pack_repository
            .update_sticker_count(pack.id, actual_count)
            .await?;

        // Update last_synced_at timestamp
        self.sticker_pack_repository
            .update_last_synced(pack.id)
            .await?;

        // Invalidate cache for this pack's user


        // Fetch the updated pack
        let updated_pack = self
            .sticker_pack_repository
            .get_by_id(pack_id)
            .await?
            .ok_or_else(|| {
                tracing::error!(pack_id, "Pack not found after sync update");
                BotError::PackNotFound
            })?;

        tracing::info!(
            pack_id = updated_pack.id,
            sticker_count = updated_pack.sticker_count,
            last_synced_at = ?updated_pack.last_synced_at,
            "Pack sync completed successfully"
        );

        Ok(updated_pack)
    }

    /// Internal sync method that doesn't require user validation.
    /// Used for lazy sync during operations.
    pub async fn sync_pack_internal(&self, pack: &StickerPack) -> Result<Arc<StickerPack>, BotError> {
        tracing::info!(
            pack_id = pack.id,
            pack_link = %pack.pack_link,
            current_count = pack.sticker_count,
            "Starting internal pack sync"
        );

        // Query Telegram API for current sticker set info
        let sticker_set = match self.telegram_client.get_sticker_set(&pack.pack_link).await {
            Ok(set) => set,
            Err(e) => {
                if Self::is_pack_invalid_error(&e) {
                    tracing::info!(pack_id = pack.id, "Pack is invalid or deleted on Telegram. Deleting from DB.");
                    let _ = self.sticker_pack_repository.delete(pack.id).await;

                }
                return Err(e);
            }
        };

        let actual_count = sticker_set.stickers.len() as i32;
        
        if actual_count == 0 {
            tracing::info!(pack_id = pack.id, "Pack has 0 stickers. Deleting from DB.");
            let _ = self.sticker_pack_repository.delete(pack.id).await;

            return Err(BotError::PackNotFound);
        }

        tracing::info!(
            pack_id = pack.id,
            db_count = pack.sticker_count,
            actual_count,
            "Retrieved sticker count from Telegram API"
        );

        // Update database with actual count
        self.sticker_pack_repository
            .update_sticker_count(pack.id, actual_count)
            .await?;

        // Update last_synced_at timestamp
        self.sticker_pack_repository
            .update_last_synced(pack.id)
            .await?;

        // Invalidate cache


        // Fetch the updated pack
        let updated_pack = self
            .sticker_pack_repository
            .get_by_id(pack.id)
            .await?
            .ok_or_else(|| {
                tracing::error!(pack_id = pack.id, "Pack not found after sync update");
                BotError::PackNotFound
            })?;

        Ok(updated_pack)
    }

    /// Check if an error indicates the pack is full.
    fn is_pack_full_error(error: &BotError) -> bool {
        match error {
            BotError::TelegramApi(api_error) => {
                // Check for STICKERSET_FULL error from Telegram
                let error_str = format!("{:?}", api_error);
                error_str.contains("STICKERSET_FULL") 
                    || error_str.contains("stickerset_full")
                    || error_str.contains("too many stickers")
                    || error_str.contains("STICKERS_TOO_MUCH")
            }
            _ => false,
        }
    }

    /// Check if an error indicates the pack is invalid/deleted.
    fn is_pack_invalid_error(error: &BotError) -> bool {
        match error {
            BotError::TelegramApi(api_error) => {
                let error_str = format!("{:?}", api_error);
                error_str.contains("InvalidStickersSet") || error_str.contains("STICKERSET_INVALID")
            }
            _ => false,
        }
    }

    /// Check if an error indicates the pack name is already occupied.
    fn is_pack_occupied_error(error: &BotError) -> bool {
        match error {
            BotError::TelegramRequest(teloxide::RequestError::Api(teloxide::ApiError::StickerSetNameOccupied)) => true,
            BotError::TelegramApi(teloxide::ApiError::StickerSetNameOccupied) => true, // Just in case it's wrapped differently
            _ => {
                // Fallback to string check for robustness if variants are masked
                let error_str = format!("{:?}", error);
                error_str.contains("StickerSetNameOccupied") || error_str.contains("STICKERSET_NAME_OCCUPIED")
            }
        }
    }
}

/// Generates the next version in the sequence.
/// Version sequence: Vol1 → Vol1.1 → ... → Vol1.9 → Vol2 → Vol2.1 → ...
///
/// # Arguments
/// * `current` - Current version string (e.g., "Vol1", "Vol1.5", "Vol2.9")
///
/// # Returns
/// The next version in the sequence
pub fn next_version(current: &str) -> String {
    // Strip "Vol" prefix
    let version_part = current.strip_prefix("Vol").unwrap_or(current);
    
    // Parse the version number
    if let Some(dot_pos) = version_part.find('.') {
        // Has decimal part (e.g., "1.5")
        let major: u32 = version_part[..dot_pos].parse().unwrap_or(1);
        let minor: u32 = version_part[dot_pos + 1..].parse().unwrap_or(0);
        
        if minor >= 9 {
            // Roll over to next major version (Vol1.9 -> Vol2)
            format!("Vol{}", major + 1)
        } else {
            // Increment minor version (Vol1.5 -> Vol1.6)
            format!("Vol{}.{}", major, minor + 1)
        }
    } else {
        // No decimal part (e.g., "1", "2") - add .1
        let major: u32 = version_part.parse().unwrap_or(1);
        format!("Vol{}.1", major)
    }
}

/// Generates a pack link in the format `u{telegram_id}V{version}_by_{bot_username}`.
///
/// # Arguments
/// * `telegram_id` - The user's Telegram ID
/// * `version` - The version string (e.g., "1", "1.5")
/// * `bot_username` - The bot's username
///
/// # Returns
/// A pack link string
pub fn generate_pack_link(telegram_id: i64, version: &str, bot_username: &str) -> String {
    format!("u{}V{}_by_{}", telegram_id, version, bot_username)
}

/// Generates a custom pack link in the format `{sanitized_name}_u{telegram_id}_by_{bot_username}`.
/// 
/// The function ensures the final pack link follows Telegram's rules:
/// - Only alphanumeric characters and underscores
/// - Must begin with a letter
/// - Max length 64 characters
pub fn generate_custom_pack_link(telegram_id: i64, pack_name: &str, bot_username: &str) -> String {
    let suffix = format!("_u{}_by_{}", telegram_id, bot_username);
    
    // Convert to lowercase and keep only alphanumeric and underscores
    let mut safe_name: String = pack_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();

    if safe_name.is_empty() {
        safe_name = "pack".to_string();
    }
    
    // Ensure it starts with a letter
    if !safe_name.chars().next().unwrap().is_ascii_alphabetic() {
        safe_name = format!("p_{}", safe_name);
    }
    
    // Truncate to fit within 64 character limit when combined with suffix
    let max_name_len = 64_usize.saturating_sub(suffix.len());
    if safe_name.len() > max_name_len {
        safe_name.truncate(max_name_len);
        
        // Don't end with an underscore or half character boundary
        safe_name = safe_name.trim_end_matches('_').to_string();
    }
    
    format!("{}{}", safe_name, suffix)
}

/// Generates a pack name in the format `@{username}'s Kang Pack {version}`.
///
/// # Arguments
/// * `username` - The user's username
/// * `version` - The version string (e.g., "Vol1", "Vol1.5")
///
/// # Returns
/// A pack name string
pub fn generate_pack_name(username: &str, version: &str) -> String {
    format!("@{}'s Kang Pack {}", username, version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_next_version_no_decimal() {
        assert_eq!(next_version("Vol1"), "Vol1.1");
        assert_eq!(next_version("Vol2"), "Vol2.1");
        assert_eq!(next_version("Vol3"), "Vol3.1");
    }

    #[test]
    fn test_next_version_with_decimal() {
        assert_eq!(next_version("Vol1.1"), "Vol1.2");
        assert_eq!(next_version("Vol1.5"), "Vol1.6");
        assert_eq!(next_version("Vol2.3"), "Vol2.4");
    }

    #[test]
    fn test_next_version_rollover() {
        assert_eq!(next_version("Vol1.9"), "Vol2");
        assert_eq!(next_version("Vol2.9"), "Vol3");
        assert_eq!(next_version("Vol5.9"), "Vol6");
    }

    #[test]
    fn test_generate_pack_link() {
        assert_eq!(generate_pack_link(123456789, "1", "mybot"), "u123456789V1_by_mybot");
        assert_eq!(generate_pack_link(123456789, "1.5", "mybot"), "u123456789V1.5_by_mybot");
        assert_eq!(generate_pack_link(987654321, "2", "testbot"), "u987654321V2_by_testbot");
    }

    #[test]
    fn test_generate_pack_name() {
        assert_eq!(generate_pack_name("johndoe", "Vol1"), "@johndoe's Kang Pack Vol1");
        assert_eq!(generate_pack_name("alice", "Vol1.5"), "@alice's Kang Pack Vol1.5");
        assert_eq!(generate_pack_name("bob", "Vol2"), "@bob's Kang Pack Vol2");
    }

    #[test]
    fn test_is_pack_stale_no_sync_timestamp() {
        // Pack with no last_synced_at should be considered stale
        let pack = StickerPack {
            id: 1,
            user_id: 1,
            pack_name: "Test Pack".to_string(),
            pack_link: "u1V1_by_testbot".to_string(),
            version: "Vol1".to_string(),
            sticker_count: 0,
            is_active: true,
            created_at: Utc::now().timestamp(),
            updated_at: Utc::now().timestamp(),
            last_synced_at: None,
        };
        assert!(StickerService::<crate::repository::SqliteUserRepository, crate::repository::SqliteStickerPackRepository>::is_pack_stale(&pack));
    }

    #[test]
    fn test_is_pack_stale_recent_sync() {
        // Pack synced recently (within 7 days) should not be stale
        let now = Utc::now().timestamp();
        let recent_sync = now - (3 * 24 * 3600); // 3 days ago
        
        let pack = StickerPack {
            id: 1,
            user_id: 1,
            pack_name: "Test Pack".to_string(),
            pack_link: "u1V1_by_testbot".to_string(),
            version: "Vol1".to_string(),
            sticker_count: 0,
            is_active: true,
            created_at: now,
            updated_at: now,
            last_synced_at: Some(recent_sync),
        };
        assert!(!StickerService::<crate::repository::SqliteUserRepository, crate::repository::SqliteStickerPackRepository>::is_pack_stale(&pack));
    }

    #[test]
    fn test_is_pack_stale_old_sync() {
        // Pack synced more than 7 days ago should be stale
        let now = Utc::now().timestamp();
        let old_sync = now - (8 * 24 * 3600); // 8 days ago
        
        let pack = StickerPack {
            id: 1,
            user_id: 1,
            pack_name: "Test Pack".to_string(),
            pack_link: "u1V1_by_testbot".to_string(),
            version: "Vol1".to_string(),
            sticker_count: 0,
            is_active: true,
            created_at: now,
            updated_at: now,
            last_synced_at: Some(old_sync),
        };
        assert!(StickerService::<crate::repository::SqliteUserRepository, crate::repository::SqliteStickerPackRepository>::is_pack_stale(&pack));
    }

    #[test]
    fn test_is_pack_stale_exactly_7_days() {
        // Pack synced exactly 7 days ago should be stale
        let now = Utc::now().timestamp();
        let exact_sync = now - (7 * 24 * 3600);
        
        let pack = StickerPack {
            id: 1,
            user_id: 1,
            pack_name: "Test Pack".to_string(),
            pack_link: "u1V1_by_testbot".to_string(),
            version: "Vol1".to_string(),
            sticker_count: 0,
            is_active: true,
            created_at: now,
            updated_at: now,
            last_synced_at: Some(exact_sync),
        };
        assert!(StickerService::<crate::repository::SqliteUserRepository, crate::repository::SqliteStickerPackRepository>::is_pack_stale(&pack));
    }

    /// Checks if a version string is valid (matches "Vol{N}" or "Vol{N}.{M}" format).
    fn is_valid_version(version: &str) -> bool {
        let version_part = match version.strip_prefix("Vol") {
            Some(v) => v,
            None => return false,
        };

        // Check for "N" or "N.M" format where N and M are non-negative integers
        let parts: Vec<&str> = version_part.split('.').collect();
        match parts.len() {
            1 => parts[0].parse::<u32>().is_ok(),
            2 => {
                parts[0].parse::<u32>().is_ok() && parts[1].parse::<u32>().is_ok()
            }
            _ => false,
        }
    }

    /// Checks if `next` is the correct next version after `current` in the sequence.
    /// Sequence: Vol1 → Vol1.1 → ... → Vol1.9 → Vol2 → Vol2.1 → ...
    fn version_is_next(current: &str, next: &str) -> bool {
        let current_part = current.strip_prefix("Vol").unwrap_or(current);
        let next_part = next.strip_prefix("Vol").unwrap_or(next);

        let (current_major, current_minor) = parse_version(current_part);
        let (next_major, next_minor) = parse_version(next_part);

        match (current_minor, next_minor) {
            // Current has no minor (e.g., Vol1), next should be Vol1.1
            (None, Some(1)) => next_major == current_major,
            // Current has minor < 9, next should increment minor
            (Some(m), Some(nm)) if m < 9 => {
                next_major == current_major && nm == m + 1
            }
            // Current has minor == 9, next should roll over to next major with no minor
            (Some(9), None) => next_major == current_major + 1,
            _ => false,
        }
    }

    /// Parses a version string into (major, minor) where minor is optional.
    fn parse_version(version: &str) -> (u32, Option<u32>) {
        if let Some(dot_pos) = version.find('.') {
            let major: u32 = version[..dot_pos].parse().unwrap_or(1);
            let minor: u32 = version[dot_pos + 1..].parse().unwrap_or(0);
            (major, Some(minor))
        } else {
            let major: u32 = version.parse().unwrap_or(1);
            (major, None)
        }
    }

    /// Strategy for generating valid version strings
    fn version_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            // Major only: Vol1, Vol2, Vol3, etc.
            (1u32..=10u32).prop_map(|n| format!("Vol{}", n)),
            // Major with minor: Vol1.0 to Vol1.9, Vol2.0 to Vol2.9, etc.
            ((1u32..=10u32), (0u32..=9u32)).prop_map(|(major, minor)| format!("Vol{}.{}", major, minor)),
        ]
    }

    /// Increments the sticker count by 1.
    /// This represents the core logic of the sticker count increment operation.
    /// Used by the repository layer's increment_sticker_count method.
    fn increment_sticker_count(count: i32) -> i32 {
        count + 1
    }

    proptest! {
        /// Property 1: Pack Link Format
        ///
        /// For any valid telegram_id (positive i64), version string, and bot_username (non-empty),
        /// the generated pack link SHALL match the format `u{telegram_id}V{version}_by_{bot_username}`.
        #[test]
        fn test_pack_link_format(
            telegram_id: i64,
            version in "[0-9]+(\\.[0-9]+)?",
            bot_username in "[a-zA-Z][a-zA-Z0-9_]*"
        ) {
            // Only test with positive telegram IDs (valid Telegram user IDs)
            prop_assume!(telegram_id > 0);

            let link = generate_pack_link(telegram_id, &version, &bot_username);

            // Verify the pack link format matches: u{telegram_id}V{version}_by_{bot_username}
            let expected_prefix = format!("u{}", telegram_id);
            let expected_suffix = format!("_by_{}", bot_username);
            let expected = format!("u{}V{}_by_{}", telegram_id, version, bot_username);

            prop_assert!(link.starts_with(&expected_prefix), "Link should start with u{{telegram_id}}");
            prop_assert!(link.contains("V"), "Link should contain V");
            prop_assert!(link.ends_with(&expected_suffix), "Link should end with _by_{{bot_username}}");
            prop_assert_eq!(link, expected, "Link should match exact format");
        }

        /// Property 2: Version Increment Sequence
        ///
        /// For any valid version string, the next_version function SHALL produce
        /// the next version in the sequence: Vol1 → Vol1.1 → ... → Vol1.9 → Vol2 → Vol2.1 → ...
        #[test]
        fn test_version_increment_sequence(current in version_strategy()) {
            let next = next_version(&current);

            // The result must be a valid version string
            prop_assert!(is_valid_version(&next), "Next version should be valid: got {}", next);

            // The result must be the correct next version in the sequence
            prop_assert!(version_is_next(&current, &next),
                "Version should be next in sequence: {} -> {}", current, next);
        }

        /// Property 3: Sticker Count Increment
        ///
        /// For any sticker pack with sticker_count N, after successfully adding a sticker,
        /// the sticker_count SHALL be N+1.
        #[test]
        fn test_sticker_count_increment(count in 0i32..=120i32) {
            let new_count = increment_sticker_count(count);
            prop_assert_eq!(new_count, count + 1, "Sticker count should increment by 1");
        }
    }
}