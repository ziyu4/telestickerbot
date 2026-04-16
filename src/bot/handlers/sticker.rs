use std::sync::Arc;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{Message, ReplyParameters, InputFile, StickerFormat, ReactionType};
use teloxide::Bot;
use tempfile::NamedTempFile;

use crate::bot::error::BotError;
use crate::repository::{StickerPackRepository, UserRepository};
use crate::service::{StickerService, UserService};

/// Handle the `/s` or `/sticker` command.
pub async fn handle_sticker<U, S>(
    bot: teloxide::adaptors::Throttle<Bot>,
    msg: Message,
    emoji_arg: String,
    user_service: Arc<UserService<U>>,
    sticker_service: Arc<StickerService<U, S>>,
) -> Result<(), BotError>
where
    U: UserRepository,
    S: StickerPackRepository,
{
    // 1. Identify target media
    let (file_id, is_video, error_message) = if let Some(reply) = msg.reply_to_message() {
        if let Some(video) = reply.video() {
            if video.duration.seconds() > 3 {
                (None, true, Some("Video is too long! Maximum duration for a sticker is 3 seconds."))
            } else {
                (Some(video.file.id.clone()), true, None)
            }
        } else if let Some(photo) = reply.photo().and_then(|p| p.last()) {
            (Some(photo.file.id.clone()), false, None)
        } else if let Some(doc) = reply.document() {
            if let Some(mime) = &doc.mime_type {
                if mime.type_() == "video" {
                    if doc.file.size > 500 * 1024 {
                        (None, true, Some("Sorry, video documents are limited to 500KB for server safety."))
                    } else {
                        (Some(doc.file.id.clone()), true, None)
                    }
                } else if mime.type_() == "image" {
                    (Some(doc.file.id.clone()), false, None)
                } else {
                    (None, false, Some("Document format not supported. Please reply to a Photo or Video."))
                }
            } else {
                (None, false, Some("Unknown document format."))
            }
        } else {
            (None, false, Some("Please reply to a Photo or Video message (Max 3 seconds) with /s."))
        }
    } else {
        // Try getting from the message itself
        if let Some(video) = msg.video() {
            if video.duration.seconds() > 3 {
                (None, true, Some("Video is too long! Maximum duration for a sticker is 3 seconds."))
            } else {
                (Some(video.file.id.clone()), true, None)
            }
        } else if let Some(photo) = msg.photo().and_then(|p| p.last()) {
            (Some(photo.file.id.clone()), false, None)
        } else if let Some(doc) = msg.document() {
            if let Some(mime) = &doc.mime_type {
                if mime.type_() == "video" {
                    if doc.file.size > 500 * 1024 {
                        (None, true, Some("Sorry, video documents are limited to 500KB for server safety."))
                    } else {
                        (Some(doc.file.id.clone()), true, None)
                    }
                } else if mime.type_() == "image" {
                    (Some(doc.file.id.clone()), false, None)
                } else {
                    (None, false, Some("Document format not supported. Please reply to a Photo or Video."))
                }
            } else {
                (None, false, Some("Unknown document format."))
            }
        } else {
            (None, false, Some("Please attach or reply to a Photo or Video (Max 3 seconds) to create a sticker."))
        }
    };

    if let Some(err) = error_message {
        bot.send_message(msg.chat.id, err)
            .reply_parameters(ReplyParameters::new(msg.id))
            .await?;
        return Ok(());
    }

    let file_id = match file_id {
        Some(id) => id,
        None => return Ok(()),
    };

    // User authentication/creation
    let telegram_id = match &msg.from {
        Some(user) => user.id.0 as i64,
        None => return Ok(()),
    };
    let username = msg.from.as_ref().and_then(|u| u.username.clone());
    let user = user_service.get_or_create(telegram_id, username).await?;

    let emojis = if emoji_arg.trim().is_empty() {
        crate::emoji::random_emoji()
    } else {
        emoji_arg.trim().to_string()
    };

    let _ = bot.set_message_reaction(msg.chat.id, msg.id)
        .reaction(vec![ReactionType::Emoji { emoji: "👍".to_string() }])
        .await;

    // Download the file from Telegram
    let file = bot.get_file(file_id).await?;
    
    let temp_input = NamedTempFile::new().map_err(|e| BotError::TelegramApi(teloxide::ApiError::Unknown(e.to_string())))?;
    let temp_output = NamedTempFile::new().map_err(|e| BotError::TelegramApi(teloxide::ApiError::Unknown(e.to_string())))?;
    
    // Maintain extension for output using the base path
    let output_path = if is_video {
        let path = temp_output.path().with_extension("webm");
        temp_output.into_temp_path().keep().unwrap(); // We'll manage deletion manually
        path
    } else {
        let path = temp_output.path().with_extension("webp");
        temp_output.into_temp_path().keep().unwrap(); // We'll manage deletion manually
        path
    };

    let input_path = temp_input.path().to_path_buf();
    
    // Execute download
    {
        let mut dst = tokio::fs::File::create(&input_path).await.map_err(|e| BotError::TelegramApi(teloxide::ApiError::Unknown(e.to_string())))?;
        bot.download_file(&file.path, &mut dst).await.map_err(|e| BotError::TelegramApi(teloxide::ApiError::Unknown(e.to_string())))?;
    }

 


    if is_video {
        crate::bot::handlers::transcoder::convert_video_to_webm(&input_path, &output_path)
            .map_err(|e| BotError::TelegramApi(teloxide::ApiError::Unknown(e)))?;
    } else {
        crate::bot::handlers::transcoder::convert_image_to_webp(&input_path, &output_path)
            .map_err(|e| BotError::TelegramApi(teloxide::ApiError::Unknown(e)))?;
    }

 


    // Pass the local file to add_sticker logic
    let format = if is_video { StickerFormat::Video } else { StickerFormat::Static };
    let input_sticker = teloxide::types::InputSticker {
        sticker: InputFile::file(output_path.clone()),
        format,
        emoji_list: vec![emojis],
        mask_position: None,
        keywords: vec![],
    };

    let result = sticker_service.kang_sticker(&user, input_sticker).await;

    // Send final result BEFORE cleanup
    match result {
        Ok(_kang_result) => {
            // Final action: Send the sticker back to the chat
            bot.send_sticker(msg.chat.id, InputFile::file(output_path.clone()))
                .reply_parameters(ReplyParameters::new(msg.id))
                .await?;
        }
        Err(e) => {
            let error_message = match &e {
                BotError::TelegramApi(api_error) => {
                    crate::bot::telegram::parse_api_error(api_error, sticker_service.bot_username())
                }
                BotError::TelegramRequest(req_error) => {
                    if let teloxide::RequestError::Api(api_error) = req_error {
                        crate::bot::telegram::parse_api_error(api_error, sticker_service.bot_username())
                    } else {
                        "A network error occurred. Please try again later.".to_string()
                    }
                }
                BotError::PackNotFound => {
                    "Sticker pack not found. Please create a pack first with /createpack.".to_string()
                }
                BotError::PackOwnershipViolation => {
                    "That sticker pack doesn't belong to you.".to_string()
                }
                _ => "An error occurred. Please try again later.".to_string(),
            };

            bot.send_message(msg.chat.id, error_message)
                .reply_parameters(ReplyParameters::new(msg.id))
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
            return Err(e);
        }
    }

    // Cleanup resources after sending
    let _ = tokio::fs::remove_file(&input_path).await;
    let _ = tokio::fs::remove_file(&output_path).await;

    Ok(())
}
