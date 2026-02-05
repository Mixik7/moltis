//! XEP-0444: Message Reactions.

use crate::minidom::Element;

use crate::stanza::{ncname, ns};

/// Build a reaction stanza.
///
/// `message_id` is the id attribute of the message being reacted to.
/// `emojis` is the set of reaction emojis to send.
pub fn build_reaction(
    from: &str,
    to: &str,
    msg_type: &str,
    message_id: &str,
    emojis: &[&str],
) -> Element {
    let mut reactions = Element::builder("reactions", ns::REACTIONS).attr(ncname("id"), message_id);

    for emoji in emojis {
        reactions = reactions.append(
            Element::builder("reaction", ns::REACTIONS)
                .append(*emoji)
                .build(),
        );
    }

    Element::builder("message", ns::JABBER_CLIENT)
        .attr(ncname("from"), from)
        .attr(ncname("to"), to)
        .attr(ncname("type"), msg_type)
        .append(reactions.build())
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_reaction() {
        let el = build_reaction("bot@example.com", "user@example.com", "chat", "msg-123", &[
            "\u{1f44d}",
        ]);
        let reactions = el.get_child("reactions", ns::REACTIONS).unwrap();
        assert_eq!(reactions.attr("id"), Some("msg-123"));
        let reaction_children: Vec<_> = reactions
            .children()
            .filter(|c| c.name() == "reaction")
            .collect();
        assert_eq!(reaction_children.len(), 1);
        assert_eq!(reaction_children[0].text(), "\u{1f44d}");
    }

    #[test]
    fn multiple_reactions() {
        let el = build_reaction("bot@example.com", "user@example.com", "chat", "msg-456", &[
            "\u{1f44d}",
            "\u{2764}",
        ]);
        let reactions = el.get_child("reactions", ns::REACTIONS).unwrap();
        let children: Vec<_> = reactions
            .children()
            .filter(|c| c.name() == "reaction")
            .collect();
        assert_eq!(children.len(), 2);
    }
}
