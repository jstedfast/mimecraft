// Adapted from MimeKit's Tnef/TnefMessageConverter.cs property conversion.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, io};

use super::{
    RtfCompressedToRtf, RtfCompressionMode, RtfDecompressionError, TnefAttachmentConversionError,
    TnefBodyError, TnefConversionLoss, TnefConversionLossKind as Loss, TnefMessage,
    TnefMessageBody, TnefName, TnefNameId, TnefPropertySet, TnefPropertyTag as Tag,
    TnefPropertyType as Type, TnefRecipientType, TnefValue, TnefValueConversionError,
    attachment_conversion::create_part,
    calendar::{CalendarLossKind, CalendarOptions, build_calendar},
    html_references::HtmlReferences,
    rtf_images::prepare_rtf_images,
};
use crate::{
    Address, AddressHeaderFormatError, AddressList, BestEncodingError, ContentDisposition,
    ContentType, DateTimeOffset, EncodingConstraint, Header, HeaderFormatError, HeaderId,
    HeaderList, MailboxAddress, MessageId, MessageIdGenerationError, MessageImportance,
    MessagePriority, MimeEntity, MimeMessage, Multipart, MultipartError, MultipartRelated,
    MultipartRelatedError, References, TextPartError, UnstructuredHeaderFormatError,
    charsets::{self, Charset},
    text::{
        HeaderFooterFormat, HtmlTagId, HtmlToken, HtmlTokenizerReader, HtmlWriter,
        RtfHtmlExtractor, RtfIssue, RtfToHtml, RtfToText,
    },
};

// PS_INTERNET_HEADERS, in Windows GUID byte order.
const INTERNET_HEADERS: [u8; 16] = [0x86, 3, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];
const FLAG_REQUEST: TnefNameId = TnefNameId {
    guid: [8, 32, 6, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46],
    name: TnefName::Id(0x8530),
};

/// Policy for the property-based TNEF-to-MIME conversion.
#[derive(Clone, Debug)]
pub struct TnefConversionOptions {
    /// Charset when neither the body nor message codepage supplies one.
    pub fallback_charset: Charset,
    /// Calendar generation, exception limits, and deterministic timestamp policy.
    pub calendar: CalendarOptions,
}

impl Default for TnefConversionOptions {
    fn default() -> Self {
        Self {
            fallback_charset: Charset::from_label("windows-1252").expect("built-in charset"),
            calendar: CalendarOptions::default(),
        }
    }
}

/// An independently owned MIME message and all reported conversion losses.
#[derive(Debug)]
#[must_use = "inspect conversion losses before using the output"]
pub struct TnefConversionResult {
    /// Message assembled from MAPI properties, not a restored MIME skeleton.
    pub message: MimeMessage,
    /// Header, calendar, body, and attachment recoveries in encounter order.
    pub losses: Vec<TnefConversionLoss>,
}

/// A conversion failure; no partially authored MIME message is returned.
#[derive(Debug)]
pub enum TnefConversionError {
    /// Incompatible, invalid, or multivalued metadata.
    Property(TnefValueConversionError),
    /// Attachment conversion failed.
    Attachment(TnefAttachmentConversionError),
    /// A typed content header could not be formatted.
    HeaderFormat(HeaderFormatError),
    /// A mailbox list could not be safely formatted.
    AddressFormat(AddressHeaderFormatError),
    /// An unstructured header could not be safely formatted.
    UnstructuredHeader(UnstructuredHeaderFormatError),
    /// A Unicode body could not be decoded.
    Body(TnefBodyError),
    /// Compressed RTF has invalid or incomplete framing.
    Rtf(RtfDecompressionError),
    /// Multipart construction failed, including OS randomness failures.
    Multipart(MultipartError),
    /// Related-container construction or root assignment failed.
    Related(MultipartRelatedError),
    /// An attachment's relative Content-Location could not be resolved.
    UriResolution(fluent_uri::resolve::ResolveError),
    /// A unique Content-Id for a displayed RTF image could not be generated.
    MessageIdGeneration(MessageIdGenerationError),
    /// Calendar MIME-part construction failed.
    TextPart(TextPartError),
    /// Calendar transfer-encoding selection failed.
    Encoding(BestEncodingError),
    /// HTML tokenization, RTF rendering, or calendar export failed.
    Io(io::Error),
}

macro_rules! conversion_errors {
    ($($variant:ident($error:ty)),+ $(,)?) => {
        impl fmt::Display for TnefConversionError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self { $(Self::$variant(error) => error.fmt(f),)+ }
            }
        }
        impl Error for TnefConversionError {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                match self { $(Self::$variant(error) => Some(error),)+ }
            }
        }
        $(impl From<$error> for TnefConversionError {
            fn from(error: $error) -> Self { Self::$variant(error) }
        })+
    };
}
conversion_errors! {
    Property(TnefValueConversionError),
    Attachment(TnefAttachmentConversionError),
    HeaderFormat(HeaderFormatError),
    AddressFormat(AddressHeaderFormatError),
    UnstructuredHeader(UnstructuredHeaderFormatError),
    Body(TnefBodyError),
    Rtf(RtfDecompressionError),
    Multipart(MultipartError),
    Related(MultipartRelatedError),
    UriResolution(fluent_uri::resolve::ResolveError),
    MessageIdGeneration(MessageIdGenerationError),
    TextPart(TextPartError),
    Encoding(BestEncodingError),
    Io(io::Error),
}

impl TnefMessage {
    /// Converts MAPI headers, text/HTML/RTF/calendar bodies, and ordered attachments.
    ///
    /// Uses [`TnefConversionOptions::default`]. No originator, subject, date, or
    /// message identifier is invented. Received headers are retained; callers
    /// resubmitting over SMTP must remove them.
    ///
    /// This is the property path, not the complete MimeKit converter. RTF best-body
    /// selection generates UTF-8 plain text and HTML, extracting encapsulated HTML
    /// when present. Extracted HTML is untrusted and is not sanitized.
    /// Stored or extracted HTML groups referenced, RenderedInBody attachments
    /// in multipart/related around the complete body. CID matching ignores ASCII
    /// case and decodes percent escapes once; locations match literally or via
    /// RFC 3986 normalization/resolution against Content-Base. No URLs are fetched.
    /// Other attachments remain ordered peers in multipart/mixed.
    /// Ordinary RTF places sniffed browser-displayable images at matching
    /// attachment placeholders, or appends them when counts differ. Displayed
    /// images receive unique Content-Ids and join multipart/related; nonimage
    /// placeholders emit nothing. Custom textual placeholders are not supported.
    /// MIME skeleton restoration is not implemented and is reported when applicable.
    /// Embedded messages pass through as TNEF, and OLE remains unrendered.
    /// Calendar exception attachments are removed only when exported.
    ///
    /// Content and headers are independently owned; the source is unchanged.
    /// Unknown metadata types return checked errors rather than silently
    /// becoming missing fields. Invalid mailbox/identifier syntax and named
    /// Internet headers are reported as losses.
    /// Unknown RTF compression is dropped with a loss; complete LZFu with a bad
    /// CRC is retained with a warning. Other decompression failures are errors.
    /// The first RTF interpretation issue per output format is reported as a loss.
    /// Replacement decoding during HTML reference scanning is reported without
    /// rewriting the emitted body bytes. Reference-index memory is bounded by
    /// attachment identifiers, not the number of URLs in the HTML.
    ///
    /// # Errors
    ///
    /// Returns metadata, body decoding/decompression, header formatting, calendar-part,
    /// transfer-encoding, HTML tokenization, URI resolution, Content-Id generation,
    /// or multipart construction errors.
    ///
    /// ```
    /// use mimecraft::tnef::TnefMessage;
    /// let tnef = TnefMessage::load(b"\x78\x9f\x3e\x22\0\0".as_slice())?;
    /// let result = tnef.to_mime_message()?;
    /// assert!(result.losses.is_empty());
    /// assert!(result.message.body().is_none());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn to_mime_message(&self) -> Result<TnefConversionResult, TnefConversionError> {
        self.to_mime_message_with_options(&TnefConversionOptions::default())
    }

    /// Converts using explicit charset and calendar policy.
    ///
    /// Scope, ownership, losses, and errors are the same as
    /// [`Self::to_mime_message`]. Resource limits govern calendar exceptions,
    /// not decoded RTF or the total MIME output size. RTF interpreters use their
    /// default state limits and replacement decoding.
    pub fn to_mime_message_with_options(
        &self,
        options: &TnefConversionOptions,
    ) -> Result<TnefConversionResult, TnefConversionError> {
        Converter {
            source: self,
            message: MimeMessage::new(),
            losses: Vec::new(),
        }
        .convert(options)
    }
}

struct Converter<'a> {
    source: &'a TnefMessage,
    message: MimeMessage,
    losses: Vec<TnefConversionLoss>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BestBody {
    Undefined,
    Plain,
    Rtf,
    Html,
}

impl Converter<'_> {
    fn loss(&mut self, kind: Loss, description: impl Into<String>) {
        self.losses.push(TnefConversionLoss {
            kind,
            description: description.into(),
            attachment_index: None,
        });
    }

    fn convert(
        mut self,
        options: &TnefConversionOptions,
    ) -> Result<TnefConversionResult, TnefConversionError> {
        self.headers()?;
        if self.source.properties().get(Tag::MIME_SKELETON).is_some() {
            self.loss(Loss::MimeSkeletonNotRestored, "MIME skeleton restoration is not implemented; the message was assembled from MAPI properties.");
        }
        let rtf = self.prepare_rtf()?;
        let use_rtf = rtf.is_some()
            && (self.best_body(true)? == BestBody::Rtf
                // NativeBody can name a missing plain/HTML body. If RTF is the
                // only body available, use it rather than emitting no body.
                || (self.source.text_body().is_none() && self.source.html_body().is_none()));
        let calendar = build_calendar(self.source, &options.calendar)?;
        self.losses
            .extend(calendar.losses.into_iter().map(|loss| TnefConversionLoss {
                kind: match loss.kind {
                    CalendarLossKind::InvalidCalendarData => Loss::InvalidCalendarData,
                    CalendarLossKind::UnsupportedCalendarData => Loss::UnsupportedCalendarData,
                    CalendarLossKind::CalendarExceptionLimitExceeded => {
                        Loss::CalendarExceptionLimitExceeded
                    }
                },
                description: loss.description,
                attachment_index: loss.attachment_index,
            }));

        // RTF placeholders refer to converted attachments. Keep every original
        // slot, including missing content and exported calendar exceptions.
        let mut converted_attachments = Vec::with_capacity(self.source.attachments().len());
        let mut attachment_losses = Vec::new();
        for (index, attachment) in self.source.attachments().iter().enumerate() {
            // Successfully exported recurrence exceptions belong to text/calendar.
            // Retain failed/limited exceptions as ordinary attachments.
            if calendar
                .calendar
                .as_ref()
                .is_some_and(|calendar| calendar.exception_attachment_indices().contains(&index))
            {
                converted_attachments.push(None);
                continue;
            }
            let converted = attachment.to_mime_part()?;
            attachment_losses.extend(converted.losses.into_iter().map(|mut loss| {
                loss.attachment_index = Some(index);
                loss
            }));
            converted_attachments.push(converted.part);
        }
        let mut displayed_images = vec![false; converted_attachments.len()];

        // MS-OXCMAIL 2.1.3.3: alternatives are ordered from least to most
        // preferred (RFC 2046 5.1.4). No text/rtf alternative is emitted.
        let mut bodies = Vec::new();
        let mut extracted_html = false;
        if use_rtf {
            // MS-OXCMAIL 2.1.3.3.5-6: when RTF is best, both alternatives SHOULD
            // be generated from it; stored plain text/HTML may be out of sync.
            (bodies, extracted_html) = self.rtf_parts(
                rtf.as_deref().expect("available RTF"),
                &mut converted_attachments,
                &mut displayed_images,
            )?;
        } else {
            for (subtype, body) in [
                ("plain", self.source.text_body()),
                ("html", self.source.html_body()),
            ] {
                if let Some(body) = body {
                    bodies.push(self.body_part(subtype, body, options)?.into());
                }
            }
        }
        drop(rtf);
        // MS-OXCMAIL 2.1.3.4.1: plain + HTML still honors references in the
        // emitted HTML, deliberately extending the plain-best-body rule.
        // Generated RTF HTML uses the displayed-image set instead; encapsulated
        // HTML has its own references and must not receive generated images.
        let references = if extracted_html || (!use_rtf && self.source.html_body().is_some()) {
            Some(self.html_references(bodies.last().expect("HTML body"))?)
        } else {
            None
        };
        // MS-OXCMAIL 2.1.3.3.8: calendar is the last alternative.
        if let Some(calendar) = &calendar.calendar {
            // The helper supplies UTF-8 and the RFC 5546 2.4 METHOD parameter;
            // select a seven-bit-safe wire encoding just as for the other bodies.
            let mut part = calendar.to_text_part()?;
            let encoding = part
                .part_mut()
                .best_encoding(EncodingConstraint::SevenBit)?;
            part.part_mut().set_content_transfer_encoding(encoding);
            bodies.push(part.into());
        }
        let mut body = match bodies.len() {
            0 => None,
            1 => bodies.pop(),
            _ => Some(multipart("alternative", bodies)?),
        };
        let mut attachments = Vec::new();
        let mut related = None;
        // Preserve body-before-attachment loss ordering despite early conversion.
        self.losses.extend(attachment_losses);
        for (index, part) in converted_attachments.into_iter().enumerate() {
            if let Some(mut part) = part {
                if displayed_images[index]
                    || references
                        .as_ref()
                        .is_some_and(|references| references.is_referenced(index))
                {
                    // Keep disposition parameters; non-inline attachments retain
                    // their stored disposition even when it already says inline.
                    let mut disposition = part
                        .content_disposition()
                        .expect("authored disposition")
                        .unwrap_or_else(ContentDisposition::default);
                    disposition.set_is_attachment(false);
                    part.set_content_disposition(Some(&disposition))?;
                    // MS-OXCMAIL 2.1.3.3.6: the complete body (possibly an
                    // alternative with calendar) is the first related child.
                    if related.is_none() {
                        let mut container = MultipartRelated::new()?;
                        container.set_root(body.take().expect("referenced HTML body"))?;
                        related = Some(container);
                    }
                    related
                        .as_mut()
                        .expect("related container")
                        .multipart_mut()
                        .children_mut()
                        .push(part.into());
                } else {
                    attachments.push(part.into());
                }
            }
        }
        if let Some(related) = related {
            body = Some(related.into());
        }
        if !attachments.is_empty() {
            // MS-OXCMAIL 2.1.3.4: preserve attachment-table order. Section
            // 2.1.3.3 requires the body first, even with just one attachment.
            let mut children = Vec::with_capacity(attachments.len() + usize::from(body.is_some()));
            children.extend(body.take());
            children.extend(attachments);
            body = Some(multipart("mixed", children)?);
        }
        self.message.set_body(body);
        Ok(TnefConversionResult {
            message: self.message,
            losses: self.losses,
        })
    }

    fn html_references(
        &mut self,
        html: &MimeEntity,
    ) -> Result<HtmlReferences, TnefConversionError> {
        let mut references = HtmlReferences::new(self.source.attachments())?;
        if references.needs_scan() {
            let MimeEntity::Part(part) = html else {
                unreachable!("generated HTML part")
            };
            let content_type = part
                .content_type()
                .expect("authored Content-Type")
                .expect("HTML Content-Type");
            let charset = Charset::from_label(content_type.charset().expect("HTML charset"))
                .expect("selected HTML charset");
            // Scan the exact bytes/charset of the emitted body, including a
            // usable meta override. Replacement follows MimeKit's StreamReader,
            // but the Rust port explicitly reports it; source bytes are unchanged.
            let (text, replaced) = charset.decode_lossy(part.content().get_ref().get_ref());
            if replaced {
                self.loss(Loss::InvalidHtmlBody, "Invalid HTML bytes were replaced while matching inline references; the emitted body bytes are unchanged.");
            }
            references.scan(&text)?;
        }
        Ok(references)
    }

    fn prepare_rtf(&mut self) -> Result<Option<Vec<u8>>, TnefConversionError> {
        let Some(body) = self.source.rtf_body() else {
            return Ok(None);
        };
        let mut filter = RtfCompressedToRtf::new();
        let mut bytes = Vec::new();
        filter.filter(body.as_bytes(), &mut bytes);
        filter.finish(&mut bytes);
        match filter.validate() {
            Ok(()) => (),
            Err(RtfDecompressionError::UnsupportedCompressionMode(mode)) => {
                // MS-OXRTFCP 2.1.3.1.1 defines only COMPRESSED and UNCOMPRESSED.
                // Unknown content is absent for best-body selection, not an
                // empty body generated from uninterpretable bytes.
                self.loss(Loss::InvalidRtfBody, format!(
                    "The compressed RTF body has an unknown compression type (0x{mode:08X}) and was dropped."
                ));
                return Ok(None);
            }
            Err(RtfDecompressionError::InvalidCrc32 { .. })
                if filter.compression_mode() == RtfCompressionMode::Compressed =>
            {
                // MS-OXRTFCP 2.1.3.1.1: CRC covers compressed CONTENTS. Damaged
                // data can still be readable, so retain it with an explicit loss.
                self.loss(Loss::RtfChecksumMismatch,
                    "The CRC of the compressed RTF body does not match its content, so the RTF body may be corrupt.");
            }
            // Preserve the Rust decoder's checked framing failures, including
            // truncated content and nonzero CRC on an uncompressed stream.
            Err(error) => return Err(error.into()),
        }
        Ok(Some(bytes))
    }

    fn best_body(&self, rtf: bool) -> Result<BestBody, TnefConversionError> {
        // MS-OXBBODY 2.1.3.1 step 1: NativeBody wins when recognized. RTF is
        // eligible only when decoded; invalid/absent RTF must not select it.
        match self.source.properties().get_i32(Tag::NATIVE_BODY)? {
            Some(1) => return Ok(BestBody::Plain),
            Some(2) if rtf => return Ok(BestBody::Rtf),
            Some(3) => return Ok(BestBody::Html),
            _ => (),
        }
        // Step 2: TNEF has no NotEnoughMemory state; NoError means present.
        // Rows 2-5 of step 3 are therefore unreachable. Missing RtfInSync is false.
        let plain = self.source.text_body().is_some();
        let html = self.source.html_body().is_some();
        if !plain && !rtf && !html {
            return Ok(BestBody::Undefined); // row 1
        }
        let in_sync = self
            .source
            .properties()
            .get_bool(Tag::RTF_IN_SYNC)?
            .unwrap_or(false);
        // Step 3 is evaluated in table order.
        if rtf && html {
            return Ok(if in_sync {
                BestBody::Rtf
            } else {
                BestBody::Html
            }); // rows 6-7
        }
        if plain && rtf {
            return Ok(if in_sync {
                BestBody::Rtf
            } else {
                BestBody::Plain
            }); // rows 8-9.1
        }
        if rtf {
            return Ok(BestBody::Rtf); // row 9.2
        }
        if html && !plain {
            return Ok(BestBody::Html); // row 9.4
        }
        // Plain alone and plain + HTML without RTF are rows 9.3 and 10.
        Ok(BestBody::Plain)
    }

    fn rtf_issue(&mut self, format: &str, issue: Option<RtfIssue>) {
        if let Some(issue) = issue {
            self.loss(Loss::RtfRenderingIssue, format!(
                "RTF {format} conversion recovered from {issue:?}; only the first issue for this output format is reported."
            ));
        }
    }

    fn rtf_parts(
        &mut self,
        bytes: &[u8],
        attachments: &mut [Option<crate::MimePart<std::io::Cursor<Vec<u8>>>>],
        displayed_images: &mut [bool],
    ) -> Result<(Vec<MimeEntity>, bool), TnefConversionError> {
        // RTF declares its own charset (\ansicpg). Feed original decoded bytes
        // to the interpreters, not a UTF-8 re-encoding of the RTF document.
        // Output is UTF-8 without a BOM; malformed \uN surrogates are replaced.
        let mut text_issue = None;
        let mut placeholders = 0;
        let text = RtfToText::new().convert_with_callbacks(
            bytes,
            |issue| {
                text_issue.get_or_insert(issue);
            },
            |index, _| {
                // Both renderers share visible-placeholder indexing. Count in
                // this existing pass rather than interpreting the RTF again.
                placeholders = index.saturating_add(1);
            },
        )?;
        self.rtf_issue("plain-text", text_issue);
        // MS-OXRTFEX 2.1.3.1.2: recover encapsulated HTML rather than rendering
        // its RTF representation. Some(empty) is recognized empty HTML, not a
        // reason to substitute generated markup. Extraction is not sanitization.
        let mut html_issue = None;
        let html = RtfHtmlExtractor::new().extract_with_issues(bytes, |issue| {
            html_issue.get_or_insert(issue);
        })?;
        let extracted = html.is_some();
        let html = match html {
            Some(html) => html,
            None => {
                html_issue = None;
                let images =
                    prepare_rtf_images(self.source.attachments(), attachments, displayed_images)?;
                let mut converter = RtfToHtml::new();
                let has_images = images.iter().any(Option::is_some);
                if has_images && placeholders == images.len() {
                    converter.convert_with_callbacks(
                        bytes,
                        |issue| {
                            html_issue.get_or_insert(issue);
                        },
                        |index, writer| {
                            if let Some(image) = &images[index] {
                                writer.write_markup_text(image);
                            }
                        },
                    )?
                } else {
                    // MS-OXRTFEX 2.2.3.4: only match positions when counts
                    // agree. Otherwise append the displayable images, in
                    // rendering order, to the end of the body.
                    if has_images {
                        let mut footer = HtmlWriter::new();
                        for image in images.iter().flatten() {
                            footer.write_start_tag("div").expect("constant tag");
                            footer.write_markup_text(image);
                            footer.write_end_tag("div").expect("constant tag");
                        }
                        converter.footer = Some(footer.finish());
                        converter.footer_format = HeaderFooterFormat::Html;
                    }
                    converter.convert_with_issues(bytes, |issue| {
                        html_issue.get_or_insert(issue);
                    })?
                }
            }
        };
        self.rtf_issue("HTML", html_issue);
        let parts = [("plain", text), ("html", html)]
            .into_iter()
            .map(|(subtype, text)| {
                let mut content_type = ContentType::new("text", subtype).expect("constant subtype");
                content_type.set_charset(Some("utf-8"));
                Ok(create_part(&content_type, text.as_bytes())?.into())
            })
            .collect::<Result<_, TnefConversionError>>()?;
        Ok((parts, extracted))
    }

    fn body_part(
        &self,
        subtype: &str,
        body: &TnefMessageBody,
        options: &TnefConversionOptions,
    ) -> Result<crate::MimePart<std::io::Cursor<Vec<u8>>>, TnefConversionError> {
        let mut content_type = ContentType::new("text", subtype).expect("constant body subtype");
        if body.tag().value_type() == Type::UNICODE {
            // PtypString is UTF-16LE (MS-OXCDATA 2.11.1); re-encode as UTF-8.
            // The Rust body API rejects malformed UTF-16 rather than replacing it.
            let text = body.text()?;
            content_type.set_charset(Some("utf-8"));
            return Ok(create_part(&content_type, text.as_bytes())?);
        }
        let mut charset = body
            .charset()
            .or_else(|| charsets::from_codepage(self.source.codepage()))
            .unwrap_or(options.fallback_charset);
        if subtype == "html"
            && let Some(declared) = html_meta_charset(body.as_bytes())?
                .and_then(|name| Charset::from_label(&name).ok())
                .filter(|charset| charset.decode(body.as_bytes()).is_ok())
        {
            // PidTagHtml is binary. A usable HTML meta declaration wins over
            // InternetCodepage; stale declarations that cannot decode do not.
            charset = declared;
        }
        content_type.set_charset(Some(charset.name()));
        Ok(create_part(&content_type, body.as_bytes())?)
    }

    fn headers(&mut self) -> Result<(), TnefConversionError> {
        self.received_headers()?;
        self.originator()?;
        let properties = self.source.properties();
        // MS-OXCMAIL 2.1.3.2.7: ClientSubmitTime is UTC. The loader maps attDateSent.
        if let Some(date) = properties.get_datetime(Tag::CLIENT_SUBMIT_TIME)? {
            self.message.set_date(
                DateTimeOffset::new(
                    date.year(),
                    date.month(),
                    date.day(),
                    date.hour(),
                    date.minute(),
                    date.second(),
                    0,
                )
                .expect("validated UTC coordinates"),
            );
        }
        // MS-OXCMAIL 2.1.3.2.8 permits Subject as seen by the sender, preferred
        // here over SubjectPrefix + NormalizedSubject. Empty is still a value.
        if let Some(subject) = self.source.subject()? {
            self.message.set_subject(&subject)?;
        } else if let Some(normalized) = properties.get_text(Tag::NORMALIZED_SUBJECT_W)? {
            let prefix = properties.get_text(Tag::SUBJECT_PREFIX_W)?;
            self.message
                .set_subject(&format!("{}{normalized}", prefix.as_deref().unwrap_or("")))?;
        }
        self.message_id()?;
        self.recipients()?;
        self.threading()?;
        self.priority()?;
        // MUST run last: named Internet headers must not replace mapped headers.
        self.internet_headers();
        Ok(())
    }

    fn received_headers(&mut self) -> Result<(), TnefConversionError> {
        // MS-OXCMAIL 2.1.3.2.27: Received precedes every other header and keeps
        // original order. The destination is unknown; SMTP resubmission callers
        // must remove these (the specification permits them for POP3/IMAP).
        if let Some(text) = self
            .source
            .properties()
            .get_text(Tag::TRANSPORT_MESSAGE_HEADERS_W)?
        {
            let text = text.trim_start_matches(['\r', '\n']);
            if text.is_empty() {
                return Ok(());
            }
            let bytes = format!("{text}\r\n\r\n");
            match HeaderList::load(&mut bytes.as_bytes()) {
                Ok(headers) => {
                    for header in headers {
                        if header.is_invalid() {
                            self.loss(
                                Loss::InvalidHeader,
                                "An invalid transport header was ignored.",
                            );
                        } else if header.id() == HeaderId::Received {
                            self.message.headers_mut().add(header);
                        }
                    }
                }
                Err(error) => self.loss(
                    Loss::InvalidHeader,
                    format!("Transport headers could not be parsed: {error}"),
                ),
            }
        }
        Ok(())
    }

    fn mailbox(
        &mut self,
        role: &str,
        properties: &TnefPropertySet,
        tags: [Option<Tag>; 5],
    ) -> Result<Option<MailboxAddress>, TnefConversionError> {
        let get = |tag: Option<Tag>| {
            tag.map(|tag| properties.get_text(tag))
                .transpose()
                .map(Option::flatten)
        };
        let name = get(tags[0])?;
        let smtp_address = get(tags[1])?;
        let email = get(tags[2])?;
        let kind = get(tags[3])?;
        let search = get(tags[4])?;
        let search = search.as_deref().map(|key| key.trim_end_matches('\0'));
        // MS-OXCMAIL 2.1.3.1.1: address-book EntryIDs and IMCEA encapsulation
        // are not implemented. Use SMTP EmailAddress, then SmtpAddress. MimeKit
        // also accepts an SMTP search key when neither supplies an address.
        let smtp = kind
            .as_deref()
            .is_none_or(|kind| kind.is_empty() || kind.eq_ignore_ascii_case("SMTP"));
        let address = email
            .as_deref()
            .filter(|value| smtp && !value.is_empty())
            .or_else(|| smtp_address.as_deref().filter(|value| !value.is_empty()))
            .or_else(|| {
                search
                    .filter(|key| {
                        smtp && key
                            .get(..5)
                            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("SMTP:"))
                    })
                    .map(|key| &key[5..])
            });
        let Some(address) = address.filter(|address| !address.is_empty()) else {
            if (!smtp && email.as_deref().is_some_and(|value| !value.is_empty()))
                || name.as_deref().is_some_and(|value| !value.is_empty())
                || search.is_some_and(|value| !value.is_empty())
            {
                self.loss(
                    Loss::UnparsableRecipient,
                    format!("The {role} has no usable Internet mailbox and was dropped."),
                );
            }
            return Ok(None);
        };
        match MailboxAddress::parse(address.as_bytes()) {
            Ok(mut mailbox) => {
                if let Some(name) = name.as_deref().filter(|name| !name.is_empty()) {
                    mailbox.set_display_name(name);
                }
                Ok(Some(mailbox))
            }
            Err(error) => {
                self.loss(
                    Loss::UnparsableRecipient,
                    format!("The {role} mailbox could not be parsed: {error}"),
                );
                Ok(None)
            }
        }
    }

    fn originator(&mut self) -> Result<(), TnefConversionError> {
        let properties = self.source.properties();
        // MS-OXCMAIL 2.1.3.1.3-4: SentRepresenting becomes From; a distinct
        // Sender becomes Sender. Without SentRepresenting, Sender becomes From.
        let representing = self.mailbox(
            "sent-representing",
            properties,
            [
                Some(Tag::SENT_REPRESENTING_NAME_W),
                None,
                Some(Tag::SENT_REPRESENTING_EMAIL_ADDRESS_W),
                Some(Tag::SENT_REPRESENTING_ADDRTYPE_W),
                Some(Tag::SENT_REPRESENTING_SEARCH_KEY),
            ],
        )?;
        let sender = self.mailbox(
            "sender",
            properties,
            [
                Some(Tag::SENDER_NAME_W),
                Some(Tag::SENDER_SMTP_ADDRESS_W),
                Some(Tag::SENDER_EMAIL_ADDRESS_W),
                Some(Tag::SENDER_ADDRTYPE_W),
                Some(Tag::SENDER_SEARCH_KEY),
            ],
        )?;
        if let Some(from) = representing {
            let sender = sender.filter(|sender| {
                !sender
                    .address()
                    .to_string()
                    .eq_ignore_ascii_case(&from.address().to_string())
            });
            self.message
                .set_from(&AddressList::from(vec![Address::Mailbox(from)]))?;
            if let Some(sender) = sender {
                self.message.set_sender(Some(&sender))?;
            }
        } else if let Some(sender) = sender {
            self.message
                .set_from(&AddressList::from(vec![Address::Mailbox(sender)]))?;
        }
        Ok(())
    }

    fn checked_id(&mut self, field: &str, value: &str) -> Option<MessageId> {
        match MessageId::parse_prefix(value.as_bytes())
            .and_then(|(id, _)| MessageId::from_value(id.as_str()))
        {
            Ok(id) => Some(id),
            Err(error) => {
                self.loss(
                    Loss::InvalidMessageId,
                    format!("The {field} identifier was ignored: {error}"),
                );
                None
            }
        }
    }

    fn message_id(&mut self) -> Result<(), TnefConversionError> {
        // MS-OXCMAIL 2.1.3.2.11: InternetMessageId is the primary source.
        if let Some(text) = self.source.internet_message_id()?
            && let Some(id) = self.checked_id("Message-Id", &text)
        {
            self.message
                .set_message_id(id.as_str())
                .expect("validated identifier");
            return Ok(());
        }
        // MS-OXCMAIL 2.1.3.2.26 recommends the containing Message-Id as the
        // TnefCorrelationKey. Using a bracketed key as fallback is an extension.
        if let Some(text) = self
            .source
            .properties()
            .get_text(Tag::TNEF_CORRELATION_KEY)?
        {
            let text = text.trim_end_matches('\0');
            if text.len() > 5
                && text.starts_with('<')
                && text.ends_with('>')
                && text.contains('@')
                && let Some(id) = self.checked_id("TNEF correlation key", text)
            {
                self.message
                    .set_message_id(id.as_str())
                    .expect("validated identifier");
            }
        }
        Ok(())
    }

    fn recipients(&mut self) -> Result<(), TnefConversionError> {
        let mut lists: [Vec<Address>; 3] = Default::default();
        // MS-OXCMAIL 2.1.3.1.1.1-2: recipient types 1, 2, 3 are To/Cc/Bcc.
        // Ignore other types; preserve duplicate mailboxes and table order.
        for recipient in self.source.recipients() {
            let index = match recipient.recipient_type()? {
                TnefRecipientType::To => 0,
                TnefRecipientType::Cc => 1,
                TnefRecipientType::Bcc => 2,
                _ => continue,
            };
            if let Some(mailbox) = self.mailbox(
                ["To recipient", "Cc recipient", "Bcc recipient"][index],
                recipient.properties(),
                [
                    Some(Tag::DISPLAY_NAME_W),
                    Some(Tag::SMTP_ADDRESS_W),
                    Some(Tag::EMAIL_ADDRESS_W),
                    Some(Tag::ADDRTYPE_W),
                    Some(Tag::SEARCH_KEY),
                ],
            )? {
                lists[index].push(Address::Mailbox(mailbox));
            }
        }
        // MimeKit extension: use ReceivedBy only when there is no recipient
        // table, not when the existing table's rows failed to yield addresses.
        if self.source.recipients().is_empty()
            && let Some(mailbox) = self.mailbox(
                "received-by",
                self.source.properties(),
                [
                    Some(Tag::RECEIVED_BY_NAME_W),
                    None,
                    Some(Tag::RECEIVED_BY_EMAIL_ADDRESS_W),
                    Some(Tag::RECEIVED_BY_ADDRTYPE_W),
                    Some(Tag::RECEIVED_BY_SEARCH_KEY),
                ],
            )?
        {
            lists[0].push(Address::Mailbox(mailbox));
        }
        for (field, list) in ["To", "Cc", "Bcc"].into_iter().zip(lists) {
            if !list.is_empty() {
                self.message
                    .headers_mut()
                    .add(AddressList::from(list).to_header(field)?);
            }
        }
        Ok(())
    }

    fn threading(&mut self) -> Result<(), TnefConversionError> {
        // MS-OXCMAIL 2.1.3.2.14 and .12: InReplyToId and InternetReferences.
        if let Some(text) = self.source.properties().get_text(Tag::IN_REPLY_TO_ID_W)?
            && let Some(id) = self.checked_id("In-Reply-To", &text)
        {
            self.message
                .set_in_reply_to(Some(id.as_str()))
                .expect("validated identifier");
        }
        if let Some(text) = self
            .source
            .properties()
            .get_text(Tag::INTERNET_REFERENCES_W)?
        {
            let mut references = Vec::new();
            for id in References::new(text.as_bytes()) {
                match id.and_then(|id| MessageId::from_value(id.as_str())) {
                    Ok(id) => references.push(id),
                    Err(error) => self.loss(
                        Loss::InvalidMessageId,
                        format!("A References identifier was ignored: {error}"),
                    ),
                }
            }
            if !references.is_empty() {
                self.message
                    .set_references(references.iter().map(MessageId::as_str))
                    .expect("validated identifiers");
            }
        }
        Ok(())
    }

    fn priority(&mut self) -> Result<(), TnefConversionError> {
        let properties = self.source.properties();
        // MS-OXCMAIL 2.1.3.2.5: retain Normal so it round-trips; ignore other values.
        match properties.get_i32(Tag::IMPORTANCE)? {
            Some(0) => self.message.set_importance(MessageImportance::Low),
            Some(1) => self.message.set_importance(MessageImportance::Normal),
            Some(2) => self.message.set_importance(MessageImportance::High),
            _ => (),
        }
        // PidTagPriority is not mapped by MS-OXCMAIL. MimeKit emits RFC 2156
        // Priority as an extension: urgent, normal, or non-urgent.
        match properties.get_i32(Tag::PRIORITY)? {
            Some(1) => self.message.set_priority(MessagePriority::Urgent),
            Some(0) => self.message.set_priority(MessagePriority::Normal),
            Some(-1) => self.message.set_priority(MessagePriority::NonUrgent),
            _ => (),
        }
        // MS-OXCMAIL 2.1.3.2.6: omit normal and out-of-range sensitivity.
        let sensitivity = match properties.get_i32(Tag::SENSITIVITY)? {
            Some(1) => Some("Personal"),
            Some(2) => Some("Private"),
            Some(3) => Some("Company-Confidential"),
            _ => None,
        };
        if let Some(value) = sensitivity {
            self.message
                .headers_mut()
                .add(Header::from_unstructured("Sensitivity", value)?);
        }
        // MS-OXCMAIL 2.1.3.2.25: X-Message-Flag from PidLidFlagRequest.
        if let Some(property) = properties.get_named(&FLAG_REQUEST) {
            let value = property.to_text()?;
            if !value.is_empty() {
                self.message
                    .headers_mut()
                    .add(Header::from_unstructured("X-Message-Flag", &value)?);
            }
        }
        Ok(())
    }

    fn internet_headers(&mut self) {
        // MS-OXCMAIL 2.1.3.2.4: PS_INTERNET_HEADERS string properties are
        // arbitrary headers, but MUST NOT duplicate headers already generated.
        // Multivalues emit one header per value (a MimeKit extension).
        for property in self.source.properties().iter() {
            let Some(TnefNameId {
                guid,
                name: TnefName::String(field),
            }) = property.name()
            else {
                continue;
            };
            if *guid != INTERNET_HEADERS
                || field.is_empty()
                || excluded_header(field)
                || self.message.headers().contains(field)
            {
                continue;
            }
            if property.values().is_empty()
                || property
                    .values()
                    .iter()
                    .any(|value| !matches!(value, TnefValue::Text(_)))
            {
                self.loss(
                    Loss::InvalidHeader,
                    format!("The {field} Internet header has no string value and was dropped."),
                );
                continue;
            }
            for value in property.values() {
                if let TnefValue::Text(value) = value {
                    match Header::from_unstructured(field, value) {
                        Ok(header) => self.message.headers_mut().add(header),
                        Err(error) => {
                            self.loss(
                                Loss::InvalidHeader,
                                format!("The {field} Internet header was dropped: {error}"),
                            );
                            break;
                        }
                    }
                }
            }
        }
    }
}

fn excluded_header(field: &str) -> bool {
    // MIME structure describes the original entity, not the converted one.
    // MS-OXCMAIL 2.1.3.2.4 also excludes Exchange-internal headers. Recursive
    // embedded conversion (which relaxes this exclusion) is not implemented.
    field.eq_ignore_ascii_case("MIME-Version")
        || [
            "Content-",
            "X-Microsoft-Exchange-Organization",
            "X-Microsoft-Exchange-Forest",
            "X-MS-Exchange-Organization-",
            "X-MS-Exchange-Forest-",
        ]
        .iter()
        .any(|prefix| {
            field
                .get(..prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        })
}

fn multipart(subtype: &str, children: Vec<MimeEntity>) -> Result<MimeEntity, MultipartError> {
    let mut part = Multipart::new(subtype)?;
    *part.children_mut() = children;
    Ok(part.into())
}

fn html_meta_charset(bytes: &[u8]) -> io::Result<Option<String>> {
    // Latin-1 maps every byte to a character, so tokenization cannot fail on
    // the unknown source charset. Stop at <body> or </head>, as MimeKit does.
    let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
    let mut tokenizer = HtmlTokenizerReader::from_text(&text);
    while let Some(token) = tokenizer.read_next_token()? {
        let HtmlToken::Tag(tag) = token else { continue };
        if tag.id() == HtmlTagId::Body || (tag.id() == HtmlTagId::Head && tag.is_end_tag()) {
            break;
        }
        if tag.id() != HtmlTagId::Meta || tag.is_end_tag() {
            continue;
        }
        let attribute = |name| {
            tag.attributes()
                .get(name)
                .and_then(|attribute| attribute.value())
        };
        if let Some(charset) = attribute("charset")
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(Some(charset.to_owned()));
        }
        if attribute("http-equiv").is_some_and(|value| value.eq_ignore_ascii_case("Content-Type"))
            && let Some(content) = attribute("content")
            && let Ok(content_type) = ContentType::parse(content.as_bytes())
            && let Some(charset) = content_type.charset().filter(|value| !value.is_empty())
        {
            return Ok(Some(charset.to_owned()));
        }
    }
    Ok(None)
}
