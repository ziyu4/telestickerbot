use teloxide::prelude::*;
use teloxide::types::ReplyParameters;
use crate::bot::error::BotError;

/// Handle the `/start` command.
///
/// Sends a welcome message and basic instructions.
pub async fn handle_start(
    bot: teloxide::adaptors::Throttle<Bot>,
    msg: Message,
) -> Result<(), BotError> {
    let welcome_text = "👋 <b>Welcome to Sticker Kang Bot!</b>\n\n\
        I can help you \"kang\" (stole) stickers from other packs and add them to your own.\n\n\
        <b>How to use:</b>\n\
        1. Reply to any sticker with /kang\n\
        2. If you don't have a pack yet, I'll create one for you!\n\
        3. Use /createpack &lt;name&gt; to start a new themed pack.\n\
        4. Use /setstickerpack to switch between your packs.\n\n\
        <i>Ready to start kanging? Send me a sticker</i>";

    bot.send_message(msg.chat.id, welcome_text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_parameters(ReplyParameters::new(msg.id))
        .await?;

    Ok(())
}
