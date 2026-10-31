// Adapted from MimeKit's TnefRecipient.cs and TnefRecipientType.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::borrow::Cow;

use super::{TnefPropertySet, TnefPropertyTag as Tag, TnefValueConversionError};

/// A TNEF recipient type ([MS-OXOMSG] section 2.2.3.1).
///
/// Integer conversion preserves unknown values without interpreting flags.
/// [`TnefRecipient::recipient_type`] strips the property's high-order flags
/// before interpreting its type.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TnefRecipientType {
    /// The originator of the message.
    Originator,
    /// A primary (To) recipient.
    To,
    /// A carbon copy (Cc) recipient.
    Cc,
    /// A blind carbon copy (Bcc) recipient.
    Bcc,
    /// An unrecognized recipient type.
    Unknown(i32),
}

impl From<i32> for TnefRecipientType {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Originator,
            1 => Self::To,
            2 => Self::Cc,
            3 => Self::Bcc,
            value => Self::Unknown(value),
        }
    }
}

impl From<TnefRecipientType> for i32 {
    fn from(value: TnefRecipientType) -> Self {
        match value {
            TnefRecipientType::Originator => 0,
            TnefRecipientType::To => 1,
            TnefRecipientType::Cc => 2,
            TnefRecipientType::Bcc => 3,
            TnefRecipientType::Unknown(value) => value,
        }
    }
}

/// An owned recipient row with convenience access to its MAPI properties.
///
/// Construct from a recipient table row returned by
/// [`super::TnefPropertyReader::read_property_set`] or
/// [`super::TnefPropertyReader::read_rows_as_property_sets`]. Construction moves
/// the set without allocating, validating fields, or discarding any properties.
/// The recipient survives destruction of the reader. Collecting a row does not
/// verify its enclosing attribute's checksum; advance the reader to do that.
///
/// Getters derive their values from the original properties. As with typed set
/// access, only an absent property permits fallback: an empty string is a value,
/// while malformed or incompatible properties produce explicit errors.
/// Addresses are returned as stored, not parsed or converted to SMTP.
///
/// ```
/// use mimecraft::tnef::{TnefAttributeTag, TnefReader, TnefRecipient};
/// # let input = b"\x78\x9f\x3e\x22\0\0\x01\x04\x90\x06\0\x14\0\0\0\x01\0\0\0\x01\0\0\0\x1f\0\x01\x30\x01\0\0\0\0\0\0\0\x53\0";
/// let mut reader = TnefReader::new(input.as_slice());
/// let mut recipients = Vec::new();
/// while reader.read_next()? {
///     if reader.attribute().unwrap().tag == TnefAttributeTag::RECIPIENT_TABLE {
///         let rows = reader.open_properties()?.read_rows_as_property_sets()?;
///         recipients.extend(rows.into_iter().map(TnefRecipient::from));
///     }
/// }
/// for recipient in &recipients {
///     let name = recipient.display_name()?;
///     let address = recipient.email_address()?;
///     // Address type describes EmailAddress, not an overriding SmtpAddress.
///     let address_type = recipient.address_type()?;
///     # let _ = (name, address, address_type);
/// }
/// # assert_eq!(recipients.len(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct TnefRecipient {
    properties: TnefPropertySet,
}

impl TnefRecipient {
    /// All of the row's properties, including duplicates, flags, and named values.
    pub fn properties(&self) -> &TnefPropertySet {
        &self.properties
    }

    /// Returns the original property set without cloning or allocating.
    pub fn into_properties(self) -> TnefPropertySet {
        self.properties
    }

    /// Gets the recipient type without its high-order flags, defaulting to To
    /// only when the RecipientType property is absent. Unknown types are retained.
    ///
    /// # Errors
    ///
    /// Returns the property's explicit scalar integer-conversion error.
    pub fn recipient_type(&self) -> Result<TnefRecipientType, TnefValueConversionError> {
        match self.properties.get_i32(Tag::RECIPIENT_TYPE)? {
            // The high-order bits of PidTagRecipientType may indicate that the
            // recipient was resent to ([MS-OXOMSG] section 2.2.3.1).
            Some(value) => Ok(TnefRecipientType::from(value & 0x0fff_ffff)),
            None => Ok(TnefRecipientType::To),
        }
    }

    /// Gets the first present RecipientDisplayName, TransmitableDisplayName,
    /// or DisplayName, without trimming. Native strings are borrowed; Binary
    /// values are decoded using the property's captured charset.
    ///
    /// # Errors
    ///
    /// Returns the first present property's text-conversion error instead of
    /// ignoring it and falling back to a lower-priority name.
    pub fn display_name(&self) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        self.first_text(&[
            Tag::RECIPIENT_DISPLAY_NAME_W,
            Tag::TRANSMITABLE_DISPLAY_NAME_W,
            Tag::DISPLAY_NAME_W,
        ])
    }

    /// Gets SmtpAddress, or EmailAddress only when SmtpAddress is absent.
    /// The latter's address type is given by [`address_type`](Self::address_type).
    /// Returned text is not trimmed, parsed, or normalized.
    ///
    /// # Errors
    ///
    /// Returns the first present property's text-conversion error.
    pub fn email_address(&self) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        self.first_text(&[Tag::SMTP_ADDRESS_W, Tag::EMAIL_ADDRESS_W])
    }

    /// Gets the type of the EmailAddress property, such as "SMTP" or "EX".
    /// An overriding SmtpAddress does not change this stored address type.
    ///
    /// # Errors
    ///
    /// Returns the property's text-conversion error.
    pub fn address_type(&self) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        self.properties.get_text(Tag::ADDRTYPE_W)
    }

    fn first_text(&self, tags: &[Tag]) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        for &tag in tags {
            if let Some(value) = self.properties.get_text(tag)? {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }
}

impl From<TnefPropertySet> for TnefRecipient {
    fn from(properties: TnefPropertySet) -> Self {
        Self { properties }
    }
}
