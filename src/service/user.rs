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
}

impl<R> UserService<R>
where
    R: UserRepository,
{
    /// Create a new UserService with the given repository.
    pub fn new(repository: Arc<R>) -> Self {
        Self { repository }
    }

    /// Get or create a user by their Telegram ID.
    pub async fn get_or_create(
        &self,
        telegram_id: i64,
        username: Option<String>,
    ) -> Result<Arc<User>, RepositoryError> {
        // Check repository
        if let Some(user) = self.repository.get_by_telegram_id(telegram_id).await? {
            return Ok(user);
        }

        // Create new user
        let new_user = NewUser {
            telegram_id,
            username,
        };
        let user = self.repository.create(new_user).await?;

        Ok(user)
    }

    /// Get a user by their Telegram ID.
    pub async fn get_by_telegram_id(
        &self,
        telegram_id: i64,
    ) -> Result<Option<Arc<User>>, RepositoryError> {
        // Query repository
        self.repository.get_by_telegram_id(telegram_id).await
    }

    /// Update a user.
    pub async fn update(&self, user: &User) -> Result<(), RepositoryError> {
        self.repository.update(user).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests would go here
    // These would require mock implementations of UserRepository and Cache
}