//! Repository layer - Data access traits and implementations

use thiserror::Error;

pub mod sticker_pack;
pub mod user;

pub use sticker_pack::{SqliteStickerPackRepository, StickerPackRepository};
pub use user::{SqliteUserRepository, UserRepository};

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("Database connection error: {0}")]
    ConnectionError(#[from] sqlx::Error),

    #[error("Entity not found")]
    NotFound,

    #[error("Unique constraint violation")]
    DuplicateEntry,
}