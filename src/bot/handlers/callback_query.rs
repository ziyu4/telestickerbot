use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::CallbackQuery;
use teloxide::Bot;

use crate::service::UserService;
use crate::repository::{UserRepository, StickerPackRepository};
use crate::bot::error::BotError;
use super::utils::escape_html;

/// Handle callback queries from inline buttons.
pub async fn handle_callback_query<U, S>(
    bot: teloxide::adaptors::Throttle<Bot>,
    query: CallbackQuery,
    user_service: Arc<UserService<U>>,
    user_repo: Arc<U>,
    pack_repo: Arc<S>,
) -> Result<(), BotError>
where
    U: UserRepository,
    S: StickerPackRepository,
{
    // Get the user's Telegram ID
    let telegram_id = query.from.id.0 as i64;
    let username = query.from.username.clone();

    // Get or create the user
    let user = user_service
        .get_or_create(telegram_id, username)
        .await?;

    // Parse callback data to extract pack_id
    let callback_data = match &query.data {
        Some(data) => data,
        None => {
            bot.answer_callback_query(query.id)
                .text("Invalid callback data")
                .await?;
            return Ok(());
        }
    };

    // Parse "pack:{pack_id}" format
    let pack_id = match callback_data.strip_prefix("pack:") {
        Some(id_str) => match id_str.parse::<i64>() {
            Ok(id) => id,
            Err(_) => {
                bot.answer_callback_query(query.id)
                    .text("Invalid pack ID")
                    .await?;
                return Ok(());
            }
        },
        None => {
            bot.answer_callback_query(query.id)
                .text("Invalid callback format")
                .await?;
            return Ok(());
        }
    };

    // Get the pack from repository
    let pack = match pack_repo.get_by_id(pack_id).await? {
        Some(p) => p,
        None => {
            bot.answer_callback_query(query.id)
                .text("Pack not found")
                .await?;
            return Ok(());
        }
    };

    // Validate pack ownership
    if pack.user_id != user.id {
        tracing::warn!(
            user_id = user.id,
            pack_id = pack.id,
            pack_owner_id = pack.user_id,
            "Pack ownership validation failed"
        );
        bot.answer_callback_query(query.id)
            .text("That pack doesn't belong to you")
            .await?;
        return Ok(());
    }

    // Update user's default_pack_id
    user_repo
        .set_default_pack(user.id, Some(pack.id))
        .await?;

    // Answer callback query with confirmation
    bot.answer_callback_query(query.id)
        .text(format!("Default pack set to: {}", pack.pack_name))
        .await?;

    // Edit the selection message with confirmation
    if let Some(message) = query.message {
        bot.edit_message_text(
            message.chat().id,
            message.id(),
            format!("✅ Default pack set to: <b>{}</b>", escape_html(&pack.pack_name)),
        )
        .parse_mode(teloxide::types::ParseMode::Html)
        .await?;
    }

    Ok(())
}
