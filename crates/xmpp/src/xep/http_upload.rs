//! XEP-0363: HTTP File Upload.
//!
//! Request an upload slot via IQ, PUT the file, and return the GET URL.

use {
    crate::minidom::Element,
    anyhow::{Result, anyhow},
};

use crate::stanza::{ncname, ns};

/// An HTTP Upload slot with PUT and GET URLs.
#[derive(Debug, Clone)]
pub struct UploadSlot {
    pub put_url: String,
    pub get_url: String,
    /// Headers to include with the PUT request.
    pub put_headers: Vec<(String, String)>,
}

/// Build an IQ request to request an HTTP Upload slot.
///
/// Returns the IQ element and the id used (for correlating the response).
pub fn build_slot_request(
    from: &str,
    upload_service: &str,
    filename: &str,
    size: u64,
    content_type: Option<&str>,
) -> (Element, String) {
    let id = format!("upload-{}", uuid_simple());

    let mut request = Element::builder("request", ns::HTTP_UPLOAD)
        .attr(ncname("filename"), filename)
        .attr(ncname("size"), size.to_string());

    if let Some(ct) = content_type {
        request = request.attr(ncname("content-type"), ct);
    }

    let iq = Element::builder("iq", ns::JABBER_CLIENT)
        .attr(ncname("from"), from)
        .attr(ncname("to"), upload_service)
        .attr(ncname("type"), "get")
        .attr(ncname("id"), &id)
        .append(request.build())
        .build();

    (iq, id)
}

/// Parse an HTTP Upload slot response from an IQ result.
pub fn parse_slot_response(element: &Element) -> Result<UploadSlot> {
    let slot = element
        .get_child("slot", ns::HTTP_UPLOAD)
        .ok_or_else(|| anyhow!("missing <slot> in upload response"))?;

    let put = slot
        .get_child("put", ns::HTTP_UPLOAD)
        .ok_or_else(|| anyhow!("missing <put> in upload slot"))?;
    let get = slot
        .get_child("get", ns::HTTP_UPLOAD)
        .ok_or_else(|| anyhow!("missing <get> in upload slot"))?;

    let put_url = put
        .attr("url")
        .ok_or_else(|| anyhow!("missing url attribute on <put>"))?
        .to_string();
    let get_url = get
        .attr("url")
        .ok_or_else(|| anyhow!("missing url attribute on <get>"))?
        .to_string();

    // Parse optional PUT headers.
    let put_headers: Vec<(String, String)> = put
        .children()
        .filter(|c| c.name() == "header")
        .filter_map(|h| {
            let name = h.attr("name")?.to_string();
            let value = h.text();
            Some((name, value))
        })
        .collect();

    Ok(UploadSlot {
        put_url,
        get_url,
        put_headers,
    })
}

/// Upload a file using the slot, then return the GET URL.
pub async fn upload_file(
    client: &reqwest::Client,
    slot: &UploadSlot,
    data: Vec<u8>,
    content_type: &str,
) -> Result<String> {
    let mut req = client
        .put(&slot.put_url)
        .header("Content-Type", content_type)
        .body(data);

    for (name, value) in &slot.put_headers {
        req = req.header(name.as_str(), value.as_str());
    }

    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "HTTP Upload PUT failed with status {}",
            resp.status()
        ));
    }

    Ok(slot.get_url.clone())
}

/// Generate a simple unique ID (not a full UUID, just enough for IQ correlation).
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{ts:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_request_structure() {
        let (iq, id) = build_slot_request(
            "bot@example.com",
            "upload.example.com",
            "photo.jpg",
            12345,
            Some("image/jpeg"),
        );
        assert_eq!(iq.name(), "iq");
        assert_eq!(iq.attr("type"), Some("get"));
        assert_eq!(iq.attr("id"), Some(id.as_str()));
        let request = iq.get_child("request", ns::HTTP_UPLOAD).unwrap();
        assert_eq!(request.attr("filename"), Some("photo.jpg"));
        assert_eq!(request.attr("size"), Some("12345"));
        assert_eq!(request.attr("content-type"), Some("image/jpeg"));
    }

    #[test]
    fn parse_slot() {
        let slot_xml = Element::builder("iq", ns::JABBER_CLIENT)
            .attr(ncname("type"), "result")
            .append(
                Element::builder("slot", ns::HTTP_UPLOAD)
                    .append(
                        Element::builder("put", ns::HTTP_UPLOAD)
                            .attr(ncname("url"), "https://upload.example.com/put/abc")
                            .build(),
                    )
                    .append(
                        Element::builder("get", ns::HTTP_UPLOAD)
                            .attr(ncname("url"), "https://upload.example.com/get/abc")
                            .build(),
                    )
                    .build(),
            )
            .build();

        let slot = parse_slot_response(&slot_xml).unwrap();
        assert_eq!(slot.put_url, "https://upload.example.com/put/abc");
        assert_eq!(slot.get_url, "https://upload.example.com/get/abc");
    }
}
