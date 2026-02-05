//! XEP-0066: Out of Band Data (media URL attachments).

use crate::minidom::Element;

use crate::stanza::{ncname, ns};

/// Build a message with an out-of-band URL attachment.
///
/// This creates a `<message>` with both a `<body>` containing the URL
/// and an `<x xmlns="jabber:x:oob"><url>` element.
pub fn build_oob_message(
    from: &str,
    to: &str,
    msg_type: &str,
    url: &str,
    description: Option<&str>,
) -> Element {
    let mut oob =
        Element::builder("x", ns::OOB).append(Element::builder("url", ns::OOB).append(url).build());

    if let Some(desc) = description {
        oob = oob.append(Element::builder("desc", ns::OOB).append(desc).build());
    }

    Element::builder("message", ns::JABBER_CLIENT)
        .attr(ncname("from"), from)
        .attr(ncname("to"), to)
        .attr(ncname("type"), msg_type)
        .append(
            Element::builder("body", ns::JABBER_CLIENT)
                .append(url)
                .build(),
        )
        .append(oob.build())
        .build()
}

/// Extract an OOB URL from a message element, if present.
pub fn parse_oob_url(element: &Element) -> Option<String> {
    element
        .get_child("x", ns::OOB)
        .and_then(|x| x.get_child("url", ns::OOB))
        .map(|url| url.text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oob_message_has_url() {
        let el = build_oob_message(
            "bot@example.com",
            "user@example.com",
            "chat",
            "https://example.com/image.png",
            Some("An image"),
        );
        let x = el.get_child("x", ns::OOB).unwrap();
        let url = x.get_child("url", ns::OOB).unwrap();
        assert_eq!(url.text(), "https://example.com/image.png");

        let desc = x.get_child("desc", ns::OOB).unwrap();
        assert_eq!(desc.text(), "An image");
    }

    #[test]
    fn parse_oob() {
        let el = build_oob_message(
            "bot@example.com",
            "user@example.com",
            "chat",
            "https://example.com/file.pdf",
            None,
        );
        assert_eq!(
            parse_oob_url(&el),
            Some("https://example.com/file.pdf".to_string())
        );
    }

    #[test]
    fn parse_no_oob() {
        let el = Element::builder("message", ns::JABBER_CLIENT).build();
        assert_eq!(parse_oob_url(&el), None);
    }
}
