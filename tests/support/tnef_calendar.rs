// Adapted from MimeKit's TnefCalendarTests and TnefCalendarRobustnessTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::tnef::{
    TnefAttributeTag as Attr, TnefDateTime as Date, TnefDateTimeKind as Kind, TnefMessage,
    TnefName, TnefNameId as Name, TnefPropertyTag as Tag, TnefPropertyType as Type,
    TnefValue as Value,
};

pub fn date(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> Date {
    Date::new(year, month, day, hour, minute, 0, Kind::Utc).unwrap()
}

pub fn stamp() -> Date {
    Date::new(2024, 1, 2, 3, 4, 5, Kind::Utc).unwrap()
}

fn variable(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value);
    bytes.extend(std::iter::repeat_n(0, (4 - value.len() % 4) % 4));
}

fn value(bytes: &mut Vec<u8>, value: Value) {
    match value {
        Value::I32(value) => bytes.extend_from_slice(&value.to_le_bytes()),
        Value::Bool(value) => bytes.extend_from_slice(&u32::from(value).to_le_bytes()),
        Value::DateTime(date) => {
            bytes.extend_from_slice(&date.to_file_time().unwrap().to_le_bytes())
        }
        Value::SysTime(value) => bytes.extend_from_slice(&value.to_le_bytes()),
        Value::Text(text) => {
            let mut text: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            text.extend_from_slice(&[0, 0]);
            variable(bytes, &text);
        }
        Value::Binary(value) | Value::Object(value) => variable(bytes, &value),
        _ => panic!("unsupported fixture value"),
    }
}

fn kind(value: &Value) -> Type {
    match value {
        Value::I32(_) => Type::LONG,
        Value::Bool(_) => Type::BOOLEAN,
        Value::DateTime(_) | Value::SysTime(_) => Type::SYS_TIME,
        Value::Text(_) => Type::UNICODE,
        Value::Binary(_) => Type::BINARY,
        Value::Object(_) => Type::OBJECT,
        _ => panic!("unsupported fixture value"),
    }
}

#[derive(Clone, Default)]
pub struct Properties {
    pub items: Vec<Vec<u8>>,
    next: u16,
}
impl Properties {
    pub fn tag(&mut self, tag: Tag, val: Value) {
        let kind = kind(&val);
        let mut bytes = Tag::new(tag.id(), kind).0.to_le_bytes().to_vec();
        if matches!(kind, Type::UNICODE | Type::BINARY | Type::OBJECT) {
            bytes.extend_from_slice(&1u32.to_le_bytes());
        }
        value(&mut bytes, val);
        self.items.push(bytes);
    }
    pub fn named(&mut self, name: Name, val: Value) {
        let kind = kind(&val);
        let mut bytes = self.header(name, kind);
        if matches!(kind, Type::UNICODE | Type::BINARY | Type::OBJECT) {
            bytes.extend_from_slice(&1u32.to_le_bytes());
        }
        value(&mut bytes, val);
        self.items.push(bytes);
    }
    fn header(&mut self, name: Name, kind: Type) -> Vec<u8> {
        let mut bytes = Tag::new(0x8000 + self.next, kind).0.to_le_bytes().to_vec();
        self.next += 1;
        bytes.extend_from_slice(&name.guid);
        match name.name {
            TnefName::Id(id) => {
                bytes.extend_from_slice(&0u32.to_le_bytes());
                bytes.extend_from_slice(&id.to_le_bytes());
            }
            TnefName::String(name) => {
                bytes.extend_from_slice(&1u32.to_le_bytes());
                let mut name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
                name.extend_from_slice(&[0, 0]);
                variable(&mut bytes, &name);
            }
            _ => unreachable!(),
        }
        bytes
    }
    pub fn categories(&mut self, categories: &[&str]) {
        let mut bytes = self.header(
            Name::keywords(),
            Type(Type::UNICODE.0 | Type::MULTI_VALUED.0),
        );
        bytes.extend_from_slice(&(categories.len() as u32).to_le_bytes());
        for category in categories {
            value(&mut bytes, Value::Text((*category).into()));
        }
        self.items.push(bytes);
    }
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = (self.items.len() as u32).to_le_bytes().to_vec();
        for item in &self.items {
            bytes.extend_from_slice(item);
        }
        bytes
    }
}

fn attribute(bytes: &mut Vec<u8>, level: u8, tag: Attr, value: &[u8]) {
    bytes.push(level);
    bytes.extend_from_slice(&tag.0.to_le_bytes());
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value);
    let checksum = value
        .iter()
        .fold(0u16, |sum, &b| sum.wrapping_add(u16::from(b)));
    bytes.extend_from_slice(&checksum.to_le_bytes());
}

#[derive(Clone)]
pub struct Fixture {
    pub class: String,
    pub properties: Properties,
    pub recipients: Vec<Properties>,
    pub attachments: Vec<Properties>,
}
impl Fixture {
    pub fn empty(class: &str) -> Self {
        Self {
            class: class.into(),
            properties: Properties::default(),
            recipients: Vec::new(),
            attachments: Vec::new(),
        }
    }
    pub fn appointment(class: &str) -> Self {
        let mut fixture = Self::empty(class);
        fixture
            .properties
            .tag(Tag::SUBJECT_W, Value::Text("Team sync".into()));
        fixture
            .properties
            .tag(Tag::CLIENT_SUBMIT_TIME, Value::DateTime(stamp()));
        fixture.properties.named(
            Name::APPOINTMENT_START_WHOLE,
            Value::DateTime(date(2024, 3, 1, 15, 0)),
        );
        fixture.properties.named(
            Name::APPOINTMENT_END_WHOLE,
            Value::DateTime(date(2024, 3, 1, 16, 0)),
        );
        fixture
    }
    pub fn recurring(recurrence: Vec<u8>) -> Self {
        let mut fixture = Self::empty("IPM.Schedule.Meeting.Request");
        fixture
            .properties
            .tag(Tag::SUBJECT_W, Value::Text("Team sync".into()));
        fixture
            .properties
            .tag(Tag::CLIENT_SUBMIT_TIME, Value::DateTime(stamp()));
        fixture.properties.named(
            Name::APPOINTMENT_START_WHOLE,
            Value::DateTime(date(2024, 7, 1, 14, 0)),
        );
        fixture.properties.named(
            Name::APPOINTMENT_END_WHOLE,
            Value::DateTime(date(2024, 7, 1, 15, 0)),
        );
        fixture
            .properties
            .named(Name::APPOINTMENT_RECUR, Value::Binary(recurrence));
        fixture.properties.named(
            Name::APPOINTMENT_TIME_ZONE_DEFINITION_RECUR,
            Value::Binary(eastern("Eastern Standard Time", 300)),
        );
        fixture
            .properties
            .named(Name::LOCATION, Value::Text("Room 1".into()));
        fixture
            .recipients
            .push(recipient(1, "Alice", "alice@example.com", Some(3)));
        fixture
            .recipients
            .push(recipient(1, "Bob", "bob@example.com", Some(1)));
        fixture
    }
    pub fn attach(&mut self, flags: i32, embedded: Vec<u8>, start: Option<Date>) {
        let mut props = Properties::default();
        props.tag(Tag::ATTACH_METHOD, Value::I32(5));
        props.tag(Tag::ATTACHMENT_FLAGS, Value::I32(flags));
        if let Some(start) = start {
            props.tag(Tag::EXCEPTION_START_TIME, Value::DateTime(start));
        }
        let mut object = vec![7, 3, 2, 0, 0, 0, 0, 0, 192, 0, 0, 0, 0, 0, 0, 70];
        object.extend(embedded);
        props.tag(Tag::ATTACH_DATA_OBJ, Value::Object(object));
        self.attachments.push(props);
    }
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0];
        attribute(&mut bytes, 1, Attr::TNEF_VERSION, &[0, 0, 1, 0]);
        attribute(
            &mut bytes,
            1,
            Attr::OEM_CODEPAGE,
            &[0xe4, 4, 0, 0, 0, 0, 0, 0],
        );
        let mut class = self.class.as_bytes().to_vec();
        class.push(0);
        attribute(&mut bytes, 1, Attr::MESSAGE_CLASS, &class);
        attribute(
            &mut bytes,
            1,
            Attr::MAPI_PROPERTIES,
            &self.properties.bytes(),
        );
        if !self.recipients.is_empty() {
            let mut rows = (self.recipients.len() as u32).to_le_bytes().to_vec();
            for row in &self.recipients {
                rows.extend(row.bytes());
            }
            attribute(&mut bytes, 1, Attr::RECIPIENT_TABLE, &rows);
        }
        for attachment in &self.attachments {
            attribute(&mut bytes, 2, Attr::ATTACH_RENDER_DATA, &[0; 14]);
            attribute(&mut bytes, 2, Attr::ATTACHMENT, &attachment.bytes());
        }
        bytes
    }
    pub fn load(&self) -> TnefMessage {
        TnefMessage::load(self.bytes().as_slice()).unwrap()
    }
}

pub fn recipient(kind: i32, name: &str, address: &str, flags: Option<i32>) -> Properties {
    let mut p = Properties::default();
    p.tag(Tag::RECIPIENT_TYPE, Value::I32(kind));
    p.tag(Tag::DISPLAY_NAME_W, Value::Text(name.into()));
    p.tag(Tag::ADDRTYPE_W, Value::Text("SMTP".into()));
    p.tag(Tag::EMAIL_ADDRESS_W, Value::Text(address.into()));
    if let Some(flags) = flags {
        p.tag(Tag::RECIPIENT_FLAGS, Value::I32(flags));
    }
    p
}

pub fn global_id(data: &[u8], year: u16, month: u8, day: u8) -> Vec<u8> {
    let mut id = vec![0; 40];
    id[..16].copy_from_slice(&[
        4, 0, 0, 0, 130, 0, 224, 0, 116, 197, 183, 16, 26, 130, 224, 8,
    ]);
    id[16..18].copy_from_slice(&year.to_be_bytes());
    id[18] = month;
    id[19] = day;
    for (i, value) in id[20..28].iter_mut().enumerate() {
        *value = 0x11 + i as u8;
    }
    id[36..40].copy_from_slice(&(data.len() as u32).to_le_bytes());
    id.extend_from_slice(data);
    id
}

pub fn system_time(month: u16, day: u16) -> Vec<u8> {
    [0u16, month, 0, day, 2, 0, 0, 0]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect()
}

pub fn eastern(key: &str, bias: i32) -> Vec<u8> {
    let key: Vec<u16> = key.encode_utf16().collect();
    let mut bytes = vec![2, 1];
    bytes.extend_from_slice(&(6 + key.len() as u16 * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&(key.len() as u16).to_le_bytes());
    bytes.extend(key.into_iter().flat_map(u16::to_le_bytes));
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&[2, 1, 0x3e, 0, 2, 0, 0xd7, 7]);
    bytes.extend_from_slice(&[0; 14]);
    bytes.extend_from_slice(&bias.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&(-60i32).to_le_bytes());
    bytes.extend(system_time(11, 1));
    bytes.extend(system_time(3, 2));
    bytes
}

pub fn structure() -> Vec<u8> {
    let mut bytes = vec![0; 48];
    bytes[..4].copy_from_slice(&300i32.to_le_bytes());
    bytes[8..12].copy_from_slice(&(-60i32).to_le_bytes());
    bytes[14..30].copy_from_slice(&system_time(11, 1));
    bytes[32..48].copy_from_slice(&system_time(3, 2));
    bytes
}
