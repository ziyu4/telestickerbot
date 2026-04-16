//! Telegram API integration for sticker operations
//!
//! This module provides functions for interacting with the Telegram Bot API
//! for sticker pack creation and management.

use teloxide::{
    prelude::*,
    types::{FileId, InputFile, InputSticker, Sticker, StickerFormat, StickerSet},
};
use std::sync::Arc;

use super::error::BotError;
use super::retry::with_retry;

/// Telegram API client for sticker operations.
///
/// This struct wraps the teloxide Bot and provides high-level methods
/// for sticker pack operations with built-in retry logic.
pub struct TelegramClient<B> {
    bot: Arc<B>,
}

impl<B> TelegramClient<B>
where
    B: teloxide::requests::Requester + Clone + Send + Sync + 'static,
    B::Err: Into<BotError>,
{
    /// Create a new TelegramClient with the given bot.
    ///
    /// # Arguments
    /// * `bot` - The teloxide Bot instance (can be wrapped with adaptors like Throttle)
    pub fn new(bot: Arc<B>) -> Self {
        Self { bot }
    }

    /// Create a new sticker set.
    ///
    /// Calls the Telegram `createNewStickerSet` API method.
    ///
    /// # Arguments
    /// * `user_id` - The Telegram user ID of the sticker pack owner
    /// * `name` - The short name of the sticker set (used in URLs)
    /// * `title` - The display title of the sticker set
    /// * `sticker` - The first sticker to add to the set
    ///
    /// # Returns
    /// `Ok(())` on success, or a `BotError` on failure.
    pub async fn create_sticker_set(
        &self,
        user_id: i64,
        name: &str,
        title: &str,
        sticker: InputSticker,
    ) -> Result<(), BotError> {
        let user_id = teloxide::types::UserId(user_id as u64);
        let name = name.to_string();
        let title = title.to_string();

        with_retry(|| {
            let bot = self.bot.clone();
            let user_id = user_id;
            let name = name.clone();
            let title = title.clone();
            let sticker = sticker.clone();

            Box::pin(async move {
                bot.create_new_sticker_set(user_id, &name, &title, std::iter::once(sticker))
                    .await
                    .map(|_| ())
                    .map_err(Into::into)
            })
        })
        .await
    }

    /// Add a sticker to an existing sticker set.
    ///
    /// Calls the Telegram `addStickerToSet` API method.
    ///
    /// # Arguments
    /// * `user_id` - The Telegram user ID of the sticker pack owner
    /// * `name` - The short name of the sticker set
    /// * `sticker` - The sticker to add
    ///
    /// # Returns
    /// `Ok(())` on success, or a `BotError` on failure.
    pub async fn add_sticker_to_set(
        &self,
        user_id: i64,
        name: &str,
        sticker: InputSticker,
    ) -> Result<(), BotError> {
        let user_id = teloxide::types::UserId(user_id as u64);
        let name = name.to_string();

        with_retry(|| {
            let bot = self.bot.clone();
            let user_id = user_id;
            let name = name.clone();
            let sticker = sticker.clone();

            Box::pin(async move {
                bot.add_sticker_to_set(user_id, &name, sticker)
                    .await
                    .map(|_| ())
                    .map_err(Into::into)
            })
        })
        .await
    }

    /// Get information about a sticker set.
    ///
    /// Calls the Telegram `getStickerSet` API method.
    ///
    /// # Arguments
    /// * `name` - The short name of the sticker set
    ///
    /// # Returns
    /// The sticker set information on success, or a `BotError` on failure.
    pub async fn get_sticker_set(&self, name: &str) -> Result<StickerSet, BotError> {
        let name = name.to_string();

        with_retry(|| {
            let bot = self.bot.clone();
            let name = name.clone();

            Box::pin(async move {
                bot.get_sticker_set(&name)
                    .await
                    .map_err(Into::into)
            })
        })
        .await
    }

    /// Get the bot's username.
    ///
    /// # Returns
    /// The bot's username on success, or a `BotError` on failure.
    pub async fn get_bot_username(&self) -> Result<String, BotError> {
        let me = with_retry(|| {
            let bot = self.bot.clone();
            Box::pin(async move { bot.get_me().await.map_err(Into::into) })
        })
        .await?;

        me.username
            .as_ref()
            .map(|u| u.to_string())
            .ok_or_else(|| BotError::TelegramApi(teloxide::ApiError::Unknown(
                "Bot has no username".to_string(),
            )))
    }
}

/// Creates an InputSticker from a sticker file ID.
///
/// # Arguments
/// * `file_id` - The Telegram file ID of the sticker
/// * `format` - The format of the sticker
/// * `emojis` - The emojis associated with the sticker
///
/// # Returns
/// An InputSticker ready to be used in API calls.
pub fn create_input_sticker_from_file_id(
    file_id: &str,
    format: &StickerFormat,
    emojis: &str,
) -> InputSticker {
    InputSticker {
        sticker: InputFile::file_id(FileId(file_id.to_string())),
        format: format.clone(),
        emoji_list: vec![emojis.to_string()],
        mask_position: None,
        keywords: vec![],
    }
}

/// Determines the sticker format from a Sticker object.
///
/// # Arguments
/// * `sticker` - The sticker to analyze
///
/// # Returns
/// The appropriate StickerFormat for the sticker.
pub fn get_sticker_format(sticker: &Sticker) -> StickerFormat {
    // Check if it's an animated sticker (TGS format)
    if sticker.is_animated() {
        return StickerFormat::Animated;
    }

    // Check if it's a video sticker (WEBM format)
    if sticker.is_video() {
        return StickerFormat::Video;
    }

    // Default to static (WEBP format)
    StickerFormat::Static
}

/// Parses a Telegram API error and returns a user-friendly error message.
///
/// # Arguments
/// * `error` - The Telegram API error
/// * `bot_username` - The bot's username to generate deep links
///
/// # Returns
/// A user-friendly error message string.
pub fn parse_api_error(error: &teloxide::ApiError, bot_username: &str) -> String {
    match error {
        teloxide::ApiError::StickerSetNameOccupied => {
            "A sticker pack with this name already exists. Please try again.".to_string()
        }
        teloxide::ApiError::InvalidStickersSet => {
            "The sticker set is invalid or has been deleted.".to_string()
        }
        teloxide::ApiError::InvalidStickerName => {
            "The sticker set name is invalid. Please try again.".to_string()
        }
        teloxide::ApiError::StickerSetOwnerIsBot => {
            "Cannot create sticker set for bots.".to_string()
        }
        teloxide::ApiError::WrongFileId => {
            "Invalid sticker file. Please use a different sticker.".to_string()
        }
        teloxide::ApiError::WrongFileIdOrUrl => {
            "Invalid sticker file or URL. Please try again.".to_string()
        }
        teloxide::ApiError::FailedToGetUrlContent => {
            "Failed to download sticker. Please try again.".to_string()
        }
        teloxide::ApiError::ImageProcessFailed => {
            "Failed to process sticker image. Please use a different sticker.".to_string()
        }
        teloxide::ApiError::Unknown(msg) => {
            // Handle specific error messages from Telegram
            if msg.contains("STICKER_VIDEO_LONG") {
                return "Video is too long! Maximum duration is 3 seconds. Please use a shorter video.".to_string();
            }
            if msg.contains("STICKER_VIDEO_TOO_LARGE") {
                return "Video file is too large! Maximum size is 256KB after conversion.".to_string();
            }
            if msg.contains("STICKER_PNG_DIMENSIONS") {
                return "Sticker dimensions are invalid. Must be 512px on one side.".to_string();
            }
            if msg.contains("STICKER_EMOJI_INVALID") {
                return "Invalid emoji(s) provided. Please use standard emojis.".to_string();
            }
            if msg.contains("STICKER_INVALID_EMOJI") {
                return "Invalid emoji(s) provided. Please use standard emojis.".to_string();
            }
            if msg.contains("STICKER_EMPTY") {
                return "The sticker file is empty or corrupted. Please try a different file.".to_string();
            }
            if msg.contains("STICKER_INVALID") {
                return "Invalid sticker format. Please try a different file.".to_string();
            }
            if msg.contains("PEER_ID_INVALID") {
                return format!(
                    "You need to start me in PM first before creating/adding stickers!\n\n<a href=\"https://t.me/{}?start=1\">👉 Start Bot Here 👈</a>",
                    bot_username
                );
            }
            tracing::error!(error = msg, "Unknown Telegram API error");
            "An unexpected error occurred. Please try again later.".to_string()
        }
        _ => {
            tracing::error!(error = ?error, "Telegram API error");
            "An error occurred while communicating with Telegram. Please try again.".to_string()
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_api_error_sticker_set_name_occupied() {
        let error = teloxide::ApiError::StickerSetNameOccupied;
        let message = parse_api_error(&error, "testbot");
        assert!(message.contains("already exists"));
    }

    #[test]
    fn test_parse_api_error_invalid_stickers_set() {
        let error = teloxide::ApiError::InvalidStickersSet;
        let message = parse_api_error(&error, "testbot");
        assert!(message.contains("invalid or has been deleted"));
    }

    #[test]
    fn test_parse_api_error_unknown() {
        let error = teloxide::ApiError::Unknown("test error".to_string());
        let message = parse_api_error(&error, "testbot");
        assert!(message.contains("unexpected error"));
    }

    #[test]
    fn test_create_input_sticker() {
        let sticker = create_input_sticker_from_file_id("test_file_id", &StickerFormat::Static, "😀");
        assert_eq!(sticker.format, StickerFormat::Static);
        assert_eq!(sticker.emoji_list, vec!["😀"]);
    }

    // Note: register_webhook requires a real Bot instance and network access,
    // so it's tested via integration tests rather than unit tests
}