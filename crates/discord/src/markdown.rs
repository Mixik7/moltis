//! Markdown formatting for Discord.
//!
//! Discord supports a subset of Markdown similar to GitHub-flavored Markdown.
//! This module provides utilities for message formatting and chunking.

/// Maximum message length for Discord regular messages.
pub const DISCORD_MAX_MESSAGE_LEN: usize = 2000;

/// Maximum description length for Discord embeds.
pub const DISCORD_EMBED_DESC_LEN: usize = 4096;

/// Discord already supports most Markdown natively, so we mainly need to
/// handle edge cases and chunking.
pub fn format_for_discord(text: &str) -> String {
    // Discord supports standard Markdown, but we should escape any
    // accidental Discord-specific formatting
    text.to_string()
}

/// Chunk a message into parts that fit within Discord's limit.
pub fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut in_code_block = false;
    let code_block_marker = "```";

    for line in text.lines() {
        // Track code block state
        let line_has_marker = line.contains(code_block_marker);
        if line_has_marker {
            // Count markers in line
            let marker_count = line.matches(code_block_marker).count();
            if marker_count % 2 == 1 {
                in_code_block = !in_code_block;
            }
        }

        let line_with_newline = if current.is_empty() {
            line.len()
        } else {
            line.len() + 1
        };

        if current.len() + line_with_newline > max_len {
            if !current.is_empty() {
                // If we're in a code block, close it before chunking
                if in_code_block && !current.ends_with(code_block_marker) {
                    current.push_str("\n```");
                }
                chunks.push(current);
                current = String::new();

                // If we were in a code block, reopen it in the new chunk
                if in_code_block {
                    current.push_str("```\n");
                }
            }

            // If single line is too long, split it
            if line.len() > max_len {
                let mut remaining = line;
                while remaining.len() > max_len {
                    let split_point = find_split_point(remaining, max_len);
                    chunks.push(remaining[..split_point].to_string());
                    remaining = &remaining[split_point..];
                }
                current = remaining.to_string();
            } else {
                current = line.to_string();
            }
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// Find a good split point near max_len, preferring word boundaries.
fn find_split_point(text: &str, max_len: usize) -> usize {
    if text.len() <= max_len {
        return text.len();
    }

    // Try to find a space near the end
    let search_start = max_len.saturating_sub(50);
    if let Some(pos) = text[search_start..max_len].rfind(' ') {
        return search_start + pos;
    }

    // No good split point, just split at max_len
    max_len
}

/// Truncate text with ellipsis if too long.
pub fn truncate_with_ellipsis(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        let truncate_at = max_len.saturating_sub(3);
        format!("{}...", &text[..truncate_at])
    }
}

/// Strip Discord user mentions from text.
///
/// Discord mentions look like `<@123456789>` or `<@!123456789>`.
pub fn strip_mentions(text: &str, bot_user_id: Option<u64>) -> String {
    let mut result = text.to_string();

    // Remove bot mention specifically
    if let Some(bot_id) = bot_user_id {
        result = result.replace(&format!("<@{bot_id}>"), "");
        result = result.replace(&format!("<@!{bot_id}>"), "");
    }

    // Clean up whitespace
    result = result.trim().to_string();

    result
}

/// Check if the message mentions the bot.
pub fn mentions_bot(text: &str, bot_user_id: u64) -> bool {
    text.contains(&format!("<@{bot_user_id}>")) || text.contains(&format!("<@!{bot_user_id}>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_short_message() {
        let text = "Hello, world!";
        let chunks = chunk_message(text, DISCORD_MAX_MESSAGE_LEN);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_chunk_long_message() {
        let text = "a".repeat(2500);
        let chunks = chunk_message(&text, DISCORD_MAX_MESSAGE_LEN);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].len() <= DISCORD_MAX_MESSAGE_LEN);
    }

    #[test]
    fn test_truncate_with_ellipsis() {
        let text = "Hello, world!";
        assert_eq!(truncate_with_ellipsis(text, 100), text);
        assert_eq!(truncate_with_ellipsis(text, 10), "Hello, ...");
    }

    #[test]
    fn test_strip_mentions() {
        let text = "<@123456789> hello there";
        assert_eq!(strip_mentions(text, Some(123456789)), "hello there");

        let text2 = "<@!123456789> hello";
        assert_eq!(strip_mentions(text2, Some(123456789)), "hello");
    }

    #[test]
    fn test_mentions_bot() {
        assert!(mentions_bot("<@123456789>", 123456789));
        assert!(mentions_bot("<@!123456789>", 123456789));
        assert!(!mentions_bot("hello", 123456789));
    }
}
