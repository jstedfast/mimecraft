// Adapted from MimeKit's Text/HtmlTagContext.cs and RtfToHtml.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{HtmlAttributeCollection, HtmlTagId, HtmlWriter, HtmlWriterError};

/// A generated HTML tag presented to a converter's tag callback.
///
/// The callback owns the writing decision: call [`Self::write_tag`] to keep
/// the tag, write a replacement with [`HtmlWriter`], or write nothing to omit it.
/// Nothing is written automatically when a start-tag callback returns.
/// Omitting a start tag does not automatically omit its end tag.
///
/// Tag identity and source attributes are read-only. To change attributes,
/// write the tag without its attributes, then write selected or replacement
/// attributes through the writer. To rename an element, write a different
/// start tag and request its end-tag callback to write the matching end tag.
///
/// The converter retains this context until the generated element closes.
/// Its end-tag callback sees the same attributes and flags, with
/// [`Self::is_end_tag`] set to true. Empty elements do not receive end callbacks.
/// Callback-written markup is trusted, not parsed, filtered, or stack-tracked.
///
/// ```
/// use mimecraft::text::{HtmlTagId, RtfToHtml};
///
/// let mut converter = RtfToHtml::new();
/// converter.output_html_fragment = true;
/// let html = converter.convert_with_tag_callback(
///     &b"{\\rtf1 hello}"[..],
///     |context, writer| {
///         if context.tag_id() == HtmlTagId::Div {
///             if context.is_end_tag() {
///                 writer.write_end_tag("p").unwrap();
///             } else {
///                 writer.write_start_tag("p").unwrap();
///                 context.invoke_callback_for_end_tag = true;
///             }
///         } else {
///             context.write_tag(writer, true).unwrap();
///         }
///     },
/// )?;
/// assert_eq!(html, "<p>hello</p>\r\n");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct HtmlTagContext {
    tag_id: HtmlTagId,
    attributes: HtmlAttributeCollection,
    is_end_tag: bool,
    /// Marks the callback's decision to delete this tag.
    ///
    /// For RTF rendering this is bookkeeping only: omit the writer call to
    /// omit the start tag, and set [`Self::delete_end_tag`] separately.
    /// Setting this flag never removes markup already written by the callback.
    /// [`super::TextToHtml`] also treats it as bookkeeping.
    /// [`super::FlowedToHtml`] uses it to suppress a paragraph's markup newline,
    /// checking it after any end-tag callback.
    pub delete_tag: bool,
    /// Omits the generated end tag and its callback, even if one was requested.
    ///
    /// For RTF block tags, also omits the associated generated newline.
    pub delete_end_tag: bool,
    /// Requests a callback instead of automatic writing of the end tag.
    ///
    /// Defaults to false. Ignored for empty elements and deleted end tags.
    pub invoke_callback_for_end_tag: bool,
    /// Suppresses this element's inner text, descendant tags and their callbacks.
    ///
    /// Does not suppress this element's own end tag or end-tag callback.
    /// RTF placeholder callbacks inside the element are also suppressed, but
    /// their indices still advance. Interpretation and diagnostics continue.
    pub suppress_inner_content: bool,
}

impl HtmlTagContext {
    pub(super) fn new(tag_id: HtmlTagId, attributes: HtmlAttributeCollection) -> Self {
        Self {
            tag_id,
            attributes,
            is_end_tag: false,
            delete_tag: false,
            delete_end_tag: false,
            invoke_callback_for_end_tag: false,
            suppress_inner_content: false,
        }
    }

    /// Returns the generated tag's identifier, not a callback's replacement.
    pub fn tag_id(&self) -> HtmlTagId {
        self.tag_id
    }

    /// Returns the generated tag's canonical name.
    pub fn tag_name(&self) -> &str {
        self.tag_id.as_str()
    }

    /// Returns the original attributes in order, also for an end-tag callback.
    pub fn attributes(&self) -> &HtmlAttributeCollection {
        &self.attributes
    }

    /// Returns whether the generated tag is an empty element, such as `br`.
    pub fn is_empty_element(&self) -> bool {
        self.tag_id.is_empty_element()
    }

    /// Returns whether the converter is closing this element.
    pub fn is_end_tag(&self) -> bool {
        self.is_end_tag
    }

    pub(super) fn mark_end_tag(&mut self) {
        self.is_end_tag = true;
    }

    /// Writes the original tag, optionally including its original attributes.
    ///
    /// Start tags remain open for additional writer attributes. End tags are
    /// complete and never include attributes, regardless of `write_attributes`.
    /// Does not inspect deletion or suppression flags: the callback decides
    /// whether to call this method.
    ///
    /// # Errors
    ///
    /// Returns errors from [`HtmlWriter`]'s tag and attribute operations.
    pub fn write_tag(
        &self,
        writer: &mut HtmlWriter,
        write_attributes: bool,
    ) -> Result<(), HtmlWriterError> {
        if self.is_end_tag {
            return writer.write_end_tag(self.tag_name());
        }
        if self.is_empty_element() {
            writer.write_empty_element_tag(self.tag_name())?;
        } else {
            writer.write_start_tag(self.tag_name())?;
        }
        if write_attributes {
            for attribute in &self.attributes {
                writer.write_html_attribute(attribute)?;
            }
        }
        Ok(())
    }
}
