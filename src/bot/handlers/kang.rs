use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ReplyParameters;
use teloxide::Bot;

use crate::service::{StickerService, UserService};
use crate::repository::{UserRepository, StickerPackRepository};
use crate::bot::telegram::get_sticker_format;
use crate::bot::error::BotError;
use super::utils::escape_html;

/// Handle the `/kang` command.
///
/// This function is designed for dptree dependency injection and handles
/// adding a sticker to the user's default or active sticker pack.
/// The command must be sent as a reply to a sticker message.
pub async fn handle_kang<U, S>(
    bot: teloxide::adaptors::Throttle<Bot>,
    msg: Message,
    user_service: Arc<UserService<U>>,
    sticker_service: Arc<StickerService<U, S>>,
) -> Result<(), BotError>
where
    U: UserRepository,
    S: StickerPackRepository,
{
    // Get the user's Telegram ID
    let telegram_id = match &msg.from {
        Some(user) => user.id.0 as i64,
        None => return Ok(()), // No user info, ignore
    };

    // Get the username for pack naming
    let username = msg.from.as_ref().and_then(|u| u.username.clone());

    // Validate that the command is a reply to a sticker OR a direct sticker message
    let sticker = match msg.reply_to_message() {
        Some(reply) => {
            match reply.sticker() {
                Some(s) => s.clone(),
                None => {
                    // Check if the message itself contains a sticker (for auto-kang in PM)
                    match msg.sticker() {
                        Some(s) => s.clone(),
                        None => {
                            bot.send_message(
                                msg.chat.id,
                                "Please reply to a sticker with /kang to add it.",
                            )
                            .reply_parameters(ReplyParameters::new(msg.id))
                            .await?;
                            return Ok(());
                        }
                    }
                }
            }
        }
        None => {
            // Check if the message itself contains a sticker (for auto-kang in PM)
            match msg.sticker() {
                Some(s) => s.clone(),
                None => {
                    bot.send_message(
                        msg.chat.id,
                        "Please reply to a sticker with /kang to add it.",
                    )
                    .reply_parameters(ReplyParameters::new(msg.id))
                    .await?;
                    return Ok(());
                }
            }
        }
    };

    // Get or create the user
    let user = user_service
        .get_or_create(telegram_id, username)
        .await?;

    // Determine sticker format (static, animated, or video)
    let sticker_format = get_sticker_format(&sticker);

    // Get emojis from the sticker, or use a default
    let emojis = sticker.emoji.clone().unwrap_or_else(|| "😀".to_string());

    let input_sticker = crate::bot::telegram::create_input_sticker_from_file_id(
        &sticker.file.id.0,
        &sticker_format,
        &emojis,
    );

    // Perform the kang operation
    let result = sticker_service
        .kang_sticker(&user, input_sticker)
        .await;

    match result {
        Ok(kang_result) => {
            let pack_link = format!(
                "https://t.me/addstickers/{}",
                kang_result.pack.pack_link
            );

            let message = if kang_result.created_new_pack {
                format!(
                    "Created new sticker pack <b>{}</b>!\n\n<a href=\"{}\">Add it here</a>",
                    escape_html(&kang_result.pack.pack_name), pack_link
                )
            } else {
                format!(
                    "<a href=\"{}\">Sticker added</a> to <b>{}</b>",
                    pack_link, escape_html(&kang_result.pack.pack_name)
                )
            };

            bot.send_message(msg.chat.id, message)
                .parse_mode(teloxide::types::ParseMode::Html)
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
                BotError::DatabaseError(_) => {
                    "A database error occurred. Please try again later.".to_string()
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
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_parameters(ReplyParameters::new(msg.id))
                .await?;
            return Err(e);
        }
    }

    Ok(())
}
