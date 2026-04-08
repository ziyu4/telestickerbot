//! Telegram Sticker Kang Bot
//!
//! A bot that enables users to "kang" (add) stickers to their personal sticker packs.

mod bot;
mod config;
mod db;
mod repository;
mod service;

use std::process::ExitCode;
use std::sync::Arc;
use tracing::info;
use teloxide::prelude::RequesterExt;
use teloxide::update_listeners::webhooks;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use crate::bot::build_dispatcher;
use crate::config::Config;
use crate::db::Database;
use crate::repository::{SqliteStickerPackRepository, SqliteUserRepository};


#[tokio::main]
async fn main() -> ExitCode {
    // Phase 1: Configuration
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = match Config::from_env() {
        Ok(config) => match config.validate() {
            Ok(()) => config,
            Err(e) => {
                eprintln!("Configuration validation error: {e}");
                return ExitCode::FAILURE;
            }
        },
        Err(e) => {
            eprintln!("Configuration error: {e}");
            return ExitCode::FAILURE;
        }
    };

    info!("Phase 1 complete: Configuration loaded and validated");

    // Phase 2: Database initialization
    let database = match Database::new(&config.database).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to connect to database: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = database.run_migrations().await {
        eprintln!("Failed to run database migrations: {e}");
        return ExitCode::FAILURE;
    }

    info!("Phase 2 complete: Database initialized and migrations applied");

    // Phase 3: Repository initialization
    let user_repo = Arc::new(SqliteUserRepository::new(database.conn().clone()));
    let pack_repo = Arc::new(SqliteStickerPackRepository::new(database.conn().clone()));

    info!("Phase 3 complete: Repositories initialized");

    // Phase 4: Service initialization
    // Use teloxide's built-in throttle with default limits
    let mut bot = teloxide::Bot::new(&config.telegram_bot_token);
    
    // Support self-hosted Telegram Bot API
    if let Some(url_str) = &config.telegram_api_url
        && let Ok(url) = url::Url::parse(url_str) {
            bot = bot.set_api_url(url);
            info!(api_url = %url_str, "Using custom Telegram Bot API URL");
        }

    let bot = bot.throttle(teloxide::adaptors::throttle::Limits::default());
    let telegram_client = Arc::new(bot::TelegramClient::new(Arc::new(bot.clone())));

    // Get bot username from Telegram
    let bot_username = match telegram_client.get_bot_username().await {
        Ok(username) => {
            info!(bot_username = %username, "Retrieved bot username from Telegram");
            username
        }
        Err(e) => {
            eprintln!("Failed to get bot username: {e}");
            return ExitCode::FAILURE;
        }
    };

    let user_service = Arc::new(service::UserService::new(
        user_repo.clone(),
    ));

    let sticker_service = Arc::new(service::StickerService::new(
        user_repo.clone(),
        pack_repo.clone(),
        bot_username.clone(),
        telegram_client,
    ));

    info!("Phase 4 complete: Services initialized");

    // Phase 5: Bot startup with mode selection
    match &config.webhook {
        Some(webhook_config) => {
            info!("Starting in WEBHOOK mode");
            
            // Build dispatcher
            let mut dispatcher = build_dispatcher(
                bot.clone(),
                user_service,
                sticker_service,
                user_repo,
                pack_repo,
                config.owner_id,
            );

            // Configure webhook options
            let addr = format!("{}:{}", webhook_config.host, webhook_config.port)
                .parse()
                .expect("Invalid WEBHOOK_HOST or WEBHOOK_PORT");
            let url: url::Url = webhook_config.url
                .parse()
                .expect("Invalid WEBHOOK_URL");
            
            let options = webhooks::Options::new(addr, url.clone())
                .secret_token(webhook_config.secret.clone());

            // Create and start the webhook listener
            info!("Bot listening on {} with public URL {}", addr, url);
            let listener = webhooks::axum(bot, options)
                .await
                .expect("Failed to setup webhook listener");

            // Dispatch updates received from the webhook
            dispatcher.dispatch_with_listener(
                listener,
                teloxide::error_handlers::LoggingErrorHandler::with_custom_text("An error from the update listener"),
            ).await;
        }
        None => {
            info!("Starting in POLLING mode");
            
            // Build dispatcher
            let mut dispatcher = build_dispatcher(
                bot,
                user_service,
                sticker_service,
                user_repo,
                pack_repo,
                config.owner_id,
            );

            // Start polling
            info!("Bot started in polling mode - listening for updates");
            dispatcher.dispatch().await;
        }
    }

    info!("Bot shutdown complete");
    ExitCode::SUCCESS
}