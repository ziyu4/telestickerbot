//! Database schema and entity structs
//!
//! This module defines the database schema and entity structs for the
//! Telegram Sticker Kang Bot.

use sqlx::FromRow;

// ============================================================================
// Schema SQL Constants
// ============================================================================

/// SQL to create the users table
pub const CREATE_USERS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    telegram_id INTEGER NOT NULL UNIQUE,
    username TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
)
"#;

/// SQL to create index on users.telegram_id
pub const CREATE_USERS_TELEGRAM_ID_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_users_telegram_id ON users(telegram_id)
"#;

/// SQL to create the sticker_packs table
pub const CREATE_STICKER_PACKS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS sticker_packs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    pack_name TEXT NOT NULL,
    pack_link TEXT NOT NULL UNIQUE,
    version TEXT NOT NULL,
    sticker_count INTEGER NOT NULL DEFAULT 0,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    last_synced_at INTEGER,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
)
"#;

/// SQL to create index on sticker_packs.pack_link
pub const CREATE_STICKER_PACKS_PACK_LINK_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_sticker_packs_pack_link ON sticker_packs(pack_link)
"#;

/// SQL to create index on sticker_packs.user_id
pub const CREATE_STICKER_PACKS_USER_ID_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_sticker_packs_user_id ON sticker_packs(user_id)
"#;

/// SQL to add default_pack_id column to users table
pub const ADD_DEFAULT_PACK_ID_COLUMN: &str = r#"
ALTER TABLE users ADD COLUMN default_pack_id INTEGER REFERENCES sticker_packs(id) ON DELETE SET NULL
"#;

/// SQL to create index on users.default_pack_id
pub const CREATE_USERS_DEFAULT_PACK_ID_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_users_default_pack_id ON users(default_pack_id)
"#;

/// SQL to populate default_pack_id for existing users with active packs
pub const MIGRATE_DEFAULT_PACK_ID: &str = r#"
UPDATE users 
SET default_pack_id = (
    SELECT id FROM sticker_packs 
    WHERE user_id = users.id AND is_active = 1 
    LIMIT 1
)
WHERE EXISTS (
    SELECT 1 FROM sticker_packs 
    WHERE user_id = users.id AND is_active = 1
)
"#;

/// All schema migrations in order
pub const SCHEMA_MIGRATIONS: &[&str] = &[
    CREATE_USERS_TABLE,
    CREATE_USERS_TELEGRAM_ID_INDEX,
    CREATE_STICKER_PACKS_TABLE,
    CREATE_STICKER_PACKS_PACK_LINK_INDEX,
    CREATE_STICKER_PACKS_USER_ID_INDEX,
    ADD_DEFAULT_PACK_ID_COLUMN,
    CREATE_USERS_DEFAULT_PACK_ID_INDEX,
    MIGRATE_DEFAULT_PACK_ID,
];

// ============================================================================
// Entity Structs
// ============================================================================

/// User entity
#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: i64,
    pub telegram_id: i64,
    pub username: Option<String>,
    pub default_pack_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Sticker pack entity
#[derive(Debug, Clone, FromRow)]
pub struct StickerPack {
    pub id: i64,
    pub user_id: i64,
    pub pack_name: String,
    pub pack_link: String,
    pub version: String,
    pub sticker_count: i32,
    pub is_active: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_synced_at: Option<i64>,
}

/// New user for insertion
#[derive(Debug, Clone)]
pub struct NewUser {
    pub telegram_id: i64,
    pub username: Option<String>,
}

/// New sticker pack for insertion
#[derive(Debug, Clone)]
pub struct NewStickerPack {
    pub user_id: i64,
    pub pack_name: String,
    pub pack_link: String,
    pub version: String,
}