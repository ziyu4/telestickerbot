pub mod start;
pub mod kang;
pub mod create_pack;
pub mod set_sticker_pack;
pub mod callback_query;
pub mod stats;
pub mod utils;

use std::sync::Arc;
use teloxide::{
    dispatching::Dispatcher,
    prelude::*,
    types::{CallbackQuery, Message},
    Bot,
};

use crate::service::{StickerService, UserService};
use crate::bot::error::BotError;
use crate::bot::commands::Command;

use self::start::handle_start;
use self::kang::handle_kang;
use self::create_pack::handle_createpack;
use self::set_sticker_pack::handle_setstickerpack;
use self::callback_query::handle_callback_query;
use self::stats::handle_stats;

/// Build the dptree dispatcher with command routing and dependency injection.
pub fn build_dispatcher(
    bot: teloxide::adaptors::Throttle<Bot>,
    user_service: Arc<UserService<crate::repository::SqliteUserRepository>>,
    sticker_service: Arc<StickerService<crate::repository::SqliteUserRepository, crate::repository::SqliteStickerPackRepository>>,
    user_repo: Arc<crate::repository::SqliteUserRepository>,
    pack_repo: Arc<crate::repository::SqliteStickerPackRepository>,
    owner_id: Option<i64>,
) -> Dispatcher<teloxide::adaptors::Throttle<Bot>, BotError, teloxide::dispatching::DefaultKey>
{
    use dptree::case;

    // Define the command handler branch
    let command_handler = teloxide::filter_command::<Command, _>()
        .branch(
            case![Command::Start]
                .endpoint(
                    |bot: teloxide::adaptors::Throttle<Bot>, 
                     msg: Message| async move {
                        handle_start(bot, msg).await
                    }
                )
        )
        .branch(
            case![Command::Kang]
                .endpoint(
                    |bot: teloxide::adaptors::Throttle<Bot>, 
                     msg: Message, 
                     user_service: Arc<UserService<crate::repository::SqliteUserRepository>>,
                     sticker_service: Arc<StickerService<crate::repository::SqliteUserRepository, crate::repository::SqliteStickerPackRepository>>| async move {
                        handle_kang(bot, msg, user_service, sticker_service).await
                    }
                )
        )
        .branch(
            case![Command::CreatePack(pack_name)]
                .endpoint(
                    |bot: teloxide::adaptors::Throttle<Bot>, 
                     msg: Message, 
                     pack_name: String,
                     user_service: Arc<UserService<crate::repository::SqliteUserRepository>>,
                     sticker_service: Arc<StickerService<crate::repository::SqliteUserRepository, crate::repository::SqliteStickerPackRepository>>| async move {
                        handle_createpack(bot, msg, pack_name, user_service, sticker_service).await
                    }
                )
        )
        .branch(
            case![Command::SetStickerPack]
                .endpoint(
                    |bot: teloxide::adaptors::Throttle<Bot>, 
                     msg: Message,
                      user_service: Arc<UserService<crate::repository::SqliteUserRepository>>,
                      pack_repo: Arc<crate::repository::SqliteStickerPackRepository>| async move {
                         handle_setstickerpack(bot, msg, user_service, pack_repo).await
                     }
                )
        )
        .branch(
            case![Command::Stats]
                .endpoint(
                    |bot: teloxide::adaptors::Throttle<Bot>, 
                     msg: Message,
                     owner_id: Option<i64>| async move {
                        handle_stats(bot, msg, owner_id).await
                    }
                )
        );

    // Define the message handler branch
    let message_handler = Update::filter_message()
        .branch(command_handler);

    // Define the callback query handler branch
    let callback_handler = Update::filter_callback_query()
        .endpoint(
            |bot: teloxide::adaptors::Throttle<Bot>, 
             query: CallbackQuery,
              user_service: Arc<UserService<crate::repository::SqliteUserRepository>>,
              user_repo: Arc<crate::repository::SqliteUserRepository>,
              pack_repo: Arc<crate::repository::SqliteStickerPackRepository>| async move {
                 handle_callback_query(bot, query, user_service, user_repo, pack_repo).await
             }
        );

    // Build the main dptree schema
    let handler = dptree::entry()
        .branch(message_handler)
        .branch(callback_handler)
        .endpoint(|| async { Ok(()) });

    // Create the dispatcher with dependency injection
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            user_service,
            sticker_service,
            user_repo,
            pack_repo,
            owner_id
        ])
        .enable_ctrlc_handler()
        .build()
}
