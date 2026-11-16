// Adapted from MimeKit's TnefMessageConverter.HtmlReferences and IsInline.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::HashMap, io};

use fluent_uri::{Uri, UriRef};

use super::{
    TnefAttachFlags, TnefAttachMethod, TnefAttachment, TnefConversionError, TnefPropertyTag as Tag,
};
use crate::{
    encodings::Rfc2231Decoder,
    parse_utils::trim_whitespace,
    text::{HtmlToken, HtmlTokenizerReader},
};

// Store only attachment identifiers, not every URL in the HTML. Memory retained
// by the reference index is bounded by the attachment table, as in MimeKit.
#[derive(Default)]
pub(super) struct HtmlReferences {
    content_ids: HashMap<String, bool>,
    locations: HashMap<String, bool>,
    candidates: Vec<[Option<String>; 3]>,
    unresolved: usize,
}

fn content_id_key(value: &str) -> Option<String> {
    // Msg-id whitespace is ASCII. In particular, NBSP is significant. Mail
    // clients vary in local-part case, so match ASCII case-insensitively throughout.
    let value = trim_whitespace(value);
    let value = trim_whitespace(
        value
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .unwrap_or(value),
    );
    (!value.is_empty()).then(|| value.to_ascii_lowercase())
}

impl HtmlReferences {
    pub(super) fn new(attachments: &[TnefAttachment]) -> Result<Self, TnefConversionError> {
        let mut references = Self::default();
        for attachment in attachments {
            let mut keys = [None, None, None];
            // MS-OXCMAIL 2.1.3.4.1: attached Message objects SHOULD NOT be
            // inline, including afEmbeddedMessage with unrecognized content.
            // Section 2.1.3.4.1.2 requires afRenderedInBody plus a reference;
            // neither a flag nor a stored inline disposition alone suffices.
            if attachment.as_bytes().is_some()
                && !attachment.is_embedded_message()
                && attachment.method()? != TnefAttachMethod::EmbeddedMessage
                && attachment
                    .flags()?
                    .contains(TnefAttachFlags::RENDERED_IN_BODY)
            {
                keys[0] = attachment.content_id()?.as_deref().and_then(content_id_key);
                if let Some(location) = attachment
                    .properties()
                    .get_text(Tag::ATTACH_CONTENT_LOCATION_W)?
                {
                    let location = location.trim();
                    // Use the same RFC 3986 acceptance as attachment authoring.
                    // Invalid values get their loss from to_mime_part, not an
                    // unresolvable related part created from the rejected header.
                    if !location.is_empty()
                        && let Ok(reference) = UriRef::parse(location)
                    {
                        keys[1] = Some(location.to_owned());
                        // MS-OXCMSG 2.2.2.29: also match the absolute location,
                        // resolving a relative value against AttachContentBase.
                        if let Ok(uri) = Uri::parse(location) {
                            keys[2] = Some(uri.normalize().into_string());
                        } else if let Some(base) = attachment
                            .properties()
                            .get_text(Tag::ATTACH_CONTENT_BASE_W)?
                            && let Ok(base) = Uri::parse(base.trim())
                        {
                            keys[2] =
                                Some(reference.resolve_against(&base)?.normalize().into_string());
                        }
                    }
                }
                for (index, key) in keys.iter().enumerate() {
                    if let Some(key) = key {
                        let map = if index == 0 {
                            &mut references.content_ids
                        } else {
                            &mut references.locations
                        };
                        if map.insert(key.clone(), false).is_none() {
                            references.unresolved += 1;
                        }
                    }
                }
            }
            references.candidates.push(keys);
        }
        Ok(references)
    }

    pub(super) fn needs_scan(&self) -> bool {
        self.unresolved != 0
    }

    fn resolve(map: &mut HashMap<String, bool>, unresolved: &mut usize, value: &str) {
        if let Some(found) = map.get_mut(value)
            && !*found
        {
            *found = true;
            *unresolved -= 1;
        }
    }

    pub(super) fn scan(&mut self, html: &str) -> io::Result<()> {
        let mut tokenizer = HtmlTokenizerReader::from_text(html);
        while self.unresolved > 0 {
            let Some(token) = tokenizer.read_next_token()? else {
                break;
            };
            let HtmlToken::Tag(tag) = token else { continue };
            // Consider every attribute, not a fixed src/href/background list.
            // RFC 2392 cid: values contain a percent-encoded msg-id, without
            // brackets. Decode exactly once; '+' is not form-encoded whitespace.
            for attribute in tag.attributes().iter() {
                let Some(value) = attribute
                    .value()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                if value
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("cid:"))
                {
                    let mut bytes = Vec::new();
                    let mut decoder = Rfc2231Decoder::new();
                    decoder.decode(&value.as_bytes()[4..], &mut bytes);
                    decoder.finish(&mut bytes);
                    if let Ok(value) = std::str::from_utf8(&bytes)
                        && let Some(key) = content_id_key(value)
                    {
                        Self::resolve(&mut self.content_ids, &mut self.unresolved, &key);
                    }
                } else if !self.locations.is_empty() {
                    Self::resolve(&mut self.locations, &mut self.unresolved, value);
                    if let Ok(uri) = Uri::parse(value) {
                        Self::resolve(
                            &mut self.locations,
                            &mut self.unresolved,
                            uri.normalize().as_str(),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn is_referenced(&self, index: usize) -> bool {
        self.candidates[index]
            .iter()
            .enumerate()
            .any(|(index, key)| {
                let map = if index == 0 {
                    &self.content_ids
                } else {
                    &self.locations
                };
                key.as_ref().is_some_and(|key| map.get(key) == Some(&true))
            })
    }
}
