# mimecraft

## What is mimecraft?

mimecraft is a Rust library for creating, parsing, and processing email messages
using Multipurpose Internet Mail Extensions (MIME). It is an idiomatic port of
[MimeKit](https://github.com/jstedfast/MimeKit)'s core library, with Rust ownership,
streaming I/O, and explicit error handling rather than a one-to-one translation
of its C# API.

**Work in progress:** mimecraft is incomplete and is not yet a replacement for
MimeKit. Its API is still evolving, and the crate is not configured for
publication. See [Porting status and compatibility](STATUS.md) for detailed
coverage, limitations, and differences from MimeKit.

## Contents

- [History](#history)
- [Performance](#performance)
- [License Information](#license-information)
- [Getting the Source Code](#getting-the-source-code)
- [Building](#building)
- [Dependencies](#dependencies)
- [Using mimecraft](#using-mimecraft)
- [Contributing](#contributing)
- [Reporting Bugs](#reporting-bugs)
- [Documentation](#documentation)

## History

mimecraft brings MimeKit's MIME parsing, message construction, and compatibility
work to Rust. The aim is to preserve its useful behavior and handling of
real-world email while offering an API suited to Rust applications.

The port covers messages and MIME trees, headers and addresses, transfer
encodings, streaming filters, and supporting text utilities. MimeKit's
cryptography functionality, including S/MIME, OpenPGP, DKIM, ARC, and DMARC,
is outside the current scope. SMTP address validation does not provide an SMTP
client or transport negotiation.

## Performance

mimecraft has a synchronous, byte-oriented MIME reader and a higher-level
message parser. Source-backed parsing can retain references to seekable input
ranges instead of copying every part's content into memory.

The default `simd` feature enables runtime-detected x86/x86-64 and AArch64
acceleration for MIME scanning and Base64/UU codecs, with scalar fallbacks.
Disabling default features removes mimecraft's own SIMD and unsafe code;
dependencies may still use their own acceleration and unsafe implementations.

See [Acceleration](STATUS.md#acceleration) and
[Benchmarks](STATUS.md#benchmarks) for implementation details, workloads, and
measurement commands. Benchmark results are not a throughput guarantee or a
direct comparison against MimeKit.

## License Information

Library code is licensed under the [MIT license](LICENSE), retaining MimeKit's
original copyright notice. The bundled Unicode tables and normalization
conformance data are covered by [Unicode License v3](LICENSE-Unicode).

## Getting the Source Code

```text
git clone https://github.com/jstedfast/mimecraft.git
cd mimecraft
```

Until the crate is published, another Cargo project can depend on the Git
repository. Pin a revision when reproducible builds are important:

```toml
[dependencies]
mimecraft = { git = "https://github.com/jstedfast/mimecraft" }
```

## Building

With a Rust toolchain supporting this crate and its dependencies, run these
commands from the repository root:

```text
cargo build --release
cargo test
```

The crate uses Rust edition 2024; a minimum supported Rust version has not yet
been established. To build without mimecraft's SIMD implementations:

```text
cargo build --release --no-default-features
```

See [Development](STATUS.md#development) for additional checks and data
regeneration instructions.

## Dependencies

The library's direct dependencies are listed in [Cargo.toml](Cargo.toml):

| Dependency | Why mimecraft uses it |
| --- | --- |
| [encoding_rs](https://github.com/hsivonen/encoding_rs) | Legacy character-set encoding and decoding for MIME text, headers, and incremental charset-conversion filters. |
| [fluent-uri](https://github.com/yescallop/fluent-uri-rs) | URI parsing and reference resolution for locating resources in `multipart/related` messages. |
| [getrandom](https://github.com/rust-random/getrandom) | Operating-system randomness for generated Message-Ids and multipart boundaries. |
| [idna](https://github.com/servo/rust-url) | Internationalized domain-name conversion and raw Punycode encoding/decoding for SMTPUTF8 validation. |
| [memchr](https://github.com/BurntSushi/memchr) | Optimized byte searches in MIME parsing, newline scanning, and transfer-encoding compliance checks. |
| [unicode-general-category](https://github.com/yeslogic/unicode-general-category) | Unicode character classification for URL/email recognition and enriched-text font-family validation. |

SMTPUTF8 policy and NFC checking use separately bundled Unicode tables, not
`unicode-general-category`'s data. See the
[SMTPUTF8 policy notes](STATUS.md#smtputf8-unicode-address-policy).

The following direct development dependencies are used by tests, not by the
library at runtime:

| Dependency | Why mimecraft uses it |
| --- | --- |
| [serde](https://github.com/serde-rs/serde) | Derives deserialization for expected MIME reader results in upstream compatibility fixtures. |
| [serde_json](https://github.com/serde-rs/json) | Reads the JSON fixtures used to check MIME structure and source offsets against MimeKit. |

## Using mimecraft

### Parsing Messages

Load a message from a file and read its decoded subject:

```rust,no_run
use mimecraft::MimeMessage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let message = MimeMessage::load_file("message.eml")?;
    if let Some(subject) = message.subject() {
        println!("Subject: {}", subject.text);
    }
    Ok(())
}
```

`MimeMessage::load` also accepts a buffered reader. The default load APIs copy
part content into owned memory; configurable limits, source-backed storage,
mbox iteration, and lower-level reader events are described in the
[reader and parser notes](STATUS.md#mime-reader-and-parser).

### Getting the Body of a Message

Request the preferred plain-text or HTML body, with transfer encoding and
character-set decoding handled by the library:

```rust,no_run
use mimecraft::{MimeMessage, TextFormat};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut message = MimeMessage::load_file("message.eml")?;
    if let Some(text) = message.text_body(TextFormat::Plain)? {
        println!("{text}");
    }
    Ok(())
}
```

Use `TextFormat::Html` for an HTML body. This selects an existing body rather
than synthesizing one from another format, and does not sanitize HTML for
display. See [Text-body selection](STATUS.md#text-body-selection) and
[Decoded body text](STATUS.md#decoded-body-text-and-flowed-conversion).

### Creating a Message with Attachments

`BodyBuilder` assembles plain text, HTML, linked resources, and attachments into
the appropriate MIME layout:

```rust
use mimecraft::{BodyBuilder, MimeMessage, attachment};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = BodyBuilder {
        text_body: Some("Here are the meeting notes.".into()),
        html_body: Some("<p>Here are the meeting notes.</p>".into()),
        ..BodyBuilder::new()
    };
    builder.attachments.push(
        attachment::from_bytes("notes.txt", b"Discuss the next release.\r\n")?,
    );

    let mut message = MimeMessage::with_body(builder.into_message_body()?);
    message.set_from(&"Alice <alice@example.com>".parse()?)?;
    message.set_to(&"Bob <bob@example.com>".parse()?)?;
    message.set_subject("Meeting notes")?;

    let mut output = Vec::new();
    message.write_to(&mut output)?;
    Ok(())
}
```

The output contains the serialized message, not an SMTP transaction. Set
metadata such as Date and Message-Id explicitly when needed, and use
[transport preparation](STATUS.md#explicit-tree-transport-preparation) before
writing for a constrained transport. Serialization does not automatically
remove Bcc or Resent-Bcc headers.

More examples cover [attachments and linked resources](STATUS.md#constructing-attachments-and-linked-resources),
[MIME tree navigation](STATUS.md#mime-tree-navigation), and
[SMTP address validation](STATUS.md#smtp-envelope-address-validation).

## Contributing

Read [Porting status and compatibility](STATUS.md) before choosing work, so
changes respect existing behavior and intentional differences from MimeKit.
Preserve applicable MimeKit inline comments, adapting explanations to Rust
rather than retaining C#-specific implementation details.

Include focused regression coverage with behavioral changes. Development
commands and upstream test provenance are documented in
[STATUS.md](STATUS.md#development).

## Reporting Bugs

Report problems through [GitHub issues](https://github.com/jstedfast/mimecraft/issues).
Include a minimal reproduction, the mimecraft revision, your Rust toolchain
and platform, and the expected and actual behavior. Remove private message
content, credentials, and personal data before sharing email samples.

## Documentation

Generate the API documentation locally:

```text
cargo doc --no-deps --open
```

[STATUS.md](STATUS.md) contains the detailed feature inventory, API examples,
compatibility notes, remaining gaps, and benchmark and regeneration guidance.
