// Ported from MimeKit's HeaderId.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::fmt;

macro_rules! header_ids {
    ($( $(#[$attribute:meta])* $id:ident => $name:literal, )*) => {
        /// An identifier for a header field recognized by MimeKit.
        ///
        /// Comparing identifiers avoids repeated string comparisons. This is
        /// name recognition only, not validation or interpretation of a value.
        /// Recognizing cryptographic header names does not provide cryptography.
        ///
        /// Unknown extension names remain available through [`crate::Header::field`].
        /// Rust variant names use idiomatic acronym casing; numeric values are
        /// not part of this API and do not reproduce MimeKit's enum ordinals.
        ///
        /// ```
        /// use mimecraft::HeaderId;
        ///
        /// assert_eq!(HeaderId::from_name("content-TYPE"), HeaderId::ContentType);
        /// assert_eq!(HeaderId::MimeVersion.as_str(), "MIME-Version");
        /// assert_eq!(HeaderId::from_name("X-Custom"), HeaderId::Unknown);
        /// ```
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
        pub enum HeaderId {
            /// A field name not recognized by this library.
            ///
            /// Also used for malformed records and empty field names.
            #[default]
            Unknown,
            $(
                #[doc = concat!("The `", $name, "` header field.")]
                $(#[$attribute])*
                $id,
            )*
        }

        impl HeaderId {
            /// Returns MimeKit's canonical field spelling.
            ///
            /// Returns `"Unknown"` for [`Unknown`](Self::Unknown). This is a
            /// diagnostic label, not the original name of an unrecognized field.
            /// Use [`crate::Header::raw`] when preserving original header bytes.
            pub const fn as_str(self) -> &'static str {
                match self {
                    Self::Unknown => "Unknown",
                    $(Self::$id => $name,)*
                }
            }
        }

        // Keep this list sorted ignoring ASCII case for allocation-free lookup.
        const HEADER_IDS: &[(&str, HeaderId)] = &[
            $(($name, HeaderId::$id),)*
        ];
    };
}

header_ids! {
    AcceptLanguage => "Accept-Language",
    AdHoc => "Ad-Hoc",
    AlternateRecipient => "Alternate-Recipient",
    ApparentlyTo => "Apparently-To",
    Approved => "Approved",
    ArcAuthenticationResults => "ARC-Authentication-Results",
    ArcMessageSignature => "ARC-Message-Signature",
    ArcSeal => "ARC-Seal",
    Archive => "Archive",
    ArchivedAt => "Archived-At",
    Article => "Article",
    AuthenticationResults => "Authentication-Results",
    /// Defined by RFC 3834; distinct from the legacy `Autosubmitted` field.
    AutoSubmitted => "Auto-Submitted",
    Autocrypt => "Autocrypt",
    AutocryptGossip => "Autocrypt-Gossip",
    AutocryptSetupMessage => "Autocrypt-Setup-Message",
    Autoforwarded => "Autoforwarded",
    /// Legacy RFC 2156 field. Usually [`AutoSubmitted`](Self::AutoSubmitted)
    /// is the intended field instead.
    Autosubmitted => "Autosubmitted",
    Base => "Base",
    Bcc => "Bcc",
    Body => "Body",
    Bytes => "Bytes",
    Cc => "Cc",
    Comments => "Comments",
    ContentAlternative => "Content-Alternative",
    ContentBase => "Content-Base",
    ContentClass => "Content-Class",
    ContentDescription => "Content-Description",
    ContentDisposition => "Content-Disposition",
    ContentDuration => "Content-Duration",
    ContentFeatures => "Content-Features",
    ContentId => "Content-Id",
    ContentIdentifier => "Content-Identifier",
    ContentLanguage => "Content-Language",
    ContentLength => "Content-Length",
    ContentLocation => "Content-Location",
    ContentMd5 => "Content-Md5",
    ContentReturn => "Content-Return",
    ContentTransferEncoding => "Content-Transfer-Encoding",
    ContentTranslationType => "Content-Translation-Type",
    ContentType => "Content-Type",
    Control => "Control",
    Conversion => "Conversion",
    ConversionWithLoss => "Conversion-With-Loss",
    Date => "Date",
    DateReceived => "Date-Received",
    DeferredDelivery => "Deferred-Delivery",
    DeliveryDate => "Delivery-Date",
    DiscloseRecipients => "Disclose-Recipients",
    DispositionNotificationOptions => "Disposition-Notification-Options",
    DispositionNotificationTo => "Disposition-Notification-To",
    Distribution => "Distribution",
    DkimSignature => "DKIM-Signature",
    DomainKeySignature => "DomainKey-Signature",
    Encoding => "Encoding",
    Encrypted => "Encrypted",
    Expires => "Expires",
    ExpiryDate => "Expiry-Date",
    FollowupTo => "Followup-To",
    From => "From",
    GenerateDeliveryReport => "Generate-Delivery-Report",
    Importance => "Importance",
    InReplyTo => "In-Reply-To",
    InjectionDate => "Injection-Date",
    InjectionInfo => "Injection-Info",
    Keywords => "Keywords",
    Language => "Language",
    LatestDeliveryTime => "Latest-Delivery-Time",
    Lines => "Lines",
    ListArchive => "List-Archive",
    ListHelp => "List-Help",
    ListId => "List-Id",
    ListOwner => "List-Owner",
    ListPost => "List-Post",
    ListSubscribe => "List-Subscribe",
    ListUnsubscribe => "List-Unsubscribe",
    ListUnsubscribePost => "List-Unsubscribe-Post",
    MessageId => "Message-Id",
    MimeVersion => "MIME-Version",
    Newsgroups => "Newsgroups",
    NntpPostingHost => "Nntp-Posting-Host",
    Organization => "Organization",
    OriginalFrom => "Original-From",
    OriginalMessageId => "Original-Message-Id",
    OriginalRecipient => "Original-Recipient",
    OriginalReturnAddress => "Original-Return-Address",
    OriginalSubject => "Original-Subject",
    Path => "Path",
    Precedence => "Precedence",
    PreventNonDeliveryReport => "Prevent-NonDelivery-Report",
    Priority => "Priority",
    Received => "Received",
    ReceivedSpf => "Received-SPF",
    References => "References",
    RelayVersion => "Relay-Version",
    ReplyBy => "Reply-By",
    ReplyTo => "Reply-To",
    RequireRecipientValidSince => "Require-Recipient-Valid-Since",
    ResentBcc => "Resent-Bcc",
    ResentCc => "Resent-Cc",
    ResentDate => "Resent-Date",
    ResentFrom => "Resent-From",
    ResentMessageId => "Resent-Message-Id",
    ResentReplyTo => "Resent-Reply-To",
    ResentSender => "Resent-Sender",
    ResentTo => "Resent-To",
    ReturnPath => "Return-Path",
    ReturnReceiptTo => "Return-Receipt-To",
    SeeAlso => "See-Also",
    Sender => "Sender",
    Sensitivity => "Sensitivity",
    Solicitation => "Solicitation",
    Status => "Status",
    Subject => "Subject",
    Summary => "Summary",
    Supersedes => "Supersedes",
    TlsRequired => "TLS-Required",
    To => "To",
    UserAgent => "User-Agent",
    XMailer => "X-Mailer",
    XMsMailPriority => "X-MSMail-Priority",
    XPriority => "X-Priority",
    XStatus => "X-Status",
    X400ContentIdentifier => "X400-Content-Identifier",
    X400ContentReturn => "X400-Content-Return",
    X400ContentType => "X400-Content-Type",
    X400MtsIdentifier => "X400-MTS-Identifier",
    X400Originator => "X400-Originator",
    X400Received => "X400-Received",
    X400Recipients => "X400-Recipients",
    X400Trace => "X400-Trace",
}

impl HeaderId {
    /// Recognizes a complete field name using ASCII case-insensitive comparison.
    ///
    /// No whitespace is trimmed and no colon or value is accepted. Unrecognized,
    /// empty, and non-ASCII names map to [`Unknown`](Self::Unknown). This method
    /// does not distinguish valid extension names from invalid field syntax;
    /// parsed records expose that distinction through [`crate::Header::is_invalid`].
    /// Lookup does not allocate.
    pub fn from_name(name: &str) -> Self {
        HEADER_IDS
            .binary_search_by(|(candidate, _)| {
                candidate
                    .bytes()
                    .map(|byte| byte.to_ascii_lowercase())
                    .cmp(name.bytes().map(|byte| byte.to_ascii_lowercase()))
            })
            .map_or(Self::Unknown, |index| HEADER_IDS[index].1)
    }
}

impl fmt::Display for HeaderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{HEADER_IDS, HeaderId};
    use crate::HeaderParser;

    #[test]
    fn lookup_table_is_sorted_and_unique() {
        assert_eq!(HEADER_IDS.len(), 131);
        let mut ids = std::collections::HashSet::new();
        for &(name, id) in HEADER_IDS {
            assert!(ids.insert(id), "duplicate identifier {id:?}");
            assert_eq!(id.as_str(), name);
            assert_ne!(id, HeaderId::Unknown);
        }
        for pair in HEADER_IDS.windows(2) {
            assert!(
                pair[0].0.to_ascii_lowercase() < pair[1].0.to_ascii_lowercase(),
                "names must be strictly sorted: {pair:?}"
            );
        }
    }

    #[test]
    fn every_name_round_trips_in_any_ascii_case() {
        for &(name, id) in HEADER_IDS {
            assert_eq!(HeaderId::from_name(name), id);
            assert_eq!(HeaderId::from_name(&name.to_ascii_lowercase()), id);
            assert_eq!(HeaderId::from_name(&name.to_ascii_uppercase()), id);
            assert_eq!(id.to_string(), name);
            for index in 0..name.len() {
                let mut bytes = name.as_bytes().to_vec();
                bytes[index] = if bytes[index].is_ascii_lowercase() {
                    bytes[index].to_ascii_uppercase()
                } else {
                    bytes[index].to_ascii_lowercase()
                };
                assert_eq!(
                    HeaderId::from_name(std::str::from_utf8(&bytes).unwrap()),
                    id
                );
            }
        }
    }

    #[test]
    fn every_identifier_is_attached_to_incrementally_parsed_headers() {
        let mut parser = HeaderParser::new();
        let mut headers = Vec::new();
        for &(name, id) in HEADER_IDS {
            let input = format!(
                "{} \t: value\r\n\tfolded\r\n\r\nbody",
                name.to_ascii_uppercase()
            );
            for byte in input.bytes() {
                parser.parse(&[byte], &mut headers).unwrap();
            }
            assert!(parser.is_finished());
            let header = headers.pop().unwrap();
            assert_eq!(header.id(), id);
            assert!(!header.is_invalid());
            assert_eq!(header.field().unwrap(), name.to_ascii_uppercase());
            assert_eq!(header.raw_value(), b" value\r\n\tfolded\r\n");
            assert_eq!(header.clone().id(), id);
            parser.reset();
        }
    }

    #[test]
    fn decorated_names_are_not_treated_as_field_names() {
        for &(name, _) in HEADER_IDS {
            for decorated in [
                format!(" {name}"),
                format!("{name}\t"),
                format!("{name}:"),
                format!("{name}: value"),
                format!("{name}\r\n"),
                format!("{name}\0"),
                format!("{name}\u{00a0}"),
            ] {
                assert_eq!(HeaderId::from_name(&decorated), HeaderId::Unknown);
            }
        }
    }
}
