// Adapted from MimeKit's TnefMessageConverter RTF attachment/image helpers.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::HashMap, io::Cursor};

use super::{TnefAttachMethod, TnefAttachment, TnefConversionError, TnefPropertyTag as Tag};
use crate::{MessageId, MimeEntity, text::HtmlWriter};

pub(super) fn sniff_image_subtype(bytes: &[u8]) -> Option<&'static str> {
    // PNG (ISO/IEC 15948, 5.2): the complete eight-byte signature.
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    // JPEG (ITU-T T.81, B.1.1.3): SOI followed by another marker.
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("gif");
    }
    // WebP: a RIFF container whose form type is WEBP.
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Some("webp");
    }
    // BM alone is too weak: require a defined DIB header size as well.
    if bytes.starts_with(b"BM") && bytes.len() >= 18 {
        let size = u32::from_le_bytes(bytes[14..18].try_into().expect("four-byte DIB size"));
        if matches!(size, 12 | 40 | 52 | 56 | 64 | 108 | 124) {
            return Some("bmp");
        }
    }
    None
}

fn content_id_url(id: &str) -> String {
    // RFC 2392 section 2: omit the brackets and percent-encode UTF-8 bytes
    // outside RFC 3986 pchar, including literal '%' (not an existing escape).
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut url = String::from("cid:");
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push('%');
            url.push(char::from(HEX[usize::from(byte >> 4)]));
            url.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    url
}

pub(super) fn rtf_placeholder_order(
    attachments: &[TnefAttachment],
) -> Result<Vec<usize>, TnefConversionError> {
    // MS-OXRTFEX 2.2.3.4: Outlook writes \objattph where each attachment
    // appears in the body. Hidden attachments and position 0xFFFFFFFF are
    // excluded. MS-OXCMAIL 2.1.3.4.1.1 matches the lowest unsigned position
    // to the first placeholder; absent positions sort last. The original
    // index makes ties stable. Only browser-displayable images produce markup;
    // other placeholders produce nothing under the default conversion policy.
    let mut positions = Vec::new();
    for (index, attachment) in attachments.iter().enumerate() {
        let properties = attachment.properties();
        if properties.get_bool(Tag::ATTACHMENT_HIDDEN)? == Some(true) {
            continue;
        }
        let position = properties.get_i32(Tag::RENDERING_POSITION)?;
        if position != Some(-1) {
            positions.push((position.map_or(u32::MAX, |value| value as u32), index));
        }
    }
    positions.sort_unstable();
    Ok(positions.into_iter().map(|(_, index)| index).collect())
}

pub(super) fn prepare_rtf_images(
    attachments: &[TnefAttachment],
    parts: &mut [Option<MimeEntity<Cursor<Vec<u8>>>>],
    displayed: &mut [bool],
    order: &[usize],
) -> Result<Vec<Option<String>>, TnefConversionError> {
    let mut content_ids = HashMap::<String, usize>::new();
    for part in parts.iter().flatten() {
        if let Some(id) = part.content_id().expect("authored Content-Id") {
            *content_ids
                .entry(id.as_str().to_ascii_lowercase())
                .or_default() += 1;
        }
    }

    let mut images = Vec::with_capacity(order.len());
    for &index in order {
        // Missing content and attachments exported to calendar still occupy
        // slots, so later images remain aligned with their placeholders.
        images.push(None);
        let Some(MimeEntity::Part(part)) = &mut parts[index] else {
            continue;
        };
        // Attached Message objects never become inline, including invalid
        // afEmbeddedMessage content retained as an ordinary MIME part.
        if attachments[index].is_embedded_message()
            || attachments[index].method()? == TnefAttachMethod::EmbeddedMessage
        {
            continue;
        }
        let mut content_type = part
            .content_type()
            .expect("authored Content-Type")
            .expect("attachment Content-Type");
        // Sniff content rather than trusting AttachMimeTag or a filename.
        // A declared non-image type must not contradict display as an image.
        if !content_type.is_mime_type("image", "*")
            && !content_type.is_mime_type("application", "octet-stream")
        {
            continue;
        }
        let Some(subtype) = sniff_image_subtype(part.content().get_ref().get_ref()) else {
            continue;
        };
        content_type
            .set_media_type("image")
            .expect("constant media type");
        content_type
            .set_media_subtype(subtype)
            .expect("sniffed image subtype");
        part.set_content_type(&content_type)?;

        // RFC 2392 requires an unambiguous ID. Include ordinary and hidden
        // attachments in the collision counts, not just displayed images.
        let mut id = part.content_id().expect("authored Content-Id");
        if id
            .as_ref()
            .is_none_or(|id| content_ids[&id.as_str().to_ascii_lowercase()] > 1)
        {
            let generated = loop {
                let generated = MessageId::generate()?;
                if !content_ids.contains_key(&generated.as_str().to_ascii_lowercase()) {
                    break generated;
                }
            };
            if let Some(id) = &id {
                *content_ids
                    .get_mut(&id.as_str().to_ascii_lowercase())
                    .expect("counted ID") -= 1;
            }
            content_ids.insert(generated.as_str().to_ascii_lowercase(), 1);
            part.set_content_id(Some(generated.as_str()))
                .expect("generated valid ID");
            id = Some(generated);
        }

        let mut writer = HtmlWriter::new();
        writer.write_empty_element_tag("img").expect("constant tag");
        writer
            .write_attribute(
                "src",
                &content_id_url(id.as_ref().expect("image ID").as_str()),
            )
            .expect("attribute in image tag");
        if let Some(name) = part
            .file_name()
            .expect("authored filename")
            .filter(|name| !name.is_empty())
        {
            writer
                .write_attribute("alt", &name)
                .expect("attribute in image tag");
        }
        *images.last_mut().expect("image slot") = Some(writer.finish());
        displayed[index] = true;
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        charsets::Charset,
        tnef::{TnefAttachmentContent, TnefProperty, TnefPropertyInfo, TnefPropertySet, TnefValue},
    };

    #[test]
    fn absent_positions_sort_last_and_unnamed_images_omit_alt() {
        let attachments: Vec<_> = [None, Some(-2), Some(1)]
            .into_iter()
            .map(|position| {
                let properties: TnefPropertySet = position
                    .into_iter()
                    .map(|position| {
                        TnefProperty::new(
                            TnefPropertyInfo {
                                tag: Tag::RENDERING_POSITION,
                                name: None,
                                offset: 0,
                            },
                            vec![TnefValue::I32(position)],
                            Charset::UTF8,
                        )
                    })
                    .collect();
                TnefAttachment::new(
                    properties,
                    Some(TnefAttachmentContent::Binary(b"GIF89a".to_vec())),
                )
            })
            .collect();
        let mut parts: Vec<_> = attachments
            .iter()
            .map(|attachment| attachment.to_mime_part().unwrap().part.map(Into::into))
            .collect();
        let mut displayed = [false; 3];
        let order = rtf_placeholder_order(&attachments).unwrap();
        let images = prepare_rtf_images(&attachments, &mut parts, &mut displayed, &order).unwrap();
        assert_eq!(displayed, [true; 3]);
        for (image, index) in images.iter().zip([2, 1, 0]) {
            let id = parts[index]
                .as_ref()
                .unwrap()
                .content_id()
                .unwrap()
                .unwrap();
            assert_eq!(
                image.as_deref(),
                Some(format!("<img src=\"cid:{}\"/>", id.as_str()).as_str())
            );
        }
    }
}
