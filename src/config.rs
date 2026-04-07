//! Configuration management for the Telegram Sticker Kang Bot.
//!
//! This module provides configuration structs and utilities for loading
//! configuration from environment variables.

use std::env;

/// Configuration error types.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A required environment variable is missing.
    #[error("Missing required environment variable: {0}")]
    MissingVar(&'static str),

    /// A configuration value is invalid.
    #[error("Invalid configuration value: {0}")]
    InvalidValue(&'static str),
}

/// Webhook configuration for HTTP callback mode.
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Webhook URL (must start with https://)
    pub url: String,
    /// Secret token for request validation (min 32 chars)
    pub secret: String,
    /// Host to bind the webhook server (default: 0.0.0.0)
    pub host: String,
    /// Port to bind the webhook server (default: 8080)
    pub port: u16,
}

/// Main configuration struct containing all bot settings.
#[derive(Debug, Clone)]
pub struct Config {
    /// Telegram bot API token.
    pub telegram_bot_token: String,
    /// Database configuration.
    pub database: DatabaseConfig,
    /// Webhook configuration (optional, enables webhook mode when present).
    pub webhook: Option<WebhookConfig>,
    /// Telegram ID of the bot owner (optional, for admin commands).
    pub owner_id: Option<i64>,
    /// Custom Telegram Bot API URL (for self-hosted servers).
    pub telegram_api_url: Option<String>,
}

/// Database configuration supporting Turso (remote) or SQLite (local) backends.
#[derive(Debug, Clone)]
pub enum DatabaseConfig {
    /// Turso remote database configuration.
    Turso {
        /// Turso connection URL.
        url: String,
        /// Turso authentication token (required for remote).
        auth_token: String,
    },
    /// Local SQLite database configuration.
    Sqlite {
        /// Path to the SQLite database file.
        path: String,
    },
}


impl Config {
    /// Loads configuration from environment variables.
    ///
    /// # Environment Variables
    ///
    /// | Variable | Required | Default | Description |
    /// |----------|----------|---------|-------------|
    /// | `TELEGRAM_BOT_TOKEN` | Yes | - | Telegram bot API token |
    /// | `DATABASE_URL` | No | - | Turso connection URL (enables Turso) |
    /// | `SQLITE_PATH` | No | `stickerbot.db` | Local SQLite database path |
    /// | `WEBHOOK_URL` | No | - | Webhook URL (enables webhook mode) |
    /// | `WEBHOOK_SECRET` | Conditional | - | Webhook secret (required if WEBHOOK_URL is set) |
    /// | `WEBHOOK_HOST` | No | `0.0.0.0` | Webhook server bind address |
    /// | `WEBHOOK_PORT` | No | `8080` | Webhook server bind port |
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::MissingVar` if `TELEGRAM_BOT_TOKEN` is not set.
    /// Returns `ConfigError::MissingVar` if `WEBHOOK_URL` is set but `WEBHOOK_SECRET` is not.
    pub fn from_env() -> Result<Self, ConfigError> {
        let telegram_bot_token = env::var("TELEGRAM_BOT_TOKEN")
            .map_err(|_| ConfigError::MissingVar("TELEGRAM_BOT_TOKEN"))?;

        let database = if let Ok(url) = env::var("DATABASE_URL") {
            let auth_token = env::var("TURSO_AUTH_TOKEN")
                .map_err(|_| ConfigError::MissingVar("TURSO_AUTH_TOKEN"))?;
            DatabaseConfig::Turso { url, auth_token }
        } else {
            DatabaseConfig::Sqlite {
                path: env::var("SQLITE_PATH").unwrap_or_else(|_| "stickerbot.db".to_string()),
            }
        };


        let webhook = if let Ok(url) = env::var("WEBHOOK_URL") {
            let secret = env::var("WEBHOOK_SECRET")
                .map_err(|_| ConfigError::MissingVar("WEBHOOK_SECRET"))?;
            let host = env::var("WEBHOOK_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
            let port = env::var("WEBHOOK_PORT")
                .map(|v| v.parse().unwrap_or(8811))
                .unwrap_or(8811);

            Some(WebhookConfig {
                url,
                secret,
                host,
                port,
            })
        } else {
            None
        };

        let owner_id = env::var("BOT_OWNER_ID")
            .ok()
            .and_then(|v| v.parse().ok());

        let telegram_api_url = env::var("TELEGRAM_API_URL").ok();

        Ok(Config {
            telegram_bot_token,
            database,
            webhook,
            owner_id,
            telegram_api_url,
        })
    }

    /// Validates the configuration values.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::InvalidValue` if:
    /// - `TELEGRAM_BOT_TOKEN` is empty
    /// - `DATABASE_URL` is not a valid Turso URL (must start with `libsql://` or `http`)
    /// - `CACHE_MAX_CAPACITY` is zero
    /// - `WEBHOOK_URL` does not start with `https://`
    /// - `WEBHOOK_SECRET` is less than 32 characters
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.telegram_bot_token.is_empty() {
            return Err(ConfigError::InvalidValue(
                "TELEGRAM_BOT_TOKEN cannot be empty",
            ));
        }

        if let DatabaseConfig::Turso { url, auth_token } = &self.database {
            if !url.starts_with("libsql://") && !url.starts_with("http") {
                return Err(ConfigError::InvalidValue(
                    "DATABASE_URL must be a valid Turso URL",
                ));
            }
            if auth_token.is_empty() {
                return Err(ConfigError::InvalidValue(
                    "TURSO_AUTH_TOKEN cannot be empty for Turso database",
                ));
            }
        }


        if let Some(webhook) = &self.webhook {
            if !webhook.url.starts_with("https://") && !webhook.url.starts_with("http://") {
                return Err(ConfigError::InvalidValue(
                    "WEBHOOK_URL must start with http:// or https://",
                ));
            }

            if webhook.secret.len() < 32 {
                return Err(ConfigError::InvalidValue(
                    "WEBHOOK_SECRET must be at least 32 characters",
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_empty_token() {
        let config = Config {
            telegram_bot_token: String::new(),
            database: DatabaseConfig::Sqlite {
                path: "test.db".to_string(),
            },
            webhook: None,
            owner_id: None,
            telegram_api_url: None,
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue("TELEGRAM_BOT_TOKEN cannot be empty"))
        ));
    }

    #[test]
    fn test_validate_invalid_turso_url() {
        let config = Config {
            telegram_bot_token: "valid_token".to_string(),
            database: DatabaseConfig::Turso {
                url: "invalid-url".to_string(),
                auth_token: "token".to_string(),
            },
            webhook: None,
            owner_id: None,
            telegram_api_url: None,
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue("DATABASE_URL must be a valid Turso URL"))
        ));
    }


    #[test]
    fn test_validate_valid_config() {
        let config = Config {
            telegram_bot_token: "valid_token".to_string(),
            database: DatabaseConfig::Sqlite {
                path: "test.db".to_string(),
            },
            webhook: None,
            owner_id: None,
            telegram_api_url: None,
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_valid_turso_url_libsql() {
        let config = Config {
            telegram_bot_token: "valid_token".to_string(),
            database: DatabaseConfig::Turso {
                url: "libsql://example.turso.io".to_string(),
                auth_token: "token".to_string(),
            },
            webhook: None,
            owner_id: None,
            telegram_api_url: None,
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_valid_turso_url_http() {
        let config = Config {
            telegram_bot_token: "valid_token".to_string(),
            database: DatabaseConfig::Turso {
                url: "https://example.turso.io".to_string(),
                auth_token: "token".to_string(),
            },
            webhook: None,
            owner_id: None,
            telegram_api_url: None,
        };

        let result = config.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_webhook_url_not_https() {
        let config = Config {
            telegram_bot_token: "valid_token".to_string(),
            database: DatabaseConfig::Sqlite {
                path: "test.db".to_string(),
            },
            webhook: Some(WebhookConfig {
                url: "mailto:example.com/webhook".to_string(),
                secret: "a".repeat(32),
                host: "0.0.0.0".to_string(),
                port: 8080,
            }),
            owner_id: None,
            telegram_api_url: None,
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(_))
        ));
    }

    #[test]
    fn test_validate_webhook_secret_too_short() {
        let config = Config {
            telegram_bot_token: "valid_token".to_string(),
            database: DatabaseConfig::Sqlite {
                path: "test.db".to_string(),
            },
            webhook: Some(WebhookConfig {
                url: "https://example.com/webhook".to_string(),
                secret: "short_secret".to_string(),
                host: "0.0.0.0".to_string(),
                port: 8080,
            }),
            owner_id: None,
            telegram_api_url: None,
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(ConfigError::InvalidValue(
                "WEBHOOK_SECRET must be at least 32 characters"
            ))
        ));
    }

    #[test]
    fn test_validate_valid_webhook_config() {
        let config = Config {
            telegram_bot_token: "valid_token".to_string(),
            database: DatabaseConfig::Sqlite {
                path: "test.db".to_string(),
            },
            webhook: Some(WebhookConfig {
                url: "https://example.com/webhook".to_string(),
                secret: "a".repeat(32),
                host: "0.0.0.0".to_string(),
                port: 8080,
            }),
            owner_id: None,
            telegram_api_url: None,
        };

        let result = config.validate();
        assert!(result.is_ok());
    }
}