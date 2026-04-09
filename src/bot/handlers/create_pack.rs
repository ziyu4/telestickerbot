use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ReplyParameters;
use teloxide::Bot;

use crate::service::{StickerService, UserService};
use crate::repository::{UserRepository, StickerPackRepository};
use crate::bot::telegram::get_sticker_format;
use crate::bot::error::BotError;
use super::utils::escape_html;

/// Handle the `/createpack` command.
pub async fn handle_createpack<U, S>(
    bot: teloxide::adaptors::Throttle<Bot>,
    msg: Message,
    pack_name: String,
    user_service: Arc<UserService<U>>,
    sticker_service: Arc<StickerService<U, S>>,
) -> Result<(), BotError>
where
    U: UserRepository,
    S: StickerPackRepository,
{
    // Validate that the command is a reply to a sticker
    let sticker = match msg.reply_to_message() {
        Some(reply) => {
            match reply.sticker() {
                Some(s) => s.clone(),
                None => {
                    bot.send_message(
                        msg.chat.id,
                        "Please reply to a sticker with /createpack <name> to create a pack.",
                    )
                    .reply_parameters(ReplyParameters::new(msg.id))
                    .await?;
                    return Ok(());
                }
            }
        }
        None => {
            bot.send_message(
                msg.chat.id,
                "Please reply to a sticker with /createpack <name> to create a pack.",
            )
            .reply_parameters(ReplyParameters::new(msg.id))
            .await?;
            return Ok(());
        }
    };

    // Get the user's Telegram ID
    let telegram_id = match &msg.from {
        Some(user) => user.id.0 as i64,
        None => return Ok(()), // No user info, ignore
    };

    // Get the username
    let username = msg.from.as_ref().and_then(|u| u.username.clone());

    // Validate pack name is not empty
    if pack_name.trim().is_empty() {
        bot.send_message(msg.chat.id, "Pack name cannot be empty.")
            .reply_parameters(ReplyParameters::new(msg.id))
            .await?;
        return Ok(());
    }

    // Validate pack name length
    if pack_name.len() > 64 {
        bot.send_message(
            msg.chat.id,
            "Pack name exceeds character limit (max 64 characters).",
        )
        .reply_parameters(ReplyParameters::new(msg.id))
        .await?;
        return Ok(());
    }

    // Get or create the user
    let user = user_service
        .get_or_create(telegram_id, username)
        .await?;

    // Determine sticker format and default emojis
    let sticker_format = get_sticker_format(&sticker);
    let emojis = sticker.emoji.clone().unwrap_or_else(|| "😀".to_string());

    // Create the custom pack
    let result = match sticker_service
        .create_custom_pack(&user, &pack_name, &sticker.file.id.0, sticker_format, &emojis)
        .await
    {
        Ok(res) => res,
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
                BotError::PackNameTooLong => {
                    "Pack name is too long.".to_string()
                }
                _ => "An error occurred while creating the sticker pack.".to_string(),
            };

            bot.send_message(msg.chat.id, error_message)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_parameters(ReplyParameters::new(msg.id))
                .await?;
            return Err(e);
        }
    };

    let pack_link = format!(
        "https://t.me/addstickers/{}",
        result.pack.pack_link
    );

    let message = format!(
        "Created custom sticker pack <b>{}</b>!\n\nIt is now your active pack.\n<a href=\"{}\">Add it here</a>",
        escape_html(&result.pack.pack_name), pack_link
    );

    bot.send_message(msg.chat.id, message)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_parameters(ReplyParameters::new(msg.id))
        .await?;

    Ok(())
}
