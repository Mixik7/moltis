//! Stanza building helpers for XMPP messages, presence, and IQ.

use crate::minidom::{Element, rxml::NcName};

/// XMPP namespace constants.
pub mod ns {
    pub const JABBER_CLIENT: &str = "jabber:client";
    pub const MUC: &str = "http://jabber.org/protocol/muc";
    pub const MUC_USER: &str = "http://jabber.org/protocol/muc#user";
    pub const CHAT_STATES: &str = "http://jabber.org/protocol/chatstates";
    pub const OOB: &str = "jabber:x:oob";
    pub const REACTIONS: &str = "urn:xmpp:reactions:0";
    pub const HTTP_UPLOAD: &str = "urn:xmpp:http:upload:0";
}

/// Convert a static string to an `NcName` for use with minidom's attribute API.
///
/// Panics if the string is not a valid NCName (should only be used with known-good names).
pub(crate) fn ncname(s: &str) -> NcName {
    NcName::try_from(s).unwrap_or_else(|_| panic!("invalid NCName: {s}"))
}

/// Build a `<message>` stanza.
///
/// `msg_type` should be `"chat"` for 1:1 or `"groupchat"` for MUC.
pub fn build_message(from: &str, to: &str, msg_type: &str, body: &str) -> Element {
    Element::builder("message", ns::JABBER_CLIENT)
        .attr(ncname("from"), from)
        .attr(ncname("to"), to)
        .attr(ncname("type"), msg_type)
        .append(
            Element::builder("body", ns::JABBER_CLIENT)
                .append(body)
                .build(),
        )
        .build()
}

/// Build a `<presence>` stanza (initial presence or directed).
pub fn build_presence(from: &str, to: Option<&str>) -> Element {
    let mut builder = Element::builder("presence", ns::JABBER_CLIENT).attr(ncname("from"), from);
    if let Some(to) = to {
        builder = builder.attr(ncname("to"), to);
    }
    builder.build()
}

/// Build a `<presence type="unavailable">` stanza.
pub fn build_unavailable(from: &str, to: Option<&str>) -> Element {
    let mut builder = Element::builder("presence", ns::JABBER_CLIENT)
        .attr(ncname("from"), from)
        .attr(ncname("type"), "unavailable");
    if let Some(to) = to {
        builder = builder.attr(ncname("to"), to);
    }
    builder.build()
}

/// Chunk a text string into segments of at most `max_len` characters,
/// splitting at newline boundaries when possible.
pub fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Try to split at a newline within the limit.
        let split_at = remaining[..max_len]
            .rfind('\n')
            .map(|i| i + 1) // Include the newline in the current chunk
            .unwrap_or(max_len);

        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_chat_message() {
        let el = build_message("bot@example.com", "user@example.com", "chat", "Hello!");
        assert_eq!(el.name(), "message");
        assert_eq!(el.attr("type"), Some("chat"));
        assert_eq!(el.attr("to"), Some("user@example.com"));
        let body = el.get_child("body", ns::JABBER_CLIENT).unwrap();
        assert_eq!(body.text(), "Hello!");
    }

    #[test]
    fn build_groupchat_message() {
        let el = build_message(
            "bot@example.com",
            "room@conference.example.com",
            "groupchat",
            "Hi room!",
        );
        assert_eq!(el.attr("type"), Some("groupchat"));
    }

    #[test]
    fn build_initial_presence() {
        let el = build_presence("bot@example.com/moltis", None);
        assert_eq!(el.name(), "presence");
        assert_eq!(el.attr("from"), Some("bot@example.com/moltis"));
        assert!(el.attr("to").is_none());
    }

    #[test]
    fn build_directed_presence() {
        let el = build_presence(
            "bot@example.com/moltis",
            Some("room@conference.example.com/botnick"),
        );
        assert_eq!(el.attr("to"), Some("room@conference.example.com/botnick"));
    }

    #[test]
    fn chunk_short_text() {
        let chunks = chunk_text("hello", 100);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn chunk_at_newline() {
        let text = "line1\nline2\nline3";
        let chunks = chunk_text(text, 10);
        assert_eq!(chunks, vec!["line1\n", "line2\n", "line3"]);
    }

    #[test]
    fn chunk_no_newline() {
        let text = "abcdefghij";
        let chunks = chunk_text(text, 4);
        assert_eq!(chunks, vec!["abcd", "efgh", "ij"]);
    }
}
