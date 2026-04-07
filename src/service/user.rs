//! User management logic
//!
//! This module provides the `UserService` which handles user-related business logic
//! including user creation, retrieval, and caching.

use std::sync::Arc;

use crate::db::schema::{NewUser, User};
use crate::repository::{RepositoryError, UserRepository};

/// Service for user-related operations.
pub struct UserService<R>
where
    R: UserRepository,
{
    repository: Arc<R>,
    cache: Arc<crate::cache::CacheLayer<i64, User>>,
}

impl<R> UserService<R>
where
    R: UserRepository,
{
    /// Create a new UserService with the given repository and cache.
    pub fn new(repository: Arc<R>, cache: Arc<crate::cache::CacheLayer<i64, User>>) -> Self {
        Self { repository, cache }
    }

    /// Get or create a user by their Telegram ID.
    pub async fn get_or_create(
        &self,
        telegram_id: i64,
        username: Option<String>,
    ) -> Result<Arc<User>, RepositoryError> {
        // Check cache first
        if let Some(user) = self.cache.get(&telegram_id).await {
            return Ok(user);
        }

        // Check repository
        if let Some(user) = self.repository.get_by_telegram_id(telegram_id).await? {
            self.cache.insert(telegram_id, user.clone()).await;
            return Ok(user);
        }

        // Create new user
        let new_user = NewUser {
            telegram_id,
            username,
        };
        let user = self.repository.create(new_user).await?;
        self.cache.insert(telegram_id, user.clone()).await;

        Ok(user)
    }

    /// Get a user by their Telegram ID.
    pub async fn get_by_telegram_id(
        &self,
        telegram_id: i64,
    ) -> Result<Option<Arc<User>>, RepositoryError> {
        // Check cache first
        if let Some(user) = self.cache.get(&telegram_id).await {
            return Ok(Some(user));
        }

        // Query repository
        let user = self.repository.get_by_telegram_id(telegram_id).await?;

        // Cache if found
        if let Some(ref u) = user {
            self.cache.insert(telegram_id, u.clone()).await;
        }

        Ok(user)
    }

    /// Update a user.
    pub async fn update(&self, user: &User) -> Result<(), RepositoryError> {
        self.repository.update(user).await?;
        self.cache.invalidate(&user.telegram_id).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests would go here
    // These would require mock implementations of UserRepository and Cache
}