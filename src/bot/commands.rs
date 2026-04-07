use teloxide::utils::command::BotCommands;

/// Bot commands supported by the Telegram Sticker Kang Bot.
///
/// This enum uses teloxide's `BotCommands` derive macro to automatically
/// implement command parsing and help message generation.
#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase")]
#[command(description = "Sticker Kang Bot commands:")]
pub enum Command {
    /// Start the bot
    #[command(description = "Start the bot and see welcome message")]
    Start,

    /// Add a sticker to your pack (reply to a sticker or send with this command)
    #[command(description = "Stole a someone's sticker to your pack")]
    Kang,

    /// Create a new sticker pack with a custom name
    #[command(description = "Create a new sticker pack with a custom name")]
    CreatePack(String),

    /// Select your default sticker pack from a list
    #[command(description = "Select your default sticker pack")]
    SetStickerPack,

    /// Show bot and system resource usage statistics
    #[command(description = "Show resource usage statistics")]
    Stats,
}
