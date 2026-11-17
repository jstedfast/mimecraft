// Adapted from MimeKit's TnefOleObjectConverter and CreateOleImageAttachment.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read};

use super::rtf_images::sniff_image_subtype;
use super::{
    TnefAttachment, TnefAttachmentConversionResult, TnefConversionError, TnefPropertyTag as Tag,
};
use crate::{ContentDisposition, ContentType};

/// Caller-provided rendering of an OLE attachment into PNG, JPEG, GIF, BMP, or WebP.
///
/// Store a shared instance in [`super::TnefConversionOptions::ole_object_converter`].
/// Options clones and recursive conversions reuse that instance. Implementations
/// that keep mutable state must synchronize it. Closures with this signature
/// also implement the trait.
///
/// Return `None` when rendering is unavailable. Returned streams are read from
/// their current position to EOF and dropped, including on failure; they need
/// not be seekable. Unrecognized image signatures retain the original OLE data
/// with a loss. Renderer/read errors abort conversion rather than masquerading
/// as an unavailable image. Mimecraft does not launch an OLE application.
/// Rendered output is collected in memory without an additional size limit;
/// implementations should bound their output and avoid untrusted external actions.
///
/// ```
/// use std::{io::{self, Read}, sync::Arc};
/// use mimecraft::tnef::{TnefAttachment, TnefConversionOptions};
///
/// fn render(_attachment: &TnefAttachment) -> io::Result<Option<Box<dyn Read>>> {
///     // Supply an owned image stream here, or decline this object.
///     Ok(None)
/// }
/// let options = TnefConversionOptions {
///     convert_embedded_messages: true,
///     ole_object_converter: Some(Arc::new(render)),
///     ..Default::default()
/// };
/// let nested_options = options.clone(); // Shares the renderer.
/// ```
pub trait TnefOleObjectConverter: Send + Sync {
    /// Renders one attachment; the source remains unchanged.
    fn convert(&self, attachment: &TnefAttachment) -> io::Result<Option<Box<dyn Read>>>;
}

impl<F> TnefOleObjectConverter for F
where
    F: Fn(&TnefAttachment) -> io::Result<Option<Box<dyn Read>>> + Send + Sync,
{
    fn convert(&self, attachment: &TnefAttachment) -> io::Result<Option<Box<dyn Read>>> {
        self(attachment)
    }
}

pub(super) fn convert(
    converter: &dyn TnefOleObjectConverter,
    attachment: &TnefAttachment,
) -> Result<Option<TnefAttachmentConversionResult>, TnefConversionError> {
    let Some(mut stream) = converter.convert(attachment)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    drop(stream);
    let Some(subtype) = sniff_image_subtype(&bytes) else {
        return Ok(None);
    };
    let mut result = attachment.to_mime_part_with_content(
        ContentType::new("image", subtype).expect("sniffed media type"),
        &bytes,
    )?;
    let part = result.part.as_mut().expect("OLE attachment has content");
    // MS-OXCMAIL 2.1.3.4.4: use DisplayName with the image extension for
    // name, filename and description. The old size describes the OLE object,
    // not the rendering, and MUST NOT be carried over.
    let display = attachment.properties().get_text(Tag::DISPLAY_NAME_W)?;
    let filename = attachment.file_name()?;
    let mut name = display
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| filename.as_deref().filter(|name| !name.trim().is_empty()))
        .unwrap_or("image")
        .to_owned();
    let extension = if subtype == "jpeg" {
        ".jpg".to_owned()
    } else {
        format!(".{subtype}")
    };
    if !name.to_ascii_lowercase().ends_with(&extension) {
        name.push_str(&extension);
    }
    let mut content_type = part
        .content_type()
        .expect("authored type")
        .expect("OLE image type");
    content_type.set_name(Some(&name));
    part.set_content_type(&content_type)?;
    let mut disposition = part
        .content_disposition()
        .expect("authored disposition")
        .unwrap_or_else(ContentDisposition::default);
    disposition.set_file_name(Some(&name));
    disposition.set_size(None).expect("removing size");
    part.set_content_disposition(Some(&disposition))?;
    part.set_content_description(Some(&name))?;
    Ok(Some(result))
}
