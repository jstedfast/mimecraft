// Adapted from MimeKit's Text/RtfKeyword.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

/// Control-word classifications used by MimeKit's RTF converters.
///
/// This is deliberately not a catalogue of every RTF control word. Unrecognized
/// words map to [`Self::Unknown`], and related rendering operations share an ID.
/// Classification does not interpret or render a token.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum RtfKeyword {
    /// A word not recognized by the converters.
    #[default]
    Unknown,
    /// RTF version.
    Rtf,
    /// ANSI character set.
    Ansi,
    /// Macintosh character set.
    Mac,
    /// IBM PC character set.
    Pc,
    /// IBM PC multilingual character set.
    Pca,
    /// ANSI code page.
    Ansicpg,
    /// Default font.
    Deff,
    /// Length of the following binary payload.
    Bin,
    /// Encapsulated HTML declaration.
    Fromhtml,
    /// RTF-only content within encapsulated HTML.
    Htmlrtf,
    /// Encapsulated HTML tag destination.
    Htmltag,
    /// Font table.
    Fonttbl,
    /// Color table.
    Colortbl,
    /// Field group.
    Field,
    /// Field instruction.
    Fldinst,
    /// Field result.
    Fldrslt,
    /// Alternative Unicode representation.
    Upr,
    /// Unicode alternative destination.
    Ud,
    /// Nested table properties and row terminator.
    Nesttableprops,
    /// A destination whose contents converters should not render.
    ///
    /// The tokenizer still emits its contents; destination skipping belongs
    /// to an interpreter, not the lexer.
    SkipDestination,
    /// Font selection or font-table entry.
    F,
    /// Font character set.
    Fcharset,
    /// Font code page.
    Cpg,
    /// Red color component.
    Red,
    /// Green color component.
    Green,
    /// Blue color component.
    Blue,
    /// Reset character formatting.
    Plain,
    /// Bold.
    B,
    /// Italic.
    I,
    /// Underline, including the supported underline-style aliases.
    Ul,
    /// Disable underline.
    Ulnone,
    /// Strike-through.
    Strike,
    /// Hidden text.
    V,
    /// Superscript.
    Super,
    /// Subscript.
    Sub,
    /// Disable superscript and subscript.
    Nosupersub,
    /// Font size.
    Fs,
    /// Foreground color.
    Cf,
    /// Background color or highlight.
    Cb,
    /// Unicode UTF-16 code unit.
    U,
    /// Number of fallback characters after a Unicode escape.
    Uc,
    /// Paragraph, section, or page break.
    Par,
    /// Reset paragraph formatting.
    Pard,
    /// Line break.
    Line,
    /// Tab.
    Tab,
    /// Left alignment.
    Ql,
    /// Center alignment.
    Qc,
    /// Right alignment.
    Qr,
    /// Justified alignment.
    Qj,
    /// Paragraph belongs to a table.
    Intbl,
    /// Table or nested-table cell terminator.
    Cell,
    /// Table or nested-table row terminator.
    Row,
    /// Em dash.
    Emdash,
    /// En dash.
    Endash,
    /// Bullet.
    Bullet,
    /// Left single quotation mark.
    Lquote,
    /// Right single quotation mark.
    Rquote,
    /// Left double quotation mark.
    Ldblquote,
    /// Right double quotation mark.
    Rdblquote,
    /// Em space.
    Emspace,
    /// En space.
    Enspace,
    /// Quarter-em space.
    Qmspace,
    /// Zero-width joiner.
    Zwj,
    /// Zero-width non-joiner.
    Zwnj,
    /// Left-to-right mark.
    Ltrmark,
    /// Right-to-left mark.
    Rtlmark,
    /// Attachment placeholder.
    Objattph,
}

impl RtfKeyword {
    /// Classifies an exact, case-sensitive name without a leading backslash.
    ///
    /// Does not allocate, trim, parse a parameter, or fold case. The original
    /// spelling remains available on the tokenizer's control-word token.
    pub fn from_name(name: &str) -> Self {
        // Static string dispatch replaces the source's runtime-sorted lookup
        // table. RTF names are case-sensitive, including mixed-case mmathPr.
        match name {
            "rtf" => Self::Rtf,
            "ansi" => Self::Ansi,
            "mac" => Self::Mac,
            "pc" => Self::Pc,
            "pca" => Self::Pca,
            "ansicpg" => Self::Ansicpg,
            "deff" => Self::Deff,
            "bin" => Self::Bin,
            // MS-OXRTFEX: control words used to encapsulate HTML within RTF.
            "fromhtml" => Self::Fromhtml,
            "htmlrtf" => Self::Htmlrtf,
            "htmltag" => Self::Htmltag,
            "fonttbl" => Self::Fonttbl,
            "colortbl" => Self::Colortbl,
            "field" => Self::Field,
            "fldinst" => Self::Fldinst,
            "fldrslt" => Self::Fldrslt,
            "upr" => Self::Upr,
            "ud" => Self::Ud,
            // This destination contains both nested-row properties and the
            // nestrow that ends the row. It must not be treated as an unknown
            // ignorable destination by an interpreter.
            "nesttableprops" => Self::Nesttableprops,
            // Metadata, pictures, embedded data, and other non-rendered
            // destinations. Many lack a leading \*, so an interpreter must
            // recognize them explicitly or their text will leak into output.
            //
            // Deliberately do not classify \object itself as skipped: its
            // \result subgroup is the display fallback for unsupported objects.
            // Only object data subgroups are skipped.
            "aftnsep" | "aftnsepc" | "annotation" | "atnauthor" | "atnid"
            | "background" | "bkmkend" | "bkmkstart" | "colorschememapping"
            | "datastore" | "docvar" | "filetbl" | "footer" | "footerf" | "footerl"
            | "footerr" | "footnote" | "ftnsep" | "ftnsepc" | "generator"
            | "header" | "headerf" | "headerl" | "headerr" | "info" | "latentstyles"
            | "listoverridetable" | "listtable" | "mhtmltag" | "mmathPr"
            // nonesttables duplicates rendering for readers without nested tables.
            | "nonesttables" | "nonshppict" | "objdata" | "pgdsctbl" | "pict"
            | "revtbl" | "rsidtbl" | "shpinst" | "stylesheet" | "tc" | "themedata"
            | "txe" | "userprops" | "xe" | "xmlnstbl" => Self::SkipDestination,
            "f" => Self::F,
            "fcharset" => Self::Fcharset,
            "cpg" => Self::Cpg,
            "red" => Self::Red,
            "green" => Self::Green,
            "blue" => Self::Blue,
            "plain" => Self::Plain,
            "b" => Self::B,
            "i" => Self::I,
            // The converters render the various underline styles as plain underline.
            "ul" | "uld" | "uldash" | "uldashd" | "uldashdd" | "uldb" | "ulhwave"
            | "ulldash" | "ulth" | "ulthd" | "ulthdash" | "ulthdashd" | "ulthdashdd"
            | "ulthldash" | "ululdbwave" | "ulw" | "ulwave" => Self::Ul,
            "ulnone" => Self::Ulnone,
            "strike" | "striked" => Self::Strike,
            "v" => Self::V,
            "super" => Self::Super,
            "sub" => Self::Sub,
            "nosupersub" => Self::Nosupersub,
            "fs" => Self::Fs,
            "cf" => Self::Cf,
            // Word ignores the documented \cb and writes \highlight or the
            // character shading color \chcbpat instead; treat them alike.
            "cb" | "chcbpat" | "highlight" => Self::Cb,
            "u" => Self::U,
            "uc" => Self::Uc,
            // Section/page breaks have no distinct meaning in reflowable output.
            "par" | "sect" | "page" => Self::Par,
            "pard" => Self::Pard,
            "line" => Self::Line,
            "tab" => Self::Tab,
            "ql" => Self::Ql,
            "qc" => Self::Qc,
            "qr" => Self::Qr,
            "qj" => Self::Qj,
            "intbl" => Self::Intbl,
            "cell" | "nestcell" => Self::Cell,
            "row" | "nestrow" => Self::Row,
            "emdash" => Self::Emdash,
            "endash" => Self::Endash,
            "bullet" => Self::Bullet,
            "lquote" => Self::Lquote,
            "rquote" => Self::Rquote,
            "ldblquote" => Self::Ldblquote,
            "rdblquote" => Self::Rdblquote,
            "emspace" => Self::Emspace,
            "enspace" => Self::Enspace,
            "qmspace" => Self::Qmspace,
            "zwj" => Self::Zwj,
            "zwnj" => Self::Zwnj,
            "ltrmark" => Self::Ltrmark,
            "rtlmark" => Self::Rtlmark,
            // MS-OXRTFEX attachment placeholder, not an embedded payload.
            "objattph" => Self::Objattph,
            _ => Self::Unknown,
        }
    }
}
