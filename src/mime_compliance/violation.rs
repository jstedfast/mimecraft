// Adapted from MimeKit's MimeComplianceViolation.cs and MimeComplianceIssue.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{MimeComplianceCategories, MimeComplianceContext, MimeComplianceSeverity};

/// A MIME compliance defect, or notification that reporting was capped.
///
/// Numeric values match MimeKit and are append-only so persisted diagnostics
/// keep their meaning. Rust does not need MimeKit's uninitialized `None` sentinel.
/// The enum includes checks not yet implemented by the reader; see
/// [`crate::MimeReader::read_message_with_logger`] for reporting coverage.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
#[repr(u32)]
pub enum MimeComplianceViolation {
    /// A bare carriage return character was found in a MIME part or message header.
    BareCarriageReturnInHeader = 1,
    /// A bare linefeed character was found in a MIME part or message header.
    BareLinefeedInHeader = 2,
    /// A bare linefeed character was found in the body of the message.
    BareLinefeedInBody = 3,
    /// A line was found that exceeded the maximum line length permitted by the Internet Message Format.
    OversizedLine = 4,
    /// A MIME part or message header contained 8-bit bytes where only 7-bit bytes were expected.
    Unexpected8BitBytesInHeader = 5,
    /// A MIME part's body contained 8-bit content where only 7-bit content was expected.
    Unexpected8BitBytesInBody = 6,
    /// A MIME part or message header contained illegal null (`0x00`) bytes.
    UnexpectedNullBytesInHeader = 7,
    /// A MIME part's body contained null (`0x00`) bytes without specifying a binary transfer encoding.
    UnexpectedNullBytesInBody = 8,
    /// A MIME part or message header contained control (or whitespace) characters in the field name.
    InvalidHeader = 9,
    /// A MIME part or message header ended prematurely at the end of the stream.
    IncompleteHeader = 10,
    /// A folded MIME part or message header contained a continuation line made up entirely of whitespace.
    BlankHeaderContinuationLine = 11,
    /// A MIME part contained multiple Content-Type headers.
    RepeatedContentType = 12,
    /// A MIME part contained multiple Content-Transfer-Encoding headers.
    RepeatedContentTransferEncoding = 13,
    /// The message contained more than one Date header field.
    RepeatedDate = 14,
    /// The message contained more than one From header field.
    RepeatedFrom = 15,
    /// The message contained more than one Sender header field.
    RepeatedSender = 16,
    /// The message contained more than one Reply-To header field.
    RepeatedReplyTo = 17,
    /// The message contained more than one To header field.
    RepeatedTo = 18,
    /// The message contained more than one Cc header field.
    RepeatedCc = 19,
    /// The message contained more than one Bcc header field.
    RepeatedBcc = 20,
    /// The message contained more than one Message-Id header field.
    RepeatedMessageId = 21,
    /// The message contained more than one In-Reply-To header field.
    RepeatedInReplyTo = 22,
    /// The message contained more than one References header field.
    RepeatedReferences = 23,
    /// The message contained more than one Subject header field.
    RepeatedSubject = 24,
    /// The message contained more than one Return-Path header field.
    RepeatedReturnPath = 25,
    /// A block of resent header fields contained more than one Resent-Date header field.
    RepeatedResentDate = 26,
    /// A block of resent header fields contained more than one Resent-From header field.
    RepeatedResentFrom = 27,
    /// A block of resent header fields contained more than one Resent-Sender header field.
    RepeatedResentSender = 28,
    /// A block of resent header fields contained more than one Resent-To header field.
    RepeatedResentTo = 29,
    /// A block of resent header fields contained more than one Resent-Cc header field.
    RepeatedResentCc = 30,
    /// A block of resent header fields contained more than one Resent-Bcc header field.
    RepeatedResentBcc = 31,
    /// A block of resent header fields contained more than one Resent-Message-Id header field.
    RepeatedResentMessageId = 32,
    /// The message did not contain a Date header field.
    MissingDate = 33,
    /// The message did not contain a From header field.
    MissingFrom = 34,
    /// The message did not contain a Message-Id header field.
    MissingMessageId = 35,
    /// The message contained MIME header fields but no MIME-Version header field.
    MissingMimeVersion = 36,
    /// A Content-Type header value was not valid.
    InvalidContentType = 37,
    /// A Content-Transfer-Encoding header value was not valid.
    InvalidContentTransferEncoding = 38,
    /// A Content-Transfer-Encoding header for a message/rfc822 part contained an illegal value.
    IllegalMessageRfc822ContentTransferEncoding = 39,
    /// A Content-Transfer-Encoding header for a multipart contained an illegal value.
    IllegalMultipartContentTransferEncoding = 40,
    /// A boundary parameter was missing from a multipart Content-Type header.
    MissingMultipartBoundaryParameter = 41,
    /// A boundary parameter in a multipart Content-Type header was not valid.
    InvalidMultipartBoundaryParameter = 42,
    /// An address contained more angle brackets than the one pair that delimits an angle-addr.
    ExcessiveAngleBracketsInAddress = 43,
    /// An address had an opening angle bracket without a closing one, or a closing bracket without an opening one.
    UnbalancedAngleBracketsInAddress = 44,
    /// An address contained a quoted-string that was never closed.
    UnbalancedQuotesInAddress = 45,
    /// An address contained an unbalanced parenthesis in a comment.
    UnbalancedParenthesesInAddress = 46,
    /// The display-name of an address contained a special character that should have been quoted.
    UnquotedDisplayName = 47,
    /// The display-name of a mailbox was itself shaped like an address.
    AddressInDisplayName = 48,
    /// The unquoted display-name of a mailbox was itself shaped like an address.
    UnquotedAddressInDisplayName = 49,
    /// The display-name of an address group was itself shaped like an address.
    AddressInGroupDisplayName = 50,
    /// The unquoted display-name of an address group was itself shaped like an address.
    UnquotedAddressInGroupDisplayName = 51,
    /// The local-part of an address was not a valid dot-atom or quoted-string.
    InvalidLocalPart = 52,
    /// Two addresses in an address list were not separated by a comma.
    MissingAddressSeparator = 53,
    /// An address list contained two addresses whose boundary was ambiguous.
    AmbiguousMailboxBoundary = 54,
    /// An address list contained a comma that did not separate two addresses.
    ExtraneousCommaInAddressList = 55,
    /// An address used the obsolete source route syntax.
    ObsoleteRouteAddress = 56,
    /// An address consisted of a local-part with no domain.
    AddressWithoutDomain = 57,
    /// The domain of an address used the obsolete syntax that allows comments and whitespace between its parts.
    ObsoleteDomainSyntax = 58,
    /// The domain of an address was not a valid dot-atom.
    InvalidDomain = 59,
    /// The domain of an address ended with a dot.
    TrailingDotInDomain = 60,
    /// A domain-literal contained whitespace.
    WhitespaceInDomainLiteral = 61,
    /// A domain-literal contained a character that the domain-literal syntax does not permit.
    InvalidCharacterInDomainLiteral = 62,
    /// An address contained 8-bit bytes that were not valid UTF-8.
    Invalid8BitAddress = 63,
    /// An address group was not terminated with a semi-colon.
    MissingGroupTerminator = 64,
    /// An address did not conform to the address syntax defined by rfc5322.
    NonConformantAddress = 65,
    /// An address contained a null byte.
    NullByteInAddress = 66,
    /// The display-name of an address or group contained a null byte.
    NullByteInDisplayName = 67,
    /// A line break appeared inside a local-part or domain.
    LineBreakInAddress = 68,
    /// An address contained a control character.
    ControlCharacterInAddress = 69,
    /// The local-part of an address contained an ISO-2022 shift or escape sequence.
    Iso2022SequenceInLocalPart = 70,
    /// An address group had an empty name.
    EmptyGroupName = 71,
    /// An address group was nested inside another address group.
    NestedAddressGroup = 72,
    /// An empty line separating the headers from the body was missing.
    MissingBodySeparator = 73,
    /// A multipart boundary was missing.
    MissingMultipartBoundary = 74,
    /// The MIME structure was nested more deeply than the parser allows.
    MimeDepthLimitExceeded = 75,
    /// The base64 encoded content of a MIME part ended with an incomplete quantum.
    IncompleteBase64Quantum = 76,
    /// The base64 encoded content of a MIME part contained invalid characters.
    InvalidBase64Character = 77,
    /// The base64 encoded content of a MIME part contained invalid padding.
    InvalidBase64Padding = 78,
    /// The base64 encoded content of a MIME part contained characters after the padding.
    Base64CharactersAfterPadding = 79,
    /// The base64 encoded content of a MIME part contained an obsolete comment.
    ObsoleteBase64Comment = 80,
    /// The quoted-printable encoded content of a MIME part contained an invalid hex sequence after an '=' character.
    InvalidQuotedPrintableEncoding = 81,
    /// The quoted-printable encoded content of a MIME part contained an invalid soft-break sequence.
    InvalidQuotedPrintableSoftBreak = 82,
    /// The uuencoded content of a MIME part contained non-whitespace content before the begin marker.
    InvalidUUEncodePretext = 83,
    /// The uuencoded content of a MIME part had an invalid file mode in the begin marker.
    InvalidUUEncodeFileMode = 84,
    /// The uuencoded content of a MIME part contained invalid characters or was otherwise malformed.
    InvalidUUEncodedContent = 85,
    /// The uuencoded content of a MIME part had an invalid encoded line length.
    InvalidUUEncodedLineLength = 86,
    /// The uuencoded content of a MIME part contained an incomplete encoded line.
    IncompleteUUEncodedLine = 87,
    /// The uuencoded content of a MIME part had extra data beyond the end of a uuencoded line.
    InvalidUUEncodedLineExtraData = 88,
    /// The uuencoded content of a MIME part contained non-whitespace content after the end marker.
    InvalidUUEncodeEndMarker = 89,
    /// The uuencoded content of a MIME part did not properly end.
    IncompleteUUEncodedContent = 90,
    /// Too many compliance issues were detected and the remainder were suppressed.
    TooManyComplianceIssues = 2147483647,
}

impl MimeComplianceViolation {
    /// All supported diagnostic identifiers, including the truncation marker.
    pub const ALL: [Self; 91] = [
        Self::BareCarriageReturnInHeader,
        Self::BareLinefeedInHeader,
        Self::BareLinefeedInBody,
        Self::OversizedLine,
        Self::Unexpected8BitBytesInHeader,
        Self::Unexpected8BitBytesInBody,
        Self::UnexpectedNullBytesInHeader,
        Self::UnexpectedNullBytesInBody,
        Self::InvalidHeader,
        Self::IncompleteHeader,
        Self::BlankHeaderContinuationLine,
        Self::RepeatedContentType,
        Self::RepeatedContentTransferEncoding,
        Self::RepeatedDate,
        Self::RepeatedFrom,
        Self::RepeatedSender,
        Self::RepeatedReplyTo,
        Self::RepeatedTo,
        Self::RepeatedCc,
        Self::RepeatedBcc,
        Self::RepeatedMessageId,
        Self::RepeatedInReplyTo,
        Self::RepeatedReferences,
        Self::RepeatedSubject,
        Self::RepeatedReturnPath,
        Self::RepeatedResentDate,
        Self::RepeatedResentFrom,
        Self::RepeatedResentSender,
        Self::RepeatedResentTo,
        Self::RepeatedResentCc,
        Self::RepeatedResentBcc,
        Self::RepeatedResentMessageId,
        Self::MissingDate,
        Self::MissingFrom,
        Self::MissingMessageId,
        Self::MissingMimeVersion,
        Self::InvalidContentType,
        Self::InvalidContentTransferEncoding,
        Self::IllegalMessageRfc822ContentTransferEncoding,
        Self::IllegalMultipartContentTransferEncoding,
        Self::MissingMultipartBoundaryParameter,
        Self::InvalidMultipartBoundaryParameter,
        Self::ExcessiveAngleBracketsInAddress,
        Self::UnbalancedAngleBracketsInAddress,
        Self::UnbalancedQuotesInAddress,
        Self::UnbalancedParenthesesInAddress,
        Self::UnquotedDisplayName,
        Self::AddressInDisplayName,
        Self::UnquotedAddressInDisplayName,
        Self::AddressInGroupDisplayName,
        Self::UnquotedAddressInGroupDisplayName,
        Self::InvalidLocalPart,
        Self::MissingAddressSeparator,
        Self::AmbiguousMailboxBoundary,
        Self::ExtraneousCommaInAddressList,
        Self::ObsoleteRouteAddress,
        Self::AddressWithoutDomain,
        Self::ObsoleteDomainSyntax,
        Self::InvalidDomain,
        Self::TrailingDotInDomain,
        Self::WhitespaceInDomainLiteral,
        Self::InvalidCharacterInDomainLiteral,
        Self::Invalid8BitAddress,
        Self::MissingGroupTerminator,
        Self::NonConformantAddress,
        Self::NullByteInAddress,
        Self::NullByteInDisplayName,
        Self::LineBreakInAddress,
        Self::ControlCharacterInAddress,
        Self::Iso2022SequenceInLocalPart,
        Self::EmptyGroupName,
        Self::NestedAddressGroup,
        Self::MissingBodySeparator,
        Self::MissingMultipartBoundary,
        Self::MimeDepthLimitExceeded,
        Self::IncompleteBase64Quantum,
        Self::InvalidBase64Character,
        Self::InvalidBase64Padding,
        Self::Base64CharactersAfterPadding,
        Self::ObsoleteBase64Comment,
        Self::InvalidQuotedPrintableEncoding,
        Self::InvalidQuotedPrintableSoftBreak,
        Self::InvalidUUEncodePretext,
        Self::InvalidUUEncodeFileMode,
        Self::InvalidUUEncodedContent,
        Self::InvalidUUEncodedLineLength,
        Self::IncompleteUUEncodedLine,
        Self::InvalidUUEncodedLineExtraData,
        Self::InvalidUUEncodeEndMarker,
        Self::IncompleteUUEncodedContent,
        Self::TooManyComplianceIssues,
    ];

    /// Rates the practical harm in transport or local storage.
    pub fn severity(self, context: MimeComplianceContext) -> MimeComplianceSeverity {
        match self {
            // Note: These are requirements of the channel rather than of the message itself. On the
            // wire they are real problems, but messages in a local store (especially on UNIX systems)
            // routinely exhibit them without any ill effect.
            Self::BareLinefeedInHeader | Self::BareLinefeedInBody | Self::OversizedLine => {
                if context == MimeComplianceContext::Storage {
                    MimeComplianceSeverity::Minor
                } else {
                    MimeComplianceSeverity::Major
                }
            }
            // Note: 8-bit content is so common that MIME parsers have had to cope with it for
            // decades by falling back to the locale charset or to iso-8859-1 (MimeKit itself tries
            // UTF-8, then the user-supplied charset, then iso-8859-1, which cannot fail). It is
            // therefore no worse on the wire than it is on disk.
            Self::Unexpected8BitBytesInHeader | Self::Unexpected8BitBytesInBody => {
                MimeComplianceSeverity::Minor
            }
            // Note: Duplicate Content-Type and Content-Transfer-Encoding headers and null bytes are
            // the classic MIME "content smuggling" vectors. In each case, a content scanner and an
            // end-user's mail client can be made to disagree about the content of the message.
            // Note: Parsers disagree about whether a bare CR ends a line, so a bare CR can make a
            // scanner and a mail client disagree about which header fields exist and where the
            // header block ends. This is the same primitive that SMTP smuggling relies on.
            // Note: A repeated originator field is the same class of attack applied to the message
            // rather than to a MIME part. An authentication mechanism such as DKIM may cover one
            // instance while the mail client renders another, so the message a filter judges to be
            // safe is not the message the recipient is shown.
            Self::RepeatedContentType
            | Self::RepeatedContentTransferEncoding
            | Self::UnexpectedNullBytesInHeader
            | Self::UnexpectedNullBytesInBody
            | Self::BareCarriageReturnInHeader
            | Self::RepeatedDate
            | Self::RepeatedFrom
            | Self::RepeatedSender
            | Self::RepeatedReplyTo
            | Self::RepeatedMessageId
            | Self::RepeatedReturnPath => MimeComplianceSeverity::Critical,
            // Ambiguity or corruption that different MIME parsers may resolve differently.
            // Note: The characters making up an RFC 1113 comment are themselves valid base64
            // characters, so decoders that do not special-case the '*' delimiters (which is nearly
            // all of them, including MimeKit's own Base64Decoder) silently absorb the comment as
            // content and corrupt everything that follows it.
            // Note: A repeated recipient or subject field cannot be used to defeat an
            // authentication mechanism the way a repeated originator field can, but a filter and a
            // mail client can still be made to disagree about who a message was addressed to or
            // what it claims to be about.
            Self::InvalidHeader
            | Self::IncompleteHeader
            | Self::BlankHeaderContinuationLine
            | Self::InvalidContentType
            | Self::InvalidContentTransferEncoding
            | Self::IllegalMessageRfc822ContentTransferEncoding
            | Self::IllegalMultipartContentTransferEncoding
            | Self::MissingBodySeparator
            | Self::MissingMultipartBoundaryParameter
            | Self::InvalidMultipartBoundaryParameter
            | Self::MissingMultipartBoundary
            | Self::IncompleteBase64Quantum
            | Self::InvalidBase64Character
            | Self::InvalidBase64Padding
            | Self::Base64CharactersAfterPadding
            | Self::ObsoleteBase64Comment
            | Self::InvalidQuotedPrintableEncoding
            | Self::InvalidQuotedPrintableSoftBreak
            | Self::InvalidUUEncodePretext
            | Self::InvalidUUEncodeFileMode
            | Self::InvalidUUEncodedContent
            | Self::InvalidUUEncodedLineLength
            | Self::IncompleteUUEncodedLine
            | Self::InvalidUUEncodedLineExtraData
            | Self::InvalidUUEncodeEndMarker
            | Self::IncompleteUUEncodedContent
            | Self::RepeatedTo
            | Self::RepeatedCc
            | Self::RepeatedBcc
            | Self::RepeatedSubject => MimeComplianceSeverity::Major,
            // Note: Obsolete-but-well-defined syntax and defects that MimeKit normalizes away. The
            // resulting address is still the one the author intended; the risk is only that another
            // implementation may normalize differently.
            // Note: Unlike unbalanced brackets, a repeated bracket leaves no doubt about where the
            // address begins and ends. Discarding the extras is trivial and every implementation
            // that does so arrives at the same mailbox.
            // Note: These affect only how a message is threaded or how its resending history is
            // displayed. Resent fields in particular are strictly informational and must not be
            // used when processing replies, so a repeated one cannot change where a reply is sent.
            Self::ObsoleteRouteAddress
            | Self::ExtraneousCommaInAddressList
            | Self::ObsoleteDomainSyntax
            | Self::TrailingDotInDomain
            | Self::WhitespaceInDomainLiteral
            | Self::ExcessiveAngleBracketsInAddress
            | Self::RepeatedInReplyTo
            | Self::RepeatedReferences
            | Self::RepeatedResentDate
            | Self::RepeatedResentFrom
            | Self::RepeatedResentSender
            | Self::RepeatedResentTo
            | Self::RepeatedResentCc
            | Self::RepeatedResentBcc
            | Self::RepeatedResentMessageId => MimeComplianceSeverity::Minor,
            // Note: Without an author, there is no domain against which to evaluate a DMARC policy,
            // so a message that omits it can sidestep the policy of the domain that sent it.
            Self::MissingFrom => MimeComplianceSeverity::Major,
            // Note: These are common in real-world mail, and agents have long since learned to cope
            // with them: Date and Message-Id are routinely filled in by the submission server, and
            // practically every parser honors the MIME header fields with or without a MIME-Version.
            Self::MissingDate | Self::MissingMessageId | Self::MissingMimeVersion => {
                MimeComplianceSeverity::Minor
            }
            // Note: Ambiguity over where an address begins and ends, or over which mailbox it names.
            Self::UnbalancedAngleBracketsInAddress
            | Self::UnquotedDisplayName
            | Self::UnquotedAddressInDisplayName
            | Self::UnquotedAddressInGroupDisplayName
            | Self::InvalidLocalPart
            | Self::InvalidDomain
            | Self::MissingAddressSeparator
            | Self::AmbiguousMailboxBoundary
            | Self::AddressWithoutDomain
            | Self::InvalidCharacterInDomainLiteral
            | Self::Invalid8BitAddress
            | Self::MissingGroupTerminator
            | Self::NonConformantAddress => MimeComplianceSeverity::Major,
            // Note: The input is legal rfc5322, so these rate as a hint rather than as damage. The
            // harm is entirely in what software does with the display-name, which is why the
            // severity is low but the Security category still applies. The unquoted forms are not
            // legal and are rated Major above.
            Self::AddressInDisplayName | Self::AddressInGroupDisplayName => {
                MimeComplianceSeverity::Minor
            }
            // Note: Both of these are injection primitives rather than mere syntax errors. A null
            // byte truncates the address for anything that treats it as a C string, and a line break
            // inside an address token is the header/command injection technique that unvalidated
            // input concatenated into a header produces. In each case two components can be made to
            // read different mailboxes, or different message boundaries, out of the same header.
            // Note: An unterminated token consumes the rest of the header value, so whatever follows
            // it is not parsed as an address and nothing reports an error. An unterminated quote can
            // still yield a mailbox when an angle-addr falls inside what it absorbs, and can fold a
            // swallowed address into the display-name of the one that survives; an unterminated
            // comment leaves nothing to recover, since text inside a comment carries no meaning.
            // Unlike the ambiguity violations above, where implementations disagree about which
            // mailbox a header names, the lost address is simply gone while the surrounding headers
            // still parse and the message looks intact.
            Self::NullByteInAddress
            | Self::NullByteInDisplayName
            | Self::LineBreakInAddress
            | Self::UnbalancedQuotesInAddress
            | Self::UnbalancedParenthesesInAddress => MimeComplianceSeverity::Critical,
            Self::ControlCharacterInAddress => MimeComplianceSeverity::Major,
            // Note: Unlike an arbitrary control character, this is a deliberate legacy convention
            // rather than damage or an injection attempt, so it is rated alongside the other obsolete
            // address syntax rather than alongside ControlCharacterInAddress.
            Self::Iso2022SequenceInLocalPart => MimeComplianceSeverity::Minor,
            Self::EmptyGroupName => MimeComplianceSeverity::Minor,
            // Note: Parsers disagree about which addresses belong to which group, or whether the header
            // can be parsed at all.
            Self::NestedAddressGroup => MimeComplianceSeverity::Major,
            // Note: This is not a defect in the message, but some of its content was not parsed, and a
            // consumer that only looks at Major and above needs to know that.
            Self::MimeDepthLimitExceeded => MimeComplianceSeverity::Major,
            // Note: This is not a defect in the message, but a consumer that only looks at Major and
            // above still needs to be told that the report it is looking at is incomplete. Rating it
            // any lower would let a flood of minor issues hide the truncation as well as the
            // violations the truncation suppressed.
            Self::TooManyComplianceIssues => MimeComplianceSeverity::Major,
        }
    }

    /// Returns the kinds of harm, independently of severity.
    pub fn categories(self) -> MimeComplianceCategories {
        let interop = MimeComplianceCategories::INTEROPERABILITY;
        let dataloss = MimeComplianceCategories::DATA_LOSS;
        let security = MimeComplianceCategories::SECURITY;
        match self {
            // Note: A bare linefeed is the basis of SMTP smuggling. A sender and a receiver that
            // disagree about whether a bare linefeed terminates a line can be made to disagree about
            // where one message ends and the next begins.
            Self::BareLinefeedInHeader | Self::BareLinefeedInBody => interop | security,
            // Note: A malformed header may be treated as a header by one parser and as the start of
            // the body (or as a continuation of the previous header) by another.
            Self::InvalidHeader => interop | security,
            // Note: A parser that strips trailing whitespace sees a whitespace-only continuation line
            // as the empty line that ends the header block, so the header fields that follow it may be
            // read as headers by one parser and as the start of the body by another.
            Self::BlankHeaderContinuationLine => interop | security,
            // Note: A truncated header is unlikely to be interpreted differently by different
            // parsers, but whatever it was meant to say has been lost.
            Self::IncompleteHeader => interop,
            // Note: When the Content-Type cannot be parsed, parsers fall back to different defaults,
            // which is a classic way of getting a scanner to skip content that a client will render.
            Self::InvalidContentType | Self::RepeatedContentType => interop | security,
            // Note: As above, but an unrecognized or duplicated encoding also means the content may
            // be decoded incorrectly (or not at all), so content can be lost as well.
            Self::InvalidContentTransferEncoding | Self::RepeatedContentTransferEncoding => {
                interop | dataloss | security
            }
            // Note: Encoding a message/rfc822 or multipart body part hides its internal structure
            // from any scanner that does not decode it, which is a well-known evasion technique.
            Self::IllegalMessageRfc822ContentTransferEncoding
            | Self::IllegalMultipartContentTransferEncoding => interop | security,
            // Note: An over-long line may be folded or truncated in transit, which alters the content.
            Self::OversizedLine => interop | dataloss,
            // Note: Without a blank line, parsers disagree about where the headers stop and the body
            // starts, so the same bytes can be read as either.
            Self::MissingBodySeparator => interop | security,
            // Note: Without a usable boundary, the body parts cannot be separated and are likely to
            // be presented as a single blob of text instead.
            Self::MissingMultipartBoundaryParameter | Self::MissingMultipartBoundary => {
                interop | dataloss
            }
            // Note: The entities nested beyond the limit were not parsed, so a content scanner that
            // relies on the result has not examined them.
            Self::MimeDepthLimitExceeded => dataloss | security,
            // Note: A boundary that needed to be repaired may be repaired differently elsewhere,
            // which changes which bytes belong to which part.
            Self::InvalidMultipartBoundaryParameter => interop | dataloss | security,
            // Note: Unencoded 8-bit bytes carry no charset information, so the text is decoded by
            // guesswork and may be mangled. Parsers have coped with this for decades, so it is not
            // much of an interoperability problem in practice.
            Self::Unexpected8BitBytesInHeader | Self::Unexpected8BitBytesInBody => {
                interop | dataloss
            }
            // Note: Software written in C or C++ treats a null byte as the end of a string, so a null
            // byte can be used to hide everything after it from one program but not from another.
            Self::UnexpectedNullBytesInHeader | Self::UnexpectedNullBytesInBody => {
                interop | security
            }
            // Note: Parsers disagree about whether a bare CR ends a line, so the same bytes can be
            // read as different header fields, or as the end of the header block, by different parsers.
            Self::BareCarriageReturnInHeader => interop | security,
            // Note: The bits in an incomplete quantum cannot be recovered.
            Self::IncompleteBase64Quantum | Self::InvalidBase64Padding => dataloss,
            // Note: Decoders disagree about whether to skip an invalid character or to stop, so they
            // can derive different content from the same bytes.
            Self::InvalidBase64Character => dataloss | security,
            // Note: A decoder that stops at the padding and one that keeps going will produce
            // different content, so data can be hidden after the padding.
            Self::Base64CharactersAfterPadding => dataloss | security,
            // Note: The letters inside an RFC 1113 comment are themselves valid base64 characters, so
            // a decoder that does not recognize the comment absorbs them as data and silently
            // corrupts everything that follows.
            Self::ObsoleteBase64Comment => dataloss | security,
            // Note: Malformed quoted-printable is repaired differently by different decoders.
            Self::InvalidQuotedPrintableEncoding | Self::InvalidQuotedPrintableSoftBreak => {
                dataloss
            }
            // Note: The uuencode header is only used to locate the start of the encoded content and
            // to name the file. Getting it wrong does not corrupt the content itself.
            Self::InvalidUUEncodePretext
            | Self::InvalidUUEncodeEndMarker
            | Self::InvalidUUEncodeFileMode => interop,
            // Note: Decoders disagree about whether to trust the length character or the line, so the
            // extra data is discarded by some and decoded as content by others.
            Self::InvalidUUEncodedLineExtraData => interop | dataloss,
            // Note: Content that cannot be decoded is content that is lost.
            Self::InvalidUUEncodedContent | Self::IncompleteUUEncodedContent => dataloss,
            // Note: A line whose length does not match its length character is ambiguous; decoders
            // disagree about whether to trust the character or the line.
            Self::InvalidUUEncodedLineLength | Self::IncompleteUUEncodedLine => dataloss,
            // Note: An unclosed quote, comment or group absorbs everything that follows it, so the
            // caller is handed a recipient list that is quietly shorter than the one on the wire.
            Self::UnbalancedQuotesInAddress
            | Self::UnbalancedParenthesesInAddress
            | Self::MissingGroupTerminator => interop | dataloss | security,
            // Note: Each of these changes how many addresses a parser sees, or which mailbox one of
            // them names, so two implementations can extract genuinely different recipient lists.
            Self::UnquotedDisplayName
            | Self::MissingAddressSeparator
            | Self::InvalidCharacterInDomainLiteral
            | Self::Invalid8BitAddress => interop | dataloss,
            // Note: The two readings of an ambiguous boundary disagree about which mailbox the
            // address names, not merely about how many there are, so a filter and a mail client can
            // be made to act on different addresses in the same header.
            Self::AmbiguousMailboxBoundary => interop | security,
            // Note: Nothing is lost and nothing is ambiguous to a conforming parser. The harm is
            // that software which displays the display-name in place of the address, or the group
            // name in place of its members, shows a mailbox that will not receive the reply.
            Self::AddressInDisplayName | Self::AddressInGroupDisplayName => security,
            // Note: The unquoted forms carry the same spoofing risk, but because the phrase ends at
            // the unquoted special rather than at the quote, a parser can also disagree about how
            // many addresses the header contains and which mailboxes they name.
            Self::UnquotedAddressInDisplayName | Self::UnquotedAddressInGroupDisplayName => {
                interop | dataloss | security
            }
            // Note: Syntax that another implementation may read differently, or repair differently,
            // but with no reading under which an address goes missing.
            Self::ExcessiveAngleBracketsInAddress
            | Self::UnbalancedAngleBracketsInAddress
            | Self::InvalidLocalPart
            | Self::InvalidDomain
            | Self::AddressWithoutDomain
            | Self::ObsoleteRouteAddress
            | Self::ExtraneousCommaInAddressList
            | Self::ObsoleteDomainSyntax
            | Self::TrailingDotInDomain
            | Self::WhitespaceInDomainLiteral
            | Self::NonConformantAddress => interop,
            Self::NullByteInAddress | Self::NullByteInDisplayName | Self::LineBreakInAddress => {
                interop | dataloss | security
            }
            Self::ControlCharacterInAddress => interop | security,
            Self::Iso2022SequenceInLocalPart => interop,
            Self::EmptyGroupName => interop,
            // Note: A parser that does not support nested groups may end the outer group early or
            // reject the header, so the recipient list it extracts can be missing addresses.
            Self::NestedAddressGroup => interop | dataloss,
            // Note: RFC 5322 section 3.6 limits each of these header fields to one occurrence. A
            // message that repeats one can be made to present different originators, recipients or
            // subjects to a filter than it presents to the recipient.
            Self::RepeatedDate
            | Self::RepeatedFrom
            | Self::RepeatedSender
            | Self::RepeatedReplyTo
            | Self::RepeatedTo
            | Self::RepeatedCc
            | Self::RepeatedBcc
            | Self::RepeatedMessageId
            | Self::RepeatedSubject
            | Self::RepeatedReturnPath => interop | security,
            // Note: These affect only threading and the display of the resending history.
            Self::RepeatedInReplyTo
            | Self::RepeatedReferences
            | Self::RepeatedResentDate
            | Self::RepeatedResentFrom
            | Self::RepeatedResentSender
            | Self::RepeatedResentTo
            | Self::RepeatedResentCc
            | Self::RepeatedResentBcc
            | Self::RepeatedResentMessageId => interop,
            // Note: Mail clients disagree about whom to present as the author when there is no From
            // header field, and a DMARC verifier has no author domain whose policy it could apply.
            Self::MissingFrom => interop | security,
            Self::MissingDate | Self::MissingMessageId | Self::MissingMimeVersion => interop,
            // Note: The message did not cause this, so no category describes harm that the message does.
            // Security applies all the same: a suppressed report is a blind spot, and hiding a violation
            // from whatever is inspecting the message is exactly what an attacker would want out of a
            // flood of cheap ones.
            Self::TooManyComplianceIssues => security,
        }
    }

    /// Returns a short description of the defect.
    pub fn description(self) -> &'static str {
        match self {
            Self::BareLinefeedInHeader => {
                "A bare linefeed character was found in a MIME part or message header."
            }
            Self::BareLinefeedInBody => {
                "A bare linefeed character was found in the body of the message."
            }
            Self::OversizedLine => {
                "A line was found that was longer than the SMTP limit of 1000 characters."
            }
            Self::Unexpected8BitBytesInHeader => {
                "A MIME part or message header contained 8-bit bytes where only 7-bit bytes were expected."
            }
            Self::Unexpected8BitBytesInBody => {
                "A MIME part's body contained 8-bit content where only 7-bit content was expected."
            }
            Self::UnexpectedNullBytesInHeader => {
                "A MIME part or message header contained illegal null (0x00) bytes."
            }
            Self::UnexpectedNullBytesInBody => {
                "A MIME part's body contained null (0x00) bytes without specifying a binary transfer encoding."
            }
            Self::BareCarriageReturnInHeader => {
                "A bare carriage return character was found in a MIME part or message header."
            }
            Self::InvalidHeader => {
                "A MIME part or message header contained control (or whitespace) characters in the field name."
            }
            Self::IncompleteHeader => {
                "A MIME part or message header ended prematurely at the end of the stream."
            }
            Self::BlankHeaderContinuationLine => {
                "A folded MIME part or message header contained a continuation line made up entirely of whitespace."
            }
            Self::RepeatedContentType => "A MIME part contained multiple Content-Type headers.",
            Self::RepeatedContentTransferEncoding => {
                "A MIME part contained multiple Content-Transfer-Encoding headers."
            }
            Self::RepeatedDate => "The message contained more than one Date header field.",
            Self::RepeatedFrom => "The message contained more than one From header field.",
            Self::RepeatedSender => "The message contained more than one Sender header field.",
            Self::RepeatedReplyTo => "The message contained more than one Reply-To header field.",
            Self::RepeatedTo => "The message contained more than one To header field.",
            Self::RepeatedCc => "The message contained more than one Cc header field.",
            Self::RepeatedBcc => "The message contained more than one Bcc header field.",
            Self::RepeatedMessageId => {
                "The message contained more than one Message-Id header field."
            }
            Self::RepeatedInReplyTo => {
                "The message contained more than one In-Reply-To header field."
            }
            Self::RepeatedReferences => {
                "The message contained more than one References header field."
            }
            Self::RepeatedSubject => "The message contained more than one Subject header field.",
            Self::RepeatedReturnPath => {
                "The message contained more than one Return-Path header field."
            }
            Self::RepeatedResentDate => {
                "A block of resent header fields contained more than one Resent-Date header field."
            }
            Self::RepeatedResentFrom => {
                "A block of resent header fields contained more than one Resent-From header field."
            }
            Self::RepeatedResentSender => {
                "A block of resent header fields contained more than one Resent-Sender header field."
            }
            Self::RepeatedResentTo => {
                "A block of resent header fields contained more than one Resent-To header field."
            }
            Self::RepeatedResentCc => {
                "A block of resent header fields contained more than one Resent-Cc header field."
            }
            Self::RepeatedResentBcc => {
                "A block of resent header fields contained more than one Resent-Bcc header field."
            }
            Self::RepeatedResentMessageId => {
                "A block of resent header fields contained more than one Resent-Message-Id header field."
            }
            Self::MissingDate => "The message did not contain a Date header field.",
            Self::MissingFrom => "The message did not contain a From header field.",
            Self::MissingMessageId => "The message did not contain a Message-Id header field.",
            Self::MissingMimeVersion => {
                "The message contained MIME header fields but no MIME-Version header field."
            }
            Self::InvalidContentType => "A Content-Type header value was not valid.",
            Self::InvalidContentTransferEncoding => {
                "A Content-Transfer-Encoding header value was not valid."
            }
            Self::IllegalMessageRfc822ContentTransferEncoding => {
                "A Content-Transfer-Encoding header for a message/rfc822 part contained an illegal value."
            }
            Self::IllegalMultipartContentTransferEncoding => {
                "A Content-Transfer-Encoding header for a multipart contained an illegal value."
            }
            Self::MissingMultipartBoundaryParameter => {
                "A boundary parameter was missing from a multipart Content-Type header."
            }
            Self::InvalidMultipartBoundaryParameter => {
                "A boundary parameter in a multipart Content-Type header was not valid."
            }
            Self::ExcessiveAngleBracketsInAddress => {
                "An address contained more angle brackets than the one pair that delimits an angle-addr."
            }
            Self::UnbalancedAngleBracketsInAddress => {
                "An address had an opening angle bracket without a closing one, or a closing bracket without an opening one."
            }
            Self::UnbalancedQuotesInAddress => {
                "An address contained a quoted-string that was never closed."
            }
            Self::UnbalancedParenthesesInAddress => {
                "An address contained an unbalanced parenthesis in a comment."
            }
            Self::UnquotedDisplayName => {
                "The display-name of an address contained a special character that should have been quoted."
            }
            Self::AddressInDisplayName => {
                "The display-name of a mailbox was itself shaped like an address."
            }
            Self::UnquotedAddressInDisplayName => {
                "The unquoted display-name of a mailbox was itself shaped like an address."
            }
            Self::AddressInGroupDisplayName => {
                "The display-name of an address group was itself shaped like an address."
            }
            Self::UnquotedAddressInGroupDisplayName => {
                "The unquoted display-name of an address group was itself shaped like an address."
            }
            Self::InvalidLocalPart => {
                "The local-part of an address was not a valid dot-atom or quoted-string."
            }
            Self::MissingAddressSeparator => {
                "Two addresses in an address list were not separated by a comma."
            }
            Self::AmbiguousMailboxBoundary => {
                "An address list contained two addresses whose boundary was ambiguous."
            }
            Self::ExtraneousCommaInAddressList => {
                "An address list contained a comma that did not separate two addresses."
            }
            Self::ObsoleteRouteAddress => "An address used the obsolete source route syntax.",
            Self::AddressWithoutDomain => "An address consisted of a local-part with no domain.",
            Self::ObsoleteDomainSyntax => {
                "The domain of an address used the obsolete syntax that allows comments and whitespace between its parts."
            }
            Self::InvalidDomain => "The domain of an address was not a valid dot-atom.",
            Self::TrailingDotInDomain => "The domain of an address ended with a dot.",
            Self::WhitespaceInDomainLiteral => "A domain-literal contained whitespace.",
            Self::InvalidCharacterInDomainLiteral => {
                "A domain-literal contained a character that the domain-literal syntax does not permit."
            }
            Self::Invalid8BitAddress => {
                "An address contained 8-bit bytes that were not valid UTF-8."
            }
            Self::MissingGroupTerminator => {
                "An address group was not terminated with a semi-colon."
            }
            Self::NonConformantAddress => {
                "An address did not conform to the address syntax defined by rfc5322."
            }
            Self::NullByteInAddress => "An address contained a null byte.",
            Self::NullByteInDisplayName => {
                "The display-name of an address or group contained a null byte."
            }
            Self::LineBreakInAddress => "A line break appeared inside a local-part or domain.",
            Self::ControlCharacterInAddress => "An address contained a control character.",
            Self::Iso2022SequenceInLocalPart => {
                "The local-part of an address contained an ISO-2022 shift or escape sequence."
            }
            Self::EmptyGroupName => "An address group had an empty name.",
            Self::NestedAddressGroup => "An address group was nested inside another address group.",
            Self::MissingBodySeparator => {
                "An empty line separating the headers from the body was missing."
            }
            Self::MissingMultipartBoundary => "A multipart boundary was missing.",
            Self::MimeDepthLimitExceeded => {
                "MIME entities were nested more deeply than the configured limit."
            }
            Self::IncompleteBase64Quantum => {
                "The base64 encoded content of a MIME part ended with an incomplete quantum."
            }
            Self::InvalidBase64Character => {
                "The base64 encoded content of a MIME part contained invalid characters."
            }
            Self::InvalidBase64Padding => {
                "The base64 encoded content of a MIME part contained invalid padding."
            }
            Self::Base64CharactersAfterPadding => {
                "The base64 encoded content of a MIME part contained characters after the padding."
            }
            Self::ObsoleteBase64Comment => {
                "The base64 encoded content of a MIME part contained an obsolete comment."
            }
            Self::InvalidQuotedPrintableEncoding => {
                "The quoted-printable encoded content of a MIME part contained an invalid hex sequence after an '=' character."
            }
            Self::InvalidQuotedPrintableSoftBreak => {
                "The quoted-printable encoded content of a MIME part contained an invalid soft-break sequence."
            }
            Self::InvalidUUEncodePretext => {
                "The uuencoded content of a MIME part contained non-whitespace content before the begin marker."
            }
            Self::InvalidUUEncodeFileMode => {
                "The uuencoded content of a MIME part had an invalid file mode in the begin marker."
            }
            Self::InvalidUUEncodedContent => {
                "The uuencoded content of a MIME part contained invalid characters or was otherwise malformed."
            }
            Self::InvalidUUEncodedLineLength => {
                "The uuencoded content of a MIME part had an invalid encoded line length."
            }
            Self::IncompleteUUEncodedLine => {
                "The uuencoded content of a MIME part contained an incomplete encoded line."
            }
            Self::InvalidUUEncodedLineExtraData => {
                "The uuencoded content of a MIME part had extra data beyond the end of a uuencoded line."
            }
            Self::InvalidUUEncodeEndMarker => {
                "The uuencoded content of a MIME part contained non-whitespace content after the end marker."
            }
            Self::IncompleteUUEncodedContent => {
                "The uuencoded content of a MIME part did not properly end."
            }
            Self::TooManyComplianceIssues => {
                "Too many compliance issues were detected and the remainder were suppressed."
            }
        }
    }

    /// Explains the defect and why it matters.
    pub fn remarks(self) -> &'static str {
        match self {
            Self::BareLinefeedInHeader => {
                "The Internet Message Format specification requires that all lines be terminated with a <CR><LF> sequence. Messages that deviate from this requirement may not be processed correctly by some mail software."
            }
            Self::BareLinefeedInBody => {
                "The Internet Message Format specification requires that all lines be terminated with a <CR><LF> sequence. Messages that deviate from this requirement may not be processed correctly by some mail software."
            }
            Self::OversizedLine => {
                "This indicates that a line was longer than the SMTP limit of 1000 characters."
            }
            Self::Unexpected8BitBytesInHeader => {
                "Older Internet Message Format specifications require that headers are strictly US-ASCII while the newer Internationalized Email Headers specification allows for UTF-8. Header values that are not US-ASCII should be encoded using the encoding mechanism described in the MIME specification and/or should be valid UTF-8 as allowed in the Internationalized Email Headers specification."
            }
            Self::Unexpected8BitBytesInBody => {
                "This indicates that the Content-Transfer-Encoding header for a MIME part was set to a 7-bit encoding (such as 7bit, quoted-printable, or base64) but contained non-ASCII text (or potentially even binary data)."
            }
            Self::UnexpectedNullBytesInHeader => {
                "Null (0x00) bytes in a message header can be used by malicious actors to prevent some MIME parsers, such as those written in languages like C or C++ which tend to use the null byte to mark the end of a buffer, from discovering content after the null byte. This technique can be used to smuggle viruses or other malicious content past content scanners."
            }
            Self::UnexpectedNullBytesInBody => {
                "Null (0x00) bytes in a message body can be used by malicious actors to prevent some MIME parsers, such as those written in languages like C or C++ which tend to use the null byte to mark the end of a buffer, from discovering content after the null byte. This technique can be used to smuggle viruses or other malicious content past content scanners."
            }
            Self::BareCarriageReturnInHeader => {
                "The Internet Message Format specification only permits a carriage return as part of a <CR><LF> line terminator. MIME parsers disagree about whether a bare carriage return ends a line: some treat it as an ordinary character, some treat it as a line terminator, and some strip it. That disagreement can be exploited to make different components see different header fields, or a different boundary between the headers and the body, for the same message. A line that consists of nothing but carriage returns, for example, is an invalid header to one parser but the blank line that ends the header block to another. Only the first bare carriage return in each header is reported."
            }
            Self::InvalidHeader => {
                "The Internet Message Format specification requires that all header field names be composed of printable US-ASCII characters and must not contain control characters or whitespace characters. Inclusion of these characters can lead to divergent behavior among various MIME parsers, resulting in differences in handling."
            }
            Self::IncompleteHeader => {
                "This usually indicates that the message was truncated somewhere in transit and may be a sign that a MIME parser implementation earlier in transit failed to properly handle certain edge cases such as a null (0x00) byte in the message header."
            }
            Self::BlankHeaderContinuationLine => {
                "Section 3.2.2 of rfc5322 forbids folding a header field in such a way that any of its lines is made up entirely of whitespace characters; such lines are only tolerated by the obsolete folding whitespace syntax in section 4.2. As described in section 7.3 of rfc7103, agents disagree about what such a line means: a parser that follows the obsolete syntax treats it as a continuation of the previous header, but an agent that strips trailing whitespace from each line sees an empty line instead, which ends the header block. That disagreement can be exploited so that the header fields that follow the line are processed as headers by one component and as part of the body by another."
            }
            Self::RepeatedContentType => {
                "The MIME specifications require that each MIME part contain only one Content-Type header. Multiple Content-Type headers can lead to ambiguity and inconsistent behavior among different MIME parser implementations which may choose to use different Content-Type headers as their \"source of truth\"."
            }
            Self::RepeatedContentTransferEncoding => {
                "The MIME specifications require that each MIME part contain only one Content-Transfer-Encoding header. Multiple Content-Transfer-Encoding headers can lead to ambiguity and inconsistent behavior among different MIME parser implementations which may choose to use different Content-Transfer-Encoding headers as their \"source of truth\"."
            }
            Self::RepeatedDate => {
                "The Internet Message Format specification permits at most one Date header field. When more than one is present, agents disagree about which instance is authoritative: some take the first, some take the last. An attacker can exploit that disagreement by crafting a message where a filter or an authentication mechanism such as DKIM validates one instance while the mail client displays another."
            }
            Self::RepeatedFrom => {
                "The Internet Message Format specification permits at most one From header field. When more than one is present, agents disagree about which instance is authoritative: some take the first, some take the last. An attacker can exploit that disagreement by crafting a message where a filter or an authentication mechanism such as DKIM validates one instance while the mail client displays another."
            }
            Self::RepeatedSender => {
                "The Internet Message Format specification permits at most one Sender header field. When more than one is present, agents disagree about which instance is authoritative: some take the first, some take the last. An attacker can exploit that disagreement by crafting a message where a filter or an authentication mechanism such as DKIM validates one instance while the mail client displays another."
            }
            Self::RepeatedReplyTo => {
                "The Internet Message Format specification permits at most one Reply-To header field. When more than one is present, agents disagree about which instance is authoritative: some take the first, some take the last. An attacker can exploit that disagreement by crafting a message where a filter or an authentication mechanism such as DKIM validates one instance while the mail client displays another."
            }
            Self::RepeatedTo => {
                "The Internet Message Format specification permits at most one To header field. When more than one is present, agents disagree about which instance is authoritative: some take the first, some take the last. A message filter may therefore evaluate one instance while the mail client displays another."
            }
            Self::RepeatedCc => {
                "The Internet Message Format specification permits at most one Cc header field. When more than one is present, agents disagree about which instance is authoritative: some take the first, some take the last. A message filter may therefore evaluate one instance while the mail client displays another."
            }
            Self::RepeatedBcc => {
                "The Internet Message Format specification permits at most one Bcc header field. When more than one is present, agents disagree about which instance is authoritative: some take the first, some take the last. A message filter may therefore evaluate one instance while the mail client displays another."
            }
            Self::RepeatedMessageId => {
                "The Internet Message Format specification permits at most one Message-Id header field. When more than one is present, agents disagree about which instance is authoritative: some take the first, some take the last. An attacker can exploit that disagreement by crafting a message where a filter or an authentication mechanism such as DKIM validates one instance while the mail client displays another."
            }
            Self::RepeatedInReplyTo => {
                "The Internet Message Format specification permits at most one In-Reply-To header field. When more than one is present, agents disagree about which instance is authoritative, which may cause the message to be threaded inconsistently between mail clients."
            }
            Self::RepeatedReferences => {
                "The Internet Message Format specification permits at most one References header field. When more than one is present, agents disagree about which instance is authoritative, which may cause the message to be threaded inconsistently between mail clients."
            }
            Self::RepeatedSubject => {
                "The Internet Message Format specification permits at most one Subject header field. When more than one is present, agents disagree about which instance is authoritative: some take the first, some take the last. A message filter may therefore evaluate one instance while the mail client displays another."
            }
            Self::RepeatedReturnPath => {
                "Legitimate messages can contain more than one Return-Path header field, but it is more often an error. All but the topmost instance should be disregarded, because the topmost was added nearest to the mailbox that received the message."
            }
            Self::RepeatedResentDate => {
                "The Internet Message Format specification permits at most one Resent-Date header field per block of resent header fields, where a block is a contiguous run of resent fields corresponding to a single resending of the message. Because the specification provides no way to delimit adjacent blocks, two blocks that are not separated by any other header field cannot be told apart and are reported as a repeated field. Resent header fields are strictly informational and must not be used when processing replies, so the practical consequences are limited to how the resending history is displayed."
            }
            Self::RepeatedResentFrom => {
                "The Internet Message Format specification permits at most one Resent-From header field per block of resent header fields, where a block is a contiguous run of resent fields corresponding to a single resending of the message. Because the specification provides no way to delimit adjacent blocks, two blocks that are not separated by any other header field cannot be told apart and are reported as a repeated field. Resent header fields are strictly informational and must not be used when processing replies, so the practical consequences are limited to how the resending history is displayed."
            }
            Self::RepeatedResentSender => {
                "The Internet Message Format specification permits at most one Resent-Sender header field per block of resent header fields, where a block is a contiguous run of resent fields corresponding to a single resending of the message. Because the specification provides no way to delimit adjacent blocks, two blocks that are not separated by any other header field cannot be told apart and are reported as a repeated field. Resent header fields are strictly informational and must not be used when processing replies, so the practical consequences are limited to how the resending history is displayed."
            }
            Self::RepeatedResentTo => {
                "The Internet Message Format specification permits at most one Resent-To header field per block of resent header fields, where a block is a contiguous run of resent fields corresponding to a single resending of the message. Because the specification provides no way to delimit adjacent blocks, two blocks that are not separated by any other header field cannot be told apart and are reported as a repeated field. Resent header fields are strictly informational and must not be used when processing replies, so the practical consequences are limited to how the resending history is displayed."
            }
            Self::RepeatedResentCc => {
                "The Internet Message Format specification permits at most one Resent-Cc header field per block of resent header fields, where a block is a contiguous run of resent fields corresponding to a single resending of the message. Because the specification provides no way to delimit adjacent blocks, two blocks that are not separated by any other header field cannot be told apart and are reported as a repeated field. Resent header fields are strictly informational and must not be used when processing replies, so the practical consequences are limited to how the resending history is displayed."
            }
            Self::RepeatedResentBcc => {
                "The Internet Message Format specification permits at most one Resent-Bcc header field per block of resent header fields, where a block is a contiguous run of resent fields corresponding to a single resending of the message. Because the specification provides no way to delimit adjacent blocks, two blocks that are not separated by any other header field cannot be told apart and are reported as a repeated field. Resent header fields are strictly informational and must not be used when processing replies, so the practical consequences are limited to how the resending history is displayed."
            }
            Self::RepeatedResentMessageId => {
                "The Internet Message Format specification permits at most one Resent-Message-Id header field per block of resent header fields, where a block is a contiguous run of resent fields corresponding to a single resending of the message. Because the specification provides no way to delimit adjacent blocks, two blocks that are not separated by any other header field cannot be told apart and are reported as a repeated field. Resent header fields are strictly informational and must not be used when processing replies, so the practical consequences are limited to how the resending history is displayed."
            }
            Self::MissingDate => {
                "The Internet Message Format specification requires every message to contain a Date header field, which records when the author considered the message complete and ready for delivery. Agents that need a date fall back on something else, such as the time the message was received, so different agents may sort and display the message differently."
            }
            Self::MissingFrom => {
                "The Internet Message Format specification requires every message to contain a From header field, which identifies its author. Mail clients disagree about whom to display in its place, with some falling back to the Sender or Return-Path header fields, and a DMARC verifier has no author domain whose policy it could apply, so a message without one can sidestep the policy of the domain that sent it."
            }
            Self::MissingMessageId => {
                "The Internet Message Format specification states that every message should contain a Message-Id header field. It is not strictly required, but agents rely on it to detect duplicate messages and to thread replies, and a message submission agent is permitted to add one when it is missing."
            }
            Self::MissingMimeVersion => {
                "The MIME specifications require a message that uses MIME to declare itself as such with a MIME-Version header field. Without one, the message is strictly a plain Internet message and a conforming agent may ignore its Content-Type and Content-Transfer-Encoding header fields, although in practice most agents honor them anyway."
            }
            Self::InvalidContentType => {
                "This indicates that the Content-Type header was not properly formatted and could not be parsed. Since MIME parsers rely on the Content-Type header to decide how to interpret the content of a MIME part, an invalid Content-Type header can lead to ambiguity and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidContentTransferEncoding => {
                "This indicates that the Content-Transfer-Encoding header did not contain a valid value and could not be parsed."
            }
            Self::IllegalMessageRfc822ContentTransferEncoding => {
                "The MIME specifications do not allow message/rfc822 Content-Transfer-Encoding headers to specify any encoding that transforms the content in any way (such as quoted-printable or base64)."
            }
            Self::IllegalMultipartContentTransferEncoding => {
                "The MIME specifications do not allow multipart Content-Transfer-Encoding headers to specify any encoding that transforms the content in any way (such as quoted-printable or base64)."
            }
            Self::MissingMultipartBoundaryParameter => {
                "The MIME specifications require that each multipart Content-Type header include a boundary parameter. A multipart that does not define a boundary can lead to ambiguity and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidMultipartBoundaryParameter => {
                "A boundary parameter in a multipart Content-Type header must be a valid boundary string as defined by the MIME specifications. Invalid boundary parameters can lead to ambiguity and inconsistent behavior among different MIME parser implementations."
            }
            Self::ExcessiveAngleBracketsInAddress => {
                "Section 7.1.2 of rfc7103 describes address values such as \"<<user@example.com>>\" and notes that they can safely be interpreted as the same address with a single pair of brackets. Unlike an unbalanced bracket, a repeated one leaves no doubt about where the address begins and ends, so implementations that discard the extras all arrive at the same mailbox. It is still a departure from the angle-addr production, and usually indicates a mailer that has wrapped an address which was already wrapped."
            }
            Self::UnbalancedAngleBracketsInAddress => {
                "Section 7.1.3 of rfc7103 describes address values such as \"Name <user@example.com\" and \"user@example.org>\". Recovering from an unbalanced bracket requires guessing where the address was meant to end, and parsers that guess differently will extract different addresses."
            }
            Self::UnbalancedQuotesInAddress => {
                "Section 7.1.6 of rfc7103 describes address values such as \"\\\"Unterminated <user@example.com>\". An unclosed quote absorbs everything that follows it. Whether an address survives depends on what gets absorbed: a quote followed by an angle-addr still yields that mailbox, but one followed only by an addr-spec yields nothing at all. No error is raised either way, so a header that contributes fewer recipients than its author wrote is indistinguishable from one that parsed cleanly."
            }
            Self::UnbalancedParenthesesInAddress => {
                "Section 7.1.4 of rfc7103 describes address values such as \"Name (unbalanced <user@example.com>\". As with an unclosed quote, an unclosed comment consumes the remainder of the header value, so any addresses that follow it are lost rather than merely misparsed: text inside a comment carries no meaning, so there is nothing left for a parser to recover. No error is raised, so the message appears intact while carrying fewer recipients than its author wrote."
            }
            Self::UnquotedDisplayName => {
                "An unquoted display-name may only contain atoms, so values such as \"Doe, John <jdoe@example.com>\" and \"user@example.com <user@example.com>\" are not valid. The comma case is the most damaging, because a parser that does not special-case it will split the one address into two."
            }
            Self::AddressInDisplayName => {
                "A display-name such as the one in \"\\\"admin@example.com\\\" <attacker@example.org>\" is legal, but software that shows the display-name in place of the address will present a mailbox that will not receive the reply. The address that rfc5322 defines as authoritative is the one inside the angle brackets."
            }
            Self::UnquotedAddressInDisplayName => {
                "A display-name such as the one in \"admin@example.com <attacker@example.org>\" carries the same spoofing risk as its quoted form, but is not legal rfc5322: a display-name is a phrase, and '@' may only appear inside a quoted-string. A parser that ends the phrase at the '@' sees a naked local-part followed by a second mailbox, so the header can be read as naming either one address or two."
            }
            Self::AddressInGroupDisplayName => {
                "A group name such as the one in \"\\\"admin@example.com\\\": attacker@example.org;\" is legal, but software that shows the group name in place of its members will present a mailbox that is not in the group. A group name is a label, not a recipient."
            }
            Self::UnquotedAddressInGroupDisplayName => {
                "A group name such as the one in \"admin@example.com: attacker@example.org;\" carries the same spoofing risk as its quoted form, but is not legal rfc5322: a group name is a phrase, and '@' may only appear inside a quoted-string. A parser that ends the phrase at the '@' sees a naked local-part rather than the name of a group, so the header can be read as naming a mailbox that is not one of the group's members."
            }
            Self::InvalidLocalPart => {
                "A dot-atom may not contain two consecutive dots or end with a dot, so local-parts such as \"first..last\" and \"first.\" are not valid. This is also reported when an unquoted special appears inside the local-part, as in \"a[b@example.com\" or \"a b@example.com\": section 3.4.1 of rfc5322 admits such characters only inside a quoted-string, so the local-part ends at the offending character and the rest of the address is left with no production that can consume it. Receiving systems differ over whether to reject such an address, strip the offending characters, or pass the local-part through verbatim."
            }
            Self::MissingAddressSeparator => {
                "Section 7.1.5 of rfc7103 describes address lists such as \"a@example.com b@example.com\". A parser must guess whether this is two addresses or one address with a malformed display-name, and the two readings produce different sets of recipients."
            }
            Self::AmbiguousMailboxBoundary => {
                "A list such as \"<attacker@example.org> <admin@example.com>\" contains an angle-addr on at least one side of a missing separator, so a parser that recovers by treating the leading text as a display-name will read a single mailbox where a parser that recovers by splitting will read two. Unlike an ordinary missing comma, the two readings do not merely differ in how many addresses they produce, they disagree about which mailbox the address belongs to."
            }
            Self::ExtraneousCommaInAddressList => {
                "Section 7.1.5 of rfc7103 describes address lists such as \"a@example.com,,,b@example.com\", as well as lists with leading or trailing commas. The empty entries are not addresses and are typically ignored, but their presence usually indicates that the generating software dropped an address it intended to include."
            }
            Self::ObsoleteRouteAddress => {
                "The obs-route syntax described in section 4.4 of rfc5322, as in \"<@a.example,@b.example:user@example.com>\", must not be generated by conforming software. The route itself is meant to be ignored, but software that does not recognize the syntax may mistake the first domain in the route for the address domain."
            }
            Self::AddressWithoutDomain => {
                "Section 7.1.7 of rfc7103 describes \"naked\" local-parts such as \"username\". Such an address is only meaningful relative to some implied domain, so different systems will complete it differently, or not at all."
            }
            Self::ObsoleteDomainSyntax => {
                "The obs-domain syntax described in section 4.4 of rfc5322 allows folding whitespace and comments around the dots of a domain, as in \"user@example (comment) .com\". A conforming domain is a single dot-atom, so software that does not implement the obsolete grammar will read a different domain than software that does."
            }
            Self::InvalidDomain => {
                "A dot-atom may not contain two consecutive dots or begin with a dot, so domains such as \"example..com\" and \".example.com\" are not valid. A domain that merely ends with a dot is reported as TrailingDotInDomain instead, because a trailing dot is the fully qualified form of a domain name in the DNS and is far more likely to be deliberate. Receiving systems differ over whether to reject such an address, collapse the empty labels, or pass the domain through verbatim."
            }
            Self::TrailingDotInDomain => {
                "A trailing dot, as in \"user@example.com.\", denotes a fully qualified domain in the DNS but is not part of the domain grammar in rfc5322. Parsers that strip it and parsers that retain it will disagree about whether two otherwise identical addresses are equal."
            }
            Self::WhitespaceInDomainLiteral => {
                "The dtext rule in section 3.4.1 of rfc5322 does not permit whitespace inside the brackets of a domain-literal, as in \"user@[ 127.0.0.1 ]\". Parsers that strip the whitespace and parsers that preserve or reject it will not agree on the address."
            }
            Self::InvalidCharacterInDomainLiteral => {
                "The dtext rule in section 3.4.1 of rfc5322 excludes '[', ']' and '\\' from the contents of a domain-literal, as in \"user@[10.0.0.1[]\". Because ']' is the only thing that can end a domain-literal, an unescapable bracket leaves no way to tell where the author intended the address to end, and the domain cannot be recovered from what was written. Control characters and invalid 8-bit bytes inside a domain-literal are reported as ControlCharacterInAddress and Invalid8BitAddress instead, and rfc6532 adds the remaining 8-bit bytes to dtext."
            }
            Self::Invalid8BitAddress => {
                "The internationalized address syntax in rfc6532 extends the address grammar to UTF-8 and to nothing else, so 8-bit bytes that are not valid UTF-8 have no defined interpretation. A parser that falls back to a single-byte charset will produce a different address than one that rejects the header, which may result in mail being delivered to the wrong mailbox."
            }
            Self::MissingGroupTerminator => {
                "The group syntax in section 3.4 of rfc5322 requires a terminating ';', as in \"Friends: a@example.com;\". Without it, a parser must guess where the group ends, and addresses that follow the group may be absorbed into it."
            }
            Self::NonConformantAddress => {
                "This is the general case, used when an address departs from the grammar in a way that none of the more specific violations describes. Receiving systems differ widely in how much malformed syntax they will accept and in how they repair what they accept, so an address that only some implementations can read may resolve to different mailboxes, or to none at all, depending on which software handles the message."
            }
            Self::NullByteInAddress => {
                "A null byte is not permitted anywhere in a header, but inside an address it is more dangerous than elsewhere. Software written in or interfacing with C treats a null as a string terminator, so an address such as \"us<NUL>er@example.com\" may be read as the complete address \"us\" by one component and as \"user@example.com\" by another. That disagreement is the point of the construct: a filter, an audit log and the delivering agent can each be made to see a different mailbox from the same header. This is reported in addition to UnexpectedNullBytesInHeader, which identifies only the line that the null byte appeared on."
            }
            Self::NullByteInDisplayName => {
                "This is reported instead of NullByteInAddress when the null falls in a display-name rather than in an addr-spec, because the two call for different handling: a display-name is presentation and can be discarded without affecting delivery, whereas an addr-spec cannot. The danger is correspondingly different rather than smaller. The mailbox is unambiguous, but a client that stops at the null shows a different name than one that does not, so \"Bank<NUL>evil\" can be made to read as \"Bank\" in the message list and as something else in a filter or an audit log. This is reported in addition to UnexpectedNullBytesInHeader, which identifies only the line that the null byte appeared on."
            }
            Self::LineBreakInAddress => {
                "Folding whitespace is permitted around the tokens of an address, but the dot-atom-text production in section 3.2.3 of rfc5322 admits none inside a local-part or domain, so a line break within one of those tokens cannot be produced by a conforming mailer. It is most often seen when an application has concatenated unvalidated input into a header, which is the header injection technique described in section 5 of rfc5321: the attacker supplies a line break in the hope that some component in the chain will treat what follows as a new header or a new command. Even where that fails, implementations differ on whether to unfold, reject or truncate the address, so the recipient that is finally used may not be the one an auditor sees. A line break inside a quoted local-part is also reported, even though section 3.2.4 of rfc5322 permits folding whitespace inside a quoted-string, because a quoted local-part is rare enough that implementations mishandle it in practice."
            }
            Self::ControlCharacterInAddress => {
                "The atom, quoted-string and domain-literal productions in rfc5322 are all built from printable characters and whitespace, so a control character such as ESC or DEL can only have been introduced deliberately or by a mangled encoding. Control characters are stripped by some implementations and preserved by others, so the address may name a different mailbox depending on which software resolves it, and an escape sequence that survives into a log or a terminal-based mail client may be interpreted there rather than displayed."
            }
            Self::Iso2022SequenceInLocalPart => {
                "ISO-2022-JP and its relatives switch between character sets using escape sequences such as \"ESC $ B\", and, in the Korean and Chinese variants, using the shift-out and shift-in control characters. Japanese mailers have historically used these inside the local-part of an addr-spec in order to carry Japanese text in a mailbox name, a practice that predates and is entirely separate from the internationalized address syntax defined by rfc6532. This is reported separately from ControlCharacterInAddress because the two call for different handling: an arbitrary control character in an address is either damage or an injection attempt, whereas a well-formed ISO-2022 sequence in a local-part is a deliberate legacy convention that a receiving system may wish to decode rather than strip. It remains a violation either way, because implementations that strip the escapes and implementations that preserve them will not agree on which mailbox the address names. The construct is particularly awkward inside a quoted local-part, because the second byte of a JIS X 0208 pair may be a backslash or a double-quote, which then has to be written as a quoted-pair in order to survive the quoted-string grammar."
            }
            Self::EmptyGroupName => {
                "The group syntax in section 3.4 of rfc5322 is display-name \":\" [group-list] \";\", and a display-name is a phrase, which requires at least one word. A group introduced by a bare colon therefore has no name for a client to display, and parsers disagree over whether to treat the colon as introducing a group at all or as a stray character in an ordinary address."
            }
            Self::NestedAddressGroup => {
                "The group syntax in section 3.4 of rfc5322 is display-name \":\" [group-list] \";\", where a group-list is a mailbox-list, so a group may only contain mailboxes. A nested group, as in \"Outer: Inner: a@example.com;;\", is not valid syntax at any conformance level. Parsers disagree over how to handle it: some flatten the nested group into its parent, some reject the header, and others end the outer group at the first ';', so that the addresses which follow it are no longer part of the group."
            }
            Self::MissingBodySeparator => {
                "The Internet Message Format specifications require that an empty line separate the headers from the body of a message. This empty line serves as a clear delimiter between the headers and the body, allowing MIME parsers to correctly identify where the headers end and the body begins. A missing body separator can lead to ambiguity when parsing the message."
            }
            Self::MissingMultipartBoundary => {
                "When a multipart does not contain any boundary markers within its content, it can lead to ambiguity and inconsistent behavior among different MIME parser implementations which may opt to treat the content as a single part rather than a multipart message."
            }
            Self::MimeDepthLimitExceeded => {
                "This is not necessarily a defect in the message, but a multipart or message part nested beyond the configured limit (ParserOptions.MaxMimeDepth) was parsed as an opaque MIME part, so the entities within it were not parsed. A content scanner that relies on the result has not examined them, while a mail client with a higher limit (or none) may still render them."
            }
            Self::IncompleteBase64Quantum => {
                "The MIME specifications require base64 encoded content be a multiple of 4 bytes (a \"quantum\") in length. An incomplete quantum at the end of the content suggests that the base64 encoded content was either truncated or otherwise corrupted and can therefore lead to inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidBase64Character => {
                "Invalid characters within base64 content can lead to decoding issues and inconsistent behavior among different MIME parser implementations which may stop decoding as soon as this scenario is encountered while others may ignore these characters and continue decoding."
            }
            Self::InvalidBase64Padding => {
                "Invalid padding within base64 content can lead to decoding issues and inconsistent behavior among different MIME parser implementations. Some base64 decoders will ignore extraneous '=' padding characters if any are found within the middle of the base64 encoded block while others will treat decode it as 6 bits of 0's and may stop decoding as soon as they are encountered."
            }
            Self::Base64CharactersAfterPadding => {
                "Base64 characters found after padding ('=') in a base64 encoded block are not allowed by the MIME specifications and can lead to inconsistent behavior among different MIME parser implementations."
            }
            Self::ObsoleteBase64Comment => {
                "RFC 1113 (a Privacy Enhanced Mail specification) allowed for comments delimited by the '*' character in what later became known as \"base64 encoding\". This was obsoleted in RFC 1421 (which replaced RFC 1113) and RFC 1341 (the first MIME specification) explicitly disallowed it, but some mailers may generate such content. Since the vast majority of MIME base64 decoders do not support comments in base64 content, the presence of such comments can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidQuotedPrintableEncoding => {
                "Incorrect hex-encoded sequences in quoted-printable content can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidQuotedPrintableSoftBreak => {
                "A soft line break in quoted-printable content is represented by an equal sign (=) character followed immediately by a <CR><LF> sequence. This error indicates that an equal sign was immediately followed by an incomplete <CR><LF> sequence which can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidUUEncodePretext => {
                "UUEncoding requires that only lines containing whitespace are allowed before the begin marker. Non-whitespace content before the begin marker can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidUUEncodeFileMode => {
                "The UUEncoding begin marker should contain a file mode that is 3-4 digits long. An invalid file mode can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidUUEncodedContent => {
                "Incorrect line lengths and/or invalid characters in uuencoded content can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidUUEncodedLineLength => {
                "Each line in UUEncoding has a specific length encoded in the first byte of the line. This length must be between 0 and 45 (inclusive) and is used to determine how many bytes of data are represented by the line. An invalid line length can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::IncompleteUUEncodedLine => {
                "Each line in UUEncoding has a specific length encoded in the first byte of the line. Incomplete lines can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidUUEncodedLineExtraData => {
                "Each line in UUEncoding has a specific length encoded in the first byte of the line. Extra data beyond the end of the uuencoded line can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::InvalidUUEncodeEndMarker => {
                "UUEncoding requires that only whitespace is allowed after the end marker. Non-whitespace content after the end marker can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::IncompleteUUEncodedContent => {
                "UUEncoding requires that the encoded content is properly terminated with an end marker. Missing or malformed end markers can lead to decoding issues and inconsistent behavior among different MIME parser implementations."
            }
            Self::TooManyComplianceIssues => {
                "A limit was placed on how many times each violation may be reported and some violation reached it, so the remaining occurrences of that violation were not reported. Other violations continue to be reported until they reach the limit themselves, but the report as a whole is no longer a complete account of what is wrong with the message."
            }
        }
    }
}
