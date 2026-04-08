//! Bot-specific error types

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BotError {
    #[error("Pack name exceeds character limit (max 64 characters)")]
    PackNameTooLong,

    #[error("Sticker pack not found.")]
    PackNotFound,

    #[error("That pack doesn't belong to you.")]
    PackOwnershipViolation,

    #[error("Telegram API error: {0}")]
    TelegramApi(#[from] teloxide::ApiError),

    #[error("Telegram request error: {0}")]
    TelegramRequest(#[from] teloxide::RequestError),

    #[error("Database error occurred. Please try again later.")]
    DatabaseError(#[from] crate::repository::RepositoryError),
}