// Adapted from MimeKit's TnefMessageConverter MIME skeleton restoration.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::HashMap, io::Cursor};

use super::*;
use crate::{MimeContent, MimeParser, MimePart};

const SKELETON_ID: &str = "X-Exchange-MIME-Skeleton-Content-Id";

fn normalize_id(value: &str) -> Option<&str> {
    let value = crate::parse_utils::trim_whitespace(value);
    let value = value
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .map(crate::parse_utils::trim_whitespace)
        .unwrap_or(value);
    (!value.is_empty()).then_some(value)
}

impl Converter<'_> {
    pub(super) fn try_skeleton(
        &mut self,
        bytes: &[u8],
        rtf: Option<&[u8]>,
        options: &TnefConversionOptions,
    ) -> Result<Option<MimeMessage>, TnefConversionError> {
        let checkpoint = self.losses.len();
        let parsed = MimeParser::new(bytes).parse_message();
        let mut message = match parsed {
            Ok(message) => message,
            Err(error) => {
                self.loss(
                    Loss::InvalidMimeSkeleton,
                    format!("The MIME skeleton could not be parsed: {error}"),
                );
                return Ok(None);
            }
        };
        // The tolerant MIME parser also accepts arbitrary headerless content.
        // A skeleton must begin with headers or an explicit empty header block.
        if message.headers().iter().any(Header::is_invalid)
            || (message.headers().is_empty()
                && message.body().is_none_or(|body| body.headers().is_empty())
                && !bytes.starts_with(b"\r\n")
                && !bytes.starts_with(b"\n"))
        {
            self.loss(
                Loss::InvalidMimeSkeleton,
                "The MIME skeleton could not be parsed as message headers.",
            );
            return Ok(None);
        }
        // RFC 2045 5.2: headers-only messages still have an implicit text/plain
        // body whose promoted content may be supplied by PidTagBody.
        if message.body().is_none() {
            message.set_body(Some(
                MimePart::from_parts(HeaderList::new(), MimeContent::new(Cursor::new(Vec::new())))
                    .into(),
            ));
        }

        let mut ids = HashMap::new();
        for (index, attachment) in self.source.attachments().iter().enumerate() {
            if let Some(id) = attachment.content_id()?.as_deref().and_then(normalize_id) {
                // Skeleton matching is ordinal and case-sensitive. The first
                // duplicate wins; every content-bearing attachment must be used.
                ids.entry(id.to_owned()).or_insert(index);
            }
        }
        let body_id = self.source.properties().get_text(Tag::BODY_CONTENT_ID_W)?;
        let body_id = body_id.as_deref().and_then(normalize_id);
        let mut consumed = vec![false; self.source.attachments().len()];
        let mut used_bodies = [false; 3];
        let mut reason = None;
        // Walk multipart structure iteratively. Existing message/rfc822 content
        // is not part of this message's attachment table.
        let mut pending = vec![message.body_mut().expect("implicit or explicit body")];
        while let Some(entity) = pending.pop() {
            if entity.headers().iter().any(Header::is_invalid) {
                reason = Some("it contains invalid MIME headers".to_owned());
                break;
            }
            if let MimeEntity::Multipart(multipart) = entity {
                pending.extend(multipart.children_mut().iter_mut().rev());
                continue;
            }
            // This synthetic header was created only when the original part
            // had no Content-Id. Remove it without inventing a real Content-Id.
            let synthetic = entity.headers_mut().remove(SKELETON_ID);
            entity.headers_mut().remove_all(SKELETON_ID);
            // MimeKit's header lookup decodes encoded words before matching.
            let synthetic = synthetic.map(|header| header.decode_value());
            if let Some(value) = &synthetic
                && !value.issues.is_empty()
            {
                reason = Some(format!(
                    "a skeleton content identifier could not be decoded without recovery: {:?}",
                    value.issues
                ));
                break;
            }
            let synthetic_id = synthetic
                .as_ref()
                .and_then(|value| normalize_id(&value.text));
            let content_id = if synthetic_id.is_some() {
                None
            } else {
                match entity.content_id() {
                    Ok(id) => id,
                    Err(error) => {
                        reason = Some(format!("a Content-Id is invalid: {error}"));
                        break;
                    }
                }
            };
            let id = synthetic_id
                .or_else(|| content_id.as_ref().and_then(|id| normalize_id(id.as_str())));
            let index = id
                .and_then(|id| ids.get(id))
                .copied()
                .filter(|&index| !consumed[index]);
            if let Some(index) = index {
                consumed[index] = true;
            }
            if let MimeEntity::Message(part) = entity {
                if let Some(index) = index {
                    let attachment = &self.source.attachments()[index];
                    if !attachment.is_embedded_message() {
                        reason = Some(format!(
                            "message/rfc822 attachment {index} is not an embedded message"
                        ));
                        break;
                    }
                    // Skeleton structure requires conversion even when the
                    // property-path ConvertEmbeddedMessages option is false.
                    let (embedded, losses) = self.convert_embedded(attachment, options)?;
                    let Some(embedded) = embedded else {
                        reason = Some(format!("embedded attachment {index} could not be loaded"));
                        break;
                    };
                    self.losses.extend(losses.into_iter().map(|mut loss| {
                        loss.attachment_index = Some(index);
                        loss
                    }));
                    part.set_message(Some(embedded));
                }
                continue;
            }
            let MimeEntity::Part(part) = entity else {
                unreachable!("leaf or message")
            };
            // Unpromoted content (calendar, vCard, delivery-status, etc.) stays
            // untouched but can still claim a matching attachment.
            if !part.content().get_ref().get_ref().is_empty() {
                continue;
            }
            if let Some(index) = index {
                let Some(bytes) = self.source.attachments()[index].as_bytes() else {
                    reason = Some(format!("attachment {index} has no content"));
                    break;
                };
                part.set_content(MimeContent::new(Cursor::new(bytes.to_vec())));
                continue;
            }
            if let (Some(id), Some(body_id)) = (id, body_id)
                && id != body_id
            {
                reason = Some(format!("no attachment has the content identifier <{id}>"));
                break;
            }
            let mut ty = match part.content_type() {
                Ok(Some(ty)) => ty,
                Ok(None) => ContentType::new("text", "plain").expect("default type"),
                Err(error) => {
                    reason = Some(format!("a Content-Type is invalid: {error}"));
                    break;
                }
            };
            let slot = if ty.is_mime_type("text", "plain") {
                Some(0)
            } else if ty.is_mime_type("text", "html") {
                Some(1)
            } else if ty.is_mime_type("text", "rtf") || ty.is_mime_type("application", "rtf") {
                Some(2)
            } else {
                None
            };
            let slot = slot.filter(|&index| !used_bodies[index]);
            let content = match slot {
                Some(0 | 1) => {
                    let body = if slot == Some(0) {
                        self.source.text_body()
                    } else {
                        self.source.html_body()
                    };
                    if let Some(body) = body {
                        let generated = self.body_part(
                            if slot == Some(0) { "plain" } else { "html" },
                            body,
                            options,
                        )?;
                        ty.set_charset(
                            generated
                                .content_type()
                                .expect("authored body type")
                                .expect("body type")
                                .charset(),
                        );
                        Some(generated.into_parts().1)
                    } else {
                        None
                    }
                }
                // RTF declares its own charset. Reuse decoded bytes and leave
                // any original MIME parameters alone; add no charset parameter.
                Some(2) => rtf.map(|bytes| MimeContent::new(Cursor::new(bytes.to_vec()))),
                _ => None,
            };
            let Some(content) = content else {
                reason = Some(format!(
                    "the empty {} part does not correspond to a body",
                    ty.mime_type()
                ));
                break;
            };
            used_bodies[slot.expect("matched body")] = true;
            part.set_content_type(&ty)?;
            // Preserve skeleton transfer encoding; the new content is decoded.
            part.set_content(content);
        }
        if reason.is_none() {
            for (index, attachment) in self.source.attachments().iter().enumerate() {
                if attachment.has_content() && !consumed[index] {
                    reason = Some(format!("it does not contain attachment {index}"));
                    break;
                }
            }
        }
        if let Some(reason) = reason {
            // Discard tentative embedded losses with the untrusted skeleton.
            // Previously recorded RTF preparation losses must survive fallback.
            self.losses.truncate(checkpoint);
            self.loss(
                Loss::InvalidMimeSkeleton,
                format!("The MIME skeleton was ignored because {reason}."),
            );
            return Ok(None);
        }
        for (index, attachment) in self.source.attachments().iter().enumerate() {
            if !attachment.has_content() {
                self.losses
                    .extend(
                        attachment
                            .to_mime_part()?
                            .losses
                            .into_iter()
                            .map(|mut loss| {
                                loss.attachment_index = Some(index);
                                loss
                            }),
                    );
            }
        }
        let previous = std::mem::replace(&mut self.message, message);
        self.importance()?;
        self.message_flag()?;
        Ok(Some(std::mem::replace(&mut self.message, previous)))
    }
}
