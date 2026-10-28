// Adapted from MimeKit's Text/HtmlTagId.cs and Text/HtmlAttributeId.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::fmt;

macro_rules! html_ids {
    ($(#[$attribute:meta])* $type:ident, $table:ident { $($id:ident => $name:literal,)* }) => {
        $(#[$attribute])*
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
        pub enum $type {
            /// An empty or unrecognized name, not necessarily an invalid name.
            #[default]
            Unknown,
            $(
                #[doc = concat!("The `", $name, "` HTML name.")]
                $id,
            )*
        }

        impl $type {
            /// Returns the canonical name, or `"Unknown"` for [`Self::Unknown`].
            ///
            /// `"Unknown"` is a diagnostic label, not the original name of an
            /// unrecognized tag or attribute. Numeric enum values are not a
            /// compatibility contract.
            pub const fn as_str(self) -> &'static str {
                match self {
                    Self::Unknown => "Unknown",
                    $(Self::$id => $name,)*
                }
            }

            fn lookup(name: &str) -> Self {
                let id = Self::lookup_exact(name);
                if id != Self::Unknown {
                    return id;
                }
                const MAX_LENGTH: usize = {
                    let mut maximum = 0;
                    $(if $name.len() > maximum { maximum = $name.len(); })*
                    maximum
                };
                if name.len() > MAX_LENGTH {
                    return Self::Unknown;
                }
                // Fold once, not during every comparison in a binary search.
                // The match lets the compiler partition names by length.
                let mut buffer = [0; MAX_LENGTH];
                let normalized = &mut buffer[..name.len()];
                normalized.copy_from_slice(name.as_bytes());
                normalized.make_ascii_lowercase();
                let normalized = std::str::from_utf8(normalized)
                    .expect("ASCII folding preserves UTF-8");
                Self::lookup_exact(normalized)
            }

            fn lookup_exact(name: &str) -> Self {
                match name {
                    $($name => Self::$id,)*
                    _ => Self::Unknown,
                }
            }

            pub(super) fn take_name(name: &mut String) -> (Self, std::borrow::Cow<'static, str>) {
                let (id, canonical) = Self::canonical_name(name);
                // Canonical names need no per-token allocation, and the scanner
                // keeps its buffer. Preserve every other spelling verbatim.
                let value = if let Some(canonical) = canonical {
                    name.clear();
                    std::borrow::Cow::Borrowed(canonical)
                } else {
                    std::borrow::Cow::Owned(std::mem::take(name))
                };
                (id, value)
            }

            pub(super) fn canonical_name(name: &str) -> (Self, Option<&'static str>) {
                let id = Self::from_name(name);
                (id, (id != Self::Unknown && name == id.as_str()).then(|| id.as_str()))
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        #[cfg(test)]
        const $table: &[(&str, $type)] = &[$(($name, $type::$id),)*];
    };
}

html_ids! {
    /// A tag name recognized by MimeKit, including its historical HTML elements.
    ///
    /// Identifiers classify names, not complete markup. They do not validate,
    /// parse, or sanitize HTML. Rust variants use idiomatic acronym casing.
    ///
    /// ```
    /// use mimecraft::text::HtmlTagId;
    ///
    /// assert_eq!(HtmlTagId::from_name("IMG"), HtmlTagId::Image);
    /// assert_eq!(HtmlTagId::Image.as_str(), "img");
    /// assert!(HtmlTagId::Image.is_empty_element());
    /// assert_eq!(HtmlTagId::from_name("!doctype"), HtmlTagId::Comment);
    /// ```
    HtmlTagId, TAG_IDS {
        Comment => "!",
        A => "a",
        Abbr => "abbr",
        Acronym => "acronym",
        Address => "address",
        Applet => "applet",
        Area => "area",
        Article => "article",
        Aside => "aside",
        Audio => "audio",
        B => "b",
        Base => "base",
        BaseFont => "basefont",
        Bdi => "bdi",
        Bdo => "bdo",
        BgSound => "bgsound",
        Big => "big",
        Blink => "blink",
        BlockQuote => "blockquote",
        Body => "body",
        Br => "br",
        Button => "button",
        Canvas => "canvas",
        Caption => "caption",
        Center => "center",
        Cite => "cite",
        Code => "code",
        Col => "col",
        ColGroup => "colgroup",
        Command => "command",
        DataList => "datalist",
        Dd => "dd",
        Del => "del",
        Details => "details",
        Dfn => "dfn",
        Dialog => "dialog",
        Dir => "dir",
        Div => "div",
        Dl => "dl",
        Dt => "dt",
        Em => "em",
        Embed => "embed",
        FieldSet => "fieldset",
        FigCaption => "figcaption",
        Figure => "figure",
        Font => "font",
        Footer => "footer",
        Form => "form",
        Frame => "frame",
        FrameSet => "frameset",
        H1 => "h1",
        H2 => "h2",
        H3 => "h3",
        H4 => "h4",
        H5 => "h5",
        H6 => "h6",
        Head => "head",
        Header => "header",
        Hr => "hr",
        Html => "html",
        I => "i",
        IFrame => "iframe",
        Image => "img",
        Input => "input",
        Ins => "ins",
        IsIndex => "isindex",
        Kbd => "kbd",
        Keygen => "keygen",
        Label => "label",
        Legend => "legend",
        Li => "li",
        Link => "link",
        Listing => "listing",
        Main => "main",
        Map => "map",
        Mark => "mark",
        Marquee => "marquee",
        Menu => "menu",
        MenuItem => "menuitem",
        Meta => "meta",
        Meter => "meter",
        Nav => "nav",
        NextId => "nextid",
        NoBr => "nobr",
        NoEmbed => "noembed",
        NoFrames => "noframes",
        NoScript => "noscript",
        Object => "object",
        Ol => "ol",
        OptGroup => "optgroup",
        Option => "option",
        Output => "output",
        P => "p",
        Param => "param",
        PlainText => "plaintext",
        Pre => "pre",
        Progress => "progress",
        Q => "q",
        Rp => "rp",
        Rt => "rt",
        Ruby => "ruby",
        S => "s",
        Samp => "samp",
        Script => "script",
        Section => "section",
        Select => "select",
        Small => "small",
        Source => "source",
        Span => "span",
        Strike => "strike",
        Strong => "strong",
        Style => "style",
        Sub => "sub",
        Summary => "summary",
        Sup => "sup",
        Table => "table",
        TBody => "tbody",
        Td => "td",
        TextArea => "textarea",
        Tfoot => "tfoot",
        Th => "th",
        THead => "thead",
        Time => "time",
        Title => "title",
        Tr => "tr",
        Track => "track",
        Tt => "tt",
        U => "u",
        Ul => "ul",
        Var => "var",
        Video => "video",
        Wbr => "wbr",
        Xml => "xml",
        Xmp => "xmp",
    }
}

impl HtmlTagId {
    /// Recognizes a name without trimming whitespace or parsing markup.
    ///
    /// Matching is ASCII case-insensitive and does not allocate. Any name
    /// beginning with `!` maps to [`Self::Comment`], following MimeKit.
    /// Other unrecognized or empty names map to [`Self::Unknown`].
    pub fn from_name(name: &str) -> Self {
        if name.starts_with('!') {
            return Self::Comment;
        }
        Self::lookup(name)
    }

    /// Tests MimeKit's empty-element classification, including legacy elements.
    ///
    /// This does not instruct [`super::HtmlWriter`] how to close a tag.
    pub const fn is_empty_element(self) -> bool {
        matches!(
            self,
            Self::Area
                | Self::Base
                | Self::Br
                | Self::Col
                | Self::Command
                | Self::Embed
                | Self::Hr
                | Self::Image
                | Self::Input
                | Self::Keygen
                | Self::Link
                | Self::Meta
                | Self::Param
                | Self::Source
                | Self::Track
                | Self::Wbr
        )
    }

    /// Tests MimeKit's formatting-element classification.
    pub const fn is_formatting_element(self) -> bool {
        matches!(
            self,
            Self::A
                | Self::B
                | Self::Big
                | Self::Code
                | Self::Em
                | Self::Font
                | Self::I
                | Self::NoBr
                | Self::S
                | Self::Small
                | Self::Strike
                | Self::Strong
                | Self::Tt
                | Self::U
        )
    }
}

html_ids! {
    /// An attribute name recognized by MimeKit.
    ///
    /// Unknown custom attributes can still be represented by
    /// [`super::HtmlAttribute`], which preserves their original names.
    ///
    /// ```
    /// use mimecraft::text::HtmlAttributeId;
    ///
    /// assert_eq!(HtmlAttributeId::from_name("HTTP-EQUIV"), HtmlAttributeId::HttpEquiv);
    /// assert_eq!(HtmlAttributeId::AcceptCharset.as_str(), "accept-charset");
    /// assert_eq!(HtmlAttributeId::from_name("data-id"), HtmlAttributeId::Unknown);
    /// ```
    HtmlAttributeId, ATTRIBUTE_IDS {
        Abbr => "abbr",
        Accept => "accept",
        AcceptCharset => "accept-charset",
        AccessKey => "accesskey",
        Action => "action",
        Align => "align",
        Alink => "alink",
        Alt => "alt",
        Archive => "archive",
        Axis => "axis",
        Background => "background",
        BgColor => "bgcolor",
        Border => "border",
        CellPadding => "cellpadding",
        CellSpacing => "cellspacing",
        Char => "char",
        CharOff => "charoff",
        Charset => "charset",
        Checked => "checked",
        Cite => "cite",
        Class => "class",
        ClassId => "classid",
        Clear => "clear",
        Code => "code",
        CodeBase => "codebase",
        CodeType => "codetype",
        Color => "color",
        Cols => "cols",
        ColSpan => "colspan",
        Compact => "compact",
        Content => "content",
        Coords => "coords",
        Data => "data",
        DateTime => "datetime",
        Declare => "declare",
        Defer => "defer",
        Dir => "dir",
        Disabled => "disabled",
        DynSrc => "dynsrc",
        EncType => "enctype",
        Face => "face",
        For => "for",
        Frame => "frame",
        FrameBorder => "frameborder",
        Headers => "headers",
        Height => "height",
        Href => "href",
        HrefLang => "hreflang",
        Hspace => "hspace",
        HttpEquiv => "http-equiv",
        Id => "id",
        IsMap => "ismap",
        Label => "label",
        Lang => "lang",
        Language => "language",
        LeftMargin => "leftmargin",
        Link => "link",
        LongDesc => "longdesc",
        LowSrc => "lowsrc",
        MarginHeight => "marginheight",
        MarginWidth => "marginwidth",
        MaxLength => "maxlength",
        Media => "media",
        Method => "method",
        Multiple => "multiple",
        Name => "name",
        NoHref => "nohref",
        NoResize => "noresize",
        NoShade => "noshade",
        NoWrap => "nowrap",
        Object => "object",
        Profile => "profile",
        Prompt => "prompt",
        ReadOnly => "readonly",
        Rel => "rel",
        Rev => "rev",
        Rows => "rows",
        RowSpan => "rowspan",
        Rules => "rules",
        Scheme => "scheme",
        Scope => "scope",
        Scrolling => "scrolling",
        Selected => "selected",
        Shape => "shape",
        Size => "size",
        Span => "span",
        Src => "src",
        StandBy => "standby",
        Start => "start",
        Style => "style",
        Summary => "summary",
        TabIndex => "tabindex",
        Target => "target",
        Text => "text",
        Title => "title",
        TopMargin => "topmargin",
        Type => "type",
        UseMap => "usemap",
        Valign => "valign",
        Value => "value",
        ValueType => "valuetype",
        Version => "version",
        Vlink => "vlink",
        Vspace => "vspace",
        Width => "width",
        XmlNs => "xmlns",
    }
}

impl HtmlAttributeId {
    /// Recognizes a complete attribute name using ASCII case-insensitive lookup.
    ///
    /// No allocation, trimming, name validation, or HTML decoding is performed.
    /// Empty and unrecognized names map to [`Self::Unknown`].
    pub fn from_name(name: &str) -> Self {
        Self::lookup(name)
    }
}

#[cfg(test)]
mod tests {
    use super::{ATTRIBUTE_IDS, HtmlAttributeId, HtmlTagId, TAG_IDS};
    use crate::text::HtmlAttribute;

    #[test]
    fn tag_names_are_complete_sorted_unique_and_round_trip() {
        assert_eq!(TAG_IDS.len(), 134);
        let mut ids = std::collections::HashSet::new();
        for &(name, id) in TAG_IDS {
            assert!(ids.insert(id));
            assert_ne!(id, HtmlTagId::Unknown);
            assert_eq!(id.as_str(), name);
            assert_eq!(id.to_string(), name);
            assert_eq!(HtmlTagId::from_name(name), id);
            assert_eq!(HtmlTagId::from_name(&name.to_ascii_uppercase()), id);
        }
        for pair in TAG_IDS.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{pair:?}");
        }
    }

    #[test]
    fn attribute_names_are_complete_sorted_unique_and_round_trip() {
        assert_eq!(ATTRIBUTE_IDS.len(), 106);
        let mut ids = std::collections::HashSet::new();
        for &(name, id) in ATTRIBUTE_IDS {
            assert!(ids.insert(id));
            assert_ne!(id, HtmlAttributeId::Unknown);
            assert_eq!(id.as_str(), name);
            assert_eq!(id.to_string(), name);
            assert_eq!(HtmlAttributeId::from_name(name), id);
            assert_eq!(HtmlAttributeId::from_name(&name.to_ascii_uppercase()), id);
            let attribute = HtmlAttribute::from_id(id, None).unwrap();
            assert_eq!(attribute.name(), name);
            assert_eq!(attribute.id(), id);
            let attribute = HtmlAttribute::new(name.to_ascii_uppercase(), None).unwrap();
            assert_eq!(attribute.id(), id);
            assert_eq!(attribute.name(), name.to_ascii_uppercase());
        }
        for pair in ATTRIBUTE_IDS.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{pair:?}");
        }
    }

    #[test]
    fn classifications_match_the_upstream_sets_exactly() {
        let empty = [
            "area", "base", "br", "col", "command", "embed", "hr", "img", "input", "keygen",
            "link", "meta", "param", "source", "track", "wbr",
        ];
        let formatting = [
            "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong",
            "tt", "u",
        ];
        for &(name, id) in TAG_IDS {
            assert_eq!(id.is_empty_element(), empty.contains(&name), "{name}");
            assert_eq!(
                id.is_formatting_element(),
                formatting.contains(&name),
                "{name}"
            );
        }
        assert!(!HtmlTagId::Unknown.is_empty_element());
        assert!(!HtmlTagId::Unknown.is_formatting_element());
    }

    #[test]
    fn folded_lookup_matches_the_name_tables() {
        for &(name, expected) in TAG_IDS {
            let mixed: String = name
                .bytes()
                .enumerate()
                .map(|(i, byte)| {
                    char::from(if i % 2 == 0 {
                        byte.to_ascii_uppercase()
                    } else {
                        byte
                    })
                })
                .collect();
            assert_eq!(HtmlTagId::from_name(&mixed), expected);
        }
        for &(name, expected) in ATTRIBUTE_IDS {
            let mixed: String = name
                .bytes()
                .enumerate()
                .map(|(i, byte)| {
                    char::from(if i % 2 == 0 {
                        byte.to_ascii_uppercase()
                    } else {
                        byte
                    })
                })
                .collect();
            assert_eq!(HtmlAttributeId::from_name(&mixed), expected);
        }
        for name in [
            "",
            "unknown",
            "Unknown",
            "CLASS\0",
            "d\u{130}v",
            "\u{e9}",
            "\u{1f600}",
            "abcdefghijklmnopqrstuvwxyz",
        ] {
            assert_eq!(HtmlTagId::from_name(name), HtmlTagId::Unknown, "{name}");
            assert_eq!(
                HtmlAttributeId::from_name(name),
                HtmlAttributeId::Unknown,
                "{name}"
            );
        }
    }
}
