// Adapted from MimeKit's MimeMessage.cs, Multipart.cs, MultipartAlternative.cs,
// MultipartRelated.cs, and TextPart.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, ops::Range, slice};

use crate::{
    ContentHeaderParseError, ContentType, MessageId, MessageIdParseError, MimeEntity, Multipart,
    MultipartError, References, TextPartError,
};

/// A text format for body selection, classification, or part construction.
///
/// These operations do not convert content between formats. Body selection
/// matches MIME types; [`crate::TextPart::format`] additionally returns Plain
/// as a classification fallback for unknown text subtypes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextFormat {
    /// `text/plain`, including flowed text.
    Plain,
    /// `text/plain` with a `format=flowed` parameter.
    Flowed,
    /// `text/html`.
    Html,
    /// `text/enriched` or the older `text/richtext` format.
    Enriched,
    /// `text/rtf` or `application/rtf`.
    RichText,
}

impl TextFormat {
    pub(crate) fn matches(self, content_type: &ContentType) -> bool {
        match self {
            Self::Plain => content_type.is_mime_type("text", "plain"),
            Self::Flowed => {
                content_type.is_mime_type("text", "plain")
                    && content_type
                        .parameters()
                        .get("format")
                        .is_some_and(|value| value.trim().eq_ignore_ascii_case("flowed"))
            }
            Self::Html => content_type.is_mime_type("text", "html"),
            Self::Enriched => {
                content_type.is_mime_type("text", "enriched")
                    || content_type.is_mime_type("text", "richtext")
            }
            Self::RichText => {
                content_type.is_mime_type("text", "rtf")
                    || content_type.is_mime_type("application", "rtf")
            }
        }
    }
}

struct Frame<'a, R> {
    children: &'a [MimeEntity<R>],
    remaining: Range<usize>,
    depth: usize,
    filter_attachment: bool,
    default_message: bool,
}

// Paths include an artificial root index, so all frames use the same rules.
pub(crate) fn select<R>(
    root: &MimeEntity<R>,
    format: TextFormat,
) -> Result<Option<Vec<usize>>, TextBodyError> {
    let mut stack = vec![Frame {
        children: slice::from_ref(root),
        remaining: 0..1,
        depth: 0,
        filter_attachment: true,
        default_message: false,
    }];
    let mut path = Vec::new();
    while let Some(frame) = stack.last_mut() {
        let Some(index) = frame.remaining.next_back() else {
            stack.pop();
            continue;
        };
        path.truncate(frame.depth);
        path.push(index);
        let entity = &frame.children[index];
        match entity {
            MimeEntity::Part(_) => {
                if format.matches(&effective_type(entity, frame.default_message)?)
                    && (!frame.filter_attachment
                        || !entity.is_attachment().map_err(TextBodyError::Disposition)?)
                {
                    return Ok(Some(path));
                }
            }
            MimeEntity::Multipart(part) => {
                let content_type = effective_type(entity, frame.default_message)?;
                let digest = content_type.is_mime_type("multipart", "digest");
                let remaining = if content_type.is_mime_type("multipart", "alternative") {
                    // Prefer the last (most faithful) suitable alternative.
                    0..part.children().len()
                } else {
                    let index = if content_type.is_mime_type("multipart", "related") {
                        // Only the root document is a body candidate, not its
                        // related resources. The root may itself be an alternative.
                        related_root(part, &content_type)?
                    } else {
                        first_body_child(part, digest)?
                    };
                    index.map_or(0..0, |index| index..index + 1)
                };
                stack.push(Frame {
                    children: part.children(),
                    remaining,
                    depth: path.len(),
                    filter_attachment: false,
                    default_message: digest,
                });
            }
            MimeEntity::Message(_) => {}
        }
    }
    Ok(None)
}

fn effective_type<R>(
    entity: &MimeEntity<R>,
    default_message: bool,
) -> Result<ContentType, TextBodyError> {
    let content_type = entity
        .headers()
        .get("Content-Type")
        .map(|header| header.parse_content_type())
        .transpose()
        .map_err(TextBodyError::ContentType)?;
    if matches!(entity, MimeEntity::Multipart(_)) {
        let value =
            content_type.ok_or(TextBodyError::Multipart(MultipartError::MissingContentType))?;
        crate::multipart::check_multipart_type(&value).map_err(TextBodyError::Multipart)?;
        return Ok(value);
    }
    Ok(content_type.unwrap_or_else(|| {
        let (media_type, subtype) = if default_message || matches!(entity, MimeEntity::Message(_)) {
            ("message", "rfc822")
        } else {
            ("text", "plain")
        };
        ContentType::new(media_type, subtype).expect("literal default media types are valid")
    }))
}

fn first_body_child<R>(part: &Multipart<R>, digest: bool) -> Result<Option<usize>, TextBodyError> {
    for (index, child) in part.children().iter().enumerate() {
        match child {
            // The body cannot follow a multipart, even if it has no match.
            MimeEntity::Multipart(_) => return Ok(Some(index)),
            MimeEntity::Part(_) => {
                let content_type = effective_type(child, digest)?;
                let text = content_type.is_mime_type("text", "*")
                    || content_type.is_mime_type("application", "rtf");
                if text && !child.is_attachment().map_err(TextBodyError::Disposition)? {
                    // The first non-attachment text is the body, even in another format.
                    return Ok(Some(index));
                }
            }
            MimeEntity::Message(_) => {}
        }
    }
    Ok(None)
}

pub(crate) fn related_root<R>(
    part: &Multipart<R>,
    content_type: &ContentType,
) -> Result<Option<usize>, TextBodyError> {
    Ok(related_root_match(part, content_type)?
        .or_else(|| (!part.children().is_empty()).then_some(0)))
}

pub(crate) fn related_root_match<R>(
    part: &Multipart<R>,
    content_type: &ContentType,
) -> Result<Option<usize>, TextBodyError> {
    if let Some(start) = content_type.parameters().get("start") {
        let reference = References::new(start.as_bytes())
            .next()
            .transpose()
            .map_err(TextBodyError::RelatedStart)?;
        let start = reference.as_ref().map_or(start, MessageId::as_str);
        for (index, child) in part.children().iter().enumerate() {
            if child
                .content_id()
                .map_err(TextBodyError::ContentId)?
                .is_some_and(|id| id.as_str() == start)
            {
                return Ok(Some(index));
            }
        }
    } else if let Some(media_type) = content_type.parameters().get("type") {
        for (index, child) in part.children().iter().enumerate() {
            if effective_type(child, false)?
                .mime_type()
                .eq_ignore_ascii_case(media_type)
            {
                return Ok(Some(index));
            }
        }
    }
    Ok(None)
}

/// A text body could not be selected or decoded.
///
/// Selection parses only visited metadata and does no source I/O. Decoded text
/// access can additionally report charset and I/O failures. Neither operation
/// falls back to another candidate after an error or mutates raw headers.
#[derive(Debug)]
pub enum TextBodyError {
    /// A visited first Content-Type could not be parsed.
    ContentType(ContentHeaderParseError),
    /// A disposition needed to exclude attachments could not be parsed.
    Disposition(ContentHeaderParseError),
    /// A multipart node has missing or non-multipart Content-Type metadata.
    Multipart(MultipartError),
    /// A Content-Id examined during related-root lookup could not be parsed.
    ContentId(MessageIdParseError),
    /// A bracketed reference in a related `start` parameter is malformed.
    RelatedStart(MessageIdParseError),
    /// Decoding the selected body's charset or reading/repositioning it failed.
    Text(TextPartError),
}

impl fmt::Display for TextBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContentType(error) => write!(f, "invalid body Content-Type: {error}"),
            Self::Disposition(error) => write!(f, "invalid body Content-Disposition: {error}"),
            Self::Multipart(error) => error.fmt(f),
            Self::ContentId(error) => write!(f, "invalid related Content-Id: {error}"),
            Self::RelatedStart(error) => write!(f, "invalid related start reference: {error}"),
            Self::Text(error) => error.fmt(f),
        }
    }
}

impl Error for TextBodyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentType(error) | Self::Disposition(error) => Some(error),
            Self::Multipart(error) => Some(error),
            Self::ContentId(error) | Self::RelatedStart(error) => Some(error),
            Self::Text(error) => Some(error),
        }
    }
}
