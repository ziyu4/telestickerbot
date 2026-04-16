//! Emoji utilities for sticker bot.
//!
//! This module provides access to emoji data fetched at build time from emoji-datasource.
//! The data is embedded in the binary and can be used to pick random emojis for stickers.

// Include the generated emoji data
include!(concat!(env!("OUT_DIR"), "/emoji_data.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emoji_data_not_empty() {
        assert!(!EMOJI_DATA.is_empty(), "EMOJI_DATA should not be empty");
    }

    #[test]
    fn test_unified_to_emoji() {
        // Test simple emoji: 😀 (grinning face)
        let emoji = unified_to_emoji("1F600");
        assert_eq!(emoji, "😀");
        
        // Test emoji with modifier: 👍 (thumbs up)
        let emoji = unified_to_emoji("1F44D");
        assert_eq!(emoji, "👍");
    }

    #[test]
    fn test_random_emoji() {
        let emoji = random_emoji();
        assert!(!emoji.is_empty(), "random_emoji should return non-empty string");
    }
}
