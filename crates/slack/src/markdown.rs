//! Markdown to Slack mrkdwn conversion.
//!
//! Slack uses its own "mrkdwn" format which differs from standard Markdown.
//! This module converts common Markdown patterns to Slack-compatible format.

/// Maximum message length for Slack (40000 chars for mrkdwn text).
pub const SLACK_MAX_MESSAGE_LEN: usize = 40_000;

/// Convert Markdown to Slack mrkdwn format.
///
/// Slack mrkdwn differences from Markdown:
/// - Bold: `**text**` or `__text__` → `*text*`
/// - Italic: `*text*` or `_text_` → `_text_`
/// - Strikethrough: `~~text~~` → `~text~`
/// - Code blocks: same (triple backticks)
/// - Inline code: same (single backticks)
/// - Links: `[text](url)` → `<url|text>`
/// - Headers: not supported in mrkdwn, convert to bold
pub fn markdown_to_slack(text: &str) -> String {
    let mut result = text.to_string();

    // Convert links: [text](url) → <url|text>
    result = convert_links(&result);

    // Convert headers to bold (# Header → *Header*)
    result = convert_headers(&result);

    // Convert bold: **text** → *text* (must do before italic)
    result = result.replace("**", "*");
    result = result.replace("__", "*");

    // Convert strikethrough: ~~text~~ → ~text~
    result = result.replace("~~", "~");

    result
}

/// Convert Markdown links to Slack format.
fn convert_links(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '[' {
            // Try to parse [text](url)
            let mut link_text = String::new();
            let mut found_close = false;

            for lc in chars.by_ref() {
                if lc == ']' {
                    found_close = true;
                    break;
                }
                link_text.push(lc);
            }

            if found_close && chars.peek() == Some(&'(') {
                chars.next(); // consume '('
                let mut url = String::new();
                let mut found_url_close = false;

                for uc in chars.by_ref() {
                    if uc == ')' {
                        found_url_close = true;
                        break;
                    }
                    url.push(uc);
                }

                if found_url_close {
                    // Slack format: <url|text>
                    result.push('<');
                    result.push_str(&url);
                    result.push('|');
                    result.push_str(&link_text);
                    result.push('>');
                    continue;
                } else {
                    // Malformed, output as-is
                    result.push('[');
                    result.push_str(&link_text);
                    result.push_str("](");
                    result.push_str(&url);
                }
            } else {
                // Not a link, output as-is
                result.push('[');
                result.push_str(&link_text);
                if found_close {
                    result.push(']');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Convert Markdown headers to bold text.
fn convert_headers(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("######") {
            lines.push(format!("*{}*", rest.trim_start()));
        } else if let Some(rest) = trimmed.strip_prefix("#####") {
            lines.push(format!("*{}*", rest.trim_start()));
        } else if let Some(rest) = trimmed.strip_prefix("####") {
            lines.push(format!("*{}*", rest.trim_start()));
        } else if let Some(rest) = trimmed.strip_prefix("###") {
            lines.push(format!("*{}*", rest.trim_start()));
        } else if let Some(rest) = trimmed.strip_prefix("##") {
            lines.push(format!("*{}*", rest.trim_start()));
        } else if let Some(rest) = trimmed.strip_prefix('#') {
            lines.push(format!("*{}*", rest.trim_start()));
        } else {
            lines.push(line.to_string());
        }
    }

    lines.join("\n")
}

/// Chunk a message into parts that fit within Slack's limit.
pub fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if current.len() + line.len() + 1 > max_len {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            // If single line is too long, split it
            if line.len() > max_len {
                let mut remaining = line;
                while remaining.len() > max_len {
                    chunks.push(remaining[..max_len].to_string());
                    remaining = &remaining[max_len..];
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

/// Strip Slack user mentions from text.
///
/// Slack mentions look like `<@U12345678>` or `<@U12345678|username>`.
pub fn strip_mentions(text: &str, bot_user_id: Option<&str>) -> String {
    let mut result = text.to_string();

    // Remove bot mention specifically
    if let Some(bot_id) = bot_user_id {
        result = result.replace(&format!("<@{bot_id}>"), "");
        result = result.replace(&format!("<@{bot_id}|"), "");
    }

    // Clean up any remaining empty mentions artifacts
    result = result.trim().to_string();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bold_conversion() {
        assert_eq!(markdown_to_slack("**bold**"), "*bold*");
        assert_eq!(markdown_to_slack("__bold__"), "*bold*");
    }

    #[test]
    fn test_strikethrough_conversion() {
        assert_eq!(markdown_to_slack("~~strike~~"), "~strike~");
    }

    #[test]
    fn test_link_conversion() {
        assert_eq!(
            markdown_to_slack("[click here](https://example.com)"),
            "<https://example.com|click here>"
        );
    }

    #[test]
    fn test_header_conversion() {
        assert_eq!(markdown_to_slack("# Header"), "*Header*");
        assert_eq!(markdown_to_slack("## Header"), "*Header*");
        assert_eq!(markdown_to_slack("### Header"), "*Header*");
    }

    #[test]
    fn test_chunk_message() {
        let text = "a".repeat(100);
        let chunks = chunk_message(&text, 50);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 50);
        assert_eq!(chunks[1].len(), 50);
    }

    #[test]
    fn test_strip_mentions() {
        let text = "<@U12345678> hello there";
        assert_eq!(strip_mentions(text, Some("U12345678")), "hello there");
    }
}
