//! XEP-0045: Multi-User Chat (MUC) join/leave helpers.

use crate::minidom::Element;

use crate::stanza::{ncname, ns};

/// Build a directed presence to join a MUC room.
///
/// `room_jid_with_nick` should be `room@conference.example.com/nickname`.
pub fn build_join_presence(from: &str, room_jid_with_nick: &str) -> Element {
    let muc_ext = Element::builder("x", ns::MUC).build();
    Element::builder("presence", ns::JABBER_CLIENT)
        .attr(ncname("from"), from)
        .attr(ncname("to"), room_jid_with_nick)
        .append(muc_ext)
        .build()
}

/// Build a presence to leave a MUC room.
pub fn build_leave_presence(from: &str, room_jid_with_nick: &str) -> Element {
    Element::builder("presence", ns::JABBER_CLIENT)
        .attr(ncname("from"), from)
        .attr(ncname("to"), room_jid_with_nick)
        .attr(ncname("type"), "unavailable")
        .build()
}

/// Extract the nickname (resource part) from a full MUC JID.
///
/// e.g. `room@conference.example.com/nick` → `Some("nick")`
pub fn extract_nick(full_jid: &str) -> Option<&str> {
    full_jid.split('/').nth(1)
}

/// Check if a presence stanza indicates a MUC self-presence
/// (i.e. the server echoing our own join back to us).
pub fn is_self_presence(element: &Element) -> bool {
    if let Some(x) = element.get_child("x", ns::MUC_USER) {
        for status in x.children().filter(|c| c.name() == "status") {
            // Status code 110 means "this is your own presence"
            if status.attr("code") == Some("110") {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_presence_has_muc_extension() {
        let el = build_join_presence("bot@example.com/moltis", "room@conference.example.com/Bot");
        assert_eq!(el.name(), "presence");
        assert!(el.attr("type").is_none()); // Available presence has no type
        let x = el.get_child("x", ns::MUC);
        assert!(x.is_some());
    }

    #[test]
    fn leave_presence_is_unavailable() {
        let el = build_leave_presence("bot@example.com/moltis", "room@conference.example.com/Bot");
        assert_eq!(el.attr("type"), Some("unavailable"));
    }

    #[test]
    fn extract_nick_works() {
        assert_eq!(extract_nick("room@conference.example.com/Bot"), Some("Bot"));
        assert_eq!(extract_nick("room@conference.example.com"), None);
    }
}
