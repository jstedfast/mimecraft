// Adapted from MimeKit's MimeReader.cs and CappedComplianceLogger.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io;

use super::encoding::EncodingValidator;
use super::scan::Scanner;
use super::{
    Budget, MimeComplianceIssue, MimeComplianceLogger, MimeComplianceOptions,
    MimeCompliancePositionKind as PositionKind, MimeComplianceValidators as Validators,
    MimeComplianceViolation as Violation,
};
use crate::{ContentEncoding, Header, HeaderId, MimeEntityKind, MimeEvent, MimePosition};

/// Disabled reads neither allocate reporting state nor scan content for defects.
pub(crate) struct Reporting<'a, L: MimeComplianceLogger + ?Sized = dyn MimeComplianceLogger + 'a>(
    Option<Box<Active<'a, L>>>,
);

struct Active<'a, L: MimeComplianceLogger + ?Sized> {
    logger: &'a mut L,
    options: MimeComplianceOptions,
    budget: Budget,
    bodies: Vec<Body>,
    message: bool,
    source_line_start: u64,
    scanner: Scanner,
}

pub(crate) enum HeaderEnd<'a> {
    Separator(&'a [u8]),
    Eof,
    Boundary,
}

impl<'a> From<Option<&'a [u8]>> for HeaderEnd<'a> {
    fn from(separator: Option<&'a [u8]>) -> Self {
        separator.map_or(Self::Eof, Self::Separator)
    }
}

impl<'a, L: MimeComplianceLogger + ?Sized> Reporting<'a, L> {
    pub(crate) fn new(
        logger: Option<&'a mut L>,
        options: MimeComplianceOptions,
        message: bool,
    ) -> Self {
        Self(logger.map(|logger| {
            Box::new(Active {
                logger,
                options,
                budget: Budget::new(options.max_issues_per_violation),
                bodies: Vec::new(),
                message,
                source_line_start: 0,
                scanner: Scanner::default(),
            })
        }))
    }

    pub(crate) fn message(&self) -> bool {
        self.0.as_ref().is_some_and(|active| active.message)
    }

    pub(crate) fn reset(&mut self) {
        if let Some(active) = self.0.as_mut() {
            active.budget.reset();
            active.bodies.clear();
            active.source_line_start = 0;
        }
    }

    pub(crate) fn column(&self, position: MimePosition) -> u64 {
        self.0.as_ref().map_or(1, |active| {
            position.offset.saturating_sub(active.source_line_start) + 1
        })
    }

    pub(crate) fn issue(
        &mut self,
        violation: Violation,
        position: MimePosition,
        column: u64,
        kind: PositionKind,
    ) -> io::Result<()> {
        if let Some(active) = self.0.as_mut() {
            active.issue(violation, position, column, kind)?;
        }
        Ok(())
    }

    // Keep optional bookkeeping out of the generic callback's hot path so
    // disabled reporting does not obstruct inlining the content callback.
    #[inline]
    pub(crate) fn emit<F>(&mut self, callback: &mut F, event: MimeEvent<'_>) -> io::Result<()>
    where
        F: crate::mime_reader::EventSink,
    {
        if let Some(active) = self.0.as_mut() {
            active.event(&event)?;
        }
        callback.event(event)
    }

    #[inline]
    pub(crate) fn entity_begin<F: crate::mime_reader::EventSink>(
        &mut self,
        callback: &mut F,
        headers: Vec<Header>,
        kind: MimeEntityKind,
        encoding: ContentEncoding,
        position: MimePosition,
        headers_end: MimePosition,
    ) -> io::Result<()> {
        if let Some(active) = self.0.as_mut() {
            active.entity_begin(kind, encoding, headers_end);
        }
        callback.entity_begin(headers, kind, encoding, position, headers_end)
    }

    #[inline]
    pub(crate) fn content(&mut self, bytes: &[u8], position: MimePosition) -> io::Result<()> {
        if !bytes.is_empty()
            && let Some(active) = self.0.as_mut()
        {
            active.content(bytes, position, true)?;
        }
        Ok(())
    }

    pub(crate) fn framing(&mut self, bytes: &[u8], position: MimePosition) -> io::Result<()> {
        if let Some(active) = self.0.as_mut() {
            // The delimiter itself is not encoded data. Flush before inspecting
            // it, but only after validating its preceding withheld newline.
            active.finish_encoding()?;
            active.content(bytes, position, false)?;
        }
        Ok(())
    }

    pub(crate) fn headers(
        &mut self,
        headers: &[Header],
        start: MimePosition,
        end: MimePosition,
        ending: HeaderEnd<'_>,
        message: bool,
        toplevel: bool,
    ) -> io::Result<()> {
        if let Some(active) = self.0.as_mut() {
            active.headers(headers, start, end, ending, message, toplevel)?;
        }
        Ok(())
    }
}

impl<L: MimeComplianceLogger + ?Sized> Active<'_, L> {
    fn event(&mut self, event: &MimeEvent<'_>) -> io::Result<()> {
        match event {
            MimeEvent::EntityBegin {
                kind,
                encoding,
                headers_end,
                ..
            } => self.entity_begin(*kind, *encoding, *headers_end),
            MimeEvent::Content {
                bytes, position, ..
            } => self.content(bytes, *position, true)?,
            MimeEvent::EntityEnd { .. } => {
                self.finish_encoding()?;
                self.bodies.pop().expect("matching entity begin");
            }
            _ => {}
        }
        Ok(())
    }

    fn entity_begin(
        &mut self,
        kind: MimeEntityKind,
        encoding: ContentEncoding,
        headers_end: MimePosition,
    ) {
        let mut body = Body::new(encoding, headers_end);
        if kind == MimeEntityKind::Part {
            body.validator = EncodingValidator::new(encoding, self.options, headers_end);
        }
        self.bodies.push(body);
    }

    fn issue(
        &mut self,
        violation: Violation,
        position: MimePosition,
        column: u64,
        kind: PositionKind,
    ) -> io::Result<()> {
        let issue = MimeComplianceIssue {
            context: self.options.context,
            violation,
            stream_offset: position.offset,
            line_number: position.line,
            column_number: column,
            position_kind: kind,
        };
        self.log(issue)
    }

    fn finish_encoding(&mut self) -> io::Result<()> {
        if let Some(validator) = self
            .bodies
            .last_mut()
            .and_then(|body| body.validator.take())
        {
            validator.finish(self)?;
        }
        Ok(())
    }

    fn content(&mut self, bytes: &[u8], position: MimePosition, validate: bool) -> io::Result<()> {
        let Some(mut body) = self.bodies.pop() else {
            return Ok(());
        };
        let result = body.scan(self, bytes, position).and_then(|()| {
            if validate && let Some(validator) = &mut body.validator {
                validator.write(bytes, self)?;
            }
            Ok(())
        });
        self.source_line_start = body.line_start.offset;
        self.bodies.push(body);
        result
    }

    fn headers(
        &mut self,
        headers: &[Header],
        mut position: MimePosition,
        end: MimePosition,
        ending: HeaderEnd<'_>,
        message: bool,
        toplevel: bool,
    ) -> io::Result<()> {
        // Upper bits track the current resent block; lower bits track the whole message.
        const MESSAGE_HEADERS_MASK: u32 = (1 << 12) - 1;
        let mut seen = 0u32;
        let mut content_type = None;
        let mut content_disposition = false;
        let mut content_id = false;
        let mut encoding = None;
        let mut mime_version = false;
        // A skipped initial UTF-8 BOM still occupies three physical byte
        // columns. Subsequent header blocks start after a newline.
        let mut end_column = if position.line == 1 {
            position.offset + 1
        } else {
            1
        };
        for header in headers {
            let raw = header.raw();
            if let Some(offset) = header.offset() {
                position.offset = offset;
            }
            let start = position;
            let start_column = end_column;
            if header.is_invalid() {
                let mut blank = None;
                let mut invalid = None;
                for (index, &byte) in raw.iter().enumerate() {
                    if byte == b':' {
                        break;
                    }
                    // Blanks may separate a field name from its colon, but
                    // a subsequent non-blank makes the first blank invalid.
                    if matches!(byte, b' ' | b'\t') {
                        blank.get_or_insert(index);
                    } else if blank.is_some() || !byte.is_ascii() || byte.is_ascii_control() {
                        invalid = Some(blank.unwrap_or(index));
                        break;
                    }
                }
                // EOF without a colon or offending byte identifies only the
                // malformed record, not an exact invalid character.
                let (position, column, kind) =
                    invalid.map_or((start, 1, PositionKind::ElementStart), |index| {
                        (
                            start.advance(&raw[..index]),
                            start_column + index as u64,
                            PositionKind::Exact,
                        )
                    });
                self.issue(Violation::InvalidHeader, position, column, kind)?;
            }
            let mut bare_cr = false;
            let mut null = false;
            let mut first_eight_bit = None;
            let mut offset = 0;
            while offset < raw.len() {
                let scanned = self
                    .scanner
                    .line(&raw[offset..], !null, first_eight_bit.is_none());
                let length = scanned.lf.map_or(raw.len() - offset, |i| i + 1);
                let line = &raw[offset..offset + length];
                let column = end_column;
                if let Some(index) = scanned.eight_bit {
                    // RFC 6532 allows UTF-8 in headers. Remember the first
                    // non-ASCII byte, then validate once the full value is available.
                    first_eight_bit = Some(offset + index);
                }
                if line.len() > 1000 {
                    self.issue(
                        Violation::OversizedLine,
                        position,
                        1,
                        PositionKind::LineStart,
                    )?;
                }
                if let Some(index) = scanned.null {
                    null = true;
                    self.issue(
                        Violation::UnexpectedNullBytesInHeader,
                        position.advance(&line[..index]),
                        column + index as u64,
                        PositionKind::Exact,
                    )?;
                }
                if !bare_cr
                    && let Some(index) =
                        memchr::memchr_iter(b'\r', line).find(|&i| line.get(i + 1) != Some(&b'\n'))
                {
                    bare_cr = true;
                    self.issue(
                        Violation::BareCarriageReturnInHeader,
                        position.advance(&line[..index]),
                        column + index as u64,
                        PositionKind::Exact,
                    )?;
                }
                if line.ends_with(b"\n") && !line.ends_with(b"\r\n") {
                    self.issue(
                        Violation::BareLinefeedInHeader,
                        position.advance(&line[..line.len() - 1]),
                        column + line.len() as u64 - 1,
                        PositionKind::Exact,
                    )?;
                }
                if offset != 0
                    && line
                        .iter()
                        .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
                {
                    // Stripping this whitespace would turn a continuation into
                    // a separator and make other parsers see different headers.
                    self.issue(
                        Violation::BlankHeaderContinuationLine,
                        position,
                        1,
                        PositionKind::LineStart,
                    )?;
                }
                position = position.advance(line);
                end_column = if line.ends_with(b"\n") {
                    1
                } else {
                    column + line.len() as u64
                };
                offset += length;
            }
            // RFC 6532 permits UTF-8 in headers, so wait for the full value
            // before reporting 8-bit bytes. Locate the first invalid sequence,
            // not an earlier valid UTF-8 sequence or the start of the header.
            if let Some(first) = first_eight_bit
                && let Some(index) = self.scanner.invalid_utf8(&raw[first..])
            {
                let (position, column) =
                    header_byte_position(raw, start, start_column, first + index);
                self.issue(
                    Violation::Unexpected8BitBytesInHeader,
                    position,
                    column,
                    PositionKind::Exact,
                )?;
            }
            // An invalid raw record has no value, even if its field bytes
            // happen to spell a known header name at EOF.
            if header.is_invalid() {
                continue;
            }
            let address = matches!(
                header.id(),
                HeaderId::From
                    | HeaderId::Sender
                    | HeaderId::ReplyTo
                    | HeaderId::To
                    | HeaderId::Cc
                    | HeaderId::Bcc
                    | HeaderId::ResentFrom
                    | HeaderId::ResentSender
                    | HeaderId::ResentReplyTo
                    | HeaderId::ResentTo
                    | HeaderId::ResentCc
                    | HeaderId::ResentBcc
                    | HeaderId::DispositionNotificationTo
            );
            if self
                .options
                .enabled_validators
                .contains(Validators::ENCODED_WORDS)
                && (address
                    || matches!(
                        header.id(),
                        HeaderId::Subject | HeaderId::Comments | HeaderId::ContentDescription
                    )
                    || header
                        .field()
                        .and_then(|field| field.as_bytes().get(..2))
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"X-")))
            {
                let value = header.raw_value();
                let (position, column) =
                    header_byte_position(raw, start, start_column, raw.len() - value.len());
                super::encoded_words::validate(
                    value,
                    address,
                    position,
                    column,
                    self.options.context,
                    self,
                )?;
            }
            if matches!(
                header.id(),
                HeaderId::ContentType | HeaderId::ContentDisposition
            ) && self
                .options
                .enabled_validators
                .contains(Validators::PARAMETERS)
            {
                let value = header.raw_value();
                let (position, column) =
                    header_byte_position(raw, start, start_column, raw.len() - value.len());
                super::parameters::validate(
                    value,
                    header.id(),
                    position,
                    column,
                    self.options.context,
                    self,
                )?;
            }
            if message {
                let repeated = repeated_header(header.id());
                let index = repeated.map(|v| v as u32 - Violation::RepeatedDate as u32);
                if index.is_none_or(|index| index < 12) {
                    // Resent fields for a single resending are grouped together.
                    // Any other field ends that block, but not ordinary counts.
                    seen &= MESSAGE_HEADERS_MASK;
                }
                if let (Some(violation), Some(index)) = (repeated, index) {
                    let mask = 1 << index;
                    let duplicate = seen & mask != 0;
                    seen |= mask;
                    if duplicate {
                        self.issue(violation, start, 1, PositionKind::ElementStart)?;
                    }
                }
            }
            match header.id() {
                HeaderId::ContentId => {
                    if content_id {
                        self.issue(
                            Violation::RepeatedContentId,
                            start,
                            1,
                            PositionKind::ElementStart,
                        )?;
                    }
                    content_id = true;
                }
                HeaderId::ContentDisposition => {
                    if content_disposition {
                        self.issue(
                            Violation::RepeatedContentDisposition,
                            start,
                            1,
                            PositionKind::ElementStart,
                        )?;
                    }
                    content_disposition = true;
                }
                HeaderId::ContentType => {
                    if content_type.is_some() {
                        self.issue(
                            Violation::RepeatedContentType,
                            start,
                            1,
                            PositionKind::ElementStart,
                        )?;
                    } else {
                        content_type = Some((header, start));
                        if crate::ContentType::parse_for_reader(header.raw_value()).1 {
                            self.issue(
                                Violation::InvalidContentType,
                                start,
                                1,
                                PositionKind::ElementStart,
                            )?;
                        }
                    }
                }
                HeaderId::ContentTransferEncoding => {
                    if encoding.is_some() {
                        self.issue(
                            Violation::RepeatedContentTransferEncoding,
                            start,
                            1,
                            PositionKind::ElementStart,
                        )?;
                    } else {
                        let parsed = ContentEncoding::parse(&header.decode_value().text);
                        let (value_position, column) =
                            header_value_position(header, start, start_column);
                        encoding = Some((parsed, value_position, column));
                        if parsed.is_err() {
                            self.issue(
                                Violation::InvalidContentTransferEncoding,
                                value_position,
                                column,
                                PositionKind::Exact,
                            )?;
                        }
                    }
                }
                HeaderId::MimeVersion => mime_version = true,
                _ => {}
            }
        }
        if let Some((header, start)) = content_type {
            let content_type = crate::ContentType::parse_for_reader(header.raw_value()).0;
            let multipart = content_type.is_mime_type("multipart", "*");
            if multipart {
                match content_type.boundary() {
                    None => self.issue(
                        Violation::MissingMultipartBoundaryParameter,
                        start,
                        1,
                        PositionKind::ElementStart,
                    )?,
                    Some(boundary) if crate::multipart::validate_boundary(boundary).is_err() => {
                        self.issue(
                            Violation::InvalidMultipartBoundaryParameter,
                            start,
                            1,
                            PositionKind::ElementStart,
                        )?
                    }
                    _ => {}
                }
            }
            if let Some((Ok(encoding), encoding_start, column)) = encoding
                && matches!(
                    encoding,
                    ContentEncoding::Base64
                        | ContentEncoding::QuotedPrintable
                        | ContentEncoding::UuEncode
                )
            {
                if multipart {
                    self.issue(
                        Violation::IllegalMultipartContentTransferEncoding,
                        encoding_start,
                        column,
                        PositionKind::Exact,
                    )?;
                } else if content_type.is_mime_type("message", "rfc822") {
                    self.issue(
                        Violation::IllegalMessageRfc822ContentTransferEncoding,
                        encoding_start,
                        column,
                        PositionKind::Exact,
                    )?;
                }
            }
        }
        let (missing_position, column, kind) = match ending {
            HeaderEnd::Separator(separator) => {
                if separator == b"\n" {
                    self.issue(
                        Violation::BareLinefeedInHeader,
                        position,
                        1,
                        PositionKind::Exact,
                    )?;
                }
                (position, 1, PositionKind::LineStart)
            }
            HeaderEnd::Boundary => {
                self.issue(
                    Violation::MissingBodySeparator,
                    end,
                    1,
                    PositionKind::ElementStart,
                )?;
                (end, 1, PositionKind::ElementStart)
            }
            HeaderEnd::Eof => {
                let incomplete = headers
                    .last()
                    .is_some_and(|header| !header.raw().ends_with(b"\n"));
                if !headers.is_empty() {
                    let violation = if incomplete {
                        Violation::IncompleteHeader
                    } else {
                        Violation::MissingBodySeparator
                    };
                    self.issue(violation, end, end_column, PositionKind::Exact)?;
                }
                (end, end_column, PositionKind::Exact)
            }
        };
        // Retain the physical input column while completed child frames are
        // unwound, so missing ancestor markers at EOF use the same position.
        self.source_line_start = end.offset
            - match ending {
                HeaderEnd::Eof => end_column - 1,
                _ => 0,
            };
        // Embedded messages are often trimmed copies; only the top-level message
        // is required to have these fields. Missing MIME-Version matters only if
        // MIME fields were actually used.
        if message && toplevel {
            for (bit, violation) in [
                (0, Violation::MissingDate),
                (1, Violation::MissingFrom),
                (7, Violation::MissingMessageId),
            ] {
                if seen & (1 << bit) == 0 {
                    self.issue(violation, missing_position, column, kind)?;
                }
            }
            if !mime_version && (content_type.is_some() || encoding.is_some()) {
                self.issue(
                    Violation::MissingMimeVersion,
                    missing_position,
                    column,
                    kind,
                )?;
            }
        }
        Ok(())
    }
}

impl<L: MimeComplianceLogger + ?Sized> MimeComplianceLogger for Active<'_, L> {
    fn log(&mut self, issue: MimeComplianceIssue) -> io::Result<()> {
        if let Some(issue) = self.budget.check(issue) {
            self.logger.log(issue)?;
        }
        Ok(())
    }
}

fn header_value_position(header: &Header, start: MimePosition, column: u64) -> (MimePosition, u64) {
    // Skip folding whitespace after the colon to identify the value even when
    // it starts on a subsequent physical line.
    let raw = header.raw();
    let value = header.raw_value();
    let value_start = raw.len() - value.len();
    let index = value
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
    // An empty value must not point into the next header: use the byte just
    // after the colon rather than the end of its whitespace.
    let index = value_start + index.unwrap_or(0);
    header_byte_position(raw, start, column, index)
}

fn header_byte_position(
    raw: &[u8],
    start: MimePosition,
    column: u64,
    index: usize,
) -> (MimePosition, u64) {
    let prefix = &raw[..index];
    let column = prefix
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(column + index as u64, |lf| (index - lf) as u64);
    (start.advance(prefix), column)
}

fn repeated_header(id: HeaderId) -> Option<Violation> {
    Some(match id {
        HeaderId::Date => Violation::RepeatedDate,
        HeaderId::From => Violation::RepeatedFrom,
        HeaderId::Sender => Violation::RepeatedSender,
        HeaderId::ReplyTo => Violation::RepeatedReplyTo,
        HeaderId::To => Violation::RepeatedTo,
        HeaderId::Cc => Violation::RepeatedCc,
        HeaderId::Bcc => Violation::RepeatedBcc,
        HeaderId::MessageId => Violation::RepeatedMessageId,
        HeaderId::InReplyTo => Violation::RepeatedInReplyTo,
        HeaderId::References => Violation::RepeatedReferences,
        HeaderId::Subject => Violation::RepeatedSubject,
        HeaderId::ReturnPath => Violation::RepeatedReturnPath,
        HeaderId::ResentDate => Violation::RepeatedResentDate,
        HeaderId::ResentFrom => Violation::RepeatedResentFrom,
        HeaderId::ResentSender => Violation::RepeatedResentSender,
        HeaderId::ResentTo => Violation::RepeatedResentTo,
        HeaderId::ResentCc => Violation::RepeatedResentCc,
        HeaderId::ResentBcc => Violation::RepeatedResentBcc,
        HeaderId::ResentMessageId => Violation::RepeatedResentMessageId,
        _ => return None,
    })
}

struct Body {
    validator: Option<EncodingValidator>,
    expected: u64,
    line_start: MimePosition,
    column: u64,
    cr: bool,
    oversized: bool,
    detect_null: bool,
    detect_8bit: bool,
}

impl Body {
    fn new(encoding: ContentEncoding, position: MimePosition) -> Self {
        Self {
            validator: None,
            expected: position.offset,
            line_start: position,
            column: 1,
            cr: false,
            oversized: false,
            // Binary explicitly permits nulls and high bytes. 8bit permits high
            // bytes but not nulls; all other encodings must remain 7-bit clean.
            detect_null: encoding != ContentEncoding::Binary,
            detect_8bit: !matches!(
                encoding,
                ContentEncoding::Binary | ContentEncoding::EightBit
            ),
        }
    }

    fn scan<L: MimeComplianceLogger + ?Sized>(
        &mut self,
        active: &mut Active<'_, L>,
        mut bytes: &[u8],
        mut position: MimePosition,
    ) -> io::Result<()> {
        if position.offset != self.expected {
            // Returning to an ancestor's epilogue skips the child's headers,
            // content and delimiter; those bytes are not one long content line.
            self.line_start = position;
            self.column = 1;
            self.cr = false;
            self.oversized = false;
        }
        while !bytes.is_empty() {
            // Reuse the header line scanner rather than examining every byte
            // again for NUL and 8-bit detection while compliance is enabled.
            let scanned = active
                .scanner
                .line(bytes, self.detect_null, self.detect_8bit);
            let length = scanned.lf.map_or(bytes.len(), |i| i + 1);
            let line = &bytes[..length];
            let oversized = if self.oversized {
                length
            } else {
                1001u64.saturating_sub(self.column).min(length as u64) as usize
            };
            // Rust reports exact body bytes, not MimeKit's refill-dependent
            // line-fragment start. Retain wire order, including a special byte
            // at the oversized-line threshold, before any LF diagnostic.
            let mut issues = [
                (
                    scanned.null.unwrap_or(length),
                    Violation::UnexpectedNullBytesInBody,
                ),
                (
                    scanned.eight_bit.unwrap_or(length),
                    Violation::Unexpected8BitBytesInBody,
                ),
                (oversized, Violation::OversizedLine),
            ];
            issues.sort_unstable_by_key(|&(index, violation)| {
                (index, violation == Violation::OversizedLine)
            });
            for (index, violation) in issues {
                if index == length {
                    continue;
                }
                match violation {
                    Violation::OversizedLine => {
                        self.oversized = true;
                        active.issue(violation, self.line_start, 1, PositionKind::LineStart)?;
                    }
                    _ => {
                        if violation == Violation::UnexpectedNullBytesInBody {
                            self.detect_null = false;
                        } else {
                            self.detect_8bit = false;
                        }
                        active.issue(
                            violation,
                            MimePosition {
                                offset: position.offset + index as u64,
                                line: position.line,
                            },
                            self.column + index as u64,
                            PositionKind::Exact,
                        )?;
                    }
                }
            }
            if scanned.lf.is_some() {
                let cr = if length == 1 {
                    self.cr
                } else {
                    line[length - 2] == b'\r'
                };
                if !cr {
                    active.issue(
                        Violation::BareLinefeedInBody,
                        MimePosition {
                            offset: position.offset + length as u64 - 1,
                            line: position.line,
                        },
                        self.column + length as u64 - 1,
                        PositionKind::Exact,
                    )?;
                }
                position.line += 1;
                self.line_start = MimePosition {
                    offset: position.offset + length as u64,
                    line: position.line,
                };
                self.column = 1;
                self.oversized = false;
            } else {
                self.column += length as u64;
            }
            self.cr = line.last() == Some(&b'\r');
            position.offset += length as u64;
            bytes = &bytes[length..];
        }
        self.expected = position.offset;
        Ok(())
    }
}
