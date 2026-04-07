use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ReplyParameters;
use teloxide::Bot;

use crate::service::UserService;
use crate::repository::{UserRepository, StickerPackRepository};
use crate::bot::error::BotError;
use super::utils::create_pack_selection_keyboard;

/// Handle the `/setstickerpack` command.
pub async fn handle_setstickerpack<U>(
    bot: teloxide::adaptors::Throttle<Bot>,
    msg: Message,
    user_service: Arc<UserService<U>>,
    pack_repo: Arc<impl StickerPackRepository>,
) -> Result<(), BotError>
where
    U: UserRepository,
{
    // Get the user's Telegram ID
    let telegram_id = match &msg.from {
        Some(user) => user.id.0 as i64,
        None => return Ok(()), // No user info, ignore
    };

    // Get the username
    let username = msg.from.as_ref().and_then(|u| u.username.clone());

    // Get or create the user
    let user = user_service
        .get_or_create(telegram_id, username)
        .await?;

    // Get all user's packs from repository
    let packs = pack_repo.get_all_by_user(user.id).await?;

    // If user has no sticker packs, inform them
    if packs.is_empty() {
        bot.send_message(
            msg.chat.id,
            "You don't have any sticker packs yet. Create one first with /createpack.",
        )
        .reply_parameters(ReplyParameters::new(msg.id))
        .await?;
        return Ok(());
    }

    // Generate inline keyboard with pack buttons
    let keyboard = create_pack_selection_keyboard(&packs);

    // Send message with keyboard
    bot.send_message(
        msg.chat.id,
        "Select your default sticker pack:",
    )
    .reply_markup(keyboard)
    .reply_parameters(ReplyParameters::new(msg.id))
    .await?;

    Ok(())
}
