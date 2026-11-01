// Adapted from MimeKit's TnefMessageLoader.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::HashSet, io::Read};

use super::{Logger, TnefEmbeddedContent, TnefEmbeddedReader, TnefReader, decode_string};
use crate::{
    charsets::{self, Charset},
    tnef::{
        TnefAttachment, TnefAttachmentContent, TnefAttribute, TnefAttributeTag as Attribute,
        TnefComplianceLogger, TnefComplianceViolation as Violation, TnefMessage, TnefMessageBody,
        TnefMessageBodyFormat as Format, TnefProperty, TnefPropertyInfo, TnefPropertySet,
        TnefPropertyTag as Tag, TnefPropertyType as Type, TnefReadError as Error, TnefRecipient,
        TnefValue as Value,
    },
};

// PidTagAttachTag for OLE attachments ([MS-OXCMSG] section 2.2.2.15): OID_TNEF.
const OLE_ATTACH_TAG: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x14, 3, 0x0a, 3, 2, 1];
// PidTagAttachEncoding for MacBinary attachments ([MS-OXCMSG] section 2.2.2.4).
const MAC_BINARY_ENCODING: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x14, 3, 0x0b, 1];
// ProviderUID of a One-Off EntryID ([MS-OXCDATA] section 2.2.5.1).
const ONE_OFF_PROVIDER: &[u8] = &[
    0x81, 0x2b, 0x1f, 0xa4, 0xbe, 0xa3, 0x10, 0x19, 0x9d, 0x6e, 0, 0xdd, 1, 0x0f, 0x54, 2,
];

// MAPI properties take precedence over corresponding legacy attributes
// ([MS-OXTNEF] section 2.1.3.4), while MAPI duplicates retain their order.
#[derive(Default)]
struct Properties {
    values: Vec<TnefProperty>,
    legacy: HashSet<u16>,
}

impl Properties {
    fn add(&mut self, property: TnefProperty) {
        // A MAPI property with a value replaces a decoded legacy property.
        if property.name().is_none()
            && property.value_count() > 0
            && self.legacy.remove(&property.tag().id())
            && let Some(index) = self
                .values
                .iter()
                .position(|p| p.tag().id() == property.tag().id())
        {
            self.values.remove(index);
        }
        self.values.push(property);
    }

    fn add_legacy(&mut self, tag: Tag, value: Value, charset: Charset, offset: u64) {
        if self.values.iter().any(|p| p.tag().id() == tag.id()) {
            return;
        }
        self.legacy.insert(tag.id());
        self.values.push(TnefProperty::new(
            TnefPropertyInfo {
                tag,
                name: None,
                offset,
            },
            vec![value],
            charset,
        ));
    }

    fn finish(self) -> TnefPropertySet {
        self.values.into()
    }
}

struct Content {
    bytes: Vec<u8>,
    tag: Option<Tag>,
    embedded: bool,
    position: u64,
    attribute: TnefAttribute,
}

#[derive(Default)]
struct Attachment {
    properties: Properties,
    content: Option<Content>,
}

#[derive(Default)]
struct Loader {
    properties: Properties,
    recipients: Vec<TnefRecipient>,
    attachments: Vec<TnefAttachment>,
    attachment: Option<Attachment>,
    bodies: [Option<TnefMessageBody>; 3],
    mapi_bodies: [bool; 3],
    owner: Option<(String, String, Charset, u64)>,
}

impl<R: Read> TnefReader<R> {
    /// Loads this complete message, discarding diagnostics.
    ///
    /// Requires an unadvanced reader. All attributes are drained and their
    /// checksums checked before returning. Embedded contents are captured,
    /// not recursively parsed. Malformed framing follows normal reader recovery.
    ///
    /// # Errors
    ///
    /// Returns errors for an advanced reader, invalid/incomplete header, I/O,
    /// assembly conversions, or byte/depth/attachment limits. A limit error does
    /// not return a shortened message or fault the reader; low-level reading may
    /// continue, but loading cannot be restarted on that advanced reader.
    pub fn load_message(&mut self) -> Result<TnefMessage, Error> {
        self.load_message_core(&mut Logger::None)
    }

    /// Loads this complete message using a fallible diagnostic logger.
    ///
    /// # Errors
    ///
    /// Shares [`load_message`](Self::load_message) errors. Source/logger failures
    /// fault the reader and any borrowed reader tree; no partial model is returned.
    pub fn load_message_with_logger(
        &mut self,
        logger: &mut dyn TnefComplianceLogger,
    ) -> Result<TnefMessage, Error> {
        self.load_message_core(&mut Logger::Borrowed(logger))
    }

    pub(super) fn load_message_core(
        &mut self,
        logger: &mut Logger<'_>,
    ) -> Result<TnefMessage, Error> {
        if self.is_faulted() {
            return Err(Error::Faulted);
        }
        if self.started {
            return Err(Error::ReaderAlreadyStarted);
        }
        let mut loader = Loader::default();
        while self.run(|reader| reader.next_core(logger))? {
            let attribute = *self.attribute.as_ref().unwrap();
            if attribute.tag.is_attachment() {
                loader.attachment_attribute(self, logger, attribute)?;
            } else {
                loader.message_attribute(self, logger, attribute)?;
            }
        }
        let legacy_key = self.legacy_key.ok_or(Error::InvalidMessageHeader)?;
        loader.finish_attachment(self, logger)?;
        loader.apply_owner()?;
        let properties = loader.properties.finish();
        let internet_charset = properties
            .get_i32(Tag::INTERNET_CODEPAGE)?
            .filter(|&page| page > 0)
            .and_then(|page| charsets::from_codepage(page as u32));
        // The codepage may have changed after a body was read when attOemCodepage
        // was absent and PidTagInternetCodepage followed it ([MS-OXTNEF] 2.3.3.2).
        for body in loader.bodies.iter_mut().flatten() {
            body.set_message_charsets(self.charset, internet_charset);
        }
        let [text_body, html_body, rtf_body] = loader.bodies;
        Ok(TnefMessage {
            codepage: self.codepage,
            legacy_key,
            properties,
            recipients: loader.recipients,
            attachments: loader.attachments,
            text_body,
            html_body,
            rtf_body,
        })
    }
}

impl TnefEmbeddedReader<'_, '_> {
    /// Loads an unadvanced borrowed message using inherited diagnostics and
    /// shared byte budgets. Captured embedded attachments open fresh budgets
    /// only when subsequently loaded through their detached APIs.
    ///
    /// # Errors
    ///
    /// Shares [`TnefReader::load_message`] errors; source/logger failures fault
    /// the entire borrowed tree.
    pub fn load_message(&mut self) -> Result<TnefMessage, Error> {
        self.reader.load_message_core(&mut self.logger)
    }
}

impl Loader {
    fn legacy<R: Read>(&mut self, reader: &TnefReader<R>, tag: Tag, value: Value) {
        self.properties.add_legacy(
            tag,
            value,
            reader.charset,
            reader.attribute.map_or(reader.position, |a| a.offset),
        );
    }

    fn invalid<R: Read>(reader: &mut TnefReader<R>, logger: &mut Logger<'_>) -> Result<(), Error> {
        reader.run(|r| {
            r.log(
                logger,
                Violation::InvalidAttributeValue,
                r.attribute.unwrap().offset,
            )
        })
    }

    fn message_attribute<R: Read>(
        &mut self,
        reader: &mut TnefReader<R>,
        logger: &mut Logger<'_>,
        attribute: TnefAttribute,
    ) -> Result<(), Error> {
        match attribute.tag {
            Attribute::MAPI_PROPERTIES => {
                let mut pr = if logger.is_some() {
                    reader.open_properties_with_logger(logger)?
                } else {
                    reader.open_properties()?
                };
                while pr.read_next_property()? {
                    let tag = pr.property().unwrap().tag;
                    if let Some(format) = Format::from_tag(tag).filter(|_| pr.has_value()) {
                        let index = match format {
                            Format::Text => 0,
                            Format::Html => 1,
                            Format::CompressedRtf => 2,
                        };
                        // The first MAPI body of each format wins. A MAPI text
                        // body replaces the legacy attBody, even when empty.
                        if !self.mapi_bodies[index] {
                            self.bodies[index] = Some(pr.read_body()?);
                            self.mapi_bodies[index] = true;
                        }
                    } else {
                        self.properties.add(pr.read_property()?);
                    }
                }
            }
            Attribute::RECIPIENT_TABLE => {
                let mut pr = if logger.is_some() {
                    reader.open_properties_with_logger(logger)?
                } else {
                    reader.open_properties()?
                };
                while pr.read_next_row()? {
                    self.recipients
                        .push(TnefRecipient::from(pr.read_property_set()?));
                }
            }
            Attribute::BODY => {
                if self.bodies[0].is_none() {
                    self.bodies[0] = Some(reader.body_core(logger)?);
                }
            }
            Attribute::DATE_SENT
            | Attribute::DATE_RECEIVED
            | Attribute::DATE_MODIFIED
            | Attribute::DATE_START
            | Attribute::DATE_END => {
                let tag = match attribute.tag {
                    Attribute::DATE_SENT => Tag::CLIENT_SUBMIT_TIME,
                    Attribute::DATE_RECEIVED => Tag::MESSAGE_DELIVERY_TIME,
                    Attribute::DATE_MODIFIED => Tag::LAST_MODIFICATION_TIME,
                    Attribute::DATE_START => Tag::START_DATE,
                    _ => Tag::END_DATE,
                };
                match reader.date_core(logger) {
                    Ok(value) if value.ticks() != 0 => {
                        self.legacy(reader, tag, Value::DateTime(value))
                    }
                    // The date reader already reports malformed/truncated dates.
                    Ok(_) | Err(Error::InvalidDate(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            Attribute::PRIORITY => {
                let value = reader.scalar_core(logger)?;
                match value {
                    1..=3 => self.legacy(reader, Tag::IMPORTANCE, Value::I32(3 - value)),
                    _ => Self::invalid(reader, logger)?,
                }
            }
            Attribute::AID_OWNER | Attribute::REQUEST_RESPONSE => {
                let value = reader.scalar_core(logger)?;
                let (tag, value) = if attribute.tag == Attribute::AID_OWNER {
                    (Tag::OWNER_APPT_ID, Value::I32(value))
                } else {
                    (Tag::RESPONSE_REQUESTED, Value::Bool(value & 0xff != 0))
                };
                self.legacy(reader, tag, value);
            }
            Attribute::SUBJECT
            | Attribute::MESSAGE_CLASS
            | Attribute::ORIGINAL_MESSAGE_CLASS
            | Attribute::MESSAGE_ID
            | Attribute::MESSAGE_STATUS
            | Attribute::DELEGATE
            | Attribute::FROM
            | Attribute::OWNER => {
                let bytes = reader.bytes_core(logger)?;
                self.message_bytes(reader, logger, attribute.tag, bytes)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn message_bytes<R: Read>(
        &mut self,
        reader: &mut TnefReader<R>,
        logger: &mut Logger<'_>,
        attribute: Attribute,
        bytes: Vec<u8>,
    ) -> Result<(), Error> {
        match attribute {
            Attribute::SUBJECT | Attribute::MESSAGE_CLASS | Attribute::ORIGINAL_MESSAGE_CLASS => {
                let text = decode_string(reader.charset, &bytes);
                let (tag, text) = if attribute == Attribute::SUBJECT {
                    (Tag::SUBJECT_W, text.as_ref())
                } else {
                    let tag = if attribute == Attribute::MESSAGE_CLASS {
                        Tag::MESSAGE_CLASS_W
                    } else {
                        Tag::ORIG_MESSAGE_CLASS_W
                    };
                    (tag, translate_message_class(text.trim()))
                };
                if !text.is_empty() {
                    self.legacy(reader, tag, Value::Text(text.to_owned()));
                }
            }
            Attribute::MESSAGE_ID => {
                // attMessageID is PidTagSearchKey encoded as hexadecimal digits.
                let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
                if end != 0 {
                    if end % 2 != 0 || bytes[..end].iter().any(|b| !b.is_ascii_hexdigit()) {
                        return Self::invalid(reader, logger);
                    }
                    let nibble = |b: u8| {
                        if b.is_ascii_digit() {
                            b - b'0'
                        } else {
                            b.to_ascii_lowercase() - b'a' + 10
                        }
                    };
                    let key = bytes[..end]
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|p| nibble(p[0]) << 4 | nibble(p[1]))
                        .collect();
                    self.legacy(reader, Tag::SEARCH_KEY, Value::Binary(key));
                }
            }
            Attribute::MESSAGE_STATUS => {
                let Some(&status) = bytes.first() else {
                    return Self::invalid(reader, logger);
                };
                // Map attMessageStatus to PidTagMessageFlags ([MS-OXTNEF] 2.1.3.4).
                let mut flags = 0;
                if status & 0x20 != 0 {
                    flags |= 0x01;
                } // fmsRead -> MSGFLAG_READ
                if status & 0x01 == 0 {
                    flags |= 0x02;
                } // !fmsModified -> MSGFLAG_UNMODIFIED
                if status & 0x04 != 0 {
                    flags |= 0x04;
                } // fmsSubmitted -> MSGFLAG_SUBMIT
                if status & 0x02 != 0 {
                    flags |= 0x08;
                } // fmsLocal -> MSGFLAG_UNSENT
                if status & 0x80 != 0 {
                    flags |= 0x10;
                } // fmsHasAttach -> MSGFLAG_HASATTACH
                self.legacy(reader, Tag::MESSAGE_FLAGS, Value::I32(flags));
            }
            Attribute::DELEGATE => {
                if !bytes.is_empty() {
                    self.legacy(
                        reader,
                        Tag::RCVD_REPRESENTING_ENTRY_ID,
                        Value::Binary(bytes),
                    );
                }
            }
            Attribute::FROM => {
                // attFrom is a TRP followed by display name and address
                // ([MS-OXTNEF] section 2.1.3.3.1); only trpidOneOff is used.
                if bytes.len() < 8 {
                    return Self::invalid(reader, logger);
                }
                let name_length = word(&bytes, 4);
                let address_length = word(&bytes, 6);
                if word(&bytes, 0) != 4 || 8 + name_length + address_length > bytes.len() {
                    return Self::invalid(reader, logger);
                }
                let name = decode_string(reader.charset, &bytes[8..8 + name_length]);
                let address = decode_string(
                    reader.charset,
                    &bytes[8 + name_length..8 + name_length + address_length],
                );
                self.address(
                    reader.charset,
                    &name,
                    &address,
                    [
                        Tag::SENDER_NAME_W,
                        Tag::SENDER_ADDRTYPE_W,
                        Tag::SENDER_EMAIL_ADDRESS_W,
                        Tag::SENDER_ENTRY_ID,
                    ],
                    reader.attribute.unwrap().offset,
                );
            }
            Attribute::OWNER => {
                // attOwner has length-prefixed name/address fields. Its mapping
                // depends on the message class, so defer it until completion.
                if bytes.len() < 2 {
                    return Self::invalid(reader, logger);
                }
                let name_length = word(&bytes, 0);
                let index = 2 + name_length;
                if index + 2 > bytes.len() {
                    return Self::invalid(reader, logger);
                }
                let address_length = word(&bytes, index);
                if index + 2 + address_length > bytes.len() {
                    return Self::invalid(reader, logger);
                }
                self.owner = Some((
                    decode_string(reader.charset, &bytes[2..index]).into_owned(),
                    decode_string(
                        reader.charset,
                        &bytes[index + 2..index + 2 + address_length],
                    )
                    .into_owned(),
                    reader.charset,
                    reader.attribute.unwrap().offset,
                ));
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn address(
        &mut self,
        charset: Charset,
        name: &str,
        full_address: &str,
        tags: [Tag; 4],
        offset: u64,
    ) {
        // Split addresses of the form "TYPE:address"; a leading colon is not a type.
        let (kind, address) = full_address
            .split_once(':')
            .filter(|(kind, _)| !kind.is_empty())
            .unwrap_or(("", full_address));
        for (tag, value) in tags[..3].iter().zip([name, kind, address]) {
            if !value.is_empty() {
                self.properties
                    .add_legacy(*tag, Value::Text(value.to_owned()), charset, offset);
            }
        }
        self.properties.add_legacy(
            tags[3],
            Value::Binary(one_off_entry_id(name, kind, address)),
            charset,
            offset,
        );
    }

    fn apply_owner(&mut self) -> Result<(), Error> {
        let Some((name, address, charset, offset)) = self.owner.take() else {
            return Ok(());
        };
        let message_class = self
            .properties
            .values
            .iter()
            .find(|p| p.tag().id() == Tag::MESSAGE_CLASS_W.id())
            .map(TnefProperty::to_text)
            .transpose()?;
        // For meeting responses, attOwner identifies the delegator, not the sender.
        let response = message_class.as_deref().is_some_and(|class| {
            class
                .trim()
                .get(.."IPM.Schedule.Meeting.Resp.".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("IPM.Schedule.Meeting.Resp."))
        });
        let tags = if response {
            [
                Tag::RCVD_REPRESENTING_NAME_W,
                Tag::RCVD_REPRESENTING_ADDRTYPE_W,
                Tag::RCVD_REPRESENTING_EMAIL_ADDRESS_W,
                Tag::RCVD_REPRESENTING_ENTRY_ID,
            ]
        } else {
            [
                Tag::SENT_REPRESENTING_NAME_W,
                Tag::SENT_REPRESENTING_ADDRTYPE_W,
                Tag::SENT_REPRESENTING_EMAIL_ADDRESS_W,
                Tag::SENT_REPRESENTING_ENTRY_ID,
            ]
        };
        self.address(charset, &name, &address, tags, offset);
        Ok(())
    }

    fn finish_attachment<R: Read>(
        &mut self,
        reader: &mut TnefReader<R>,
        logger: &mut Logger<'_>,
    ) -> Result<(), Error> {
        let Some(builder) = self.attachment.take() else {
            return Ok(());
        };
        let properties = builder.properties.finish();
        let content = if let Some(content) = builder.content {
            // AttachMethod usually follows AttachDataObject. Binary embedded
            // messages can only be recognized after all attachment properties.
            let embedded = content.embedded
                || (content.tag.is_some() && properties.get_i32(Tag::ATTACH_METHOD)? == Some(5));
            Some(if embedded {
                if reader.depth >= reader.options.max_nesting_depth {
                    let attribute = reader.attribute.replace(content.attribute);
                    let property = reader.property_tag.replace(content.tag.unwrap());
                    let result =
                        reader.run(|r| r.log(logger, Violation::NestingTooDeep, content.position));
                    reader.attribute = attribute;
                    reader.property_tag = property;
                    result?;
                    return Err(Error::NestingLimitExceeded);
                }
                TnefAttachmentContent::Embedded(
                    reader.run(|r| TnefEmbeddedContent::new(r, content.bytes, content.position))?,
                )
            } else if content
                .tag
                .is_some_and(|tag| tag.value_type() == Type::OBJECT)
            {
                TnefAttachmentContent::Object(content.bytes)
            } else {
                TnefAttachmentContent::Binary(content.bytes)
            })
        } else {
            None
        };
        self.attachments
            .push(TnefAttachment::new(properties, content));
        Ok(())
    }

    fn attachment_attribute<R: Read>(
        &mut self,
        reader: &mut TnefReader<R>,
        logger: &mut Logger<'_>,
        attribute: TnefAttribute,
    ) -> Result<(), Error> {
        if attribute.tag == Attribute::ATTACH_RENDER_DATA || self.attachment.is_none() {
            self.finish_attachment(reader, logger)?;
            // Start a new attachment only while within the per-message limit.
            if self.attachments.len() >= reader.options.max_attachments {
                reader.run(|r| r.log(logger, Violation::TooManyAttachments, attribute.offset))?;
                return Err(Error::AttachmentLimitExceeded);
            }
            self.attachment = Some(Attachment::default());
        }
        let builder = self.attachment.as_mut().unwrap();
        match attribute.tag {
            Attribute::ATTACHMENT => {
                let mut pr = if logger.is_some() {
                    reader.open_properties_with_logger(logger)?
                } else {
                    reader.open_properties()?
                };
                while pr.read_next_property()? {
                    let tag = pr.property().unwrap().tag;
                    let data = tag.id() == Tag::ATTACH_DATA_BIN.id()
                        && !tag.is_multi_valued()
                        && matches!(tag.value_type(), Type::BINARY | Type::OBJECT);
                    if data && pr.has_value() {
                        if !builder
                            .content
                            .as_ref()
                            .is_some_and(|content| content.tag.is_some())
                        {
                            let embedded = pr.is_embedded_message();
                            let position = pr.position();
                            // MAPI content replaces legacy content. Byte-limit
                            // failures propagate rather than silently omitting it.
                            builder.content = Some(Content {
                                bytes: pr.read_value_bytes()?,
                                tag: Some(tag),
                                embedded,
                                position,
                                attribute,
                            });
                        }
                    } else {
                        builder.properties.add(pr.read_property()?);
                    }
                }
            }
            Attribute::ATTACH_DATA => {
                if builder.content.is_none() {
                    let position = reader.position;
                    builder.content = Some(Content {
                        bytes: reader.bytes_core(logger)?,
                        tag: None,
                        embedded: false,
                        position,
                        attribute,
                    });
                }
            }
            Attribute::ATTACH_CREATE_DATE | Attribute::ATTACH_MODIFY_DATE => {
                let tag = if attribute.tag == Attribute::ATTACH_CREATE_DATE {
                    Tag::CREATION_TIME
                } else {
                    Tag::LAST_MODIFICATION_TIME
                };
                match reader.date_core(logger) {
                    Ok(value) if value.ticks() != 0 => builder.properties.add_legacy(
                        tag,
                        Value::DateTime(value),
                        reader.charset,
                        attribute.offset,
                    ),
                    Ok(_) | Err(Error::InvalidDate(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            _ => {
                let bytes = reader.bytes_core(logger)?;
                let mut add = |tag, value| {
                    builder
                        .properties
                        .add_legacy(tag, value, reader.charset, attribute.offset)
                };
                match attribute.tag {
                    Attribute::ATTACH_RENDER_DATA => {
                        // attAttachRendData ([MS-OXTNEF] section 2.1.3.3.11).
                        if bytes.len() < 14 {
                            return Self::invalid(reader, logger);
                        }
                        add(
                            Tag::RENDERING_POSITION,
                            Value::I32(i32::from_le_bytes(bytes[2..6].try_into().unwrap())),
                        );
                        // Attachment type 2 is an OLE object.
                        if word(&bytes, 0) == 2 {
                            add(Tag::ATTACH_TAG, Value::Binary(OLE_ATTACH_TAG.to_vec()));
                        }
                        // Flag 0x00000001 indicates MacBinary encoding.
                        if bytes[10] & 1 != 0 {
                            add(
                                Tag::ATTACH_ENCODING,
                                Value::Binary(MAC_BINARY_ENCODING.to_vec()),
                            );
                        }
                    }
                    Attribute::ATTACH_TITLE | Attribute::ATTACH_TRANSPORT_FILENAME => {
                        let text = decode_string(reader.charset, &bytes);
                        let tag = if attribute.tag == Attribute::ATTACH_TITLE {
                            Tag::ATTACH_LONG_FILENAME_W
                        } else {
                            Tag::ATTACH_TRANSPORT_NAME_W
                        };
                        if !text.is_empty() {
                            add(tag, Value::Text(text.into_owned()));
                        }
                    }
                    Attribute::ATTACH_META_FILE => {
                        if !bytes.is_empty() {
                            add(Tag::ATTACH_RENDERING, Value::Binary(bytes));
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(())
    }
}

fn word(bytes: &[u8], index: usize) -> usize {
    usize::from(u16::from_le_bytes([bytes[index], bytes[index + 1]]))
}

// Translate legacy classes listed in [MS-OXTNEF] section 2.1.3.4.1.
fn translate_message_class(message_class: &str) -> &str {
    let value = message_class
        .strip_prefix("Microsoft Mail v3.0 ")
        .unwrap_or(message_class);
    match value {
        "IPM.Microsoft Mail.Note" => "IPM.Note",
        "IPM.Microsoft Mail.Read Receipt" => "Report.IPM.Note.IPNRN",
        "IPM.Microsoft Mail.Non-Delivery" => "Report.IPM.Note.NDR",
        "IPM.Microsoft Schedule.MtgRespP" => "IPM.Schedule.Meeting.Resp.Pos",
        "IPM.Microsoft Schedule.MtgRespN" => "IPM.Schedule.Meeting.Resp.Neg",
        "IPM.Microsoft Schedule.MtgRespA" => "IPM.Schedule.Meeting.Resp.Tent",
        "IPM.Microsoft Schedule.MtgReq" => "IPM.Schedule.Meeting.Request",
        "IPM.Microsoft Schedule.MtgCncl" => "IPM.Schedule.Meeting.Canceled",
        _ => message_class,
    }
}

// Create a One-Off EntryID with Unicode strings ([MS-OXCDATA] 2.2.5.1).
fn one_off_entry_id(name: &str, kind: &str, address: &str) -> Vec<u8> {
    let mut bytes = vec![0; 4];
    bytes.extend_from_slice(ONE_OFF_PROVIDER);
    // Version 0; MAPI_ONE_OFF_UNICODE (0x8000), little-endian.
    bytes.extend_from_slice(&[0, 0, 0, 0x80]);
    for text in [name, kind, address] {
        bytes.extend(text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    }
    bytes
}
