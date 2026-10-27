// Adapted from MimeKit's MimeReader.cs and CappedComplianceLogger.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io;

use super::encoding::EncodingValidator;
use super::{
    Budget, MimeComplianceIssue, MimeComplianceLogger, MimeComplianceOptions,
    MimeCompliancePositionKind as PositionKind, MimeComplianceViolation as Violation,
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
            } => {
                let mut body = Body::new(*encoding, *headers_end);
                if *kind == MimeEntityKind::Part {
                    body.validator = EncodingValidator::new(*encoding, self.options, *headers_end);
                }
                self.bodies.push(body);
            }
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
        let mut seen = 0u32;
        let mut resent = 0u32;
        let mut content_type = None;
        let mut encoding = None;
        let mut mime_version = false;
        let mut end_column = 1;
        for header in headers {
            let raw = header.raw();
            if let Some(offset) = header.offset() {
                position.offset = offset;
            }
            let start = position;
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
                    } else if blank.is_some() || byte.is_ascii_control() {
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
                            index as u64 + 1,
                            PositionKind::Exact,
                        )
                    });
                self.issue(Violation::InvalidHeader, position, column, kind)?;
            }
            let mut bare_cr = false;
            let mut null = false;
            for (number, line) in raw.split_inclusive(|&b| b == b'\n').enumerate() {
                if line.len() > 1000 {
                    self.issue(
                        Violation::OversizedLine,
                        position,
                        1,
                        PositionKind::LineStart,
                    )?;
                }
                if !null && line.contains(&0) {
                    null = true;
                    self.issue(
                        Violation::UnexpectedNullBytesInHeader,
                        position,
                        1,
                        PositionKind::LineStart,
                    )?;
                }
                if !bare_cr
                    && let Some(index) = line
                        .iter()
                        .enumerate()
                        .position(|(i, &byte)| byte == b'\r' && line.get(i + 1) != Some(&b'\n'))
                {
                    bare_cr = true;
                    self.issue(
                        Violation::BareCarriageReturnInHeader,
                        position.advance(&line[..index]),
                        index as u64 + 1,
                        PositionKind::Exact,
                    )?;
                }
                if line.ends_with(b"\n") && !line.ends_with(b"\r\n") {
                    self.issue(
                        Violation::BareLinefeedInHeader,
                        position.advance(&line[..line.len() - 1]),
                        line.len() as u64,
                        PositionKind::Exact,
                    )?;
                }
                if number != 0
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
                    line.len() as u64 + 1
                };
            }
            if std::str::from_utf8(raw).is_err() {
                self.issue(
                    Violation::Unexpected8BitBytesInHeader,
                    start,
                    1,
                    PositionKind::ElementStart,
                )?;
            }
            // An invalid raw record has no value, even if its field bytes
            // happen to spell a known header name at EOF.
            if header.is_invalid() {
                continue;
            }
            if message {
                let repeated = repeated_header(header.id());
                let index = repeated.map(|v| v as u32 - Violation::RepeatedDate as u32);
                if index.is_none_or(|index| index < 12) {
                    // Resent fields for a single resending are grouped together.
                    // Any other field ends that block, but not ordinary counts.
                    resent = 0;
                }
                if let (Some(violation), Some(index)) = (repeated, index) {
                    let flags = if index < 12 { &mut seen } else { &mut resent };
                    let mask = 1 << index;
                    let duplicate = *flags & mask != 0;
                    *flags |= mask;
                    if duplicate {
                        self.issue(violation, start, 1, PositionKind::ElementStart)?;
                    }
                }
            }
            match header.id() {
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
                        let (value_position, column) = header_value_position(header, start);
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

fn header_value_position(header: &Header, start: MimePosition) -> (MimePosition, u64) {
    // A header begins at column one. Skip folding whitespace after the colon to
    // identify the value even when it starts on a subsequent physical line.
    let raw = header.raw();
    let value = header.raw_value();
    let value_start = raw.len() - value.len();
    let index = value
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
    // An empty value must not point into the next header: use the byte just
    // after the colon rather than the end of its whitespace.
    let index = value_start + index.unwrap_or(0);
    let prefix = &raw[..index];
    let column = prefix
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(index as u64 + 1, |lf| (index - lf) as u64);
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
        bytes: &[u8],
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
        for &byte in bytes {
            if self.detect_null && byte == 0 {
                self.detect_null = false;
                active.issue(
                    Violation::UnexpectedNullBytesInBody,
                    position,
                    self.column,
                    PositionKind::Exact,
                )?;
            }
            if self.detect_8bit && !byte.is_ascii() {
                self.detect_8bit = false;
                active.issue(
                    Violation::Unexpected8BitBytesInBody,
                    position,
                    self.column,
                    PositionKind::Exact,
                )?;
            }
            if !self.oversized && self.column > 1000 {
                self.oversized = true;
                active.issue(
                    Violation::OversizedLine,
                    self.line_start,
                    1,
                    PositionKind::LineStart,
                )?;
            }
            if byte == b'\n' {
                if !self.cr {
                    active.issue(
                        Violation::BareLinefeedInBody,
                        position,
                        self.column,
                        PositionKind::Exact,
                    )?;
                }
                position.line += 1;
                self.line_start = MimePosition {
                    offset: position.offset + 1,
                    line: position.line,
                };
                self.column = 1;
                self.oversized = false;
            } else {
                self.column += 1;
            }
            self.cr = byte == b'\r';
            position.offset += 1;
        }
        self.expected = position.offset;
        Ok(())
    }
}
