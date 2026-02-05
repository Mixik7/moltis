//! XEP-0085: Chat State Notifications (composing, active, paused).

use crate::minidom::Element;

use crate::stanza::{ncname, ns};

/// Chat state types per XEP-0085.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatState {
    Active,
    Composing,
    Paused,
    Inactive,
    Gone,
}

impl ChatState {
    fn element_name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Composing => "composing",
            Self::Paused => "paused",
            Self::Inactive => "inactive",
            Self::Gone => "gone",
        }
    }
}

/// Build a chat state notification stanza.
///
/// This creates a `<message>` with a chat state child element and no `<body>`.
pub fn build_chat_state(from: &str, to: &str, msg_type: &str, state: ChatState) -> Element {
    Element::builder("message", ns::JABBER_CLIENT)
        .attr(ncname("from"), from)
        .attr(ncname("to"), to)
        .attr(ncname("type"), msg_type)
        .append(Element::builder(state.element_name(), ns::CHAT_STATES).build())
        .build()
}

/// Parse a chat state from a message element, if present.
pub fn parse_chat_state(element: &Element) -> Option<ChatState> {
    for child in element.children() {
        if child.ns() == ns::CHAT_STATES {
            return match child.name() {
                "active" => Some(ChatState::Active),
                "composing" => Some(ChatState::Composing),
                "paused" => Some(ChatState::Paused),
                "inactive" => Some(ChatState::Inactive),
                "gone" => Some(ChatState::Gone),
                _ => None,
            };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composing_notification() {
        let el = build_chat_state(
            "bot@example.com",
            "user@example.com",
            "chat",
            ChatState::Composing,
        );
        assert_eq!(el.name(), "message");
        let composing = el.get_child("composing", ns::CHAT_STATES);
        assert!(composing.is_some());
        // No body element
        assert!(el.get_child("body", ns::JABBER_CLIENT).is_none());
    }

    #[test]
    fn parse_active_state() {
        let el = build_chat_state(
            "bot@example.com",
            "user@example.com",
            "chat",
            ChatState::Active,
        );
        assert_eq!(parse_chat_state(&el), Some(ChatState::Active));
    }

    #[test]
    fn parse_no_state() {
        let el = Element::builder("message", ns::JABBER_CLIENT).build();
        assert_eq!(parse_chat_state(&el), None);
    }
}
