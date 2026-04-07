use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use std::sync::Arc;

/// Escape special characters for HTML format.
pub fn escape_html(text: &str) -> String {
    text.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
}

/// Create an inline keyboard for pack selection.
///
/// Generates buttons for each sticker pack, showing the pack name and
/// sticker count. The callback data is encoded as "pack:{pack_id}".
pub fn create_pack_selection_keyboard(packs: &[Arc<crate::db::schema::StickerPack>]) -> InlineKeyboardMarkup {
    let buttons: Vec<Vec<InlineKeyboardButton>> = packs
        .iter()
        .map(|pack| {
            vec![InlineKeyboardButton::callback(
                format!("{} ({} stickers)", pack.pack_name, pack.sticker_count),
                format!("pack:{}", pack.id),
            )]
        })
        .collect();

    InlineKeyboardMarkup::new(buttons)
}
