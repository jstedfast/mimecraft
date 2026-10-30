# Porting status and compatibility

This document tracks mimecraft's implementation of MimeKit's core MIME library:
supported behavior, remaining gaps, intentional differences, upstream test
coverage, and implementation details. Behavioral compatibility with MimeKit is
the goal, not a one-to-one translation of its C# API.

For an introduction, dependencies, build instructions, and common usage examples,
see the [README](README.md).

This is an early, incomplete implementation, not yet a usable replacement for
MimeKit. The crate is not configured for publication.

## Contents

- [Current functionality](#current-functionality)
- [MIME reader and parser](#mime-reader-and-parser)
- [Message ownership and I/O](#message-ownership-and-io)
- [Building common message bodies](#building-common-message-bodies)
- [Constructing attachments and linked resources](#constructing-attachments-and-linked-resources)
- [Raw header parsing](#raw-header-parsing)
- [Address specifications](#address-specifications)
- [SMTP envelope address validation](#smtp-envelope-address-validation)
- [SMTPUTF8 Unicode address policy](#smtputf8-unicode-address-policy)
- [Character sets](#character-sets)
- [Content-Type and Content-Disposition](#content-type-and-content-disposition)
- [Scope and compatibility](#scope-and-compatibility)
- [Acceleration](#acceleration)
- [Development](#development)
- [Regenerating HTML entities](#regenerating-html-entities)
- [Benchmarks](#benchmarks)

## Current functionality

- Incremental Base64 encoding with MIME line wrapping, padding, and CRLF by default.
- Incremental, tolerant MIME Base64 decoding, including concatenated padded blocks.
- Table-driven scalar decoding and runtime-dispatched SSSE3/AVX2 (x86/x86-64)
  and NEON (AArch64) acceleration for Base64 and UU encoding and decoding.
- Incremental quoted-printable body encoding and tolerant decoding, including
  soft line breaks, trailing whitespace, and malformed escape handling.
- Incremental UU payload encoding and tolerant decoding, with optional
  `begin`-line scanning and DOS/Unix new-line formats.
- Incremental yEnc payload encoding and tolerant decoding, with CRC-32 tracking,
  optional `=ybegin`/`=ypart` scanning, and DOS/Unix new-line formats.
- Complete synchronous streaming yEnc block reading and writing with checked
  single-part/multipart metadata, filename charsets, size/line limits, and
  checksum trailers and validation.
- RFC 2231 parameter percent encoding and incremental, tolerant decoding.
- MIME parameter-list parsing with quoted values, RFC 2231 continuations,
  case-insensitive lookup, and explicit syntax errors and recovery diagnostics.
- Constructible, editable Content-Type and Content-Disposition values, with MIME-type matching,
  decoded parameters, charset resolution, filenames, dates, and size metadata.
- Filename MIME-type guesses and preferred extensions from MimeKit-derived
  tables with selected IANA updates and instance-local custom registrations.
- Ordered parameter editing with validated names, case-insensitive uniqueness,
  explicit duplicate/index errors, and atomic validation failures.
- RFC 2047 B/Q payload encoding and Q payload decoding, plus encoded-word
  text/phrase encoding with bounded, self-contained words and decoding with
  explicit charset recovery diagnostics.
- `ContentEncoding` metadata, byte-preserving pass-through codecs, and common
  `MimeEncoder`/`MimeDecoder` traits for incremental codec use.
- Buffer-oriented filters for CRLF/LF conversion, pass-through, and incremental
  encoding/decoding, with enum-based transfer-encoding factories.
- Mbox `From ` escaping, quoted-printable `From ` armoring, and trailing-space/tab
  removal, including markers and whitespace runs split across input chunks.
- Byte masking that preserves MIME whitespace and byte counts, for explicit
  use in filtering pipelines rather than automatic message anonymization.
- Raw address, unstructured, content-header, and Received masking with preserved
  syntax, parameters, trace keywords, and recognized date suffixes for inspection.
- Received-header parsing with ordered clauses, nested comments,
  multi-word values, preserved date spelling, charset recovery reporting, and
  typed syntax errors with source byte offsets; checked construction and
  convenience setters, editable clauses/comments, date-cache invalidation,
  and complete-header formatting with enforced line limits and UTF-8 opt-in.
- Field-aware masking of complete raw header records, including byte masking of
  malformed records, default metadata preservation, and configurable preserved
  fields, plus masked header-list output with CRLF/LF conversion.
- Masked message-header sections, including root-body headers and a separator,
  without reading content or modifying the message.
- Streaming masking of ordinary part content after serialization, including
  transfer-encoded payloads and UU wrappers, plus complete masked ordinary parts.
- Specialized decoded-field masking of delivery-status and disposition-
  notification bodies, with configurable preservation and parse limits.
- Raw-byte masking output with newline conversion and optional termination
  for supplied preamble/epilogue regions.
- Transfer-encoding analysis using MimeKit's heuristics and seven-bit, eight-bit,
  or unrestricted transport constraints.
- A synchronous `FilteredReader` with `Read`/`BufRead` support, EOF finalization,
  and ordered composition through nested readers.
- A synchronous `FilteredWriter` with retry-safe output buffering, non-finalizing
  `flush`, and explicit one-time filter finalization.
- Seekable MIME content streams with raw copying, decoded copying, and
  exclusively borrowed decoded readers using the existing transfer codecs.
- Single-part MIME models with authoritative raw headers, checked content
  metadata edits, and independent stored-body and transfer-header encodings.
- Synchronous single-part writing with streaming transfer conversion, UU
  wrappers, header/body separation, and configurable newline handling.
- Text parts with automatic MIME/BOM/HTML charset detection and recovery,
  strict explicit overrides, borrowed or owned streams, and atomic text edits.
- Whole-string flowed-text wrapping and unwrapping, with quote-depth handling,
  space stuffing, UTF-8-safe wrapping, and explicit CRLF/LF output.
- Plain-text previews with Unicode whitespace normalization, checked length
  limits, and character-safe ellipsis truncation.
- HTML body previews with image alternative text, list numbering, scalar-safe
  truncation, and constant-time open-element tracking.
- MIME-part previews capped at 16 KiB of transfer-decoded input, with
  charset/BOM recovery and automatic plain/HTML preview dispatch.
- HTML-to-HTML rewriting with original-tag callbacks, fragment/comment
  filtering, noscript modes, implicit foreign closures, and UTF-8 I/O.
- HTML character-data and quoted attribute encoding with decimal Unicode
  references and MimeKit-compatible control-character handling.
- Enriched-text HTML rendering with character styles, paragraph formatting,
  validated parameters, malformed-command recovery, and explicit resource limits.
- HTML character-reference decoding with the complete named-entity table,
  longest-name matching, numeric-reference recovery, attribute-context rules,
  a bounded incremental decoder, and opt-in, per-violation-capped diagnostics.
- HTML-to-text conversion with paragraphs, headings, lists, quotations,
  preformatted text, optional link/image text, bounded UTF-8 input/output,
  and synchronous reader/writer adapters.
- Streaming byte-oriented RTF tokenization with bounded owned tokens,
  case-sensitive keyword classifications, binary-payload skipping, source
  positions, optionally capped lexical recovery callbacks, and a borrowed parser
  for decoded hyperlink-field instructions.
- Streaming RTF header/group-structure validation with optional per-violation
  limits on the combined lexical and structural report, without changing tokens.
- RTF-to-text conversion with bounded group/font state, code-page and Unicode
  decoding, hidden-text/destination suppression, tables, attachment-placeholder
  callbacks, and combined capped diagnostics.
- RTF-to-HTML rendering with character styles, paragraph alignment, colors,
  tables, bounded hyperlink fields, text or trusted-HTML decorations, and
  generated-tag callbacks with optional content suppression.
- Separate raw encapsulated-HTML extraction with bounded RTF interpreter state,
  document/font code-page handling, and optional capped diagnostics.
- Delivery-status parts with bounded raw header-group parsing, encoded-remainder
  compatibility, and atomic edits that preserve outer metadata.
- Disposition-notification parts with bounded raw field access, global-subtype
  support, and explicit atomic replacement of owned content.
- Feedback-report parts with bounded raw field access, duplicate-field
  preservation, and atomic replacement of owned content.
- Multipart reports with checked report-type access, atomic parameter editing,
  and ordinary multipart ownership and serialization.
- Related multipart containers with shared root-document lookup and atomic
  replacement of roots and their type/start parameters.
- Nested MIME entity and multipart ownership models with checked boundary
  edits, raw preamble/epilogue storage, and iterative leaf traversal.
- Synchronous MIME tree writing with iterative traversal, checked ancestor
  boundaries, and delimiter framing that preserves binary body suffixes.
- Explicit transport encoding preparation across MIME trees and attached messages,
  with iterative traversal and conditional message MIME-Version insertion.
- Buffered loading of complete MIME entity trees into owned body streams,
  with raw headers, stored transfer encodings, and input/header/depth limits.
- Synchronous `MimeReader` events with borrowed content and original byte/line
  positions, bounded standalone header reading, and an owned `MimeParser`
  shared by the existing load APIs.
- Message ownership, buffered loading, and synchronous writing, with separate
  message/body headers, optional bodies, and no generated metadata.
- Headers-only message serialization, including root-body headers and the
  separator without reading content or traversing descendants.
- Typed Date, Resent-Date, and MIME-Version accessors with explicit absence,
  parse errors, and canonical setters backed by current raw headers.
- Sender and Resent-Sender mailbox accessors with configurable prefix parsing,
  checked address formatting, and atomic replacement.
- From, Reply-To, To, Cc, Bcc, and Resent-* address-list accessors with
  MimeKit-style message-header recovery, diagnostics, and atomic setters.
- Duplicate-preserving recipient collection with resent-field selection,
  flattened groups, and header-indexed parsing diagnostics.
- Subject accessors with tolerant decoding and checked, folded unstructured
  header authoring through reusable `Header::from_unstructured` factories.
- Typed importance and priority headers with independent raw-header access,
  checked numeric-prefix parsing, and canonical setters that preserve no-ops.
- Structured attached-message ownership and iterative writing, with nested
  message traversal and boundary checks across message wrappers.
- Full-tree MIME iteration with part paths, ancestry, subtree skipping, and
  direct entity lookup without scanning unrelated siblings.
- Returned-header containers for text/rfc822-headers and message/global-headers,
  sharing structured-message ownership, loading, and serialization.
- Partial-message fragments with checked ID/number/total metadata, incremental
  decoded-stream reassembly, parser limits, and original-message header merging.
- Seekable bounded stream views with range-limited reads/writes and relative
  positions, allowing MIME content to reference source ranges without copying.
- Seekable chained streams with cross-child reads/writes and combined-offset
  seeking, without combining child data into a new buffer.
- Block-backed memory streams with reading, writing, seeking, zero-initialized
  resizing, and contiguous copying without relocating existing data on growth.
- A seekable measuring sink for serialized/encoded output sizes without storing
  the output bytes, with logical resizing and checked byte counts.
- I/O-independent incremental raw header parsing, preserving field spelling,
  folded values, malformed records, and the exact header/body boundary.
- Standalone header parsing with terminal-newline canonicalization and
  decoded display values with unfolding and charset recovery diagnostics.
- Typed raw-value parsing and checked address/content-header record creation,
  composing with collection replacement without modifying unrelated bytes.
- Ordered raw-header collections with duplicate-preserving storage,
  case-insensitive lookup, and explicit positional or field-based edits.
- Buffered header-list loading without losing body read-ahead, plus verbatim
  and newline-normalizing synchronous writing with malformed-line preservation.
- All 131 known MimeKit header identifiers, with case-insensitive lookup,
  canonical names, and cached identifiers on parsed header records.
- Transfer-encoding value parsing and canonical tokens, plus MIME version
  parsing with comments, checked numeric components, and byte-offset errors.
- Message-Id and Content-Id value parsing, with complete-value and prefix APIs,
  UTF-8 validation, domain literals, and MimeKit's compatibility workarounds.
- Lazy References/In-Reply-To enumeration with explicit errors and recovery.
- Message-ID and reply-threading accessors with assignment-validated, atomic
  header replacement and reference-chain folding.
- Separate message-ID assignment validation, with folding normalization,
  NUL/line-break checks, and original-input error offsets.
- Ordered domain routes with complete/prefix parsing, comments, UTF-8 atoms,
  domain literals, and obsolete comma handling, sharing message-ID domain rules.
- International-domain ASCII/Unicode conversion with explicit IDNA and
  MIME-safety errors, plus validated ASCII route formatting.
- Address-specification parsing with quoted local parts, compatibility modes,
  explicit Latin-1 recovery, component access, and domain-only IDNA conversion.
- Single-mailbox parsing with decoded display names, source routes, comment
  names, angle-bracket recovery, and complete/prefix APIs.
- Group/address-list parsing with bounded nesting, editable ordered lists,
  depth-first mailbox traversal, and explicit malformed-member errors.
- Mailbox/group construction and editing with typed addresses, trimmed Unicode
  names, mutable routes/members, and explicit decoding-diagnostic lifetimes.
- Unencoded address display formatting with quoted mailbox names, preserved
  source routes/group boundaries, and iterative nested-group formatting.
- Address equality and compatible hashing using exact names/address text and
  ordered groups/lists, without including routes or decoding diagnostics.
- Checked ASCII address-value encoding with RFC 2047 names, IDNA domains/routes,
  iterative nested groups, and explicit errors identifying failed members.
- Complete address headers with checked field names, CRLF/LF endings,
  grammar-aware folding, enforced physical-line limits, and opt-in UTF-8.
- Quoted-string utilities with append-only output, tolerant unquoting, optional
  tab conversion, and byte-preserving operation without charset decoding.
- Date-header parsing and canonical formatting, with legacy format recovery,
  checked calendar components, numeric offsets, and Unix timestamps.
- MIME charset lookup and whole-buffer text conversion using `encoding_rs`,
  with true ASCII/Latin-1 semantics and explicit errors or reported replacement.
- Incremental charset conversion with split-character/shift-state preservation,
  bounded intermediate buffers, and separate source/target replacement flags.
- Cloneable codec state, explicit reset, and a safe public API.

```rust
use mimecraft::encodings::{Base64Decoder, Base64Encoder};

let mut encoded = Vec::new();
let mut encoder = Base64Encoder::new();
encoder.encode(b"hello ", &mut encoded);
encoder.encode(b"world", &mut encoded);
encoder.finish(&mut encoded);
assert_eq!(encoded, b"aGVsbG8gd29ybGQ=\r\n");

let mut decoded = Vec::new();
let mut decoder = Base64Decoder::new();
for chunk in encoded.chunks(3) {
    decoder.decode(chunk, &mut decoded);
}
assert_eq!(decoded, b"hello world");
```

Codecs append to caller-owned byte buffers. Callers can consume and clear those
buffers between chunks without losing codec state. Incremental body encoding
requires an explicit `finish` to write any pending bytes and end-of-input output.
Stateless header payload encoders write complete results without `finish`.

Wrapping body encoders default to **CRLF** and can instead emit **LF**, matching the
updated MimeKit encoders. Line-length limits exclude newline bytes.
Base64 encodes input bytes unchanged, including any input newlines. Its decoder
is deliberately permissive, not a validation API: non-alphabet bytes (including
CR and LF) are ignored, padding does not end the stream, and incomplete final
quartets are not decoded.

For Base64 and quoted-printable, use `with_new_line_format` for the default width,
or `with_options` to select both:

```rust
use mimecraft::encodings::{Base64Encoder, NewLineFormat, QuotedPrintableEncoder};

let base64 = Base64Encoder::with_new_line_format(NewLineFormat::Unix).unwrap();
let quoted_printable = QuotedPrintableEncoder::with_options(72, NewLineFormat::Unix).unwrap();
assert_eq!(base64.new_line_format(), NewLineFormat::Unix);
assert_eq!(quoted_printable.line_length(), 72);
```

`NewLineFormat` mirrors MimeKit's `Dos` (CRLF), `Unix` (LF), and `Mixed` variants.
Both of these encoders reject `Mixed`: `with_new_line_format` and `with_options` return
`Result` with an `EncoderOptionsError`. `with_options` checks the format before
the line length, matching MimeKit. Existing `new`, `default`, and
`with_line_length` constructors still use CRLF and keep their existing signatures.
Both the line length and new-line format are preserved by `Clone`, `finish`,
and `reset`.

### Common codec interfaces and pass-through

`mimecraft::ContentEncoding` mirrors MimeKit's `Unspecified`, `SevenBit`, `EightBit`,
`Binary`, `Base64`, `QuotedPrintable`, and `UuEncode` variants. It is metadata,
not an encoder factory or content validator. Its `parse` method recognizes
Content-Transfer-Encoding values; it does not parse an entire header line.

`PassThroughEncoder::new(encoding)` and `PassThroughDecoder::new(encoding)`
append input bytes unchanged. Their `default()` selects `ContentEncoding::Unspecified`.
The configured encoding is only a label: even `SevenBit` does not reject
non-ASCII bytes, and selecting `Base64` does not perform Base64 encoding.
Neither codec normalizes line endings; the encoder's `finish` emits nothing.
Cloning and resetting preserve the label.

`encodings::MimeEncoder` exposes `encoding`, `encode`, `finish`, and `reset`;
`encodings::MimeDecoder` exposes `encoding`, `decode`, and `reset`. The traits
delegate to the existing implementations, keeping their streaming state,
configuration, SIMD dispatch, and permissive decoding behavior.

```rust
use mimecraft::{
    ContentEncoding,
    encodings::{Base64Encoder, MimeEncoder, PassThroughEncoder},
};

fn encode_chunks(encoder: &mut dyn MimeEncoder, chunks: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    for chunk in chunks {
        encoder.encode(chunk, &mut output);
    }
    encoder.finish(&mut output);
    output
}

let mut base64 = Base64Encoder::new();
assert_eq!(encode_chunks(&mut base64, &[b"C", b"at"]), b"Q2F0\r\n");
assert_eq!(base64.encoding(), ContentEncoding::Base64);

let mut raw = PassThroughEncoder::new(ContentEncoding::Binary);
assert_eq!(encode_chunks(&mut raw, &[b"a\n", b"\xff"]), b"a\n\xff");
```

Both traits support generic code and trait objects. Concrete codecs retain their
existing inherent methods, so existing callers need not import the traits.
Boxed codecs also implement the traits, allowing generic adapters to own
`Box<dyn MimeEncoder>` or `Box<dyn MimeDecoder>`.
All incremental codecs implement the appropriate trait, including `Rfc2231Decoder`.
As in MimeKit, yEnc codecs and `Rfc2231Decoder` report `ContentEncoding::Unspecified`;
there are no dedicated enum variants for those formats.

Output buffers grow as needed, replacing MimeKit's output-size estimates.
Concrete codecs implement standard Rust `Clone`; generic code that needs
snapshots can require `MimeEncoder + Clone` or `MimeDecoder + Clone`.
`Clone` is deliberately not a supertrait, allowing `dyn` dispatch.
Stateless header payload encoders are outside the incremental encoder trait.
There is no decoder `finish`, and encoder `finish` is not an I/O flush or a
uniform reset: UU emits a terminator on every call, yEnc retains its checksum,
and pass-through emits nothing. Use `reset` before starting an independent body.

### Streaming filters

`mimecraft::filters` ports the first part of MimeKit's `IO.Filters` layer.
`MimeFilter` exposes `filter(input, output)`, `finish(output)`, and `reset()`.
Like the codecs, filters append to caller-owned buffers that may be drained or
cleared between calls. `finish` corresponds to MimeKit's filter `Flush`, but
without a final input argument; filter any remaining input first. It is an
end-of-input operation, not an I/O flush, and is not called automatically on drop.

`Dos2UnixFilter` converts CRLF to LF without discarding lone CR bytes. It holds a
trailing CR across input chunks, emitting it on `finish` if no LF follows.
`Unix2DosFilter` expands bare LF to CRLF without doubling existing CRLF, even
when a pair spans chunks. Other bytes pass through unchanged.

Both default to leaving the final newline as-is. Use
`with_ensure_new_line(true)` to ensure the appropriate ending, including for an
empty stream. A final lone CR becomes LF in `Dos2UnixFilter`; `Unix2DosFilter`
completes it with LF. Repeated finishes without new input do not add more
newlines. Cloning preserves pending state; resetting discards it but retains
the final-newline option.

```rust
use mimecraft::filters::Dos2UnixFilter;

let mut filter = Dos2UnixFilter::with_ensure_new_line(true);
let mut output = Vec::new();
filter.filter(b"one\r", &mut output);
filter.filter(b"\ntwo\r", &mut output);
filter.finish(&mut output);
assert_eq!(output, b"one\ntwo\n");
```

`MboxFromFilter` changes exact, case-sensitive `From ` at line starts to
`>From `. `ArmoredFromFilter` instead writes `=46rom `, encoding the initial
`F` as quoted-printable. Both recognize line starts only at the beginning of
the stream or after LF; bare CR does not start another line. Existing `>From `
lines are unchanged, rather than recursively escaped. Neither filter changes
line endings. Incomplete final prefixes such as `From` are emitted literally
on `finish`; cloning preserves pending prefixes, and resetting discards them.

Mbox escaping is lossy because original and inserted `>` prefixes cannot be
distinguished. Armoring is not a complete quoted-printable encoder: apply it
after body quoted-printable encoding and treat the result as quoted-printable.
It does not wrap lines, and each replacement adds two bytes. Reserve those
columns in the preceding encoder's width, for example by choosing 74 instead
of 76.

`TrailingWhitespaceFilter` removes only ASCII spaces and tabs immediately
before CR, LF, or end of input. It preserves newline bytes, indentation,
internal blanks, and other whitespace such as vertical tab and form feed.
Pending spaces/tabs are run-length packed, with up to 255 equal bytes per
16-bit entry. Empty filter calls retain pending blanks; `finish` and `reset`
discard them, while cloning preserves them.

`BestEncodingFilter` passes bytes through unchanged while tracking NUL/high-bit
bytes, line lengths, and LF-terminated lines beginning with `From `. It chooses
an encoding using MimeKit's heuristic; it does not apply that encoding or
measure every encoder's actual output size. The root `EncodingConstraint`
enum offers `SevenBit`, `EightBit`, and `None` (unrestricted, the enum default).

`best_encoding(constraint)` uses a maximum line length of 78;
`best_encoding_with_line_length(constraint, limit)` accepts 60..=998 without
capping the limit at 76 and returns `encodings::InvalidLineLength` on invalid
input. CR immediately before LF is excluded from line width, even across chunks.
The final unterminated line counts toward width, but its `From ` marker is not
recorded until LF arrives, matching MimeKit. `finish` emits nothing and retains
all statistics; queries do not reset state, cloning preserves it, and `reset`
clears it.

```rust
use mimecraft::{ContentEncoding, EncodingConstraint, filters::BestEncodingFilter};

let mut filter = BestEncodingFilter::new();
let mut output = Vec::new();
filter.filter(b"hello\0world", &mut output);
filter.finish(&mut output);
assert_eq!(output, b"hello\0world");
assert_eq!(filter.best_encoding(EncodingConstraint::SevenBit), ContentEncoding::Base64);
assert_eq!(filter.best_encoding(EncodingConstraint::None), ContentEncoding::Binary);
```

The analyzer preserves MimeKit's threshold distinction: seven-bit transport
selects Base64 when a nonzero high-bit count is **at least** the truncated
17-percent threshold; unrestricted transport uses **greater than** when line
length or a `From ` line requires encoding. Newline bytes count toward the
total. Counters are widened to 64 bits and the threshold uses exact integer
arithmetic, avoiding the source's signed 32-bit overflow. Exceeding `u64::MAX`
accumulated bytes explicitly panics rather than silently wrapping.

`EncoderFilter::new(encoder)` and `DecoderFilter::new(decoder)` adapt any
incremental codec, including yEnc and parameter percent decoding. They own the
codec without mandatory boxing; boxed codecs work too. `encoder()`/`decoder()`,
their `_mut` variants, and `into_inner()` expose or recover the codec without
resetting it. `encoding()` returns its metadata. `EncoderFilter::has_output()`
tracks output emitted through the filter since construction or reset, excluding
bytes already in the output buffer.

Adapters retain codec-specific finish behavior: UU always emits a terminator,
yEnc retains its checksum, and decoder finish passes empty input through the
decoder without flushing or discarding incomplete groups. Cloning an adapter
copies its state when the underlying codec implements `Clone`.
`PassThroughFilter` simply copies bytes; its finish and reset do nothing.

`AnonymizeFilter` replaces every byte except SPACE, TAB, CR, and LF with ASCII
`x`, matching MimeKit's byte-whitespace rules. Vertical tabs, form feeds, NUL,
and all non-ASCII bytes are masked. Multibyte characters produce one `x` per
byte, including Unicode whitespace. It appends output without changing input
or existing output; finish and reset do nothing.

```rust
use mimecraft::filters::AnonymizeFilter;

let mut output = Vec::new();
AnonymizeFilter::new().filter(b"Hello\tworld!\r\n", &mut output);
assert_eq!(output, b"xxxxx\txxxxxx\r\n");
```

This is not full-message anonymization: byte counts and whitespace layout
remain visible, and headers or other content outside the supplied bytes are
untouched. It performs no transfer decoding, charset conversion, MIME parsing,
or secure erasure. Decode content before masking when decoded layout is wanted,
then re-encode explicitly if needed. `FilteredReader` and `FilteredWriter`
provide the existing streaming adapters. Use the `anonymize` tree writers for
structured message and entity masking; output still needs review before sharing.

`encoder_filter(encoding)` and `decoder_filter(encoding)` return
`Box<dyn MimeFilter>`. Base64, quoted-printable, and UU select their codecs;
`Unspecified`, `SevenBit`, `EightBit`, and `Binary` pass through without validation
or newline conversion. The UU decoder factory scans for a `begin ` header;
use `DecoderFilter::new(UuDecoder::for_payload())` for bare encoder output.
These factories take `ContentEncoding`, not header strings.

`encoder_filter_with_options(encoding, line_length, new_line_format)` defaults
neither argument. It rejects `Mixed` first for every encoding. Only Base64 and
quoted-printable validate the requested width, accepting 60..=998 and applying
their usual caps/rounding; UU and pass-through ignore width, matching MimeKit.
The default factory requests width 78 and CRLF (effective width 76 for Base64/QP).

The filters themselves operate on buffers. For manual composition, feed each
filter's final output through all downstream filters before finishing those
filters; otherwise buffered bytes or an ensured final newline can be lost.
Reset before an independent stream. For read-side I/O, nested `FilteredReader`
adapters handle the finalization order automatically.

### Header and part-content masking

`anonymize::write_bytes(input, output)` masks supplied raw bytes and converts
newlines to CRLF, without forcing a final newline. Its `_with_options` variant
takes `NewLineFormat` and an `ensure_new_line` boolean after the destination.
It ports MimeKit's preamble/epilogue helper: every byte except SPACE, TAB, CR,
and LF becomes `x`, then newlines are converted. No charset/transfer decoding,
header interpretation, or automatic MIME-tree traversal occurs.

For nonempty input, ensuring a newline adds one only if needed. A trailing
bare CR becomes CRLF for DOS output or LF for Unix output; without ensuring,
bare CR is retained. Empty input produces no bytes **and does not flush**, even
when ensuring a newline, unlike using the newline filters directly. Each call
starts fresh state, so supply a complete region instead of splitting CRLF
across calls. Buffers are bounded and input remains unchanged. `Mixed` is
rejected before I/O, including for empty input. Nonempty input flushes the
destination but does not close or finalize caller-owned filters; I/O errors
propagate with possible partial output. Retrying starts the region over.

`anonymize::address_header_value(&[u8])` and
`anonymize::unstructured_header_value(&[u8])` port MimeKit's raw-value masking
helpers. Each returns a new byte buffer exactly as long as the supplied value,
without modifying input, decoding words/charsets, repairing syntax, or
normalizing line endings. Pass `Header::raw_value()`, not a complete record.
These are whole-value operations; splitting a value across calls resets state.

Both preserve SPACE, TAB, CR, LF, and selected `=`/`?` delimiters. Potential
encoded-word charset and encoding labels pass through verbatim, without
validation. Payload bytes are masked with `x`, except whitespace and `?`;
`=` within a payload is masked unless it closes `?=`. Malformed or incomplete
sequences follow the source's state machine, not RFC 2047 parsing rules.

Address masking additionally retains `()<>[]:;@,.`, quotes, and backslashes.
An odd backslash run causes the following byte to pass through verbatim.
Inside unescaped quotes, CR/LF survive but spaces, tabs, and other unescaped
text are masked. Quotes and escapes pause the encoded-word state rather than
resetting it. Unstructured masking does not interpret quoting or escaping.

```rust
use mimecraft::{Header, anonymize};

let header = Header::parse(b"To: Alice <alice@example.com>\r\n")?;
assert_eq!(
    anonymize::address_header_value(header.raw_value()),
    b" xxxxx <xxxxx@xxxxxxx.xxx>\r\n",
);
# Ok::<(), mimecraft::HeaderParseError>(())
```

`anonymize::content_type_value(&[u8])` and
`anonymize::content_disposition_value(&[u8])` mask raw parameter values with the
same length-preserving, whole-value contract. Everything through the first
semicolon is copied without validation, as are parameter names and continuation
suffixes. Both helpers preserve values whose base names are `boundary`,
`charset`, `delsp`, or `format`, matched ASCII-case-insensitively before the
first `*`. Names are not trimmed or decoded; continuation suffixes are not
validated. Other values, including filenames, sizes, and dates, become `x`.

Whitespace outside quotes survives. Inside quotes, CR/LF, quotes, and
backslashes survive, but spaces, tabs, and other bytes of masked values do not.
Unlike address masking, escaping an ordinary byte does not preserve it.
The raw state machine does not decode RFC 2231 or encoded words, or repair
missing semicolons and unclosed quotes. An incomplete starred parameter ending
at `;` retains its accumulated base name for the next parameter, matching
MimeKit's recovery behavior; this can affect which later values are preserved.

```rust
use mimecraft::anonymize::content_type_value;

assert_eq!(
    content_type_value(b"text/plain; charset=utf-8; name=report.txt"),
    b"text/plain; charset=utf-8; name=xxxxxxxxxx",
);
```

`anonymize::received_header_value(&[u8])` preserves MIME whitespace, `<>[]:@,.`,
and semicolons outside comments. It retains `from`, `by`, `via`, `with`, `id`,
and `for` case-insensitively at token boundaries, keeping their original spelling.
Keywords require following SPACE, TAB, CR, or LF; an exact word at EOF is masked.
Comments delimit tokens, but other punctuation does not introduce keywords.

Nested comments retain parentheses, backslashes, whitespace, and the same
punctuation, but mask keyword text. An odd backslash run masks the following
byte, even whitespace or a parenthesis. Unclosed comments extend through EOF;
unmatched closing parentheses outside comments are masked.

After an outside-comment semicolon, the helper skips MIME whitespace and tries
`DateTimeOffset::parse` on the remainder. If accepted, it preserves the entire
suffix verbatim, including any comments, ignored tokens, or unrelated trailing
data. Rejected suffixes resume masking. This reuses the tolerant date parser,
not strict date validation, and never reformats the date. Keyword and date
lookahead mean a masked prefix can differ from that prefix of a complete result.

`anonymize::header(&Header)` applies MimeKit's default field dispatch to a complete
stored record. It preserves the original field spelling, blanks before the colon,
and colon, then uses the relevant value helper. Standard From/Sender/To/Cc/Bcc/
Reply-To fields, their Resent counterparts, and Disposition-Notification-To use
address masking. Content-Id, Message-Id, Original-Message-Id, Resent-Message-Id,
References, and In-Reply-To also use address masking. Received, Content-Type,
and Content-Disposition use their respective helpers.

Date, MIME-Version, and Content-Transfer-Encoding pass through verbatim,
even with invalid values. All other fields use unstructured masking, including
authentication/signature fields, unknown or empty names, and unlisted
address/date-like fields such as Return-Path and Resent-Date. No value validation,
signature verification, or inferred field grammar is performed.

Malformed records have no usable field/value boundary, so their entire raw
contents are processed with `AnonymizeFilter`, preserving only MIME whitespace.
A valid empty field name still retains its colon. The result is a new byte
buffer exactly as long as `Header::raw()`, not a reparsed or modified header.
No newline or body separator is added, and line endings are not normalized;
Received-comment escape rules can still mask individual newline bytes.

```rust
use mimecraft::{Header, anonymize};

let record = Header::parse(b"tO \t: Alice <alice@example.com>\n")?;
assert_eq!(
    anonymize::header(&record),
    b"tO \t: xxxxx <xxxxx@xxxxxxx.xxx>\n",
);
# Ok::<(), mimecraft::HeaderParseError>(())
```

`anonymize::header_with_options(&Header, &anonymize::HeaderOptions)` additionally
preserves fields listed in `preserve_headers: Vec<String>`. Matching uses the
complete field name, ignoring ASCII case. Entries are not trimmed, validated,
or canonicalized; wildcards, prefixes, and Unicode case folding are not used.
Order and duplicates do not affect matching. An empty name matches a valid
empty field, but malformed records are still byte-masked.

Preservation bypasses the value helper and copies the entire record, including
invalid or sensitive values. Defaults preserve no additional fields; Date,
MIME-Version, and Content-Transfer-Encoding still pass through regardless.
The list is checked afresh for every call without mutating it or the record.
Options do not affect raw-value helpers or automatically traverse MIME trees or
status-report bodies; callers explicitly select records.

```rust
use mimecraft::{Header, anonymize::{self, HeaderOptions}};

let record = Header::parse(b"Subject: keep this exact value\n")?;
let options = HeaderOptions {
    preserve_headers: vec!["sUbJeCt".into()],
};
assert_eq!(anonymize::header_with_options(&record, &options), record.raw());
# Ok::<(), mimecraft::HeaderParseError>(())
```

`anonymize::write_headers(headers, output)` masks an iterable of borrowed records
with default preservation and CRLF output. `write_headers_with_options(headers,
output, &HeaderOptions, NewLineFormat)` adds explicit preservation and CRLF/LF
selection. Both accept a borrowed `HeaderList`, slice, or iterator, retaining
order and duplicates. Masking runs on whole records before newline conversion,
including for preserved fields. Only one masked record is buffered at a time.

Neither missing record terminators nor a body separator are inserted; bare CR
bytes survive unless followed by LF. Newline state spans the supplied records,
while masking state starts fresh for each. Like MimeKit's anonymizer, malformed
records also undergo newline conversion, without `HeaderList`'s special
round-trip protection for CR-only invalid lines. Repeated LF conversion can
eventually turn such a line into a blank separator.

Writers finalize the newline filter and flush, but do not close the destination.
`Mixed` is rejected before iteration or I/O. Write and flush errors propagate,
possibly after partial output; retrying with a fresh iterator starts over.
Writing into another filter writer does not finalize that destination's filter.
No headers, options, or bodies are mutated or automatically authored.

`anonymize::write_message_headers(&message, output)` interleaves masked message
and root-body headers in their loaded order, then writes one CRLF separator
newline. Its `_with_options` variant takes `&HeaderOptions` and `NewLineFormat`
after the destination. Both lists share the configured masking rules and newline state.
Loading retains original stream byte offsets, including for duplicates and
malformed records. Leading newly prepended message headers come first; loaded
records then merge by original position, followed by remaining message headers
and newly added body headers, following MimeKit's merge rules. Authored
replacements have no stream offset and follow the new-header rules. Lists are
not sorted or repartitioned, so explicit in-list reordering is not undone.
Independently authored lists without
loaded ordering information use message-first order. No MIME-Version or other
metadata is generated. Bcc and Resent-Bcc are masked, not removed, and explicitly
preserving them exposes their values.

Only the root's header section is visited: no content reads/seeks, report
parsing, descendant headers, or body-metadata validation occurs. Attached-message
roots contribute only wrapper headers. The message is immutably borrowed and
its source type needs no I/O traits. Like the record masking writer, missing
terminators are not repaired, so the added separator newline need not form a
blank line in truncated input. Empty messages still write one newline.
`Mixed` fails before I/O; destination errors may leave partial output. The
destination is flushed after headers and again after the separator, not closed
or finalized if it is another filter.

`anonymize::write_part_content(&mut part, output)` masks an ordinary `MimePart`'s
serialized body without writing headers or their separator.
`write_part_content_with_options(&mut part, output, &MimePartWriteOptions)` uses
the ordinary content writer's encoding, width, newline, and final-newline settings.
Masking happens **after** serialization, not on decoded bytes before encoding.
For example, Base64 output `Q2F0\r\n` becomes `xxxx\r\n`, not Base64-encoded `xxx`.

Only SPACE, TAB, CR, and LF survive; every other serialized byte becomes `x`,
including UU wrapper keywords and filenames. Output preserves the length and
whitespace positions of ordinary serialized content, not necessarily of stored
or decoded bytes, and need not be a valid encoded payload. Binary serialization
does not normalize newlines or ensure a final newline, but is still masked.

Body writing uses bounded buffers and leaves headers, stored bytes, encoding
labels, and newline hints unchanged. It starts at source offset zero and rewinds
on success. Options and required metadata fail before I/O; read, seek, write,
and flush errors propagate without rollback. The destination is flushed, not
closed or finalized if it is another filter writer. Retrying starts over.
These helpers treat even report/message payloads as opaque ordinary-part content:
they do not apply `HeaderOptions` inside bodies, interpret status groups, or
traverse MIME trees.

`anonymize::write_part(&mut part, output)` combines field-aware masked headers,
one separator newline, and that ordinary-part content path. Its `_with_options`
variant takes `&HeaderOptions` and `&MimePartWriteOptions` after the destination.
Header preservation affects only outer fields, never the body. Both sections use
the selected newline format, except binary body output retains its whitespace
unchanged. Header terminators are not repaired; truncated records can run into
one another and the added newline need not produce a blank separator.

Invalid serialization options fail before I/O. As in MimeKit's anonymizer,
content metadata is checked after headers and the separator are emitted, unlike
the ordinary complete-part serializer's preflight. Unknown transfer encodings,
invalid required UU filenames, or source failures can therefore leave a masked
header prefix. I/O errors propagate without rollback; retrying starts the part
over. Reports and attached messages remain opaque on this path.

`anonymize::write_delivery_status_content(&mut status, output)` and
`write_disposition_notification_content(&mut notification, output)` instead parse
the typed report views and mask their decoded fields. Their `_with_options`
variants take `&HeaderOptions`, `NewLineFormat`, and the corresponding report
parse options after the destination. Defaults use CRLF and the existing parser
limits. Both global report subtypes are supported.

Delivery-status output has one newline between parsed groups and no trailing
blank separator. The parser skips empty groups and handles the one-time
Office365 encoded remainder. Disposition notifications emit only the first
field block; leading blank lines give empty output, and later data is not
emitted. Both decode the entire stored body before writing, even ignored MDN
trailing data, which counts toward limits. Missing record terminators are not
repaired. Empty output still flushes the destination.

These specialized writers emit decoded text, not transfer-reencoded content.
Retained outer or group encoding fields may no longer describe that output.
They neither write outer headers nor mutate stored bytes or metadata. Parsing
uses stored encoding, not a newly edited outer encoding header; successful
decoding rewinds the source. Parsing buffers decoded data and records within
the documented resource limits.

Invalid newlines, limits, and media metadata fail before I/O. Source and parse
errors occur before any destination I/O; unlike MimeKit's fallback, failures do
not silently switch to byte masking. Destination errors can leave partial
output; retrying starts over.

`anonymize::write_delivery_status(&mut status, output)` and
`write_disposition_notification(&mut notification, output)` add masked outer
headers and one separator newline to the corresponding decoded-field path.
Their `_with_options` variants accept the same header options, newline format,
and parse limits as the content-only helpers. Preservation applies to both
outer headers and parsed body fields. Empty reports still include the outer
header section; missing terminators are not repaired.

These complete-report writers retain MimeKit's headers-before-content order:
invalid newline formats fail before I/O, but invalid report metadata, zero or
exceeded parse limits, and source errors can leave the outer header prefix.
Report fields are only emitted after successful parsing, without an opaque
masking fallback. Retained transfer-encoding headers may disagree with the
decoded output; neither metadata nor stored content is rewritten.

`anonymize::write_entity(&mut entity, output)` masks a complete MIME tree;
`write_message(&mut message, output)` first writes the interleaved message/body
headers and then masks the root content without duplicating its headers.
`write_entity_content` omits only the root entity's headers and separator,
retaining complete children and embedded messages. Each has a `_with_options`
variant accepting `&anonymize::WriteOptions`.

`WriteOptions` combines `headers` preservation, `part` serialization settings,
and per-report `delivery_status` and `disposition_notification` parsing limits.
Report limits are not a cumulative tree budget. Invalid serialization options
or zero report limits fail before I/O, including limits for unused report types.

Tree masking shares the ordinary writer's iterative traversal, boundary and
ancestor-overlap checks, multipart/digest defaults, and delimiter-owned newlines.
Boundary markers remain verbatim; preambles and epilogues are byte-masked with
newline conversion. Nonempty preambles are terminated even for empty multiparts.
Root final-newline enforcement propagates through attached-message wrappers,
but not into multipart child bodies or specialized report fields.

The first Content-Type on a leaf selects decoded-field masking for both report
types and their global variants. Missing Content-Type selects ordinary masking;
malformed Content-Type is an explicit error. Other leaves use serialized-byte
masking, even opaque message/rfc822 payloads; no hidden MIME parsing occurs.
Headers are masked before content metadata is checked, so errors can leave a
header prefix, ancestor framing, or earlier siblings. Report parsing never
falls back to opaque masking. Sources rewind after successful copying; headers,
stored bytes, newline hints, and tree structure remain unchanged. Destination
errors propagate without rollback or finalizing failed content output.

These helpers are not guaranteed removal of sensitive information. Lengths,
syntax, escaped bytes, potential charset/encoding labels, leading content-header
text, parameter names, selected values, and accepted date suffixes remain visible,
even when malformed or containing unrelated private data. Complete-record masking
also retains valid field names, the three built-in pass-through fields, and
configured preserved fields in their entirety.
Output can retain controls, non-ASCII bytes, or invalid UTF-8;
it is not necessarily valid MIME or safe display text. Review it before sharing.
Use `AnonymizeFilter` when only MIME whitespace should survive masking.
Value reformatting remains deferred. Parsed delimiters retain their spelling
and padding, but the anonymizer normalizes their line endings.

### Incremental charset conversion

`filters::CharsetFilter` converts a byte stream between supported character
sets. Construct it with resolved `Charset` values, or use
`CharsetFilter::from_labels(source, target)` to resolve labels through the
existing charset lookup. Unsupported labels return `CharsetError`.

```rust
use mimecraft::{charsets::Charset, filters::CharsetFilter};

let mut filter = CharsetFilter::new(Charset::UTF8, Charset::LATIN1);
let mut output = Vec::new();
filter.filter(b"Ren\xc3", &mut output);
filter.filter(b"\xa9e", &mut output);
filter.finish(&mut output);
assert_eq!(output, b"Ren\xe9e");
assert!(!filter.had_errors());
```

Incomplete characters, UTF-16 surrogate pairs, and stateful encoding shifts
survive chunk boundaries. Decoded characters are passed through the target
encoder before processing further input; intermediate storage is bounded
independently of stream size. Existing output is preserved, and callers can
drain or clear it between calls.

This implements the infallible `MimeFilter` trait with an **explicit replacement
policy**, not strict validation: malformed source sequences become U+FFFD, and
characters unrepresentable in the target become `?`, one per Unicode scalar.
`had_decode_errors()` and `had_encode_errors()` report these separately;
`had_errors()` combines them. Flags remain set until `reset()`. A literal U+FFFD
in otherwise valid input is not a decode error. Replacement flags give no
counts or byte offsets. For strict complete-buffer conversion, use
`Charset::decode` and `Charset::encode` instead.

ASCII and Latin-1 retain their MIME meanings rather than becoming Windows-1252.
BOMs remain ordinary characters; the filter neither sniffs nor inserts them.
UTF-16 output uses the requested byte order. Legacy encoding uses the same
backend mappings as complete-buffer conversion, but replaces unmappable
characters with `?` rather than HTML numeric references. .NET best-fit/custom
fallbacks and unsupported code pages are not emulated.

`finish()` resolves even an empty final input, including an incomplete character
retained from an earlier chunk, and then emits any final target shift sequence.
It is idempotent. Empty `filter()` calls do not finalize. `reset()` discards
pending state and diagnostics while retaining the source/target charsets;
nonempty input after finishing without resetting explicitly panics.

Use `FilteredReader`/`FilteredWriter` for I/O and their `filter()` accessors to
inspect diagnostics. Decode transfer encoding before charset conversion when
reading; convert charset before transfer encoding when writing. Finish nested
writers from the outside inward so charset-finalization bytes reach the next
encoder. `source_charset()` and `target_charset()` expose resolved metadata.

### Filtered reading

`mimecraft::io::FilteredReader<R, F>` adapts a source implementing `std::io::Read`
and a `MimeFilter` to both `Read` and `BufRead`. It preserves a supplied filter's
existing state. Concrete, boxed, and mutably borrowed filters are supported.
The source can likewise be owned or borrowed.

```rust
use std::io::Read;
use mimecraft::{
    encodings::Base64Encoder,
    filters::{Dos2UnixFilter, EncoderFilter},
    io::FilteredReader,
};

let normalized = FilteredReader::new(
    &b"x\r"[..],
    Dos2UnixFilter::with_ensure_new_line(true),
);
let mut encoded = FilteredReader::new(normalized, EncoderFilter::new(Base64Encoder::new()));
let mut output = Vec::new();
encoded.read_to_end(&mut output).unwrap();
assert_eq!(output, b"eAo=\r\n"); // encodes x followed by the ensured LF
```

Source reads use 4096-byte chunks; transformed output is retained until consumed.
Filters that temporarily produce nothing do not cause premature EOF. The first
source read returning zero permanently ends input and finishes the filter once,
including on empty input. Final output is delivered before the adapter returns
EOF. Nested readers therefore pass upstream final bytes through downstream
filters before finishing them.

Empty destination reads do no I/O. Source errors, including `Interrupted` and
`WouldBlock`, propagate without resetting or finishing the filter; pending
state survives retries. Standard helpers such as `read_to_end` retain their
usual error handling. `fill_buf`, `consume`, and ordinary reads share one output
cursor; `buffer()` only inspects unread output without fetching more input.

`get_ref()` and `get_mut()` expose the source, while `filter()` exposes the
filter for inspection, such as transfer-encoding analysis. Both reflect
read-ahead input, which can extend beyond delivered output. Reading or seeking
the source directly bypasses filtering. `into_parts()` returns the source,
filter, and unread filtered bytes without finishing or resetting; preserve
those bytes before continuing a partially processed stream. It does not return
the EOF latch, so rewrapping components after EOF can finish the filter again.
Dropping an adapter performs no I/O or finalization; unread output is discarded
unless recovered first. Use a borrowed source/filter to keep them alive.

This ports the synchronous read side of MimeKit's `FilteredStream`, not its
combined read/write API. Writing uses the separate adapter below. Seeking,
asynchronous I/O, and a mutable list-based filter pipeline are not implemented yet.

### Filtered writing

`mimecraft::io::FilteredWriter<W, F>` wraps a `std::io::Write` sink and a
`MimeFilter`, preserving their existing state. Sinks and filters may be owned,
boxed, or mutably borrowed. Each nonempty `write` first drains earlier filtered
output, then accepts up to 4096 input bytes and buffers their transformed output.
It returns the number of **input bytes accepted**, not output bytes delivered.
New output is drained by the next nonempty write, `flush`, or `finish`.

```rust
use std::io::Write;
use mimecraft::{
    encodings::Base64Encoder,
    filters::EncoderFilter,
    io::FilteredWriter,
};

let mut writer = FilteredWriter::new(Vec::new(), EncoderFilter::new(Base64Encoder::new()));
writer.write_all(b"C").unwrap();
writer.flush().unwrap();
assert!(writer.get_ref().is_empty()); // the incomplete group is not padded
writer.write_all(b"at").unwrap();
writer.finish().unwrap();
assert_eq!(writer.get_ref(), b"Q2F0\r\n");
```

**Call `finish()` explicitly.** Unlike MimeKit's filter-finalizing stream
`Flush`, this adapter's `flush()` drains already produced output and flushes
the underlying sink without ending the content. Incomplete codec groups and
other pending filter state are retained. `finish()` invokes the filter's finish
once, delivers the remaining output, and flushes the sink. After finalization
begins, further nonempty writes return `BrokenPipe`, even if delivery failed.
Repeated `finish()` calls retry delivery/flush without repeating final bytes.
Empty writes are always no-ops, including after finishing.

Short writes advance only the delivered prefix. `Interrupted` errors while
draining are retried; other write errors and all sink-flush errors propagate.
A zero-byte write with output pending returns `WriteZero`. Unwritten output
survives errors. A failed `write` has not consumed any of that call's new input,
so that slice can be retried. An entire failed `write_all` operation must not
be blindly replayed: its earlier successful calls may already have accepted
input. Retry `flush` or `finish` after a delivery failure as appropriate.

`get_ref()`/`get_mut()` expose the sink, `filter()` exposes accepted-input state,
and `buffer()` exposes transformed bytes not yet accepted by the sink.
`into_parts()` returns the sink, filter, and unwritten bytes without doing I/O.
Deliver those bytes directly to the sink, not through the filter again, before
continuing. As with the reader, dismantling does not retain the finalization
latch. Dropping this adapter neither flushes nor finishes it; pending output is
discarded unless recovered first, and owned components are dropped normally.

For nested writers, data flows from the outermost filter inward. Finish the
outer writer successfully, then finish its inner writer via `get_mut()`, and
continue inward. Flushing the outer writer does **not** finalize inner filters.
For example, finish a newline-converting outer writer before its Base64-encoding
inner writer so the ensured final newline is included in the encoded content.

### MIME content streams

`MimeContent<R>` wraps a `Read + Seek` source and the transfer encoding of its
stored bytes. `new(source)` labels unencoded content as `ContentEncoding::Unspecified`;
`with_encoding(source, encoding)` labels already-encoded content. Neither
constructor performs I/O, validates the body, or encodes it.

```rust
use std::io::Cursor;
use mimecraft::{ContentEncoding, MimeContent};

let mut content = MimeContent::with_encoding(
    Cursor::new(b"Q2F0\r\n"),
    ContentEncoding::Base64,
);
let mut decoded = Vec::new();
assert_eq!(content.decode_to(&mut decoded).unwrap(), 3);
assert_eq!(decoded, b"Cat");

let mut stored = Vec::new();
assert_eq!(content.write_to(&mut stored).unwrap(), 6);
assert_eq!(stored, b"Q2F0\r\n");
assert_eq!(content.get_ref().position(), 0);
```

`write_to` copies stored bytes unchanged and does not flush the destination.
`decode_to` copies decoded bytes and flushes the destination afterward. Both
return the number of bytes written, excluding anything already in the destination,
and neither closes it. Flushing a destination `FilteredWriter` does not finalize
its encoder; call that writer's `finish` separately when its input is complete.

`open()` rewinds the source and returns a lazy decoded `Read`/`BufRead` view.
Decoding uses bounded input buffers rather than materializing the entire body.
The returned reader exclusively borrows the content, so overlapping opens or
copies cannot accidentally reposition the same stream. Dropping the reader
does not rewind; it discards any unread decoded output and pending codec state.
Opening again starts from zero with a fresh decoder.

All operations start at absolute source position zero, not the position at
construction. Successful copies rewind to zero again. Initial seek failures
precede copying; read/write failures leave source position and partial output
as-is. A final rewind can fail after all bytes have been written. `decode_to`
flushes only after that rewind succeeds, so a flush failure leaves the source
already rewound. Errors propagate as `io::Error`; repeating a failed copy starts
over and can duplicate output unless the destination is cleared or replaced.

Base64 and quoted-printable use the tolerant body decoders. UU expects a
`begin ` header, as in MimeKit's decoding filter factory. Other encodings pass
through unchanged, including non-ASCII bytes labelled seven-bit. Incomplete
encoded tails retain the existing decoders' permissive behavior. These methods
do not perform charset conversion, newline normalization, or strict validation.

The source can be owned or borrowed, for example `File`, `Cursor<Vec<u8>>`, or
`&mut Cursor<&[u8]>`. `get_ref` and `get_mut` expose it; `into_inner` returns it
without I/O or repositioning. Rust ownership replaces disposal checks, and
dropping content does not close a borrowed source. The optional `new_line_format`
hint can be set or cleared, including `Mixed`; it is metadata only and is neither
inferred nor used to transform bytes.

These operations copy body content, not complete entities or parsed attachments.
Cancellation-token and async operations remain deferred.

### Single-part MIME model

`MimePart<R>` owns a `HeaderList` and a seekable `MimeContent<R>`.
`new(content)` adds `Content-Type: application/octet-stream`;
`with_content_type(&value, content)` formats a checked Content-Type instead.
Neither adds a transfer-encoding header or inspects the body. `from_parts`
preserves supplied raw headers, including duplicates and malformed records,
without inserting defaults or inferring the stored content's encoding.

```rust
use std::io::Cursor;
use mimecraft::{ContentDisposition, ContentEncoding, ContentType, MimeContent, MimePart};

let mut part = MimePart::with_content_type(
    &ContentType::new("text", "plain").unwrap(),
    MimeContent::new(Cursor::new(b"hello")),
).unwrap();
let mut disposition = ContentDisposition::default();
disposition.set_file_name(Some("hello.txt"));
part.set_content_disposition(Some(&disposition)).unwrap();
part.set_content_transfer_encoding(ContentEncoding::Base64);

// This requests Base64 for transmission; it does not change the stored bytes.
assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
assert_eq!(part.content_transfer_encoding().unwrap(), ContentEncoding::Base64);
let mut decoded = Vec::new();
part.content_mut().decode_to(&mut decoded).unwrap();
assert_eq!(decoded, b"hello");
```

Raw headers remain the source of truth. Content-Type and Content-Disposition
getters parse the first matching raw value on every call, returning detached
snapshots. Missing fields return `None`; invalid first values return errors,
even when a later duplicate is valid. Options variants forward parameter
parsing settings. Editing a snapshot requires an explicit setter to update
the part; `headers_mut()` edits are immediately visible without cache repair.

Typed setters format before changing headers, retain the first matching slot,
remove later duplicates, and preserve unrelated records byte-for-byte.
Formatting failure leaves headers and content unchanged. Setting disposition
to `None` removes all occurrences without checking formatting options.
Transfer-encoding lookup follows MimeKit's decoded-header-value semantics:
missing means `Unspecified`, but empty or unknown values return explicit errors.
Setting `Unspecified` removes all transfer-encoding headers; other encodings write
one canonical token with CRLF termination.

`content_id()` is available on `MimePart`, `Multipart`, `MessagePart`, and
`MimeEntity`, returning `Result<Option<MessageId>, MessageIdParseError>`.
It parses the first raw Content-Id with the tolerant prefix grammar, without
RFC 2047 decoding: brackets are removed, compatibility forms are normalized,
case is preserved, and trailing text and later duplicates are ignored. Missing
is `Ok(None)`; malformed first values report errors with raw-value offsets
without preventing message loading. A container or wrapper returns only its
own ID, not an ID from a child or enclosed message.

`set_content_id(Some(value))` validates bare or bracketed input with
`MessageId::from_value`, writes a canonical bracketed CRLF-terminated header
at the first matching slot, and removes duplicates. Equal values are also
reformatted. Invalid assignments leave all headers unchanged; `None` removes
every occurrence, while empty text is invalid. Reading is deliberately more
tolerant than authoring, so even a parsed ID must pass assignment validation.
Neither operation needs source I/O traits. Text parts expose these methods
through `part()`/`part_mut()`. IDs are not generated or checked for uniqueness;
changing one does not update HTML references or parent `multipart/related`
selectors. Related-root lookup observes the current raw headers on each call.

`file_name()` returns `Result<Option<String>, ContentHeaderParseError>`, checking
the first Content-Disposition's `filename` parameter before the first
Content-Type's `name`. The decoded value is Unicode-trimmed. An empty filename
prevents fallback; an invalid disposition reports an error rather than falling
back. Content-Type is not parsed when a disposition filename is available.
`file_name_with_options()` accepts `ParameterParseOptions`; recovery diagnostics
remain available through the corresponding parsed header's parameter issues.
The returned name is untrusted metadata, **not a sanitized filesystem path**.

`set_file_name(Some(name))` updates both parameters atomically. It preserves
existing disposition tokens and other parameters, creating an `attachment`
disposition only when absent. It requires an existing valid Content-Type rather
than guessing a media type. The assignment is stored literally, without the
getter's trimming; Unicode, controls, and encoded-word lookalikes are encoded
by the checked header formatter. Empty text writes empty parameters.

`set_file_name(None)` removes both parameters while preserving existing headers
and other parameters; it does not create missing fields or clear the attachment
flag. Present headers are always reformatted, with duplicate replacement at
their first positions. `_with_options` accepts `HeaderFormatOptions`. Neither
header changes if parsing or formatting either replacement fails; errors are
reported through `FileNameEditError`. Removing a name when both headers are
absent is a no-op, even with invalid formatting options.

Filename accessors and the underlying content-header metadata methods require
no source I/O traits. Text parts expose them through `part()`/`part_mut()`.
The UU writer shares the same lookup precedence, but still rejects control
characters **before trimming** and uses `unknown` only when no name exists.

`content_description()` returns the first Content-Description as an owned
`Option<rfc2047::DecodedHeader>`, retaining tolerant decoding diagnostics.
Its `_with_options` variant accepts explicit decoding settings. Missing and
present-empty descriptions are distinct; decoded text is unfolded and trimmed,
not sanitized for display.

`set_content_description(Some(text))` uses the same checked unstructured-header
formatter as Subject. Its `_with_options` variant accepts
`UnstructuredHeaderFormatOptions`. Assignment trims and unfolds text, encodes
controls and literal encoded-word lookalikes, and replaces duplicates at the
first slot. `None` removes every occurrence without checking formatting options;
empty text leaves one empty header. Formatting errors leave all records intact.

`content_duration()` returns `Result<Option<i32>, InvalidContentDuration>` in
whole seconds. It follows MimeKit by decoding and unfolding the first header,
then accepting ASCII decimal digits through `i32::MAX`, including leading
zeros. The decoded value is Unicode-trimmed; trailing NULs, optionally preceded
by ASCII whitespace, are accepted as in the upstream integer parser. Signs,
comments, fractions, internal spaces, and overflow produce an accessor error,
not a message-loading failure or a fallback to later duplicate headers.
Charset diagnostics remain available through the raw header's decoder.

`set_content_duration(Some(seconds))` rejects negatives before editing and
writes a canonical CRLF-terminated value; `None` removes every occurrence.
Both metadata setters always canonicalize assigned values and collapse
duplicates, even when the decoded value is equal, rather than reproducing
MimeKit's cache-dependent no-op. Both accessors observe raw edits immediately
and need no source I/O trait bounds. Text parts expose them through
`part()`/`part_mut()`; no media inspection or duration inference is performed.

`content_md5()` reads the first Content-MD5 as detached decoded text with
charset-recovery diagnostics, using the same tolerant decoding, unfolding, and
Unicode trimming as Content-Description. `content_md5_with_options` accepts
`rfc2047::DecodeOptions`. Missing and present-empty headers remain distinct;
arbitrary or malformed checksum text is preserved rather than rejected.
Raw edits are observed immediately, and later duplicates do not override the
first. This does not Base64-decode a digest, validate its length, or verify it.

`set_content_md5(Some(value))` assigns caller-supplied metadata using checked
unstructured-header formatting; `_with_options` accepts
`UnstructuredHeaderFormatOptions`. Assignment trims and unfolds, safely encodes
controls and literal encoded-word lookalikes, and replaces duplicates at the
first slot with canonical `Content-Md5` spelling. Equal values are reformatted.
Empty text leaves an empty header; `None` removes all occurrences without
validating options. Formatting errors leave headers intact. No source I/O,
checksum computation, or cryptographic dependency is involved. Later body edits
do not recompute or remove this metadata. Text parts expose it through
`part()`/`part_mut()`.

The stored encoding belongs to `MimeContent`, independently of the requested
transfer-encoding header. Metadata edits do no I/O or transcoding, do not move
the source cursor, and do not validate body structure or transport suitability.
`content_mut()` provides body operations; `set_content()` returns the old body
without changing headers or either source's position. `into_parts()` extracts
both components. Sources may be owned or borrowed; use an empty stream for an
empty body.

`best_encoding(constraint)` recommends a transfer encoding using MimeKit's
heuristic and a 78-byte maximum content-line length. For `text/*` and
`message/*`, it incrementally analyzes decoded content with `BestEncodingFilter`.
The stored `MimeContent` encoding controls decoding; the requested
Content-Transfer-Encoding header is not consulted. Charset decoding and newline
conversion are not performed. Empty text/message content recommends `SevenBit`.

Other media types, including `application/rtf`, recommend `Binary` for
`EncodingConstraint::None` or `Base64` for either restricted constraint,
without reading or seeking content, even for empty bodies. This is a media-type
shortcut, not an exact comparison of every possible encoded output size.
`best_encoding_with_line_length(constraint, limit)` accepts `60..=998`, excluding
line terminators. Exact-width lines fit; unterminated final lines still count.
The limit is checked before metadata or I/O on every path.

`BestEncodingError` distinguishes invalid limits, missing or malformed first
Content-Type, and source I/O failures. A successful scan rewinds to zero rather
than restoring the initial position; the media-type shortcut leaves it untouched.
Read failures do not trigger a cleanup seek, and a final rewind failure is
reported even after the whole body was analyzed. Selection is never cached.
No headers, stored encoding, or newline hints change: explicitly call
`set_content_transfer_encoding` to apply a recommendation. Text parts expose
selection through `part_mut()`.

`prepare(constraint)` applies MimeKit's leaf-part transport encoding selection
using default `MimePartWriteOptions`. `prepare_with_options(constraint, &options)`
uses the supplied content-line limit (`60..=998`). Pass those same options to
subsequent serialization: unlike MimeKit, the part does not retain a private
encoder-width setting. Preparation does not transcode or replace stored bytes.

Requested Base64, quoted-printable, and UU encodings are retained without reading
Content-Type or content. Binary is likewise retained for an unrestricted
transport. Other cases use decoded-content analysis and the media-type shortcut
above. An absent transfer-encoding header stays absent if SevenBit suffices.
Equal encodings and safe shortcuts preserve raw headers, including duplicates;
changes write one canonical header at the first matching slot.

`PartPrepareError` distinguishes invalid write options, invalid first
Content-Transfer-Encoding, and underlying `BestEncodingError` failures. All write
options are validated before metadata or I/O, even for shortcuts. Failure leaves
headers unchanged; scans retain the analyzer's source positioning rules.
Preparation trusts already-safe requested encodings: it does not verify their
payloads, rewrap stored encoded content, or refold raw headers. It is not a full
transport-safety validator. Text parts expose leaf preparation through
`part_mut()`; the tree APIs described below also visit attached message bodies.

Complete entity loading is available through `MimeEntity::load`, and message
loading through `MimeMessage::load`, including transparent attached messages.
Encoded message-wrapper interpretation remains deferred.

### Single-part serialization

`MimePart::write_to(&mut output)` writes normalized headers, one blank
separator, and the body using CRLF and the first Content-Transfer-Encoding
header (or `Unspecified` when absent). It does not change headers or stored bytes.
Use `write_to_with_options` for explicit `MimePartWriteOptions`:

```rust
use std::io::Cursor;
use mimecraft::{ContentEncoding, MimeContent, MimePart, MimePartWriteOptions};
use mimecraft::encodings::NewLineFormat;

let mut part = MimePart::new(MimeContent::new(Cursor::new(b"hello")));
part.set_content_transfer_encoding(ContentEncoding::Base64);
let options = MimePartWriteOptions {
    new_line_format: NewLineFormat::Unix,
    ..Default::default()
};
let mut output = Vec::new();
part.write_to_with_options(&mut output, &options).unwrap();
assert!(output.ends_with(b"\n\naGVsbG8=\n"));
assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
```

Different stored/requested encodings trigger incremental decoding and encoding.
Matching encodings retain payload spelling rather than re-encoding; nonbinary
output receives newline conversion in either case. Binary output is never
newline-normalized, and Base64 encodes decoded bytes without prior newline
conversion. Existing newline filters and tolerant codecs define malformed-input
behavior. The optional stored newline hint does not enable signature-verification
pass-through.

`new_line_format` defaults to CRLF and accepts LF, not `Mixed`.
`max_line_length` defaults to 78 and must be 60..=998. It controls newly
encoded Base64/QP only, capped at 76; existing encoded bodies, UU's fixed width,
and raw header lines are not rewrapped. `ensure_new_line` defaults to false:
when enabled, nonbinary output ends with the selected newline, including authored
empty Base64/QP bodies. New encoders terminate nonempty output, and UU emits a
zero-length line. Parsed absent content stays absent rather than gaining encoder
output or an extra body newline.

New UU output includes `begin 0644 filename` and `end` lines. The filename uses
Content-Disposition, then Content-Type's `name`, then `unknown`, with surrounding
whitespace trimmed. Control characters are rejected rather than introducing
extra wrapper lines. Already UU-encoded content retains its existing wrapper.

Header normalization preserves field spelling, folding, duplicates, malformed
records, and value bytes using `HeaderList`'s rules. Unlike standalone list
writing, authored part output terminates records before writing the separator.
Loaded EOF-truncated records and absent separators are preserved unless edits
require a following record or body separator. No automatic MIME-Version
header, content-type fallback, duplicate removal, or transport validation occurs.

Options and required metadata are checked before source I/O or destination
writes/flushes. Invalid options return `io::ErrorKind::InvalidInput`; an unknown
transfer encoding or invalid required UU filename metadata returns `InvalidData`.
An unchanged unknown transfer encoding accepted by reader recovery can still
be serialized; changing that invalid header reinstates normal validation.
Unneeded content metadata is not parsed. I/O errors propagate and can leave
partial output. Successful body copies rewind the source before filter
finalization; subsequent write or flush failures may therefore leave it at zero.
The borrowed destination is flushed, never closed. Retrying restarts the entire
part, not the failed write; clear or replace the destination first. Neither the
source body nor the output is materialized in its entirety.

`write_content_to` and `write_content_to_with_options` serialize only the
body, omitting this part's headers and blank separator. This is **not raw
copying or decoded-content extraction**: the requested transfer encoding,
newline options, encoder widths, and UU wrapper lines still apply. Required
metadata is still validated before I/O; unrelated headers are not written.
The same source rewind, destination flush, and error/retry rules apply.

### Text parts

`TextPart<R>` composes a `MimePart<R>` with text-specific operations.
`TextPart::new(subtype, text)` creates a UTF-8 in-memory body and a
`text/subtype; charset=utf-8` header. `with_charset(subtype, text, charset)`
encodes with an explicit `Charset`, rejecting unmappable characters rather
than inserting replacements. Neither constructor selects a transfer encoding.

```rust
use mimecraft::{ContentEncoding, TextPart};
use mimecraft::charsets::Charset;

let mut text = TextPart::with_charset("plain", "Ren\u{e9}e", Charset::LATIN1).unwrap();
assert_eq!(text.text().unwrap(), "Ren\u{e9}e");
text.part_mut().set_content_transfer_encoding(ContentEncoding::Base64);
text.set_text("Now UTF-8: \u{1f408}").unwrap();
assert_eq!(text.part().content_type().unwrap().unwrap().charset(), Some("utf-8"));

let mut output = Vec::new();
text.part_mut().write_to(&mut output).unwrap();
```

`from_part(part)` validates the first Content-Type as `text/*` without source
I/O; it accepts owned or borrowed seekable content. Failed wrapping drops the
supplied part but leaves borrowed streams usable and unmoved. `part()` and
`part_mut()` expose metadata, content, and serialization; `into_part()` returns
the original model without copying. Raw Content-Type edits remain authoritative:
text operations and format predicates recheck them, reporting missing,
malformed, or non-text values rather than using a cache or later duplicate.

`with_format(format, text)` constructs UTF-8 text from a `TextFormat`;
`with_format_and_charset(format, text, charset)` uses an explicit charset.
Plain and Flowed use `text/plain`, Html uses `text/html`, Enriched uses
`text/enriched`, and RichText uses `text/rtf`. Flowed adds `format=flowed`, but
does not add `delsp`, wrap lines, stuff spaces, or convert the supplied text.
All format constructors share the existing strict charset and checked header
formatting path, without selecting a transfer encoding.

`format()` classifies current metadata without source I/O. `is_plain`,
`is_flowed`, `is_html`, `is_enriched`, and `is_rich_text` share the body selector's
case-insensitive matching. Flowed is also plain; `format=flowed` is
Unicode-trimmed and only applies to `text/plain`. The older `text/richtext`
counts as Enriched, not RTF. Charset labels, markup, and body bytes are not
validated. Unknown `text/*` subtypes classify as Plain, matching MimeKit's
fallback, but do not satisfy `is_plain()` or plain-body selection.

The existing text-only wrapper policy remains: missing, invalid, or non-text Content-Type
returns `TextPartError`, including `application/rtf`. Such content can remain
an ordinary `MimePart` and match RichText body selection instead. Format
classification does not broaden which bodies are eligible for selection.

`text()` transfer-decodes using the stored content encoding, then selects a
supported MIME charset, a UTF-8/UTF-16LE/UTF-16BE BOM, or an HTML meta charset,
in that order. Unsupported MIME declarations allow subsequent detection.
HTML scanning uses the first **1024 transfer-decoded bytes**, interpreted as
Latin-1 for tokenization. Explicit html/head starts are required; a body tag
stops scanning. Comments and raw-text elements cannot spoof metadata.
First valued duplicate attributes win. Direct `charset` overrides `content`;
the latter requires `http-equiv=Content-Type`, compared without trimming.
Unsupported meta labels allow later meta tags. HTML `x-user-defined` maps to
Windows-1252; UTF-16 meta labels mean UTF-8. MimeKit's end-head behavior is
preserved: `</head>` alone does not stop its scanner.

`detect_encoding()` returns `Result<Option<(Charset, TextEncodingConfidence)>,
TextPartError>`. MIME declarations and BOMs are Certain; HTML metadata is
Tentative; undetected input is `None`. Supported declarations need no source
I/O. Otherwise detection rewinds first and reads up to three decoded bytes,
then up to 1024 total for HTML when no BOM is found; it does not rewind afterward.
Transfer decoders may read ahead in the encoded source. Read/seek failures are
errors, not unsuccessful detection. All MIME content has a stream, so there is
no counterpart to MimeKit's null-content ASCII/Irrelevant result: an unlabelled
empty stream is undetected and automatic text decoding uses UTF-8.

Undetected input, BOM-selected UTF-8, and coerced HTML UTF-16 declarations use
strict UTF-8 with Latin-1 fallback. Other detected charsets use replacement
decoding, matching MimeKit's decoder selection. Replacements use U+FFFD uniformly,
not .NET's charset-dependent defaults. Incomplete trailing code units are
finalized rather than reproducing MimeKit's charset-filter tail truncation.
`text_and_charset()` also returns the charset actually used, including fallback.
`text_with_charset(charset)` remains strict, overriding metadata without editing
headers; malformed bytes are errors. Neither path normalizes newlines or strips
BOMs. Codec support remains that of `Charset`, not every .NET code page.
Whole-text reads allocate decoded bytes and the resulting string, reading the
body once even when fallback is needed. Successful copies rewind the source
before charset decoding; I/O failures follow `MimeContent`'s existing rules.

Detection and recovery cases from `UnitTests/TextPartTests.cs` are ported,
adapting absent content to the always-present stream model. Extra coverage
checks exact byte limits, transfer decoding, truncated tags, attribute
precedence/duplicates, tokenizer contexts, short/interrupted reads, errors,
strict overrides, and unchanged headers, bytes, and parsed framing.
Upstream contains no dedicated text-part charset-detection benchmarks.

Text setters are available for the default `TextPart<Cursor<Vec<u8>>>` storage,
not arbitrary file or borrowed-stream types. `set_text(text)` selects UTF-8;
`set_text_with_charset(text, charset)` uses the specified codec. Both encode and
format before committing the body and charset change. Other Content-Type
parameters survive, duplicates collapse at the first slot, and unrelated headers
remain untouched. In particular, the requested transfer encoding is preserved.
The new body stores unencoded bytes at position zero with no newline hint.
Failure leaves all old bytes, headers, and source position unchanged.

Automatic text-format conversion remains separate. The `text::TextPreviewer`
type provides bounded, best-effort previews as described below.

### Multipart ownership and MIME trees

`MimeEntity<R>` is `Part(MimePart<R>)`, `Multipart(Multipart<R>)`, or
`Message(MessagePart<R>)`.
Converting a `TextPart` into an entity retains the same leaf headers and body
without copying; extracting that leaf and using `TextPart::from_part` restores
text-specific APIs. All leaves in a tree share one source type, defaulting to
`Cursor<Vec<u8>>`; owned or borrowed seekable sources are supported.

```rust
use mimecraft::{MimeEntity, Multipart, TextPart};

let mut alternatives: Multipart = Multipart::with_boundary("alternative", "alt-boundary").unwrap();
alternatives.children_mut().push(TextPart::new("plain", "hello").unwrap().into());
alternatives.children_mut().push(TextPart::new("html", "<b>hello</b>").unwrap().into());

let mut mixed: Multipart = Multipart::with_boundary("mixed", "outer-boundary").unwrap();
mixed.set_preamble_text("This message has multiple parts.");
mixed.children_mut().push(alternatives.into());
let root: MimeEntity = mixed.into();
assert_eq!(root.leaf_parts().count(), 2);
```

`Multipart::new(subtype)` creates an empty container with one checked Content-Type
and a generated boundary. `Multipart::with_boundary(subtype, boundary)` accepts
an explicit boundary instead. Checked creation,
Content-Type replacement, and boundary edits require the RFC 2046 boundary
alphabet, 1..=70 bytes, and no trailing space. Input is not trimmed.
Boundary errors report the first invalid byte. Model edits do not scan sources
or check collisions: callers must choose values that do not collide with body
data or ancestor/descendant delimiters. Serialization checks ancestor overlap.

`Multipart::new(subtype)` uses MimeKit-style boundary generation;
`Multipart::generate_boundary()` returns a standalone string for
explicit constructors or `set_boundary()`, including specialized related/report
containers. Both use the existing `getrandom` dependency to obtain 16 OS-random
bytes, encoded as unwrapped, padded Base64 with the `=-` prefix. The resulting
boundary is exactly 26 ASCII bytes. `MultipartError::Random` preserves entropy
failures without a weak fallback or partial result. The generated constructor
validates subtype syntax before requesting entropy and creates only Content-Type.
Neither operation reads child sources or scans for collisions; uniqueness is
probabilistic. Explicit-boundary construction, getters, loading, and serialization
retain their existing behavior; missing boundaries are not silently repaired.

```rust
use mimecraft::Multipart;

let mixed: Multipart = Multipart::new("mixed").unwrap();
assert_eq!(mixed.boundary().unwrap().unwrap().len(), 26);
```

Headers remain authoritative. `content_type()` returns a detached parsed value,
including a non-multipart value introduced by raw editing. `boundary()` checks
multipart type and boundary syntax, returning `None` for an absent Content-Type
or boundary rather than inventing one. An invalid first header never falls back
to a valid duplicate. `set_boundary()` can repair a missing/invalid parameter
on an existing multipart type; `set_content_type()` can replace missing metadata.
Setters format before committing, retain other parameters, collapse duplicate
Content-Type records, and leave unrelated bytes and children untouched.

`children_mut()` exposes normal vector insertion, removal, replacement, and
reordering. Removed children retain their source ownership; clearing drops them.
Raw `set_preamble` and `set_epilogue` take byte vectors and return the previous
bytes, without decoding, folding, or newline conversion. These vectors exclude
delimiter lines; epilogue begins after the closing delimiter's line ending.
Taking `children_mut()` or replacing the raw preamble/epilogue discards this container's
parsed framing, even without subsequent vector edits; writing then generates
complete delimiters with a header/body separator.
`from_parts`/`into_parts` transfer all four components without validation,
default insertion, source I/O, or copying; private parsed-framing provenance is
not transferred. Empty vectors represent absent text.

`preamble_text()` and `epilogue_text()` decode strict UTF-8 without changing the
stored bytes; their `*_with_charset()` variants accept an explicit `Charset`.
Malformed input returns `CharsetError`, not replacement characters or guessed
encodings. Empty vectors decode to empty strings. BOMs and newlines survive,
including an epilogue's leading blank lines: its delimiter newline is already
stored separately.

`set_preamble_text()` and `set_epilogue_text()` encode UTF-8 and fold words with
MimeKit's soft 78-column target, measured in UTF-8 bytes rather than UTF-16 code
units. Long words are not split. LF and its preceding whitespace become the
selected newline; bare CR otherwise remains ordinary whitespace. Nonempty
final lines receive a newline. Output defaults to CRLF; the
`*_with_new_line_format()` variants accept DOS or Unix and reject Mixed without
changing text or framing. No delimiter-owned newline is prepended to epilogues.
Empty text clears the corresponding vector. Byte-identical folded output is
a no-op, retaining parsed markers and truncation; other changes use the raw
setter's framing invalidation. In particular, a changed epilogue restores a
truncated multipart's closing delimiter. Headers and child sources are untouched.

`leaf_parts()` visits leaves depth-first using an explicit stack, descending
through attached messages as well as multiparts. Empty containers and absent
messages/bodies contribute no leaves, and a leaf root visits itself. Traversal does
not inspect metadata or source bytes. Programmatic depth is unrestricted:
callers must bound it because derived debugging and ordinary drop still recurse.
Editing headers does not change an enum variant or reinterpret its body.
`MimeEntity::body_parts()` instead stops at attached-message wrappers;
`attachments()` filters those non-multipart nodes by their own dispositions.

Content collision scanning remains deferred.

### Multipart serialization

`Multipart::write_to` and `MimeEntity::write_to` serialize trees, including parsed
truncated containers, with default CRLF text output. Their options variants reuse
`MimePartWriteOptions`; a leaf entity root behaves exactly like `MimePart`.

```rust
use mimecraft::{Multipart, TextPart};

let mut mixed: Multipart = Multipart::with_boundary("mixed", "message-boundary").unwrap();
mixed.children_mut().push(TextPart::new("plain", "hello").unwrap().into());
let mut output = Vec::new();
mixed.write_to(&mut output).unwrap();
assert!(output.ends_with(b"\r\n\r\nhello\r\n--message-boundary--\r\n"));
```

Writing uses an explicit stack proportional to nesting depth, not recursive
calls or whole-body buffering. Headers use the leaf writer's normalization and
parsed-separator rules. Preamble and epilogue newlines are normalized without
charset decoding or folding; nonempty preambles are terminated before the
first emitted delimiter. Authored empty containers emit a terminated closing
delimiter. Unchanged parsed containers instead preserve raw marker bytes,
including padding and original line endings, missing closing markers, trailing
opening markers, and absent final terminators. Selecting another output newline
does not rewrite parsed markers; the anonymizer retains its normalization policy.

A **separate delimiter-owned newline** precedes the next marker unless the
child's parsed empty-body or truncated framing already owns that newline.
No binary body suffix is consumed or rewritten to provide framing. Children
normally disable `ensure_new_line`; truncated containers pass it to their
final child or trailing marker instead of an absent epilogue. Binary tails
remain unchanged. Encoder-generated endings are retained.
With LF output, a child ending in bare CR receives a CRLF delimiter prefix:
otherwise its CR and the delimiter's LF would look like one CRLF, losing the
body byte when parsed. Generated boundary marker lines still use LF. This also
protects nested epilogues ending in bare CR.

Authored or edited container Content-Type and boundary must be valid.
Unchanged parsed metadata retains its recovered interpretation, including
missing boundaries whose content stays entirely in the preamble. Equal or
prefix-overlapping ancestor boundaries are preserved only when the container
and every active multipart ancestor retain original parsed framing, including
across attached-message wrappers. Taking mutable children, editing framing
headers, or moving a parsed subtree under an authored container restores strict
overlap rejection. Unrelated header edits do not invalidate parsed framing.
Siblings may reuse boundaries. Multipart
Content-Transfer-Encoding must be absent, 7bit, 8bit, or binary. These metadata
checks do not scan bodies, preambles, or epilogues for delimiter collisions;
choosing collision-free values remains the caller's responsibility.

Options are validated before any I/O. Container metadata is checked before its
own headers are written, but validation follows traversal order: earlier
siblings and ancestor framing can already be present when a descendant fails.
Invalid options return `InvalidInput`, invalid metadata returns `InvalidData`,
and source/destination errors propagate. No rollback occurs; retries restart
serialization. Sources follow the leaf writer's rewind rules, the destination
is flushed but not closed, and stored headers, text, bodies, and structure are
unchanged. Writing does not prepare implicitly; signature-specific handling
remains deferred.

`Multipart` and `MimeEntity` also provide `write_content_to` and
`write_content_to_with_options`. Only the root header section is omitted:
preambles, complete child entities, boundary markers, and epilogues remain.
In particular, child headers are not suppressed, delimiter-owned newlines are
unchanged, and the root boundary remains active for ancestor-overlap checks.
`MimeEntity` dispatches leaves to the single-part content writer and message
wrappers to their enclosed-message writer. Content-only output uses the same
iterative traversal and metadata validation as complete serialization.

### Entity loading

`MimeEntity::load(&mut buffered_reader)` reads one complete entity through EOF,
returning `MimeEntity<Cursor<Vec<u8>>>`. `load_with_options` accepts
`MimeEntityParseOptions`: positive limits for total consumed wire bytes
(default 64 MiB), entity depth including the root (64), and each folded raw
header record (16 KiB). `parse_message_parts` defaults to true; set it to false
to retain all message parts as opaque leaves.

```rust
use mimecraft::{MimeEntity, TextPart};

let mut input = &b"Content-Type: text/plain; charset=utf-8\r\n\r\nhello"[..];
let MimeEntity::Part(part) = MimeEntity::load(&mut input).unwrap() else {
    unreachable!()
};
let mut text = TextPart::from_part(part).unwrap();
assert_eq!(text.text().unwrap(), "hello");
text.set_text("edited").unwrap();
let mut output = Vec::new();
text.part_mut().write_to(&mut output).unwrap();
```

For body-only input whose Content-Type was parsed separately, use
`MimeEntity::load_content(&content_type, &mut input)` or
`load_content_with_options(&content_type, &mut input, &options)`. These mirror
MimeKit's Content-Type/content loading overloads for HTTP-style response bodies.
They stream a formatted Content-Type header and CRLF separator ahead of the
borrowed content, then reuse the copying loader, including multipart and nested
message parsing. Header-looking body bytes remain content; no root transfer
encoding is inferred. The returned model borrows neither argument, and no
seeking or source closing occurs. Limits and header offsets include the
synthesized prefix. Invalid options and Content-Type formatting errors fail
before reading the body.

The loader uses `MimeParser` over `MimeReader`'s iterative event traversal and
incremental buffered reads. The parser retains the resulting headers,
preambles, epilogues, and encoded leaf bodies; owned-tree loading is not
constant-memory or persistent-source parsing. The reader streams ordinary
content even across long physical lines, while retaining header state and
delimiter lookahead rather than accumulating complete bodies.
The wire-byte limit does not describe total allocation overhead or buffered
reader read-ahead. Sources need `BufRead`, not seeking; use `BufReader` around a
plain `Read` source. The borrowed source is never closed.

Headers retain raw bytes, order, folds, duplicates, and malformed records.
The first Content-Type selects multipart, attached message, or leaf; a missing
type remains absent and normally selects an opaque leaf. Immediate
multipart/digest children instead default to message/rfc822. Invalid present
Content-Type falls back to application/octet-stream, while a recognizable type
survives invalid parameters. Unknown transfer encodings use `Unspecified`.
Raw headers remain authoritative, so explicit typed getters can still report
syntax errors. Stored encoding is initialized from
the first Content-Transfer-Encoding header, or `Unspecified` when absent. Bodies
are neither decoded nor normalized and start at position zero. Newline hints
describe their stored LF/CRLF sequences, excluding boundary framing.

Boundaries match whole case-sensitive lines, with optional trailing space/tab.
LF, CRLF, and a closing delimiter at EOF are accepted. Leaf bodies and nested
epilogues exclude exactly the newline immediately before their parent delimiter;
at EOF their trailing bytes are retained. Preambles retain their final newline.
Epilogues begin after the closing delimiter's newline, matching the owned model.
Empty containers and empty child bodies are supported.

Default recovery accepts headers interrupted by EOF or an enclosing delimiter,
unwinds truncated containers, and permits missing or invalid multipart boundaries.
Missing boundary parameters leave content in the preamble; literal marker
matching checks the nearest container first, including overlapping boundaries.
Raw records and offsets survive without inventing separator bytes or guessing
where body text starts. Standalone empty message wrappers acquire an empty
embedded message at EOF; wrappers immediately followed by an enclosing marker
remain without an attached message. `allow_header_only` defaults to true; its
existing false value rejects missing separators, not all malformed input.
Use a fallible compliance logger for selective rejection instead.
Empty top-level input, invalid first headers, exceeded limits, source errors,
and logger/callback errors remain fatal. Parsed marker spelling, truncation,
separator presence, and no-content newline ownership are retained. Authored or
edited invalid metadata still fails writer validation. Taking `content_mut()`
conservatively invalidates a leaf's no-content/separator absence even when only
decoding; replacing content also requires a separator. Replacing an attached
message, or taking `message_mut()` when present, resets wrapper framing.
Header and nonbinary content newline conversion
still applies, so load/write is not an unconditional lossless raw-wire API.
Recognized transparent message wrappers become structured attached messages;
encoded wrappers and other non-multipart types remain opaque leaves.
Zero-copy source ranges and async loading remain deferred.

### MIME reader and parser

The streaming implementation lives under `src/mime_reader`: `engine.rs` owns
resumable, byte-fed scanning, tree state, and framing; `driver.rs` supplies
the synchronous `BufRead` loop; `parser.rs` exposes `MimeEventParser` for direct
byte-fed use. `options.rs` defines the shared parse options, and `scan.rs` plus
`scan/` contain scalar/SIMD scanning and scanner tests.
`src/mime_reader.rs` provides the synchronous reader API and source lifecycle;
`src/mime_parser.rs` builds owned MIME trees from its events.
`src/mime_entity_load.rs` contains only the entity-loading convenience methods.
`MimeEntityParseOptions` remains available at the crate root.

`MimeReader<R: BufRead>` provides the first synchronous reader layer,
independent of model construction. `read_entity` and `read_message` invoke a
fallible callback with `MimeEvent` values: message and entity begin/end events,
and borrowed body, preamble, and epilogue chunks. Entity-begin events own their
raw `HeaderList` and include structural kind, stored transfer encoding, and
header/body positions. Message-root entity headers include both message and
Content-* fields; ordinary nested entities keep all of their headers.

Ordinary message parsing now skips one leading UTF-8 BOM, as MimeKit's
message-source initialization does. The same prefix scanner serves mbox and
byte-fed message parsing. Its bytes count toward input limits and original
offsets, but not header length, and are not retained in the message model.
Partial prefixes at EOF yield `UnexpectedEof`; mismatching prefixes yield
`InvalidData`. Source replacement resets recognition. Entity and standalone
header parsing bypass this step, matching upstream; BOMs in bodies and nested
message headers remain ordinary input. Rust retains its early `MessageBegin`
callback at the source origin so callers can abort without I/O; root
`EntityBegin` and header offsets follow the BOM.

```rust
use mimecraft::{MimeContentKind, MimeEvent, MimeReader};

let mut reader = MimeReader::new(b"Subject: example\r\n\r\nbody".as_slice());
let mut body_bytes = 0;
reader.read_message(|event| {
    if let MimeEvent::Content { kind: MimeContentKind::Body, bytes, .. } = event {
        body_bytes += bytes.len();
    }
    Ok(())
}).unwrap();
assert_eq!(body_bytes, 4);
assert_eq!(reader.position().offset, 24);
```

Content is not transfer-decoded or newline-normalized, and callback chunks
contain at most 4096 bytes. The newline immediately before a multipart
delimiter belongs to the delimiter and is withheld from body/epilogue events;
preamble newlines are retained. A trailing CR is checked against the next byte
so chunk boundaries cannot break CRLF ownership. Leaf-end events report the
newline format of emitted content, not discarded framing.

`MimePosition` counts original bytes from zero and physical lines from one.
LF advances the line; bare CR does not. Positions are relative to the supplied
input, not an underlying seek position. End positions are exclusive.
The reader's consumed position can be ahead of a callback because of lookahead.
Header offsets use the same byte origin. Empty message wrappers emit no inner
message events; successful begin/end events otherwise nest in wire order.

Ordinary content borrows buffered input directly across physical lines, without
copying each line or invoking a callback for every line. The default `simd`
feature fuses `\n--` / mbox `\nFr` candidate detection, line counting, and CRLF/LF
classification with runtime-detected SSE2/AVX2 or NEON. Scalar matching remains
the fallback, and bulk byte searches skip newline-free runs. Partial prefixes
and full delimiter validation stay with the line scanner. Only ambiguous
trailing newline bytes are copied on this fast path, and delimiter-owned
newlines do not contribute to body format hints. Header storage, nesting
state, and delimiter lookahead still allocate. A potential delimiter followed
by arbitrarily long transport padding must remain buffered until its full line
can be classified; this buffering is bounded by `max_input_bytes`, not the
callback chunk size. Newly appended padding is checked once rather than
repeatedly rescanning the growing line. Scanner scratch buffers are reused
between messages. This is not an unconditional constant-memory guarantee.

`MimeParser` consumes the same reader events to build existing owned models.
`parse_entity` and `parse_message` return their respective models, retaining
stored encoded bodies in `Cursor<Vec<u8>>` at position zero by default. Message parsing
partitions root Content-* fields onto the body without losing header offsets.
`MimeEntity::load` and `MimeMessage::load` now delegate to this parser.

`MimeEntity::load_file(path)` and `MimeMessage::load_file(path)` open a file
read-only and use the same copying parser. Their `load_file_with_options(path,
&options)` variants validate limits before opening the file. All four close
the file before returning, even on parsing failure; returned models retain no
file handles. Paths accept `AsRef<Path>`, and platform open/read errors propagate
unchanged. For source-backed file content instead, pass an owned `std::fs::File`
to `load_with_storage` with `MimeContentStorage::Source`.

`MimeParser::with_storage(source, storage)` selects body storage explicitly
on the same parser. `MimeContentStorage::Memory` copies encoded bodies;
`MimeContentStorage::Source` retains bounded ranges in the original seekable input, implementing
MimeKit's formerly named persistent mode. Both choices return models whose
body streams are `MimeContentStream<S>` instead of the default cursor type.
The ordinary `new`/`with_options` constructors and `load` methods remain copying
APIs with their original return types and accept nonseekable `BufRead` sources.

```rust
use std::io::Cursor;
use mimecraft::{MimeContentStorage, MimeParser};

let input = Cursor::new(b"Subject: example\r\n\r\nbody".to_vec());
let mut parser = MimeParser::with_storage(input, MimeContentStorage::Source).unwrap();
let mut message = parser.parse_message().unwrap();
drop(parser);
let mut output = Vec::new();
message.write_to(&mut output).unwrap();
assert_eq!(output, b"Subject: example\r\n\r\nbody");
```

The explicit-storage constructors require `Read + Seek`, with no nonseekable
fallback. They query the initial position and add a 4 KiB input buffer;
`with_storage_and_options(source, storage, options)` validates limits first.
Reader/header offsets remain relative to that initial position, while body
ranges include its absolute offset. Headers, tree structure, preambles,
epilogues, and original boundary framing remain in memory. Source mode avoids
body copies but still scans all input and enforces the same limits and reporting.

An owned source stays alive until its last parser/body/shared handle drops.
Passing `&mut source` instead borrows it; returned source-backed models cannot
outlive that borrow. The explicit-storage return type carries the source's
lifetime even when `Memory` is selected; use the ordinary copying constructors
when borrowed input must be reusable while the returned models remain alive.
The backing data must remain unchanged; writes through other file handles,
truncation, or rewriting an mbox archive invalidate stored ranges.

Each source-backed body and the parser have independent cursors. Reads may be
interleaved with subsequent mbox parses, including buffered read-ahead, or run
on separate threads when the source is `Send`. Shared seek/read operations are
synchronized, and source errors propagate rather than falling back to copies.
`replace_storage_source(source)` queries the replacement's position before
resetting the parser, returns the previous buffered shared source, and preserves
the selected storage mode and options. Old messages keep the old source.
A rejected replacement leaves parser state unchanged. The ordinary
`replace_source` also accepts an already buffered `SharedStream` in this mode.

`MimeEntity::load_with_storage(source, storage)` and
`MimeMessage::load_with_storage(source, storage)` provide the same storage
choice without retaining a parser. Each also has
`load_with_storage_and_options(source, storage, &options)` for explicit limits
and nested-message interpretation. They load one complete entity/message through
EOF, using the same parser and preserving its headers, recovery, and framing.

```rust
use std::io::Cursor;
use mimecraft::{MimeContentStorage, MimeMessage, TextFormat};

let source = Cursor::new(b"Content-Type: text/plain\r\n\r\nbody");
let mut message = MimeMessage::load_with_storage(source, MimeContentStorage::Source).unwrap();
assert_eq!(message.text_body(TextFormat::Plain).unwrap().as_deref(), Some("body"));
```

The storage loaders accept owned or borrowed seekable sources with the same
lifetime rules as `with_storage`. Their internal parser is dropped on return;
source-backed bodies retain the backing source, while copied bodies do not.
Invalid options fail before I/O, and source/parse errors return no partial
model. Failure drops an owned source, leaves a borrowed source open without
rewinding, and discards internal read-ahead. Use `MimeParser` directly when
recovering that buffer, iterating mbox messages, or attaching a compliance logger.
The original `load`/`load_with_options` APIs remain nonseekable-compatible
copying loaders.

Compliance reporting is opt-in. Reader methods `read_entity_with_logger`,
`read_message_with_logger`, `read_mbox_message_with_logger`,
`read_headers_with_logger`, and `read_status_group_with_logger` borrow a
`MimeComplianceLogger` for one operation. The parser has matching
`parse_*_with_logger` methods. A closure taking `MimeComplianceIssue` and
returning `io::Result<()>` implements the logger trait.

```rust
use mimecraft::{MimeComplianceIssue, MimeComplianceViolation, MimeParser};
use std::io;

let mut parser = MimeParser::new(b"Subject: example\r\n\r\nbody".as_slice());
let message = parser.parse_message_with_logger(&mut |issue: MimeComplianceIssue| {
    if issue.violation == MimeComplianceViolation::RepeatedContentType {
        return Err(io::Error::new(io::ErrorKind::InvalidData, issue.to_string()));
    }
    Ok(())
}).unwrap();
```

Logger errors propagate unchanged, stop further events, and return no partial
owned model. No strict-mode switch is introduced. Use `set_compliance_options`
with `MimeComplianceOptions` to select transport/storage severity and a
per-violation report cap (zero means unlimited). `enabled_validators` selects
`MimeComplianceValidators::BASE64`, `QUOTED_PRINTABLE`, and `UU_ENCODE`;
combine flags with `|`, use `ALL` (the default), or `NONE` to keep only
structural and raw-byte checks. Disabling a validator changes reporting, not
parsing or recovery. Budgets reset per operation,
including each mbox message; the first suppressed issue emits one
`TooManyComplianceIssues` marker. `CappedMimeComplianceLogger` exposes the
same quota mechanism for independent use. Loggers retaining issues should
also limit their own storage.

The complete violation identifiers, severity/category mappings, descriptions,
and remarks are ported. Issues carry original offsets, one-based lines and
byte columns, and an exact/line-start/element-start position kind. Reporting
currently covers raw-header syntax and line format, repeated/missing message
fields, MIME metadata, framing/depth failures, and body line format/null/high
bytes. Header blocks are checked before their entity-begin event; body checks
are incremental. Streaming validators check Base64 alphabet/padding/quantums,
quoted-printable escapes/soft breaks, and UU markers/file modes/encoded lines.
They preserve the original encoded bytes and finish before the part's end
event. The newline before a MIME or mbox boundary is validated even though it
is excluded from content callbacks; the marker itself is not encoded content.
Diagnostics retain MimeKit's per-line suppression of repeated malformed-byte
findings, in addition to the configurable per-violation quotas.

Quoted-printable literal runs use bulk byte searches; UU scanning skips whole
runs where classification is no longer needed. Base64 validation currently uses
MimeKit's table-driven scalar logic, not its separate SIMD validator kernels.
Deep address validation is not yet ported. An empty report is not a guarantee
of full MIME compliance.

Accepting a diagnostic preserves the same default recovery as an unlogged read.
Source, callback, logger, and resource-limit failures remain fatal. Ordinary methods
without a logger perform no compliance scans or allocate reporting state and
retain the SIMD fast path; logged reads do additional inspection.

```rust
use mimecraft::MimeParser;

let mut parser = MimeParser::new(b"Subject: example\r\n\r\nbody".as_slice());
let message = parser.parse_message().unwrap();
assert!(message.headers().contains("Subject"));
assert!(!message.body().unwrap().headers().contains("Subject"));
```

`MimeReader::read_headers` reads a standalone raw header block and emits one
`MimeEvent::Headers`, without entity/message events or MIME metadata validation.
The event owns the complete `HeaderList` and reports original start/end positions
and a borrowed `body_separator`: CRLF, LF, lone CR at EOF, or `None` when EOF ends
the headers without a separator. Empty input produces an empty block;
unterminated and malformed records retain their exact bytes. Like the existing
raw `HeaderParser`, this accepts even an invalid first record, unlike MimeKit's
higher-level parser. This does not add recovery to entity/message loading.

`MimeParser::parse_headers` returns that ordered list, including Content-* fields,
without partitioning or constructing a body. It consumes the separator but does
not store it in `HeaderList`; use the reader event when separator metadata matters.
Both operations stop without consuming body bytes, even when the buffered source
has already prefetched them. The input limit includes headers and their separator
but not the body; the per-record limit includes folds. Depth and message-part
interpretation do not apply. EOF at the exact input limit succeeds, but exhausting
the limit while more header input remains is an error, not an artificial EOF.

```rust
use mimecraft::MimeParser;

let mut parser = MimeParser::new(b"Subject: example\r\n\r\nbody".as_slice());
let headers = parser.parse_headers().unwrap();
assert!(headers.contains("Subject"));
assert_eq!(parser.position().offset, 20);
assert_eq!(parser.into_inner(), b"body");
```

Both types provide `with_options(source, MimeEntityParseOptions)`, `options`,
`position`, `get_ref`, and `into_inner`. These copying/event constructors perform no I/O; zero
limits fail immediately. Each installed source supports one standalone operation,
or the status-group or mbox iteration modes described below. Standalone header reading
stops at the separator or EOF, while entity/message reading runs through EOF.
Repeating a standalone operation returns `InvalidInput`, including after failure.
Source and callback errors propagate without rollback or a partial model;
callback failure stops further events, and interrupted source reads retry.
After failure, begin events may remain unmatched. Borrow an existing source
with `&mut` to retain external ownership, and keep the same buffered source
when recovering it with `into_inner` so read-ahead is not discarded.

`replace_source(source)` starts a new operation on an existing reader or parser,
including after failure. It preserves options, resets positions to offset zero
and line one, and clears completion/failure state. It returns the previous
buffered source at its current position, retaining unread read-ahead bytes;
neither source is read, sought, flushed, or closed. This is the idiomatic
owned-source adaptation of MimeKit's `SetStream`, not resumption of a partial
parse. Previously returned models retain their independent owned content.
The replacement must have the same Rust source type; construct a new reader
or parser to change that type.

`is_end_of_stream()` reports whether a successful operation finished at an
observed EOF, without doing I/O. It starts false and becomes true after successful
entity/message parsing. Header reading reports true only when EOF terminated
the block; a separator stops reading without probing the following byte, even
when that separator happens to be the source's final bytes. Thus false does not
prove more input exists. Parse/source/callback failures do not mark successful
completion, and source replacement resets the status. In mbox mode, only the
last message or an empty archive marks EOF, not a next-message delimiter.
Completion does not permit repeating a standalone operation.

`MimeParser::parse_status_group()` selects incremental delivery-status group
parsing and returns `Some(HeaderList)` for each nonempty group, then `None` at
EOF. `MimeReader::read_status_group(callback)` provides the same operation as
one `MimeEvent::Headers` and a true return value, or false without an event at
EOF. Leading, repeated, and trailing CRLF/LF blank lines are skipped; a lone
trailing CR creates no group. Raw fields and unterminated final records use the
same header scanner as standalone parsing.

Group offsets and line positions include all earlier groups and skipped lines.
The input limit is cumulative across calls, including separators; the header
limit applies to individual folded records. Each call stops after its group's
separator or EOF without reading the next group's bytes. Only the current group
is buffered by the parser. Fields are not interpreted or transfer-decoded;
callers can recover a separately encoded remainder through `into_inner`.
Repeated group reads after EOF return `None`/false without I/O. Source, parse,
or callback failures prevent further reads until source replacement.
Other operation modes cannot be mixed with group iteration on the same source.

```rust
use mimecraft::MimeParser;

let input = b"Reporting-MTA: dns; example.com\r\n\r\n\
              Final-Recipient: rfc822; user@example.com\r\n\r\n";
let mut parser = MimeParser::new(input.as_slice());
assert!(parser.parse_status_group().unwrap().unwrap().contains("Reporting-MTA"));
assert!(parser.parse_status_group().unwrap().unwrap().contains("Final-Recipient"));
assert!(parser.parse_status_group().unwrap().is_none());
```

`MimeParser::parse_mbox_message()` explicitly selects Unix mbox iteration and
returns `Some(MimeMessage)` per message, then `None` at EOF.
`MimeReader::read_mbox_message(callback)` returns true/false and emits a raw
`MimeEvent::MboxMarker` before that message's normal events. Marker bytes include
their LF/CRLF ending and have original start/end positions; they are not message
headers and are not stored in owned models.

```rust
use mimecraft::MimeParser;

let archive = b"From sender@example.com\nSubject: first\n\nbody\n\
                From other@example.com\nSubject: second\n\nlast\n";
let mut parser = MimeParser::new(archive.as_slice());
assert!(parser.parse_mbox_message().unwrap().is_some());
assert!(parser.parse_mbox_message().unwrap().is_some());
assert!(parser.parse_mbox_message().unwrap().is_none());
```

Like MimeKit, a line beginning with exactly `From ` is a marker, without
validating its sender/date. A leading UTF-8 byte order mark and initial
non-marker lines are skipped under the input budget, without removing their
bytes from reported offsets. The byte order mark is recognized only at the
start of each installed source; incomplete marks are errors and a mark alone
is an empty archive. Nonempty input without a marker and unterminated marker lines
are errors. `>From ` remains unchanged ordinary content. The newline preceding
a next marker belongs to framing; EOF retains trailing content newlines.
Automatic unescaping is not implemented. To honor the outer message's
Content-Length, set `MimeEntityParseOptions::respect_content_length`:

```rust
use mimecraft::{MimeEntityParseOptions, MimeParser};

let archive = b"From first\nContent-Length: 10\n\nFrom body\n\nFrom second\n\nlast";
let options = MimeEntityParseOptions {
    respect_content_length: true,
    ..Default::default()
};
let mut parser = MimeParser::with_options(archive.as_slice(), options).unwrap();
assert!(parser.parse_mbox_message().unwrap().is_some());
assert!(parser.parse_mbox_message().unwrap().is_some());
assert!(parser.parse_mbox_message().unwrap().is_none());
```

This option defaults to false and applies only to mbox iteration. Like MimeKit,
it ignores `From ` lines whose start precedes the declared body end, including
inside embedded messages, while still recognizing MIME boundaries. The length
is a minimum marker offset, not a hard body limit: parsing continues until an
eligible marker or EOF. An oversized length can absorb subsequent messages;
it does not bypass input limits or produce a truncation error at EOF.
The first usable outer Content-Length wins. A usable raw value begins with
space, tab, CR, or LF, then unsigned decimal digits fitting in `i32`; trailing
text is ignored. Invalid values and nested Content-Length fields do not change
the threshold. Original header bytes are retained.

Positions remain archive-relative. The input budget resets per message,
including its envelope and framing (and initially skipped data), but excludes
next-marker lookahead. `max_header_length` also bounds complete marker lines.
Other resource limits remain in force. Truncated headers and multiparts recover
at actual EOF or the next-message marker.
Failures stop iteration without resynchronizing; modes cannot be mixed until
source replacement. Previously returned messages remain independent.

The reader consumes just five bytes of the next marker as lookahead, not its
remaining line or headers. `pending_input()` exposes these bytes (`b"From "`).
`into_parts()` returns `(source, pending)`; replay `pending` before `source`
when resuming elsewhere. `get_ref`/`into_inner` expose only the physical buffered
source, and `replace_source` discards reader-owned lookahead, so save
`pending_input()` first if extracting a partial archive.

The applicable synchronous MimeKit reader/parser regression batch is complete,
with the intentional API and recovery differences documented below.
Entity/message parsing uses default malformed-message recovery and optional
compliance diagnostics. Source-backed storage is implemented; async APIs and
further compliance-validator implementation remain deferred.
`HeaderList::load` continues to support independent raw header blocks.

Zero limits return `InvalidInput` before I/O. Exceeded limits return `InvalidData`.
Entity/message parsing also rejects invalid initial headers with `InvalidData`,
and empty input with `UnexpectedEof`.
Header-length errors retain `HeaderTooLong`, whose `offset()` identifies the
oversized header's start within the installed input source and `limit()`
reports the configured maximum.
Interrupted reads retry; other I/O failures propagate without returning a
partial tree or restoring the source position. Retain the buffered reader
after a failure; retries require restarting at the entity's beginning.

### Message ownership and I/O

`MimeMessage<R = Cursor<Vec<u8>>>` owns a raw `HeaderList` and an optional
`MimeEntity<R>` body. `new`/`default` create an empty message; `with_body` takes
an existing tree, and `from_parts` takes both components without validation.
`headers_mut` and `body_mut` allow direct edits, `set_body` returns the old
tree, `into_parts` extracts ownership, and `leaf_parts` traverses iteratively.
These operations do no source I/O or automatic header synchronization.

```rust
use mimecraft::{AddressList, Header, MimeMessage, TextPart};

let mut message = MimeMessage::with_body(
    TextPart::new("plain", "hello").unwrap().into(),
);
message.headers_mut().add(
    AddressList::parse(b"Alice <alice@example.com>")
        .unwrap().to_header("From").unwrap(),
);
message.headers_mut().add(Header::parse(b"Subject: greeting").unwrap());
message.headers_mut().add(Header::parse(b"MIME-Version: 1.0").unwrap());
let mut output = Vec::new();
message.write_to(&mut output).unwrap();

let parsed = MimeMessage::load(&mut output.as_slice()).unwrap();
assert!(parsed.headers().contains("From"));
assert!(!parsed.headers().contains("Content-Type"));
assert!(parsed.body().unwrap().headers().contains("Content-Type"));
```

`load` and `load_with_options` reuse the buffered entity loader, including its
limits, errors, and in-memory storage. The total byte limit includes message
headers; depth starts at the body root, not the `MimeMessage` wrapper. At each
message root, headers are partitioned: parsed case-insensitive `Content-*` fields
belong to the body, including unknown extensions; all others, including MIME-Version and
malformed records, belong to the message. `Header::is_content_header` exposes
this name-only classification. Records retain bytes and order within each
list, with original stream byte offsets retained for ordinary and masked output.
Ordinary descendant entity headers are not repartitioned.

`Header::offset()` returns the parsed stream byte position as `Option<u64>`,
not an index in either list. Stream loaders count from the beginning of their
input, including all MIME framing, and nested headers share that origin.
These non-seeking APIs do not query the underlying reader's seek position.
Standalone parsing and authored headers have `None`; clones retain offsets,
but authored replacements do not inherit the replaced record's offset.
Offsets do not affect header value equality.

Loading always produces a body, even for empty content, and requires a complete
header/body separator. Missing From/Date and invalid non-content metadata are
not rejected. Recognized attached messages are parsed by default, sharing the
same limits across all message and multipart nesting. Raw constructors and
edits do not repartition misplaced headers; callers must keep Content-* fields on the
body and manage any duplicates themselves.

`write_to` and `write_to_with_options` interleave message and root-body headers
in their loaded order, then write one separator and body content, reusing
`MimePartWriteOptions` and the tree writer. An absent body still gets a separator;
an entirely empty message writes one newline. Options fail before I/O; later metadata or I/O
errors can leave partial output. Sinks are flushed, not closed, and successful
body copying rewinds sources. Headers and stored bytes are not mutated.
Body metadata is checked after the merged header section and separator are
written, so metadata errors can leave that prefix. Embedded messages use the
same merge independently; the body's headers are not written twice.

`write_headers_to` and `write_headers_to_with_options` write only that header
section: interleaved message/root-body headers, then the blank separator. They take
`&self` and need no source `Read`/`Seek` bounds. Multipart children, attached
message contents, preambles, epilogues, and delimiters are never visited or
written. An attached-message root contributes its wrapper headers only.

Both ordinary writers use the same merge rules as masked message headers:
leading new message headers come first, loaded records merge by original
stream offset until a new body header, then remaining message headers precede
remaining body headers. Authored replacements have no offset; lists are not
sorted or repartitioned. Independently authored lists without loaded positions
use message-first order.

Headers-only output uses the same newline conversion, authored-record termination,
and preservation of parsed EOF terminators/separator absence, without decoding
or refolding values. It does not validate body metadata,
so an invalid boundary or transfer encoding does not prevent header output.
All `MimePartWriteOptions` are validated before destination I/O; valid encoder
widths and `ensure_new_line` have no additional effect without a body. Write
and flush failures propagate, potentially after partial output, while source
positions and raw headers remain unchanged. Retrying restarts at the beginning.

Writing generates no Date, Message-Id, From, Subject, or MIME-Version. This is not
transport preparation: **Bcc and Resent-Bcc are written as supplied**.
Other message-specific typed convenience setters, hidden-header filtering,
and automatic transport preparation remain deferred.

### Explicit tree transport preparation

`MimeEntity`, `Multipart`, `MessagePart`, and `MimeMessage` expose
`prepare(constraint)` and `prepare_with_options(constraint, &options)`.
They apply the leaf preparation rules above in depth-first child order,
including enclosed message bodies. Traversal uses an explicit stack with
storage proportional to depth, not the number of siblings. Ordinary destruction
and derived debugging still recurse.

Multipart preparation only delegates to children; message wrappers only
delegate to an enclosed message, if present. Neither operation validates or
rewrites container headers, boundaries, transfer encodings, preambles, or
epilogues. The model variant controls traversal, not raw Content-Type spelling.
An opaque message payload remains a leaf and uses leaf encoding selection.
An absent enclosed message is a no-op after options validation.

Message preparation with a body parses the first message-level MIME-Version.
If absent and the body's root header list is nonempty, it appends
`MIME-Version: 1.0` before preparing the body. Descendant headers do not count,
and misplaced body-level versions do not replace message metadata. Existing
valid versions, including non-1.0 values and duplicates, remain unchanged.
A bodyless message skips the version check. No Date, Message-Id, address, or
other metadata is generated, and Bcc/Resent-Bcc remain present.

`MimePrepareError` distinguishes invalid options, invalid first MIME-Version,
and underlying `PartPrepareError` failures. Invalid versions are explicit errors,
not MimeKit's missing-value fallback, even when the body has no headers.
Options are checked before any traversal or mutation. A later error stops
traversal without rolling back earlier leaf edits or inserted versions; later
siblings remain untouched. Source positioning follows the leaf analyzer.
Retrying starts a fresh traversal against the current headers and sources.

Use the same `MimePartWriteOptions` for preparation and serialization, as no
private line-width state is retained. Stored bytes and newline hints are not
changed. Preparation trusts safe requested encodings and does not refold raw
headers or repair container metadata: success does not guarantee that writing
will succeed or that all output is transport-safe. Constructors, loading, and
writing never invoke preparation implicitly.

```rust
use mimecraft::{EncodingConstraint, MimeMessage, MimePartWriteOptions, TextPart};

let mut message = MimeMessage::with_body(TextPart::new("plain", "From sender\n")?.into());
let options = MimePartWriteOptions { max_line_length: 60, ..Default::default() };
message.prepare_with_options(EncodingConstraint::SevenBit, &options)?;
let mut output = Vec::new();
message.write_to_with_options(&mut output, &options)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Message subjects and unstructured headers

`MimeMessage::subject()` reads the first Subject and returns an owned
`Option<rfc2047::DecodedHeader>`. Absence is distinct from present empty text.
Decoding is tolerant: malformed encoded-word wrappers remain literal and
charset fallback/replacement is reported in `issues`. The `_with_options`
getter accepts `rfc2047::DecodeOptions`. Like `Header::decode_value`, decoding
precedes display unfolding; diagnostic ranges refer to the raw header value.
Getters never modify headers or inspect body sources.

`set_subject` and `set_subject_with_options` format a complete replacement
before editing. Like MimeKit, they trim surrounding Unicode whitespace and
remove CR/LF without inserting replacement spaces. Other controls are encoded,
not silently removed. Assigned encoded-word lookalikes are literal text, not
pre-encoded input.

```rust
use mimecraft::{Header, MimeMessage};

let mut message: MimeMessage = MimeMessage::new();
message.set_subject("  Hello\r\n world  ").unwrap();
assert_eq!(message.subject().unwrap().text, "Hello world");
let header = Header::from_unstructured("Comments", "caf\u{e9}").unwrap();
assert!(header.raw().is_ascii());
assert_eq!(header.decode_value().text, "caf\u{e9}");
```

Subject assignment replaces all occurrences with one canonical `Subject` at
the first slot, matching MimeKit's singleton-header replacement.
It always reformats, even for equal decoded text. Empty input creates an empty
Subject; remove all occurrences through `headers_mut().remove_all("Subject")`
when absence is desired. Formatting failures leave all headers unchanged.

`Header::from_unstructured` and its `_with_options` variant provide the same
authoring path for arbitrary unstructured fields. `UnstructuredHeaderFormatOptions`
defaults to ASCII output, UTF-8 encoded words, CRLF, and enforced 78-byte physical
lines. It accepts limits in `60..=998`, LF endings, and explicit RFC 2047 encoding
options. Field names must be nonempty printable ASCII without a colon and fit
with their colon on one line.

Set `UnstructuredHeaderFormatOptions::international` to `true` for raw UTF-8
unstructured text, including Subject and Content-Description. Raw text ignores
the encoded-word charset; folding still counts **UTF-8 bytes**, not characters,
and preserves existing spaces. This is separate from
`HeaderFormatOptions::international`, which controls MIME parameters.

```rust
use mimecraft::{MimeMessage, UnstructuredHeaderFormatOptions};

let mut message: MimeMessage = MimeMessage::new();
let options = UnstructuredHeaderFormatOptions {
    international: true,
    ..Default::default()
};
message.set_subject_with_options("caf\u{e9}", &options).unwrap();
assert_eq!(
    message.headers().get("Subject").unwrap().raw(),
    "Subject: caf\u{e9}\r\n".as_bytes()
);
```

International mode retains whole-value RFC 2047 encoding for controls (including
tabs), literal encoded-word lookalikes, and tokens too large for a continuation
line. Like MimeKit's lossless folding, it does not insert spaces into decoded
text. The selected encoding charset and encoded-word limit
apply to this fallback, with errors for unrepresentable text or an insufficient
word limit. Invalid options are still rejected even for raw or empty output.
For SMTP delivery, generally set `international` to match whether the server
advertises `SMTPUTF8` in its `EHLO` response. Keep it false if the extension is
unavailable; enabling it is optional when supported. The caller must ensure that
the destination accepts UTF-8 headers; this does not negotiate transport
capabilities or enable international address formatting.

Folding preserves interior spaces without splitting tokens or encoded words.
Oversized plain tokens are encoded instead of having spaces inserted into them.
When encoding is needed, the encoder's whole-value policy is retained rather
than MimeKit's mixed plain/encoded-word heuristics. Unmappable text and invalid
options return explicit errors; encoding offsets refer to the normalized text.

These factories do not dispatch on field names or validate structured grammar:
use address/content/identifier formatters for those fields. Safe header framing
is not display sanitization; decoded NUL and other controls can remain present.

### Message address lists

`MimeMessage::from()`, `reply_to()`, `to()`, `cc()`, and `bcc()` (and their
`resent_*` counterparts) aggregate every matching message header in order.
They return `MessageAddresses`: an owned `addresses` list and `issues` containing
syntax diagnostics with the original header index and raw-value byte offset.
Groups, duplicates, source routes, and address/name decoding diagnostics survive.
Missing, empty, and valid comment-only fields contribute no addresses.

Parsing follows MimeKit's internal message-header recovery rather than the
standalone `AddressList::parse` fail-fast contract. Invalid entries are skipped
to commas outside quoted strings and comments; group terminators stay with the
group parser. Recovery works inside groups, and unterminated groups survive.
As in MimeKit, a failed separator or whitespace/comment production discards the
current list (one header's addresses, or one group's members), not other header
occurrences. Diagnostics expose rejected input without preventing inspection of
the message. `_with_options` getters accept `AddressParseOptions`; syntax
strictness and group-depth limits remain independent of recovery.

```rust
use mimecraft::{AddressList, Header, MimeMessage};

let mut message: MimeMessage = MimeMessage::new();
message.headers_mut().add(
    Header::parse(b"To: alice@example.com, @, bob@example.com")?,
);
let mut parsed = message.to();
assert_eq!(parsed.addresses.len(), 2);
assert_eq!(parsed.issues.len(), 1);
parsed.addresses.as_mut_vec().push("carol@example.com".parse()?);
message.set_to(&parsed.addresses)?;
assert!(message.to().issues.is_empty());
message.set_to(&AddressList::new())?;
assert!(!message.headers().contains("To"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

Getters never change raw headers or access body sources. Edits to returned lists
take effect only after explicit assignment. Setters accept `&AddressList`; empty
lists remove all occurrences and ignore unused formatting options. Nonempty
lists use checked address-header formatting before replacing all occurrences
at the first slot. Invalid options, encoding failures, or oversized unbreakable
pieces leave headers unchanged. Equal lists are still reformatted so route-only
edits are not lost. `_with_options` setters accept `AddressHeaderFormatOptions`.

Defaults use ASCII output, UTF-8 encoded names, IDNA domains/routes, CRLF, and
78-byte physical lines. `AddressHeaderFormatOptions::international` enables UTF-8
output; callers must ensure compatible transport support. SMTPUTF8 negotiation,
field-specific address-count/group validation,
Sender inference, reply-address fallback, and resent-block selection are not
provided. Resent accessors aggregate and replace across all matching blocks.
**Bcc and Resent-Bcc are not automatically hidden when writing messages**;
remove them explicitly before delivery when recipient privacy requires it.

### Message recipient collection

`MimeMessage::recipients()` returns `Result<MessageRecipients, MailboxParseError>`.
`recipients_with_options(&AddressParseOptions)` configures both mailbox parsing
and address-list recovery. The detached result contains `mailboxes`, header-indexed
syntax `issues`, and `uses_resent_headers`.

Like MimeKit's `GetRecipients(false)`, a parsed Resent-Sender selects resent
recipients. Only when it is absent is Resent-From parsed: any recovered address,
even an empty group, selects resent fields. Empty, comment-only, or wholly
rejected Resent-From lists do not. A Resent-To header alone is insufficient.
An invalid first Resent-Sender returns its existing mailbox error, rather than
falling back to Resent-From or a later duplicate. Parsing options apply to that
selector too.

The selected fields are collected in To, Cc, Bcc order, or Resent-To,
Resent-Cc, Resent-Bcc order. All matching occurrences retain header order within
each field; groups flatten depth-first and empty groups contribute no mailboxes.
All duplicates, mailbox spelling, names, routes, and mailbox decoding diagnostics
survive. Sender, From, and Reply-To are not recipient sources. Empty resent
recipient fields do not fall back to ordinary recipients, and this does not
select the newest resent block: all matching recipient headers participate.

```rust
use mimecraft::MimeMessage;

let mut message: MimeMessage = MimeMessage::new();
message.set_to(&"Friends: alice@example.com, bob@example.com;".parse()?)?;
message.set_bcc(&"alice@example.com".parse()?)?;
let recipients = message.recipients()?;
assert_eq!(recipients.mailboxes.len(), 3);
assert!(!recipients.uses_resent_headers);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Recovery reuses the message address-list parser. Resent-From diagnostics, when
that selector is evaluated, come first, followed by diagnostics from each
selected field; original header indices and raw-value offsets are retained,
not sorted. Group names/structure are omitted from the flattened result.
Unselected recipient fields are not parsed. Each call reads current message
headers without body I/O or raw edits, and modifying returned mailboxes never
changes the message. Body headers and attached messages do not contribute.

Case-insensitive recipient deduplication (`GetRecipients(true)`), sender-inclusive
collection, and automatic SMTP envelope validation remain deferred. Explicit
validation is available through `MailboxAddress::validate_smtp` and
`validate_smtp_utf8`. Collection does not remove or hide Bcc/Resent-Bcc during
serialization.

### Message sender mailboxes

`MimeMessage::sender()` and `resent_sender()` read only the first matching raw
header, returning `None` when absent and explicit `MailboxParseError` values
for malformed or disallowed input. Like MimeKit, they parse a single mailbox
prefix, not a whole address list. Trailing comments/whitespace are consumed;
other suffixes after a successful prefix are ignored. Groups are rejected,
and later duplicate headers are not used as fallbacks.

The `_with_options` getters accept `MailboxParseOptions` for compatibility and
name decoding. Error offsets refer to the raw header value; returned mailboxes
retain name/address decoding diagnostics and source routes. Getters reparse
current bytes without caching, inspecting body headers, or reading sources.
Returned mailboxes are owned, detached values:

```rust
use mimecraft::{MailboxAddress, MimeMessage};

let mut message: MimeMessage = MimeMessage::new();
let sender: MailboxAddress = "Agent <agent@example.com>".parse().unwrap();
message.set_sender(Some(&sender)).unwrap();
let mut edited = message.sender().unwrap().unwrap();
edited.set_display_name("Delivery Agent");
assert_eq!(message.sender().unwrap().unwrap().display_name(), "Agent");
message.set_sender(Some(&edited)).unwrap();
message.set_resent_sender(Some(&sender)).unwrap();
```

Setters accept `Option<&MailboxAddress>` and format through the existing address
header writer before changing any headers. `Some` replaces all occurrences at
the first slot, or appends if absent. Every assignment reformats, even for equal
mailboxes: mailbox equality ignores source routes, so it cannot safely suppress
route-only changes. `None` removes all occurrences, including malformed values.

Defaults use ASCII output, UTF-8 encoded display names, IDNA-normalized domains
and routes, CRLF, and an enforced 78-byte physical-line limit. The `_with_options`
setters accept `AddressHeaderFormatOptions`. Invalid options, encoding errors,
international local parts in ASCII mode, and unbreakable oversized pieces leave
all headers unchanged. Removal ignores unused formatting options. Input mailboxes
and body sources are never modified.

Sender and Resent-Sender are independent. No From/Resent-From inference, resent
block construction, SMTP envelope generation, or SMTPUTF8 negotiation is provided.
Set `AddressHeaderFormatOptions::international` for UTF-8 header output.

### Message dates and MIME version

`MimeMessage::date()`, `resent_date()`, and `mime_version()` parse the first
matching raw header with the existing `DateTimeOffset` and `MimeVersion` parsers.
Absent fields return `None`, while values those parsers cannot interpret return
errors. No current date, minimum date, or default MIME version is substituted.
Date parsing retains its legacy recovery rules; version error offsets are
relative to the raw value. Encoded words are not decoded, and later duplicates
and body headers are ignored.

```rust
use mimecraft::{DateTimeOffset, MimeMessage, MimeVersion};

let mut message: MimeMessage = MimeMessage::new();
let date = DateTimeOffset::new(2007, 6, 28, 12, 47, 52, -300).unwrap();
message.set_date(date);
message.set_resent_date(date);
message.set_mime_version(MimeVersion::V1_0);
assert_eq!(message.date().unwrap(), Some(date));
assert_eq!(message.mime_version().unwrap(), Some(MimeVersion::V1_0));
```

Setters accept checked values and perform no source I/O. Equal parsed values
leave original bytes unchanged only without duplicates for Date and MIME-Version;
Resent-Date retains duplicates on equal assignments. Otherwise they replace all
occurrences at the first slot, or append if absent. New date records use English
names, seconds, and numeric offsets; versions use decimal components. Both use
CRLF. Malformed existing values are replaced, and raw removal restores absence.

Date equality includes local components and offset, following the Rust value
type rather than C#'s instant-only comparison: assigning the same instant in
a different offset changes the header. Version component counts are significant,
so `1.0`, `1.0.0`, and `1.0.0.0` remain distinct. Getters always reparse current
bytes; no cache masks the date parser's narrower range. A checked date before
1969 may fail to parse after assignment, and an offset below -12 hours recovers
as UTC even though the constructor accepts offsets down to -14 hours.

Date and Resent-Date remain independent. These APIs do not construct resent
blocks, synthesize metadata during writing, or implement other MIME versions
merely because a version other than 1.0 was assigned.

### Message identifiers and reply threading

`MimeMessage::message_id()` and `resent_message_id()` read the first matching
raw header using `MessageId::parse_prefix`: absence returns `None`, malformed
values return errors, and bytes after a bracketed identifier are ignored.
`in_reply_to()` instead scans for the first bracketed candidate using
`References`, skipping prose, comments, and quoted prose. It returns `None`
when absent or when no candidate exists, but reports an error encountered
before the first valid ID rather than silently recovering. Later candidates
and duplicate headers are not inspected.

`references()` borrows the first References header as a lazy `References`
iterator, or yields nothing when absent. It preserves identifier order,
duplicates, explicit errors, and recovery when iteration continues. All getters
read current raw bytes, without encoded-word decoding, caching, or body I/O.
Diagnostic offsets are relative to the first header's raw value.

```rust
use mimecraft::{MessageId, MimeMessage};

let mut message: MimeMessage = MimeMessage::new();
message.set_message_id("reply@example.com").unwrap();
message.set_resent_message_id("forwarded@example.com").unwrap();
message.set_in_reply_to(Some("<parent@example.com>")).unwrap();
message.set_references(["root@example.com", "parent@example.com"]).unwrap();

let references = message.references().collect::<Result<Vec<MessageId>, _>>().unwrap();
assert_eq!(references[1].as_str(), "parent@example.com");
assert_eq!(message.message_id().unwrap().unwrap().as_str(), "reply@example.com");
```

Setters apply `MessageId::from_value` before changing any headers. Single-ID
setters preserve original bytes when the normalized ID equals the current getter
result and no duplicate headers exist. Resent-Message-Id also preserves duplicates
on equal assignments. Otherwise they replace all occurrences at
the first slot, or append if absent, using angle brackets, UTF-8, and CRLF.
Malformed old values are replaced. Single IDs are never folded or truncated,
and no identifier is generated. `set_in_reply_to(None)` removes all occurrences;
other identifiers can be removed through the raw header list.

`set_references` accepts an iterator of strings, validates the entire replacement
before editing, and always reformats nonempty input. Order and duplicate IDs
remain, but duplicate References headers collapse at the first slot. An empty
sequence removes all occurrences. Folds occur only between IDs at a soft
78-byte target, counting UTF-8 bytes, brackets, and indentation. The first ID
stays on the field line, and an oversized ID remains intact even when it exceeds
the target. This is not the strict line-limit enforcement of address/content
header formatters. Errors leave all headers unchanged, including when a later
ID is invalid; their offsets refer to that input string.

These APIs do not synchronize In-Reply-To with References, construct reply or
resent messages, generate IDs, sanitize existing raw headers, or prepare mail
for transport. Assignment validation retains MimeKit's compatibility grammar,
not strict RFC constraints; callers remain responsible for transport suitability.

### Message importance and priority

`MessageImportance`, `MessagePriority`, and `XMessagePriority` model the
independent Importance, Priority, and X-Priority headers. Each provides `parse`,
`FromStr`, `as_str`, `Display`, and a canonical CRLF `to_header`. Their default
is Normal; X-Priority also provides `number()` from 1 (Highest) to 5 (Lowest).

```rust
use mimecraft::{MessageImportance, MessagePriority, MimeMessage, XMessagePriority};

let mut message: MimeMessage = MimeMessage::new();
message.set_priority(MessagePriority::Normal);
assert!(message.headers().is_empty());
message.set_importance(MessageImportance::High);
message.set_priority(MessagePriority::Urgent);
message.set_x_priority(XMessagePriority::High);
assert_eq!(message.priority().unwrap(), MessagePriority::Urgent);
assert_eq!(message.x_priority().unwrap().number(), 2);
```

`importance()`, `priority()`, and `x_priority()` read only the first matching
message header on every call. Missing fields yield Normal, while invalid
present values return `MessagePriorityParseError` rather than MimeKit's Normal
fallback. Importance and Priority use `Header::decode_value`, then trim Unicode
whitespace and compare complete tokens ignoring ASCII case. Supported tokens
are `low`/`normal`/`high` and `non-urgent`/`normal`/`urgent`, respectively.
Comments and unknown tokens are errors. Decode the raw header directly for
charset-recovery diagnostics; standalone enum parsing does not decode words.

X-Priority reads raw bytes instead: skip space/tab/CR/LF, parse an unsigned
ASCII decimal prefix in `0..=i32::MAX`, then clamp to 1 through 5. Remaining
bytes are ignored, even malformed comments or non-UTF-8; leading signs,
comments, encoded words, missing digits, and numeric overflow are errors.

Setters compare the freshly parsed value, not a cached field. Without duplicates,
equal values leave raw bytes untouched, including Normal on an absent
header and Highest on a raw `0`. Duplicate, changed, or invalid values are replaced
at the first slot, later duplicates removed, and missing non-Normal values appended.
New X-Priority values include comments such as `2 (High)`. To materialize an
explicit Normal field even when absent, use the enum's `to_header` with
`HeaderList::replace_all`; remove raw occurrences to restore absence.
No setter synchronizes another priority header, examines body metadata, or
accesses content streams. These conveniences require no `Read`/`Seek` bounds.

### Body parts and attachments

`MimeMessage` and `MimeEntity` expose three lazy, depth-first traversals:

| Method | Returned items | Attached messages |
| --- | --- | --- |
| `body_parts()` | `&MimeEntity<R>` for non-multipart nodes | Returns the wrapper, does not enter its message |
| `attachments()` | `Result<&MimeEntity<R>, ContentHeaderParseError>` | Filters the wrapper's own disposition |
| `leaf_parts()` | `&MimePart<R>` | Descends into the message's body |

`BodyParts`, `Attachments`, and `LeafParts` use an explicit stack and implement
`FusedIterator`. They perform no source I/O and require no `Read`/`Seek` bounds.
Body/leaf traversal does not parse metadata. An absent message body and empty
multipart contribute no body parts; an attached-message wrapper contributes
one body part even when its contained message is absent.

```rust
use mimecraft::{MessagePart, MimeEntity, MimeMessage, TextPart};

let inner = MimeMessage::with_body(TextPart::new("plain", "attached").unwrap().into());
let mut part = MessagePart::new(inner);
part.set_is_attachment(true).unwrap();
let message = MimeMessage::with_body(part.into());
let attachments = message.attachments().collect::<Result<Vec<_>, _>>().unwrap();
assert_eq!(attachments.len(), 1);
assert!(matches!(attachments[0], MimeEntity::Message(_)));
assert_eq!(message.leaf_parts().count(), 1);
```

`is_attachment()` on `MimeEntity`, `MimePart`, `Multipart`, and `MessagePart`
parses only that node's first Content-Disposition
and compares its disposition with `attachment`, ignoring ASCII case. Missing
disposition is false; filenames, media types, message-level headers, and inner
attachments do not imply attachment status. Raw header edits are immediately
visible, and later duplicates do not override the first record.

`set_is_attachment(true)` creates a missing disposition or changes the first
disposition to `attachment`, retaining its decoded parameters.
`set_is_attachment(false)` changes an attachment to `inline`; it does not remove
the header, create a missing one, or replace non-attachment extension tokens.
Text parts expose these methods through `part()`/`part_mut()`.

When the first header already has the requested flag, every raw record,
including duplicate dispositions, is preserved. For an actual flag change,
checked formatting occurs before replacing the first matching slot and removing
duplicates. Filenames, dates, sizes, and extension parameters from the first
disposition survive; comments, original folds, and malformed fragments skipped
by tolerant parsing do not. The `_with_options` setter accepts
`HeaderFormatOptions`, checked only when a changed header needs formatting.
No-op assignments ignore invalid formatting options.

`AttachmentEditError::ContentDisposition` reports a malformed first header;
`AttachmentEditError::HeaderFormat` reports a formatting failure. Neither error
modifies headers or sources, and later valid duplicates are not used as fallback.
Setters require no source I/O traits and do not modify nested entities or infer
filenames, encodings, or media types. Selection and attachment enumeration
observe the edited raw headers on their next call.

Malformed dispositions produce errors, not silent skips or attachment
fallbacks. `Attachments` yields the error and can resume at the next body part;
collecting into `Result<Vec<_>, _>` stops at the first failure. Multipart nodes
themselves are excluded from both body-part and attachment enumeration, but
their children are visited regardless of the container's disposition.
Attached-message contents are not inspected; enumerate the contained
`MimeMessage` explicitly when needed.

### MIME tree navigation

`MimeMessage::iter()` returns a `MimeIterator`, which implements
`Iterator<Item = &MimeEntity<R>>` and visits every entity in depth-first order.
Unlike `body_parts()`, it returns multipart containers and enters attached
messages. Empty containers and message wrappers without bodies are still
returned. Navigation follows the stored enum variants, not Content-Type
headers, and never reads or seeks content sources.

After `next()`, query `current()`, `parent()`, `depth()`, `index()`,
`current_message()`, `ancestor(generation)`, or `path_specifier()`. The root has
depth zero and no parent. `index()` is the zero-based child index only when the
parent is a multipart. A wrapper belongs to its outer message; its body belongs
to the wrapped message. Before iteration or after exhaustion, position queries
return `None`. An ancestor generation of zero or beyond the depth also returns
`None`.

Part paths use MimeKit's numbering: multipart children start at `1`, and a
non-multipart message body is `1`. A multipart message body has no IMAP part
number; MimeKit's `0` extension identifies it. Thus a root multipart is `0`,
an attached-message wrapper might be `3`, its multipart body `3.0`, and that
body's children `3.1` and `3.2`. Ordinary nested multiparts retain their child
number. Zero cannot precede another component.

```rust
use mimecraft::{MessagePart, MimeMessage, Multipart, TextPart};

let inner = MimeMessage::with_body(TextPart::new("plain", "hello").unwrap().into());
let mut mixed = Multipart::with_boundary("mixed", "example-boundary").unwrap();
mixed.children_mut().push(MessagePart::new(inner).into());
let message = MimeMessage::with_body(mixed.into());
let mut iter = message.iter();
let mut paths = Vec::new();
while iter.next().is_some() {
    paths.push(iter.path_specifier().unwrap());
}
assert_eq!(paths, ["0", "1", "1.1"]);
assert!(message.entity_at("1.1").unwrap().is_some());
assert!(message.entity_at("2").unwrap().is_none());
```

`skip_children()` skips the current subtree on the next advance and reports
`MimeIteratorPositionError` if there is no current entity. `reset()` restarts
before the first entity and cancels a pending skip. Ordinary `next()` calls
remain exhausted after returning `None`.

`move_to(path)` navigates directly without enumerating unrelated siblings.
It returns `Ok(true)` at the target, with subsequent iteration continuing
**after** that entity. `Ok(false)` means a valid but nonexistent path and leaves
the iterator exhausted. Both outcomes cancel pending skips; navigation can
move backwards or restart an exhausted iterator.

Paths accept only dot-separated ASCII decimal components in `0..=i32::MAX`,
including leading zeros. Invalid syntax or overflow produces
`MimePathParseError` with a byte offset, leaving the position and pending skip
unchanged. `MimeMessage::entity_at(path)` shares these rules and returns
`Result<Option<&MimeEntity<R>>, MimePathParseError>` rather than conflating
malformed and missing paths.

Traversal uses a depth-proportional frame stack, without recursion. Path
formatting allocates only when requested; navigation validates the complete
path before editing iterator state. The tree remains immutably borrowed, so
returned references survive iterator advancement but tree edits during
traversal are not supported. Parsing, body decoding, and mutable cursors are
separate concerns; ordinary model destruction still recurses.

### Building common message bodies

`BodyBuilder` assembles existing in-memory entities with plain text and HTML
into common MIME layouts. Set its public `text_body`, `html_body`,
`body_encoding`, `linked_resources`, and `attachments` fields, then consume it
with `into_message_body()`. `new()` and `default()` start empty with UTF-8.

```rust
use mimecraft::{BodyBuilder, MimeMessage, TextFormat};

let builder = BodyBuilder {
    text_body: Some("hello".into()),
    html_body: Some("<p>hello</p>".into()),
    ..BodyBuilder::new()
};
let mut message = MimeMessage::with_body(builder.into_message_body().unwrap());
assert_eq!(message.text_body(TextFormat::Plain).unwrap().as_deref(), Some("hello"));
assert_eq!(message.text_body(TextFormat::Html).unwrap().as_deref(), Some("<p>hello</p>"));
```

Plain text precedes HTML in multipart/alternative. Linked resources wrap HTML
in multipart/related, with HTML first, `type=text/html`, and no `start`
parameter. Attachments follow the body in multipart/mixed. All multipart
boundaries are generated, and resource/attachment insertion order is retained.
A lone attachment without either text body is returned directly. With no text
or attachments, an empty text/plain body is produced in the selected charset.
`Some("")` counts as a supplied body, unlike `None`. Linked resources without
HTML are ignored and dropped with the consumed builder.

Both text bodies use `body_encoding` with strict charset conversion; unencodable
characters return `BodyBuilderError::Text` rather than replacement characters.
Text, newlines, markup, and resource references are stored literally. There is
no HTML rendering, resource discovery, or automatic transport preparation.

The collections are `Vec<MimeEntity>` using the default `Cursor<Vec<u8>>`
storage. Entities are moved without copying or reading their content; headers,
stored encoding, parsed framing, and source positions remain unchanged. Supply
filenames, dispositions, Content-Ids, and Content-Locations explicitly before
building, or use the standalone `attachment` helpers below.
Construction errors include formatting and OS-randomness failures and retain
their underlying causes. On failure, the consumed builder and its entities
are dropped rather than returning a partial body.

### Constructing attachments and linked resources

`attachment::from_bytes`, `from_reader`, and `from_file` return owned in-memory
entities for ordinary vectors, including `BodyBuilder`'s collections. Their
`_with_options` counterparts accept `&AttachmentOptions` to override the inferred
Content-Type, mark a linked resource, or configure input/message-parser limits.

```rust
use mimecraft::{AttachmentOptions, BodyBuilder, attachment};

let mut builder = BodyBuilder {
    html_body: Some("<p>Report</p><img src=\"logo.png\">".into()),
    ..BodyBuilder::new()
};
builder.attachments.push(attachment::from_bytes("report.txt", b"report\r\n").unwrap());
let inline = AttachmentOptions {
    linked_resource: true,
    ..AttachmentOptions::default()
};
builder.linked_resources.push(
    attachment::from_file_with_options("logo.png", &inline).unwrap()
);
let body = builder.into_message_body().unwrap();
```

MIME types are inferred from the final filename extension using `MimeTypes`;
unknown extensions become application/octet-stream. Both slash styles are
recognized in filename metadata. The Content-Type `name` and disposition
`filename` contain only the basename. Linked resources receive inline disposition
and an escaped relative Content-Location; other attachments receive attachment
disposition. Content-Ids are never generated. Literal URI syntax and UTF-8
filename bytes are percent-encoded; an escaped location may contain at most
997 bytes so its folded header respects the 998-byte physical-line limit.
Empty, whitespace-only, dot-component, or control-containing names are rejected.
File paths require a Unicode filename.

Text attachments use the existing seven-bit encoding heuristic with a 78-column
content-line threshold; other opaque types request Base64. Stored bytes remain
unencoded, with no charset guessing or transcoding. Explicit Content-Type
parameters are retained except `name`. A message/rfc822 attachment instead parses
into a fresh `MessagePart` with filename metadata and no transfer-encoding header.
Its supplied type parameters are not retained. Unlike MimeKit's inferred-message
fallback, malformed messages return parsing errors; explicitly select
application/octet-stream to retain them as opaque data.

All inputs are copied and bounded by `AttachmentOptions::parse_options.max_input_bytes`
(64 MiB by default); other parser limits apply to embedded messages. All numeric
limits and metadata are validated before I/O. Borrowed readers need only `Read`,
start at their current position, and are not closed or rewound. Limit detection
consumes at most one excess byte, while an exact-limit input still probes EOF.
Parsing occurs after the bounded copy. Errors return no partial entity and retain
their causes; file handles are closed before return. Input budgets do not bound
decoded payloads or total allocation overhead. Async loading, streaming-backed
attachments, and collection wrappers are not provided.

### Text-body selection

`MimeMessage` and `MimeEntity` provide `text_body_part(format)` and
`text_body_part_mut(format)`, returning `Result<Option<&MimePart<R>>,
TextBodyError>` and its mutable-borrow equivalent. `TextFormat` supports
`Plain`, `Flowed`, `Html`, `Enriched`, and `RichText`. Selection borrows the
original part without source I/O or `Read`/`Seek` bounds; it never enters
attached messages.

| Container | Selection rule |
| --- | --- |
| `multipart/alternative` | Last suitable child first, descending nested containers |
| `multipart/related` | Only the root selected by `start`, or by `type` when `start` is absent; unmatched selectors fall back to the first child |
| Other multipart types | First multipart or first non-attachment text child, stopping even when it has no body in the requested format |
| Root leaf | Matching text that is not an attachment |

These are MimeKit's selection rules, not a search through all text leaves.
In particular, alternative children and related roots are **not** excluded
by their disposition, and container dispositions do not suppress traversal.
Related Content-Ids are case-sensitive. The first bracketed reference in
`start` is normalized; bare `start` values are matched literally. Content-Id
headers use the existing prefix parser. No URI escaping or resolution is
performed. A missing Content-Type means text/plain for ordinary leaves and
message/rfc822 for immediate digest children, without changing raw headers.

```rust
use mimecraft::{MimeMessage, Multipart, TextFormat, TextPart};

let mut alternative = Multipart::with_boundary("alternative", "alt").unwrap();
alternative.children_mut().push(TextPart::new("plain", "hello").unwrap().into());
alternative.children_mut().push(TextPart::new("html", "<b>hello</b>").unwrap().into());
let mut message = MimeMessage::with_body(alternative.into());
let html = message.text_body_part_mut(TextFormat::Html).unwrap().unwrap();
let mut decoded = Vec::new();
html.content_mut().decode_to(&mut decoded).unwrap();
assert_eq!(decoded, b"<b>hello</b>");
```

The returned content remains transfer-encoded until explicitly decoded.
Selection does not decode charsets, reflow flowed text, convert rich text,
render/sanitize HTML, or author headers. `Plain` includes flowed text;
`Enriched` includes text/enriched and text/richtext; `RichText` includes
text/rtf and application/rtf. `TextPart::from_part` continues to require an
explicit text/* Content-Type; body selection does not relax that API.

Only metadata needed by the search is parsed, using first-header semantics.
Malformed metadata returns `TextBodyError` instead of trying another
candidate. Multipart nodes require a valid multipart Content-Type, but
selection does not validate boundaries, charsets, or transfer encodings.
Raw edits are visible immediately. Both lookup and mutable path resolution
are iterative, with auxiliary storage proportional to nesting depth.

### Alternative multipart bodies

`MultipartAlternative<R>` composes `Multipart<R>` and selects the most faithful
body in a requested `TextFormat`. `new()` generates the boundary;
`with_boundary(boundary)` accepts an explicit value. `from_multipart` checks
only the first Content-Type, without validating boundaries or children,
repairing headers, or accessing content.

```rust
use mimecraft::{MultipartAlternative, TextFormat, TextPart};

let mut alternative: MultipartAlternative = MultipartAlternative::new().unwrap();
alternative.multipart_mut().children_mut().extend([
    TextPart::new("plain", "hello").unwrap().into(),
    TextPart::new("html", "<p>hello</p>").unwrap().into(),
]);
assert_eq!(alternative.text_body(TextFormat::Plain).unwrap().as_deref(), Some("hello"));
assert_eq!(alternative.text_body(TextFormat::Html).unwrap().as_deref(), Some("<p>hello</p>"));
```

`text_body_part(format)` borrows the selected leaf without source trait bounds
or I/O. Alternatives are searched last-to-first, including attached text parts;
nested containers follow their own selection rules. Related containers search
only their root, and mixed containers stop at their first body candidate.
Attached messages are not entered. A later flowed alternative also matches Plain.

`text_body(format)` requires `Read + Seek` and uses the shared transfer, charset,
and flowed decoding described below. Only the selected source is read.
Missing formats return `None`; empty bodies return `Some("")`. Invalid visited
metadata and decoding errors propagate without falling back to another body.
Reads preserve headers, stored bytes, and parsed framing, including truncated
containers. `multipart_mut()` exposes ordinary editing APIs; subsequent body
access rechecks the container's first Content-Type. Conversion to `MimeEntity`
retains its ordinary multipart variant.

### Related multipart roots

`MultipartRelated<R>` composes `Multipart<R>`, generating a boundary in `new()`
or accepting an explicit value in `with_boundary(boundary)`. Generation uses
`Multipart::new` and wraps entropy errors in `MultipartRelatedError::Multipart`;
neither constructor creates children, IDs, or type/start parameters.
Checked wrapping through `from_multipart` checks
only the first Content-Type for multipart/related; it does not require a valid
boundary, selectors, or children. Loading and conversion keep the ordinary
`MimeEntity::Multipart` variant. No source traits or I/O are required.

```rust
use mimecraft::{MimeEntity, MultipartRelated, TextFormat, TextPart};

let mut related: MultipartRelated = MultipartRelated::new().unwrap();
related.set_root(TextPart::new("html", "<p>Root document</p>").unwrap().into()).unwrap();
assert_eq!(related.root_index().unwrap(), Some(0));
let entity: MimeEntity = related.into();
assert!(entity.text_body_part(TextFormat::Html).unwrap().is_some());
```

`root_index()`, `root()`, and `root_mut()` share the text-body selector's rules:
start takes precedence; type is considered only when start is absent; unmatched
selectors fall back to the first child. An empty container has no root unless
invalid visited metadata produces an error. IDs are case-sensitive, type
matches ignore ASCII case, and only immediate children are considered.
Malformed visited metadata fails rather than silently selecting another root.
Raw child edits through `root_mut()` do not synchronize parent parameters.

`set_root(entity)` replaces a child **matched by the existing selectors**,
returning the old entity. If no selector matches, it inserts at index zero and
returns `None`, rather than replacing the getter's fallback child. Empty
containers insert at zero without interpreting stale selectors. Neither
operation copies or reads body streams.

The new root must have an explicit valid first Content-Type, whose media type
becomes the parent's type parameter. A first-child root removes start to
preserve MimeKit's Thunderbird compatibility workaround, without inspecting
the new root's Content-Id. A non-first replacement uses its valid existing
Content-Id or generates and assigns one with `MessageId::generate()`, storing
its bracketed ID in start. Set an ID beforehand to choose an explicit domain.
Malformed existing IDs are errors, not reasons to generate replacements.
Other child headers are unchanged; duplicate IDs are not rejected, and subsequent
lookup picks the first matching child.

`set_root_with_options` accepts `HeaderFormatOptions`. Both setters validate
the existing boundary and serialize the complete parent Content-Type before
editing container headers or children. They collapse Content-Type duplicates at the
first slot while preserving other parameters, unrelated headers, other
children, preamble, epilogue, and source positions. Metadata, ID generation, and
formatting errors leave the container unchanged; the supplied root is dropped on failure,
following ownership-taking API conventions. Borrowed sources stay open.

`index_of_content_id(&MessageId)` returns the first matching immediate-child
index; `get_by_content_id(&MessageId)` borrows that child. Both return `None`
when no child matches and perform no source I/O. These port the Content-Id
branch of MimeKit's resource lookup using a parsed ID, not a `cid:` URI.
Matching is case-sensitive and shares root selection's first-header parser.
Missing IDs are skipped; malformed visited IDs return
`MultipartRelatedError::ContentId`, without falling back to duplicate headers
or later children. Children after a match are not examined.

Lookup checks the container's first Content-Type but does not interpret root
selectors, validate boundaries, inspect child media types or dispositions, or
descend into children. Matches may be leaf parts, multiparts, or attached-message
wrappers. Headers, parsed framing, and source positions are preserved.

`open_by_content_id(&MessageId)` opens an incremental decoded `Read`/`BufRead`
view of the first matching leaf resource. It ports MimeKit's resource `Open`
without URI parsing. Missing IDs and first matches that are multipart or
attached-message wrappers return `MultipartRelatedError::ResourceNotFound`;
a later matching leaf does not replace that first match. Empty leaf bodies
open successfully. Lookup errors occur before any source I/O.

Opening rewinds only the selected source and delegates to `MimeContent::open`,
using its stored transfer encoding rather than the current header. It does
not parse the leaf's Content-Type, convert charsets, normalize newlines, or
buffer the entire body. Use `open_by_content_id_with_metadata` when media-type
or charset metadata is needed together with the reader. The returned reader
exclusively borrows the container; dropping it does not rewind, and opening
again restarts with fresh decoder state. Initial rewind errors are wrapped in
`MultipartRelatedError::Io`; later read errors come from the reader.
Headers, encoded bytes, parsed boundaries, and missing separators are preserved.

`index_of_uri(&str)`, `get_by_uri(&str)`, and `open_by_uri(&str)` provide the
corresponding URI-based operations, using `fluent-uri` internally rather than
requiring a public URI type. A case-insensitive `cid:` scheme matches Content-Id
only: the path is percent-decoded once (without converting `+` to space),
validated as a bare `MessageId` without header normalization, and compared
case-sensitively. CID references with an authority, query, or fragment are
rejected; escape ID characters instead.

Other references match the first Content-Location header after ordinary header
decoding and unfolding. Relative references match relative locations literally.
For absolute references, relative locations are resolved against the child's
first Content-Base or, when absent, the container's first Content-Base. Without
either base, that relative location cannot match. Absolute locations and
relative-only comparisons do not consult Content-Base. Bases must be absolute
URIs without fragments.

Location comparison is byte-for-byte, not .NET's URI equality: scheme/host case,
explicit ports, percent-escape spelling, queries, and fragments are significant.
Resolution uses `fluent-uri`'s dot-segment handling, including encoded dot
segments, without additional normalization. This ports MimeKit's lookup/base
precedence, not every .NET URI equivalence or the full MHTML base-discovery
algorithm. Raw headers remain authoritative; author Content-Location and
Content-Base through the existing `HeaderList` API.

Malformed input, visited locations, and required bases return explicit
`MultipartRelatedError` variants. Invalid first headers never fall back to
duplicate headers or later matches. URI opening preserves the same lazy decoding
and parsed framing as ID opening; missing or non-leaf matches return
`MultipartRelatedError::UriResourceNotFound` containing the requested reference.
No HTML base discovery, network fetching, or recursive resource search occurs.

`contains_uri(&str)` and `contains_content_id(&MessageId)` expose the same lookup
as a boolean, including non-leaf matches. Errors remain errors, not false results.
`open_by_uri_with_metadata` and `open_by_content_id_with_metadata` return
`RelatedResource`, pairing an owned `content_type` with a borrowed decoded
`reader`. Use `content_type.mime_type()` and `content_type.charset()` for MimeKit's
metadata outputs. Missing Content-Type defaults to text/plain without inserting
a header. Malformed first Content-Type values fail before source I/O; declared
charsets, even unknown or empty ones, are returned without conversion.

`MultipartRelated::text_body_part(format)` searches only the selected root,
including an attached text root, and follows nested multipart roots and
alternatives using the shared body selector. Missing formats return `None`;
visited metadata errors propagate. This requires no source traits or source I/O.

### Decoded body text and flowed conversion

`MimeMessage::text_body(format)` and `MimeEntity::text_body(format)` return
`Result<Option<String>, TextBodyError>`. They use the same selection rules as
`text_body_part`, then read only the selected source. No match returns `None`;
an empty matching body returns `Some(String::new())`.

Transfer decoding uses the stored content encoding, not a newly edited
Content-Transfer-Encoding header. Charset decoding reuses `TextPart::text()`'s
automatic MIME/BOM/HTML detection and recovery, including replacement and
Latin-1 fallback. Implicit text/plain bodies retain their default type without
adding headers. Selection errors happen before source I/O.
Successful byte copying rewinds the source before charset decoding;
read/seek failures follow `MimeContent::decode_to`'s existing behavior.
Failures do not trigger fallback to an earlier alternative.

For `text/plain; format=flowed`, both `Plain` and `Flowed` requests additionally
unwrap paragraphs using `text::FlowedToText`. A `delsp=yes` parameter enables
deletion of one soft-break space; comparison ignores ASCII case but does not
trim the value. Other formats preserve decoded newlines, BOMs, and markup.
No HTML sanitization or rich-text rendering is performed.

```rust
use mimecraft::{MimeMessage, TextFormat};

let mut input = &b"Content-Type: text/plain; format=flowed\r\n\r\nhello \r\nworld"[..];
let mut message = MimeMessage::load(&mut input).unwrap();
assert_eq!(message.text_body(TextFormat::Plain).unwrap().as_deref(),
           Some("hello world\r\n"));
```

`text::FlowedToText` also operates independently on complete Unicode strings.
It removes one stuffing space, joins soft lines, and lets quote-depth changes
start a new paragraph. Set its `delete_space` field to control soft-break space
removal. `new()` emits CRLF on every platform; `with_new_line_format(Unix)`
selects LF and `Mixed` is rejected. Input CRLF, LF, and bare CR are accepted.
Hard paragraphs receive an output newline even for an unterminated final
input line. A nonempty pending flowed paragraph at EOF has no final newline.

The converter deliberately preserves MimeKit's existing signature-separator
behavior: `-- ` is treated as a soft line, not a special hard separator.
It is not a strict RFC 3676 validator. `TextPart::text()` remains unchanged
and returns decoded text without reflowing it. Use borrowed body-part access
for raw flowed formatting, explicit charset overrides through existing APIs,
or a different conversion policy.

These convenience operations materialize decoded bytes, Unicode text, and
any converted text in memory. They do not change raw headers or stored bytes.
Read-only decoding also preserves parsed multipart delimiters and missing
header/body separators; public mutable body-part access remains an editing API.
Streaming text converters and automatic HTML/RTF conversion remain deferred.

### Copying and decorating plain text

`text::TextToText` copies a complete Unicode string verbatim, optionally
prepending its `header` and appending its `footer`. Both fields are
`Option<String>` and default to `None`. Empty decorations behave like absent
ones; nonempty decorations are emitted even for an empty input.

```rust
use mimecraft::text::TextToText;

let converter = TextToText {
    header: Some("Header\r\n".into()),
    footer: Some("\r\nFooter".into()),
};
assert_eq!(converter.convert("body"), "Header\r\nbody\r\nFooter");
assert_eq!(TextToText::new().convert("body"), "body");
```

No separators, final newline, escaping, or whitespace normalization are added.
CRLF, LF, bare CR, Unicode scalars, controls, and markup remain unchanged in
all three pieces. Headers and footers here are plain text, not MIME metadata.
Conversion leaves the input and converter unchanged; it does not select MIME
bodies, decode charsets, or provide streaming adapters.

### Generating flowed text

`text::TextToFlowed` wraps a complete Unicode string for use as
`text/plain; format=flowed; delsp=yes`. It prefers word boundaries, repeats
normalized quote prefixes, and space-stuffs lines starting with space,
`From `, or `>` after any quote markers. Soft breaks add a trailing space;
original separator spaces remain, so decoding must enable `delsp=yes`.
The converter does not set MIME headers or change existing parts.

```rust
use mimecraft::{TextPart, text::TextToFlowed};

let flowed = TextToFlowed::new().convert("From someone\n> quoted text");
assert_eq!(flowed, " From someone\r\n> quoted text\r\n");
let mut part = TextPart::new("plain", &flowed).unwrap();
let mut content_type = part.part().content_type().unwrap().unwrap();
content_type.parameters_mut().set("format", "flowed").unwrap();
content_type.parameters_mut().set("delsp", "yes").unwrap();
part.part_mut().set_content_type(&content_type).unwrap();
```

Lines target 78 UTF-8 bytes including quote markers, stuffing, and soft-break
spaces, but not the newline. Words can exceed that target up to 998 bytes
before splitting; splits never divide a UTF-8 character, but may divide a
grapheme cluster. Both limits expand to at least twice the quote/stuffing prefix
length for deeply quoted input, preserving MimeKit's protection against output
growth proportional to quote depth times word count. Unlike MimeKit's UTF-16
character counts, these are UTF-8 byte lengths before transfer encoding, not
universal wire limits for other charsets or deeply quoted messages.

Input CRLF, LF, and bare CR are accepted. `new()`/`default()` emit CRLF;
`with_new_line_format(Unix)` emits LF and `Mixed` is rejected. Every input line
gets an output newline, including an unterminated final line; empty input stays
empty. Leading quote markers may have one space between them and are normalized
to consecutive `>` characters followed by one space. All trailing ASCII spaces
are trimmed before wrapping, including the space in `-- ` signature separators.
Quote-only lines retain a trailing stuffing space as in MimeKit. This is not a
lossless transformation or strict RFC validation. No stream I/O, charset
conversion, or header/footer insertion occurs.

### Plain-text previews

`text::PlainTextPreviewer` generates a short preview of already-decoded plain
text. It collapses Unicode whitespace plus U+200B through U+200D into single
ASCII spaces and removes leading and trailing spaces. `new()`/`default()` use
a maximum length of 230; `with_maximum_preview_length` accepts 1 through 1024
and returns `text::InvalidPreviewLength` outside that range.

```rust
use mimecraft::text::PlainTextPreviewer;

let previewer = PlainTextPreviewer::with_maximum_preview_length(12).unwrap();
assert_eq!(
    previewer.preview(" \tHello\r\nworld!  More"),
    "Hello world\u{2026}"
);
assert_eq!(previewer.preview("  short  "), "short");
```

If input remains when the normalized output reaches the limit, the final
retained character is replaced with U+2026 (ellipsis). As in MimeKit's string
overload, trailing-space removal happens first: a truncated preview can be
shorter than the limit, and unread whitespace alone still triggers an ellipsis.
For example, a limit of 3 converts `"ab cd"` to `"a\u{2026}"`, not `"ab\u{2026}"`.
Truncation does not prefer word boundaries.

The limit counts Unicode scalar values, not UTF-8 bytes or MimeKit's UTF-16
code units, so supplementary characters remain intact. Grapheme clusters can
still be split, and the inherited treatment of zero-width joiners as whitespace
also applies inside emoji sequences. The output is bounded by the configured
limit without first normalizing the entire input; long whitespace runs may
still need to be scanned.

The previewer performs no HTML rendering or escaping, flowed-text unwrapping,
legacy charset decoding, or MIME body selection. Use
`MimeMessage::text_body(TextFormat::Plain)` for explicit decoded body selection
and flowed conversion before previewing, or pass an already-decoded string.

`preview_reader(&mut reader)` accepts a `BufRead` UTF-8 source at its current
position and uses the same normalization and truncation rules as `preview`.
Unlike MimeKit's separate reader algorithm, it also normalizes U+200B through
U+200D and trims before replacing the final character. A UTF-8 BOM is preserved
as U+FEFF rather than stripped.

```rust
use mimecraft::text::PlainTextPreviewer;

let previewer = PlainTextPreviewer::with_maximum_preview_length(5).unwrap();
let mut input = b"Hello world".as_slice();
assert_eq!(previewer.preview_reader(&mut input).unwrap(), "Hell\u{2026}");
assert_eq!(input, b" world");
```

The reader path consumes complete Unicode scalars until the output reaches its
limit or EOF, then peeks without consuming an unread suffix. Preview storage is
bounded; long whitespace runs may still require scanning the whole source.
Use `BufReader` for an unbuffered source and continue through the same reader
after previewing to retain any underlying read-ahead. No seeking, draining, or
closing occurs. Invalid or incomplete UTF-8 in the scanned prefix returns
`InvalidData`; unread suffixes are not validated. I/O failures, including
lookahead failures, propagate without a partial preview; interruptions retry.
The standalone previewer does not detect charsets/BOMs or provide
charset-selecting overloads. `text::TextPreviewer` supplies MIME-aware
decoding and preview dispatch.

### MIME-part previews

`text::TextPreviewer` ports `TextPreviewer.GetPreviewText(TextPart)` as a
separate reusable type, not a method on `TextPart`. Its `preview(&mut body)`
method reads at most the first 16 KiB of transfer-decoded content, then selects
`HtmlTextPreviewer` for
`text/html` and `PlainTextPreviewer` for every other text subtype.
The default limit is 230 scalars; `with_maximum_preview_length` accepts 1 through
1024, consistently with the individual previewers. HTML still requires an
explicit body element; plain/flowed/enriched/RTF inputs are not converted
between formats.
Message-body selection is separate.

The stored content encoding determines transfer decoding, independently of
the Content-Transfer-Encoding header. A supported declared charset is honored
with replacement decoding. With a missing/unsupported charset, UTF-16LE/BE or
UTF-8 BOMs select the encoding; otherwise strict UTF-8 is attempted.
Invalid UTF-8 encountered while previewing retries the original prefix as
Latin-1. UTF-16 selected by a BOM uses replacement, matching the source.
Only a matching initial preamble is removed: a conflicting BOM does not override
a supported declaration, and Latin-1 replay includes the original BOM bytes.
No HTML meta-charset sniffing or UTF-32 detection is added.

The existing Rust charset mappings and uniform U+FFFD replacement apply,
rather than .NET's charset-dependent replacement defaults. Plain previews retain
the library's shared string/reader normalization and scalar-safe truncation.
Undeclared UTF-8 validation follows the existing preview readers rather than
.NET's character-buffer read-ahead; an unneeded invalid suffix may never be
examined, but completing an HTML token can still encounter it.
A 16-KiB cutoff splitting a character uses the same replacement/fallback
rules as malformed input. The prefix cap does not itself append an ellipsis.

Metadata is checked before I/O, and invalid/missing/non-text Content-Type
still returns `TextPartError`. Input is reopened from the beginning with fresh
transfer-decoder state on each call. The complete bounded prefix is collected
before previewing, so early text truncation cannot hide a source read error.
Short reads and interrupted reads are handled; seek/read failures propagate
without partial output. No final rewind or close occurs. Encoded source
read-ahead can exceed the decoded byte limit, and encoded whitespace or other
filter-discarded input is not subject to that limit.
Headers, body bytes, and parsed framing are unchanged.

Previewing does not alter whole-text decoding or headers: `text()` has its own
automatic detection/recovery and `text_with_charset()` remains strict.
Preview output is plain text and needs
escaping before HTML embedding. Charset-selecting standalone preview readers
and automatic message-body preview selection remain deferred.

All applicable `TextPreviewerTests.cs` cases are ported: four original document
fixtures and the three unknown-charset recovery cases. The new plain-text
Planet Fitness fixture is copied byte-for-byte; the HTML fixtures are reused.
Additional cases cover BOM precedence/replay, supported legacy charsets,
Unicode limits, encoded-versus-decoded cutoffs, split characters, all transfer
decoders, errors, reuse, and preservation of parsed framing and text getters.
Upstream contains no text-preview benchmarks to port.

### HTML text and attribute encoding

`text::html_encode` escapes plain text for HTML character data.
`text::html_attribute_encode` encodes an attribute value and includes its
surrounding quotes. `text::HtmlAttributeQuote` selects `Double` (also its
default) or `Single`; invalid quote characters cannot be supplied.

```rust
use mimecraft::text::{HtmlAttributeQuote, html_attribute_encode, html_encode};

assert_eq!(html_encode("<b>caf\u{e9}</b>"), "&lt;b&gt;caf&#233;&lt;/b&gt;");
assert_eq!(
    html_attribute_encode("a \"quote\" & 'another'", HtmlAttributeQuote::Double),
    "\"a &quot;quote&quot; &amp; 'another'\""
);
assert_eq!(
    html_attribute_encode("it's", HtmlAttributeQuote::Single),
    "'it&#39;s'"
);
```

Both functions escape ampersands and angle brackets. Text encoding also
escapes both quote characters; attribute encoding escapes only its delimiter
and leaves the other quote literal. Tabs, CR, LF, and form feed are preserved
without newline normalization. Other C0 controls and U+007F through U+009F are
dropped, matching MimeKit rather than providing a lossless transformation.
Characters from U+00A0 onward become decimal numeric references, making the
result ASCII. Supplementary characters produce one reference per Unicode
scalar; Rust strings cannot contain MimeKit's unpaired UTF-16 surrogates.

Existing references are escaped again, not parsed: `"&amp;"` becomes
`"&amp;amp;"`. These helpers do not sanitize HTML, validate URL schemes or
attribute names, escape JavaScript/CSS, add document markup, or perform
stream I/O. Encoding an event handler or URL attribute does not make its
meaning safe. `text::HtmlWriter` provides separate in-memory markup construction;
`text::HtmlToHtml` handles HTML rewriting with callbacks, and `text::HtmlToText`
handles plain-text rendering of HTML input.

### Writing HTML in memory

`text::HtmlWriter` builds HTML in an owned `String`, using MimeKit's tag and
attribute state transitions and the encoding helpers above.

```rust
use mimecraft::text::{HtmlWriter, HtmlWriterError, HtmlWriterState};

let mut html = HtmlWriter::new();
html.write_start_tag("a")?;
assert_eq!(html.state(), HtmlWriterState::Tag);
html.write_attribute("href", "https://example.com/?a=1&b=2")?;
html.write_text("Visit <example>");
html.write_end_tag("a")?;
assert_eq!(
    html.finish(),
    "<a href=\"https://example.com/?a=1&amp;b=2\">Visit &lt;example&gt;</a>"
);
# Ok::<(), HtmlWriterError>(())
```

`write_start_tag` leaves a tag unfinished for attributes; `write_empty_element_tag`
does the same but closes with `/>` instead of `>`. Another tag, text, raw markup,
or `flush()` completes the pending tag. `write_end_tag` writes a complete end tag
immediately. `finish()` flushes and consumes the writer, returning its string.
Neither method synthesizes missing element end tags. `as_str()` borrows the
current buffer **without flushing**, so it may contain an unfinished start tag.

`HtmlWriterState` has `Content` (MimeKit's `Default`), `Tag`, and `Attribute`.
`write_attribute_name` enters `Attribute` state; an optional
`write_attribute_value` then appends a complete double-quoted, escaped value.
Omitting the value leaves a valueless attribute; `write_attribute(name, "")`
instead produces `name=""`. `write_attribute(name, value)` writes both pieces.
Names and duplicate attributes retain their original spelling and order.
`HtmlWriterError` reports invalid names and invalid attribute/value states;
rejected operations leave the buffer and state unchanged.

Name validation follows MimeKit's delimiter checks, not full HTML grammar.
Tag names reject NUL, TAB, CR, LF, form feed, SPACE, `/`, and `>`; attribute
names additionally reject `=`, quotes, and `<`. Names and raw markup must be
trusted: this is not a sanitizer or a check of nesting, void elements, URL
schemes, or event-handler attributes. `write_markup_text` copies markup
verbatim without parsing it. `write_text` escapes character data but does not
perform JavaScript/CSS escaping, automatic linking, or whitespace normalization.
Even empty text or markup completes the pending tag.

String-based operations use `&str` slices instead of buffer/index/count overloads.
`write_html_attribute` accepts `HtmlAttribute` objects, preserving valueless
versus empty values and malformed names retained by the tokenizer. If a name
begins with `=`, a preceding valueless attribute receives an explicit empty
value so it cannot absorb that name as its value. Name-based public operations
retain their existing validation. Identifier enums expose canonical names through
`as_str()` for the existing string-based tag and attribute operations.
`write_token` flushes any pending syntax and writes a complete `HtmlToken`,
returning to `Content` state even for a tag token or empty data. It respects
the token's encoding mode and does not validate raw token names or contents.
The standalone writer remains in-memory. `HtmlToHtml` uses it as a staging
buffer for its UTF-8 writer adapter, retaining pending tag/attribute state
across buffer drains. Charset conversion is not provided.

### HTML identifiers and attributes

`text::HtmlTagId` recognizes all 134 MimeKit tag names; `HtmlAttributeId`
recognizes all 106 attribute names. `from_name()` performs allocation-free,
ASCII case-insensitive lookup without trimming whitespace or parsing markup.
Unrecognized names map to `Unknown`; any tag name beginning with `!` maps to
`Comment`. `as_str()` and `Display` return canonical names, including `img`
for `HtmlTagId::Image` and `accept-charset` for `HtmlAttributeId::AcceptCharset`.
`Unknown.as_str()` is the diagnostic label `"Unknown"`, not an original name;
do not use it to serialize an unrecognized name.

Tag identifiers expose MimeKit's exact `is_empty_element()` and
`is_formatting_element()` classifications, including legacy elements. They do
not change writer behavior or provide HTML validation. Enum numeric values are
not a compatibility contract; acronym variants use Rust casing such as `Hr`,
`Em`, `NoBr`, `BgColor`, and `XmlNs`.

`HtmlAttribute::new(name, Option<String>)` preserves a checked name and an
unescaped value. `from_id()` uses the canonical name and rejects `Unknown`;
custom names are accepted by `new()`. Both report `HtmlAttributeError` on
invalid construction. Names use the same delimiter checks as the HTML writer,
not full HTML grammar. `None` means valueless, while `Some(String::new())` means
an explicit empty value. Names and values are immutable, and `id()` does not
change their original spelling.

```rust
use mimecraft::text::{HtmlAttribute, HtmlAttributeCollection, HtmlAttributeId,
                     HtmlTagId, HtmlWriter};

let attributes: HtmlAttributeCollection = vec![
    HtmlAttribute::from_id(HtmlAttributeId::Disabled, None)?,
    HtmlAttribute::new("ID", Some("a&b".into()))?,
].into();
assert_eq!(attributes.index_of("id"), Some(1));

let mut html = HtmlWriter::new();
html.write_empty_element_tag(HtmlTagId::Input.as_str())?;
for attribute in &attributes {
    html.write_html_attribute(attribute)?;
}
assert_eq!(html.finish(), "<input disabled ID=\"a&amp;b\"/>");
# Ok::<(), Box<dyn std::error::Error>>(())
```

`HtmlAttributeCollection` owns an ordered, read-only vector; construct one from
a vector or by collecting an iterator. Duplicate names retain their original
case and order. `get`, `contains`, and `index_of` use ASCII case-insensitive
names; `get_by_id`, `contains_id`, and `index_of_id` compare identifiers.
Every lookup returns the first match, including a valueless attribute.
Looking up `Unknown` finds the first custom/unrecognized name rather than
rejecting the identifier. Non-ASCII name characters compare exactly, unlike
.NET's Unicode ordinal case folding.

Use indexing or `as_slice()` for positional access, `iter()` or borrowed
iteration for traversal, and `into_vec()` or owned iteration to recover the
attributes. There is no unchecked public constructor or tokenizer path yet.
These types do not sanitize attribute names, URL schemes, or event handlers.

### HTML token serialization

`text::HtmlToken` is an owned enum replacing MimeKit's HTML token hierarchy:
`CData(String)`, `Comment(HtmlCommentToken)`, `Data(HtmlDataToken)`,
`DocType(HtmlDocTypeToken)`, `ScriptData(String)`, and `Tag(HtmlTagToken)`.
`kind()` returns `HtmlTokenKind`; `to_html()` creates a string and `write_to()`
appends to an existing string. Neither method parses HTML, inserts separators,
checks nesting, or performs stream I/O.

```rust
use mimecraft::text::{HtmlDataToken, HtmlDocTypeToken, HtmlToken, HtmlWriter};

let mut doctype = HtmlDocTypeToken::new();
doctype.set_name(Some("html".into()))?;
let mut html = HtmlWriter::new();
html.write_token(&HtmlToken::DocType(doctype));
html.write_start_tag("p")?;
html.write_token(&HtmlToken::Data(HtmlDataToken::from_text("a < b")));
html.write_end_tag("p")?;
assert_eq!(html.finish(), "<!DOCTYPE html><p>a &lt; b</p>");
# Ok::<(), Box<dyn std::error::Error>>(())
```

`HtmlDataToken::new` preserves already-encoded or intentionally raw contents,
matching MimeKit's public constructor. `from_text` instead applies the HTML
character-data encoder. `from_data_state` represents raw tokenizer data-state
contents: only a final literal `<` is encoded, preventing it from forming a new
tag if intervening markup is removed. It does not validate the asserted context
or escape other content. Script data stays raw, and CDATA is wrapped in
`<![CDATA[` and `]]>` without rewriting embedded terminators.

`HtmlCommentToken::new` writes a normal comment. `bogus` restores `<` when the
contents begin with `?`, otherwise `</`; `bang` restores `<!`. This retains the
malformed-end-tag distinction needed for bogus-comment round-tripping.
Contents are not escaped or checked for embedded closing delimiters.

`HtmlTagToken::start(name, attributes)`, `empty(name, attributes)`, and `end(name)`
create complete tags. Original names, attribute order, duplicates, and valueless
versus empty values are preserved; values use double-quoted attribute encoding.
Unlike the writer's name-based operations, token names are deliberately
unvalidated, matching MimeKit's token constructors. Empty-element spelling is
explicit rather than inferred from `id()`. Attributes still require checked
construction; unchecked tokenizer attributes are not yet exposed.

`HtmlDocTypeToken` defaults to an unnamed `<!DOCTYPE>`. Its checked `set_name`,
`set_public_identifier`, and `set_system_identifier` methods accept
`Option<String>` and leave every field unchanged on error. Names reject empty
strings, NUL, TAB, LF, form feed, CR, SPACE, and `>`; identifiers reject NUL,
`>`, and the presence of both quote characters. Empty identifiers are allowed.
`HtmlDocTypeError` distinguishes the three rejected edits.

DOCTYPE identifiers remain literal rather than entity-encoded, with a usable
quote delimiter selected automatically. Missing names suppress identifier
output without deleting stored values. Keyword getters retain MimeKit's
PUBLIC/SYSTEM edit transitions, including a retained PUBLIC keyword after
clearing its identifier. `force_quirks_mode` is metadata only and does not
affect output.

**Tokens are representations, not sanitizers.** Raw names, comments, script
contents, CDATA, and already-encoded data must be trusted. The tokenizer described
below also preserves malformed attribute names without weakening public checked
constructors. Reusable token objects, token-filtering callbacks, and HTML
rewriting remain deferred.

### HTML structural token state

`text::HtmlTreeConstruction` ports MimeKit's structural controller as a
reusable consumer of existing `HtmlTagToken` values. This is the structural
prerequisite for the HTML tokenizer, not a character-scanning tokenizer or DOM.

`process_tag` returns `HtmlTagResult`: the following `HtmlContentMode`
(`Data`, `RcData`, `RawText`, `ScriptData`, or `PlainText`), the
`HtmlTagDisposition` (`Normal`, `Ignored`, or `AttributesMerged`), and whether
the tag broke out of foreign content. A normal result does not necessarily
mean an element was inserted. `last_tag` is absent before the first tag.

The controller retains SVG/MathML integration points, foreign breakouts,
scope boundaries, implied list/paragraph/table closures, head/noscript and
frameset rules, form-pointer lifetime, and formatting-end-tag stack effects.
HTML non-void elements ignore self-closing spelling. Foreign self-closing
elements are not pushed. Consumers must use the returned lexical mode when
identifying subsequent tokens; a tag-name-only scanner is not equivalent.

`process_character_data` accepts decoded ordinary data or CDATA, skipping
HTML whitespace; it is not for RCDATA, RAWTEXT, or script contents.
Non-whitespace data can imply body and close head-noscript, while NUL does
not disallow framesets. `tracks_character_data`, `is_in_foreign_content`,
`is_noscript_open`, and `is_in_frameset` expose structural decisions.
Scripting defaults to enabled and can be configured for subsequent tokens.

`set_track_closed_elements(true)` collects `HtmlClosedElement` runs in
innermost-first order. Each includes original name, repeat count, and foreign
status. A tag clears the preceding collection; character data may append.
Explicit end-tag targets and stack-only adoption/form rearrangements are not
reported as implicit closures. Clearing or disabling collection does not
alter structural state.

The private stack uses cached scope/name indexes and an amortized-growth
`Vec`, folding consecutive identical elements before applying the depth
limit. The default limit is 4096 distinct entries, not physical element count.
Checked construction and limit changes reject zero with
`InvalidHtmlElementDepth`. Overflow is sticky and selects literal plaintext
through EOF, including overflow during implied table insertion or formatting
replacement. Unlike MimeKit's internal helper, this public API defensively
ignores subsequent tags and character data after overflow rather than
continuing to mutate incomplete structural state. Raising the limit does
not recover the failed controller.

`HtmlNamespace` recognizes the six HTML/MathML/SVG/XLink/XML/XMLNS URLs.
`from_url` returns `None` for unknown URLs and does not trim them.
`html_namespace` instead retains MimeKit's unknown-declaration-to-HTML
fallback and last-non-null xmlns selection; it is separate from the current
node's namespace. Duplicate encoding/type attributes use the first value.
Names and namespace URLs use ASCII case-insensitive matching, consistent
with existing Rust identifiers rather than .NET Unicode ordinal folding.

Applicable upstream inline rationale is retained. All direct
`HtmlOpenElementStackTests` and `HtmlTreeConstructionTests` cases are ported,
with additional namespace/duplicate-attribute, head/template/frameset, scope,
form, depth, and implicit-closure regressions. Stack caches are also checked
against linear searches after mixed mutations. Formatting regressions adapt
`HtmlTokenizerTests`' adoption-agency sequences, including folded runs, the
eight-pass outer limit, and retained inner-node boundaries.

**This is not a sanitizer or complete HTML tree builder.** As in MimeKit,
there is no DOM reparenting, active-formatting reconstruction, foster
parenting, or complete select/template insertion-mode implementation.
HTML character scanning now uses this controller as described below.
HTML rewriting and callbacks use this state through `HtmlToHtml`; the
remaining streaming text adapters are still deferred.

### Sans-I/O HTML tokenization

`text::HtmlTokenizer` ports MimeKit's character scanner as an I/O-independent
engine. `push(&[u8])` copies input, `next_token()` returns an owned `Token`,
`NeedInput`, or `EndOfInput`, and `finish()` explicitly signals final input.
Empty chunks and repeated needs-input polls never trigger EOF recovery.
Finishing is idempotent; additional input is rejected. Partial UTF-8 scalars,
character references, markup, and structural state survive suspension.

`HtmlTokenizerReader<R: BufRead>` is a thin synchronous driver over that
engine, supplying at most 4096 bytes per refill. `Interrupted` is retried;
other source errors preserve the engine for a later retry. `into_parts()`
returns both source and engine so buffered input is not silently lost.
A future async adapter can drive exactly the same scanner; no async runtime
dependency or async adapter is added in this batch.

The scanner includes data, RCDATA, RAWTEXT, plaintext, script escape and
double-escape states, attributes, character references, comments, bogus
declarations, DOCTYPE, and foreign CDATA. The structural controller selects
modes for SVG/MathML integration, forms, tables, noscript, framesets, formatting
closures, and depth overflow. A recognized tag name alone does not determine
whether its contents are raw text.

Tag/attribute spelling, ordered duplicates, malformed attribute names, and
DOCTYPE keyword/identifier delimiters are retained. Attribute references
always decode; data/RCDATA decoding is chosen at construction and stays fixed
across partial tokens. Even when preserving references in output, the scanner
interprets them when structural state needs to distinguish whitespace from
visible text. Truncated tags default to encoded literal text, with an option
to discard them; that option does not discard comments or bare `<`/`</`.
Raw and script end-tag mismatches reconsume the unmatched character rather
than hiding subsequent markup. Serialization protects a preceding valueless
attribute when a malformed following name begins with `=`.

The input contract is strict UTF-8, not MimeKit's charset/BOM-detecting stream
overloads. There is no BOM removal or newline normalization. Invalid UTF-8
is a sticky error; a valid incomplete scalar waits for input and only fails
at explicit EOF. Positions count UTF-8 bytes and Unicode-scalar columns,
with LF advancing the line. Malformed HTML instead uses lexical recovery.
Owned token payloads can grow with token length; the default 4096-entry
structural limit is not a total-memory or token-size cap. After structural
overflow, remaining literal data is encoded on output even if decoding is off.

The scanner validates contiguous UTF-8 spans once and scans ordinary text,
names, values, comments, and script runs in bulk, adapting MimeKit's run
scanners and line-position updates. Complete ordinary tags build attributes
directly from validated spans; references, NUL, quoted `>`, and incomplete or
malformed closing syntax retain the resumable state-machine path. Lookahead,
partial scalars, and errors remain ordered relative to emitted tokens.
Canonical tag/attribute and stack names reuse static spellings without
changing case or unknown names. Rare DOCTYPE metadata is boxed internally
so it does not enlarge every returned token; public token variants and
constructors are unchanged. Short character-data and attribute-value strings
use `CompactString` internally (up to 24 bytes inline on 64-bit targets),
retaining scanner scratch capacity when possible; longer strings still
transfer their allocations. This avoids
per-value heap allocation without tying tokens to the scanner lifetime or
changing their thread-safety traits. No unsafe UTF-8 conversion is used.

Applicable upstream inline rationale is retained. Coverage includes 160
canonical context/reconsumption/adoption cases, 35 canonical DOCTYPE assertion
sets, and all nine original non-charset document fixtures with expected token
descriptions and serialized output. Document inputs are transcoded to UTF-8
for the explicit engine contract. Partition checks compare full token values
at every split of the small context cases, in bytewise/17-byte/4096-byte
document chunks, and at every character-boundary prefix of 158 canonical
recovery inputs. Additional cases cover strict UTF-8, empty chunks, repeated
polls, explicit EOF, source errors at every byte, interrupted reads, source
handoff, source positions, malformed attributes, and serialization boundaries.
Long Unicode/newline runs, direct-tag fallback after preceding attributes,
ring-buffer scalar splits, and deferred/sticky UTF-8 errors additionally
exercise the optimized paths, including per-token scalar positions.

This remains a tokenizer, not a sanitizer or full DOM parser. HTML compliance
logging, charset/BOM adapters, token-reuse optimizations,
and async adapters remain deferred.

### Rewriting HTML

`text::HtmlToHtml` ports MimeKit's HTML-to-HTML conversion using the existing
sans-I/O tokenizer. `convert(&str)` and `convert_reader` return an owned string;
`convert_to` writes UTF-8 incrementally. Each has a corresponding
`*_with_tag_callback` method accepting the shared `HtmlTagContext` and
`HtmlWriter`. Each conversion starts fresh tokenizer and callback-context state.

The default preserves comments and document wrappers, preserves references in
text, re-encodes attribute values, emits recognized void tags with `/>`, and
discards EOF-truncated tags. It does not construct a DOM or synthesize missing
end tags at EOF. `filter_comments` drops comments; `output_html_fragment`
omits doctype/html/body wrappers and head content without invoking callbacks.
Head suppression ends when the input implicitly starts body content, including
non-whitespace text and foreign elements, rather than waiting only for `</head>`.

`HtmlNoScriptHandling::Unwrap` is the default: fallback is parsed as markup and
the noscript wrappers are omitted, so it remains markup regardless of the
renderer's scripting setting. `ScriptingEnabled` retains wrappers and raw
fallback text; `ScriptingDisabled` retains wrappers and parses fallback markup.
Those retained-wrapper modes must match the renderer. Otherwise markup can
bypass callbacks when raw text becomes markup or hidden noscript ends change
the renderer's parsing context. **This converter is not an HTML sanitizer.**
It does not validate CSS, URL schemes, event handlers or callback-written HTML.

Callbacks own tag writing; `write_tag` preserves original spelling and source
attributes, while writing nothing omits a tag. `delete_tag` is bookkeeping,
not retroactive removal. `delete_end_tag` omits automatic end output.
For this converter, an explicitly requested end callback takes precedence over
end deletion, unlike generated plain/flowed/RTF tags. End callbacks receive
the end token's attributes with copied start flags, not the start attributes.
Unmatched ends can also reach the callback. Suppression hides inner content
and descendant callbacks without silently ending at a scope-ignored end tag.

ASCII-folded name chains and a suppression count avoid per-token depth scans.
Only flag snapshots are retained for open tags, not their attributes. Out-of-order
ends remove only the innermost matching context. Implicit foreign closures emit
explicit ends through the outermost foreign element, including intervening HTML
elements, so removing a breakout tag does not leave output in foreign content.
Frameset-ignored tags and ineffective frameset starts are omitted to avoid
changing interpretation when surrounding document structure is removed.
Applicable upstream inline comments explaining these rules are preserved.

The positive tokenizer depth limit defaults to 4096 distinct entries; identical
runs share entries. The overflowing start tag is the last callback-visible tag;
remaining input is encoded literal text. This is not a total-memory limit:
tokens, callback-context counts and output size can grow with input.
The callback's writer is a periodically drained staging buffer, not the entire
document; pending tags/attributes survive drains. It may temporarily hold a
large token or arbitrary callback output.

Readers require strict UTF-8 without charset/BOM sniffing or newline changes.
I/O and UTF-8 errors propagate; interruptions retry. Writer failures can leave
partial output, read-ahead input cannot be recovered, and failed conversions
cannot resume. Destinations are not flushed or closed. A footer is written only
after successful body conversion. Optional headers/footers bypass callbacks;
HTML decorations are copied, and text decorations use `TextToHtml` fragment
encoding, links and line breaks.

All 110 upstream parameterized rewriting cases and applicable standalone tests
are ported, including original Xamarin input/expected fixtures, the Word HTML
suppression input, and 100,000-element complexity/depth regressions. Additional
coverage includes Unicode, short/erroring I/O, callback flag precedence, original
end attributes, pending-attribute buffer drains and converter reuse.
Four full-document output fingerprints match MimeKit's normal/filtered benchmark
results. A development comparison also matched 4096 mixed/malformed inputs
across fragment/comment settings, all noscript modes, depth limits and callbacks,
with both one-byte and normal reader chunks.

Tokenizer compliance logging/options, charset-selecting stream overloads,
automatic encapsulated-RTF-HTML routing and async adapters remain deferred.
Existing RTF rendering and raw extraction keep their documented behavior.

### Converting HTML to plain text

`text::HtmlToText` ports MimeKit's HTML-to-text converter. `convert(&str)`
returns owned plain text, `convert_reader` accepts a strict UTF-8 `BufRead`,
and `convert_to` writes UTF-8 incrementally to a `Write`. All calls use fresh
conversion state and the existing sans-I/O tokenizer; no new I/O enters the
scanner. Generated line endings default to CRLF on every platform, with
explicit LF selection through `with_new_line_format`.

Paragraphs, headings, lists (including signed start/value numbers), quotations,
definition lists, preformatted text, explicit breaks, and flattened tables
follow upstream formatting. Structural trailing spacing is omitted, but
explicit breaks and preformatted whitespace remain. Link destinations are
appended unless they match the rendered link text; empty links emit their URL.
Image alternative text is included by default. Both behaviors are configurable.
Output is plain text, not Markdown, and is not word-wrapped.

Scripts, styles, titles, templates, frames and hidden content are suppressed.
Noscript fallback is included because scripting is disabled. The tokenizer's
implicit closures and ignored/merged tag dispositions inform conversion.
Indexed name chains avoid repeated depth scans for unmatched end tags;
visibility snapshots include inherited suppression. Repeated open elements
count separately against the converter's depth budget, unlike folded
tokenizer stack entries. Applicable upstream inline rationale is retained.
`HtmlTokenizer::clear_closed_elements` lets consumers acknowledge collected
closures without changing the structural controller.

Positive input, output and depth limits default to 16 MiB, 64 MiB and 4096.
Input counts all original UTF-8 bytes, including suppressed and unfinished
tokens; output counts generated UTF-8 bytes, including indentation, URLs and
line endings. These are byte limits rather than MimeKit's UTF-16 counts.
Literal headers and footers are outside both length budgets and receive no
automatic separators. Input is bounded before tokenizer buffering, and output
is checked before each write.

Limit failures carry `HtmlToTextError` inside an `io::Error` of kind
`InvalidData`; malformed UTF-8 instead retains `HtmlTokenizerError`. I/O errors
propagate, and interrupted operations retry. String conversion discards partial
output on failure; writer conversion can leave partial output, does not flush,
and only appends the footer after success. Borrowed sources/destinations remain
usable, but a failed conversion cannot resume and its source may have read ahead.

All 71 upstream formatting cases and applicable options, reuse, depth/length,
long-token, sequential-element and I/O tests are ported. Reader coverage includes
one-byte UTF-8 chunks. Additional tests cover Unicode URL matching, duplicate
attributes, list-number boundaries, exact byte limits, hidden invalid UTF-8,
interrupted/short I/O and both newline formats. Full-document output length and
fingerprints for both HTML benchmark fixtures were generated with MimeKit.
An additional development comparison matched 4096 generated mixed/malformed
markup inputs against MimeKit with whole-string and one-byte reader input.

No charset/BOM sniffing, CSS evaluation, resource fetching, DOM construction,
HTML sanitization, MIME-body replacement or async adapter is provided.
Escape the result before embedding it in HTML.

### HTML body previews

`text::HtmlTextPreviewer` ports MimeKit's HTML preview heuristics through the
existing sans-I/O tokenizer. `preview(&str)` returns a string;
`preview_reader(&mut reader)` accepts a strict UTF-8 `BufRead`. The default
maximum length is 230; `with_maximum_preview_length` accepts 1 through 1024
Unicode scalars, matching `PlainTextPreviewer` rather than counting UTF-16
code units. Both previewers share whitespace normalization and ellipsis logic.

An explicit non-self-closing `body` tag is required; ordinary HTML fragments
produce no preview. Paragraphs and breaks introduce spaces, image alternative
text is included, and ordered list items are numbered starting at one.
List start/value attributes, CSS, and hidden attributes are not interpreted.
Suppression depends on the innermost open element rather than inherited
visibility: list/table containers suppress data while their items/cells allow
it. Script tokens, comments and CDATA are omitted. Scripting remains enabled;
EOF-truncated tags are discarded. These are upstream preview heuristics, not
the richer formatting or visibility rules of `HtmlToText`.

Whitespace, including U+200B through U+200D, collapses to ASCII spaces.
Only additional eligible text establishes truncation; reaching the limit at
the end of a token does not itself append an ellipsis. Trailing space is trimmed
before the last retained scalar is replaced. Prefix spacing and image/list text
participate in that decision. Supplementary characters remain intact; grapheme
clusters can split. A doubly-linked slot arena and per-tag identifier chains
preserve constant-time middle removal and list lookup, reusing removed slots
instead of retaining every historical element.

The reader may consume ahead in chunks of up to 4096 bytes; queued tokenizer
input is discarded on return, so unlike the plain previewer's reader it does
not preserve the exact unused suffix. I/O and strict UTF-8 errors propagate
without partial output; interruptions retry. Completing a token can encounter
invalid UTF-8 beyond the desired preview text. Only preview output is bounded:
individual tokens and independent preview contexts can grow with the input,
and suppressed or body-less input can require scanning the whole source.
The tokenizer's default structural depth limit remains unchanged.

All applicable upstream preview cases are ported, including the three original
HTML fixtures with first-16-KiB replacement decoding, seven nested/misnested
cases, and three 100,000-element complexity regressions. Additional coverage
includes Unicode, token-boundary truncation, duplicate image attributes, strict
UTF-8, tiny readers, interruptions, read-ahead, context-slot reuse and errors.
Applicable inline comments explaining constant-time tracking are retained.
A development comparison also matched 4096 generated mixed/malformed inputs
against MimeKit with whole-string and one-byte reader input.
There is no corresponding upstream preview benchmark to port.

The standalone HTML previewer has no charset/BOM detection or charset-selecting
stream overloads; `text::TextPreviewer` adds MIME-part decoding and dispatch.
Automatic message-body selection for previews and async adapters remain deferred.
The output is plain text, not sanitized HTML; escape it before HTML embedding.

### HTML character-reference decoding

`text::html_decode` decodes named and numeric references in a complete Unicode
string, using MimeKit's character-data rules.

```rust
use mimecraft::text::html_decode;

assert_eq!(html_decode("&lt;b&gt;caf&#233;&lt;/b&gt;"), "<b>caf\u{e9}</b>");
assert_eq!(html_decode("&notin; &notit;"), "\u{2209} \u{ac}it;");
assert_eq!(html_decode("&#x110000; &unknown;"), "\u{fffd} &unknown;");
```

The sorted static table contains all 2,231 names from MimeKit, including 106
legacy spellings without semicolons. Matching is case-sensitive and chooses
the longest known prefix; unknown names and unmatched suffixes are retained.
Some names expand to two Unicode scalars. Decimal and hexadecimal numeric
references accept missing semicolons and arbitrarily long digit runs, including
leading zeros. Saturating arithmetic prevents overflow without stopping digit
consumption.

Numeric zero, surrogate values, and values beyond U+10FFFF become U+FFFD.
Selected C1 control values use the HTML Windows-1252 compatibility mappings;
other controls and noncharacters remain unchanged. References without any
digits stay literal. These recovery rules match MimeKit; the string-returning
helpers do not perform compliance checks.

This is a single decoding pass, so `"&amp;lt;"` becomes `"&lt;"`, not `"<"`.
Literal Unicode, markup, quotes, whitespace, and line endings are preserved.
`html_decode` does not apply attribute-context restrictions: `"&notit;"` becomes
`"\u{ac}it;"`, even inside text resembling an HTML attribute. This is not an
HTML parser, sanitizer, or HTML-to-text converter. It can introduce markup
characters; escape its result again if embedding it as literal HTML text.
No charset conversion or stream I/O occurs.

`text::html_attribute_decode` instead applies the historical attribute rule:
a named reference without `;` remains literal when the character immediately
after the longest match is `=` or an ASCII letter/digit. Numeric references
and semicolon-terminated names still decode normally.

```rust
use mimecraft::text::html_attribute_decode;

assert_eq!(html_attribute_decode("?a=1&not=2"), "?a=1&not=2");
assert_eq!(html_attribute_decode("&noti;&notin;&not!"), "&noti;\u{2209}\u{ac}!");
assert_eq!(html_attribute_decode("&#65= &amp;lt;"), "A= &lt;");
```

Supply attribute contents, not a complete tag or document. This helper does not
remove quotes, parse unquoted delimiters, normalize whitespace, replace literal
NUL characters, or validate URLs. `HtmlAttribute` constructors still accept
already-decoded logical values and do not automatically decode them.

`text::HtmlEntityDecoder` decodes a single reference character by character.
Push the initial `&`, then continue until `push` returns `Ok(false)` or input
ends. The refused character remains the caller's responsibility. A first
character other than `&` returns `HtmlEntityDecoderError::ExpectedAmpersand`;
an accepted semicolon makes all subsequent pushes return `Ok(false)`.

```rust
use mimecraft::text::HtmlEntityDecoder;

let mut decoder = HtmlEntityDecoder::new();
for character in "&noti".chars() {
    assert!(decoder.push(character).unwrap());
}
assert_eq!(decoder.value(), "\u{ac}i");
assert_eq!(decoder.attribute_value(None), "&noti");
for character in "n;".chars() {
    assert!(decoder.push(character).unwrap());
}
assert_eq!(decoder.value(), "\u{2209}");
assert!(!decoder.push('!').unwrap());
decoder.reset();
assert_eq!(decoder.value(), "");
```

`value()` uses character-data rules; `attribute_value(next)` uses attribute
rules, with the first unconsumed character or `None` for end-of-input. Any
pushed suffix beyond the longest matching name takes precedence over that
lookahead. Neither accessor changes state. State survives input chunk
boundaries, so do not finalize or reset merely because a chunk ended.
Call `reset()` between references; it retains the allocated buffer.

The decoder retains at most 33 ASCII input characters, including `&`, even
while consuming arbitrarily long numeric references. A narrowing range into
the shared sorted entity table replaces MimeKit's generated transition tables.
This is not a chunk-fed whole-document decoder or HTML tokenizer.

#### Character-reference diagnostics

`html_decode_with_options` and `html_attribute_decode_with_options` return
`DecodedHtml { text, issues }`. Their text exactly matches the corresponding
string-only helpers. Diagnostics do not change recovery or stop decoding.

```rust
use mimecraft::text::{
    HtmlDecodeOptions, HtmlEntityViolation, html_decode_with_options,
};

let options = HtmlDecodeOptions { max_issues_per_violation: 1 };
let decoded = html_decode_with_options("&#0;&#0; &foo;", &options);
assert_eq!(decoded.text, "\u{fffd}\u{fffd} &foo;");
assert_eq!(decoded.issues[0].violation, HtmlEntityViolation::NullCharacterReference);
assert_eq!(decoded.issues[1].violation, HtmlEntityViolation::TooManyComplianceIssues);
assert_eq!(decoded.issues[1].offset, 4);
assert_eq!(decoded.issues[2].violation, HtmlEntityViolation::UnknownNamedCharacterReference);
```

`HtmlEntityViolation` covers missing digits, values outside the Unicode range,
controls, missing semicolons, noncharacters, nulls, surrogates, and unknown
names. At a shared offset, a missing semicolon is reported before a
numeric-value violation. Classification uses the original numeric value,
before C1 remapping or replacement. TAB, LF, and FF are exempt from the
control-character diagnostic, but CR is not.

An unknown name requires ASCII alphanumeric characters followed by `;` and
no recognized prefix. Thus `&foo;` is unknown, `&foo` has no violation, and
`&noti;` instead lacks a semicolon after the recognized `not` prefix. In
attribute context that last reference is preserved literally without a
violation, as are other historical attribute exceptions.

Each `HtmlDecodeIssue` stores a violation and the zero-based **UTF-8 byte
offset** of its original leading `&`, not MimeKit's UTF-16 offset or a position
in the decoded result. BOMs, line endings, quotes, and other literal characters
are preserved and counted as supplied. Line/column metadata is not synthesized.

`HtmlDecodeOptions::default()` reports every issue (`max_issues_per_violation`
is zero). Set a positive limit when report size must be bounded. Budgets are
per violation and per call, so repeated low-interest issues cannot hide a
different kind. At most eight times the limit plus one issues are stored.
The first suppressed issue emits one `TooManyComplianceIssues` marker at its
offset; reaching the budget without suppressing anything emits no marker.
Decoding and other violation budgets continue normally.

For character-fed use, `HtmlEntityDecoder::violations()` and
`attribute_violations(next)` inspect current state without changing it or
allocating an issue collection. They yield at most two reference violations.
Read them at the end of a reference, not an intermediate chunk boundary.
Unknown names require scanning the refused suffix through a possible `;`,
which the whole-string APIs do but these accessors do not.

These are reference-only diagnostics, not an HTML compliance framework:
literal controls, tag structure, comment/script context, and URL safety are
not checked. General HTML compliance logging, severity/category metadata, and
HTML tokenization remain deferred.

### Streaming RTF tokenization

`text::RtfTokenizer<R: BufRead>` reads original RTF bytes and returns owned
`RtfToken { kind, position }` values. Byte slices already implement `BufRead`;
wrap files and other `Read` sources in `std::io::BufReader`.

```rust
use mimecraft::text::{RtfKeyword, RtfTokenKind, RtfTokenizer};

let mut tokenizer = RtfTokenizer::new(&b"{\\rtf1 hello}"[..]);
assert_eq!(tokenizer.read_token()?.unwrap().kind, RtfTokenKind::GroupStart);
let token = tokenizer.read_token()?.unwrap();
let RtfTokenKind::ControlWord(word) = token.kind else { panic!("expected word") };
assert_eq!(word.name(), "rtf");
assert_eq!(word.id(), RtfKeyword::Rtf);
assert_eq!(word.parameter(), Some(1));
assert_eq!(tokenizer.read_token()?.unwrap().kind, RtfTokenKind::Text(b"hello".to_vec()));
# Ok::<(), std::io::Error>(())
```

`RtfTokenKind` carries group starts/ends, control words, control symbols,
hexadecimal bytes, or raw text bytes. `RtfControlWord` retains the original
bounded name, its ID, an optional `i32` parameter, and truncation/invalid-parameter
flags. Tokens survive subsequent reads. Text tokens may split at reader buffer
boundaries; concatenate adjacent text tokens when a contiguous run is needed.

`RtfKeyword::from_name` is allocation-free and case-sensitive. Its 134 spellings
match MimeKit's converter-focused table, not every control word in RTF.
Underline styles, background-color spellings, and other rendering equivalents
share IDs. `SkipDestination` classifies non-rendered destinations but does **not**
make this tokenizer skip them. In particular, `object` is intentionally not a
skipped destination, so a later interpreter can preserve its display fallback.

The scanner retains at most 32 letters per word and emits at most 4096 bytes
per text token. Overlong names remain unknown. Only the first ten parameter
digits contribute; all remaining digits are consumed and flagged invalid.
The retained signed value clamps to `i32::MIN..=i32::MAX`. This preserves
MimeKit's handling of excess leading zeros: `\fs00000000001` yields parameter
zero with the invalid flag, rather than one.

A positive `\binN` parameter causes exactly N following bytes to be skipped,
after the word's single delimiting space if present. No allocation is
proportional to N, and braces, backslashes, and line feeds inside the payload
are not interpreted as RTF syntax. Zero and negative lengths do not skip data.
Source bytes, raw text, and hex escapes are not charset-decoded or assumed
to be UTF-8; font code pages and Unicode fallback processing belong to a
future interpreter.

Raw CR/LF are ignored, while escaped CR or LF becomes an LF control symbol.
Only one delimiting space after a word or numeric parameter is consumed.
A minus sign followed by a non-digit yields no parameter, flags it invalid,
and leaves that next byte untouched. A trailing minus at EOF instead follows
MimeKit's pending-word finalization without setting the flag.
Malformed hex escapes with no digits are dropped; one-digit escapes yield that
digit's value. A final backslash is dropped.

Each `RtfSourcePosition` stores a zero-based byte offset, one-based line and
byte-column numbers, and enclosing group depth. Only LF advances the line.
Both braces have the depth outside their group. Binary payloads count toward
positions, including any LF they contain. Unmatched closing braces still
produce tokens; structural validation belongs to an interpreter.

`read_token_with_issues` accepts a callback for `RtfLexicalIssue` values:
`KeywordTooLong`, `InvalidParameter`, `InvalidHexEscape`, and `TruncatedStream`.
The tokenizer retains no issue list; callers control storage and limits.
Issues for words or escapes point to the backslash. Truncation points to EOF
and is reported once even if both a payload and groups are incomplete.
Word-length issues precede parameter issues for the same word.

```rust
use mimecraft::text::{RtfLexicalViolation, RtfTokenizer};

let mut tokenizer = RtfTokenizer::new(&b"{\\'z"[..]);
let mut issues = Vec::new();
while tokenizer.read_token_with_issues(|issue| issues.push(issue))?.is_some() {}
assert_eq!(issues[0].violation, RtfLexicalViolation::InvalidHexEscape);
assert_eq!(issues[1].violation, RtfLexicalViolation::TruncatedStream);
# Ok::<(), std::io::Error>(())
```

`RtfTokenizer::with_options(reader, &RtfTokenizerOptions)` can limit how many
issues reach those callbacks. `max_issues_per_violation` defaults to zero
(unlimited); a positive limit applies independently to each violation for the
entire tokenizer. A flood of one kind therefore cannot hide different kinds.
The first **suppressed** issue emits one `TooManyComplianceIssues` marker with
that issue's original byte offset, line, column, and depth. Merely reaching the
limit emits no marker. Later suppressions do not repeat it.

```rust
use mimecraft::text::{RtfLexicalViolation, RtfTokenizer, RtfTokenizerOptions};

let options = RtfTokenizerOptions { max_issues_per_violation: 1 };
let mut tokenizer = RtfTokenizer::with_options(&b"\\'z\\'z\\'z"[..], &options);
let mut issues = Vec::new();
while tokenizer.read_token_with_issues(|issue| issues.push(issue))?.is_some() {}
assert_eq!(issues.len(), 2);
assert_eq!(issues[1].violation, RtfLexicalViolation::TooManyComplianceIssues);
assert_eq!(issues[1].position.offset, 3);
# Ok::<(), std::io::Error>(())
```

Options are copied at construction; each new tokenizer gets fresh counters.
Budgets persist across reads, callback changes, and retryable source errors.
Calls to `read_token` discard issues without charging those budgets, and do not
defer issues or report-limit markers until a later reporting call.
The tokenizer stores four fixed counters and no issue collection. With a
positive limit, it delivers at most four times that limit plus one issues.
`TooManyComplianceIssues` means the **report** is incomplete; it is distinct
from `TruncatedStream`, which describes EOF inside a group or binary payload.
Limits do not affect returned tokens, lexical recovery, or binary skipping.

Source errors, including `Interrupted` and `WouldBlock`, are returned without
resetting partial words, hex escapes, or binary-skip progress. Retrying is
supported; issues already delivered before an error are not replayed.
The first empty source buffer latches EOF. Pending words and hex values are
emitted before `None`, after which further calls perform no I/O.
`into_inner()` recovers the buffered source and its unread bytes, but discards
pending lexical state; it is not a way to resume a partially read token.

This is tokenization only: RTF headers, destination groups, code pages, fields,
formatting, Unicode fallback bytes, and attachments are not interpreted.
Use `RtfToText` or `RtfToHtml` below for rendering. Token serialization,
comprehensive compliance reporting, and async I/O remain deferred.

### Validating RTF document structure

`text::RtfValidator<R: BufRead>` wraps the tokenizer and adds MimeKit's initial
header, unmatched-closing-brace, and content-after-document checks. It returns
the same owned tokens, including malformed tokens and destination contents;
validation never rejects input, changes recovery, or renders text.

```rust
use mimecraft::text::{RtfValidationOptions, RtfValidator, RtfViolation};

let options = RtfValidationOptions { max_issues_per_violation: 2 };
let mut validator =
    RtfValidator::with_options(&b"{\\rtf1 hello}}junk"[..], &options);
let mut issues = Vec::new();
while validator.read_token_with_issues(|issue| issues.push(issue))?.is_some() {}
assert_eq!(issues.len(), 2);
assert_eq!(issues[0].violation, RtfViolation::UnbalancedGroupEnd);
assert_eq!(issues[1].violation, RtfViolation::ContentAfterDocument);
# Ok::<(), std::io::Error>(())
```

`RtfIssue` carries an `RtfViolation` and the original `RtfSourcePosition`.
The report includes all tokenizer violations plus these structural checks:

- `MissingRtfHeader`: the first two tokens must be `{` and the case-sensitive
  `\rtf` control word. The version parameter and remaining header are not
  validated. An empty stream or EOF after just `{` also reports a missing header.
- `UnbalancedGroupEnd`: each `}` without a matching `{` is reported, but retained.
- `ContentAfterDocument`: reported once after the first closed top-level group.
  Raw spaces, tabs, NUL padding, and ignored CR/LF are harmless; additional `}`
  tokens receive only their unmatched-brace issue. Later groups do not restart
  header checking.

Lexical issues precede structural checks on each returned token. At EOF,
`TruncatedStream` precedes `MissingRtfHeader` if both apply. Structural positions
point to the affected token's start, or the actual EOF cursor. A trailing text
token containing both padding and non-padding reports at its start, so its
location can depend on the source's buffer boundaries.

Options are copied at construction. Default/zero limits are unlimited;
positive limits apply independently per violation across reads, callback
changes, and source-error retries. The combined report emits just one
`TooManyComplianceIssues` marker at the first suppressed issue, not on merely
reaching a limit. Lexical and structural floods cannot exhaust each other's
quotas. With a positive limit, at most seven times the limit plus one issues
reach callbacks.

`read_token()` discards issues without spending reporting budgets. Structural
state still advances, so later callbacks neither recheck a consumed header nor
replay discarded once-per-stream issues. Source errors preserve partial tokens
and state for retry; issues already delivered are not repeated. After `None`,
further reads perform no I/O or reporting. `get_ref`, `get_mut`, and `into_inner`
have the same source-access restrictions as the tokenizer.

Structural tracking uses constant additional storage, without recursion, a
group stack, or a retained issue list. This is not comprehensive RTF validation:
destination semantics, formatting, font/color tables, charset and Unicode
interpretation, and rendering are outside this token-preserving adapter.

### Converting RTF to plain text

`text::RtfToText` consumes original bytes from a `BufRead` source and returns a
complete Unicode `String`. Do not decode the input before conversion: raw bytes,
hex escapes, and binary-payload lengths must retain their original meaning.

```rust
use mimecraft::text::RtfToText;

let converter = RtfToText::new();
let text = converter.convert(
    &b"{\\rtf1\\ansi caf\\'e9\\par a\\cell b\\cell\\row}"[..],
)?;
assert_eq!(text, "caf\u{e9}\r\na\tb\r\n");
# Ok::<(), std::io::Error>(())
```

Group-scoped character state, font charset/code-page overrides, signed UTF-16
`\uN` escapes, `\ucN` fallback skipping, surrogate pairs, and `\upr`/`\ud`
alternatives follow MimeKit's interpretation. Incremental decoding preserves
multibyte characters across input and escape boundaries. Hidden text and
non-text destinations are suppressed; field results and object-result fallback
text are retained. Encapsulated HTML uses its RTF rendering, not its markup.
Cells are separated by tabs without a trailing tab at row ends.

Line and paragraph breaks default to CRLF regardless of platform.
`with_new_line_format(NewLineFormat::Unix)` selects LF; `Mixed` is rejected.
Optional `header` and `footer` strings are copied verbatim without separators.
`convert_with_callbacks` additionally accepts a diagnostic callback and an
attachment-placeholder callback receiving `(zero_based_index, &mut String)`.
Only visible placeholders count. The immediately following literal or escaped
space is consumed, even without a placeholder callback.

`max_group_depth` and `max_font_table_entries` are nonzero limits, each defaulting
to 4096. Identical saved states fold into one stack entry; groups needing a new
entry beyond the limit are skipped rather than rendered with incorrect state.
Font-table overflow rejects new numbers but permits existing entries to be
updated. Font names and hyperlink targets are not retained. The converter
caches at most 32 code-page resolutions and uses a fixed-size byte buffer with
one incremental decoder. These bounds do not limit the returned string or
anything retained or appended by caller callbacks.

Charset support is the crate's existing `Charset`/`encoding_rs` support, not
every .NET code page. Unsupported pages (including DOS 437/850 and Johab 1361)
and new pages beyond the cache limit fall back to Windows-1252; cached pages
remain usable. BOMs are not sniffed. Decoding errors emit replacement characters.

`convert_with_issues` reports `RtfIssue` values for lexical and structural
problems plus group/font limits, undefined fonts, invalid font sizes, unsupported
or excess code pages, invalid byte sequences, raw high bytes, and unpaired
surrogates. `max_issues_per_violation` defaults to zero (unlimited); positive
limits apply independently to all sixteen violation kinds with a single
first-suppression marker, delivering at most sixteen times the limit plus one
issues. Budgets and interpreter state start fresh for each conversion.
Tokenization/structure checks precede interpretation; decoder diagnostics use
the token triggering decoding (possibly a later flushing token), or EOF.
Callback-free conversion discards
diagnostics without changing rendering.

Source I/O errors propagate, without returning partial output. Unlike the
tokenizer/validator, a failed whole-document conversion cannot resume; the
source may have been consumed and callback side effects are not rolled back.
The text converter does not collect or validate colors and field instructions;
these are enabled only for HTML rendering. Streaming-output conversion remains
deferred.

### Rendering RTF as HTML

`text::RtfToHtml` shares the text converter's byte-oriented interpreter and
returns a complete HTML document or body fragment. Input must retain its original
bytes; charset decoding, Unicode recovery, hidden-text suppression, and skipped
destinations follow `RtfToText`.

```rust
use mimecraft::text::RtfToHtml;

let mut converter = RtfToHtml::new();
converter.output_html_fragment = true;
let html = converter.convert(&b"{\\rtf1 Hello {\\b world}!}"[..])?;
assert_eq!(
    html,
    "<div>Hello <span style=\"font-weight: bold;\">world</span>!</div>\r\n",
);
# Ok::<(), std::io::Error>(())
```

Rendering includes bold, italic, underline, strikeout, superscript/subscript,
half-point font sizes, foreground/background colors, paragraph alignment,
line breaks, and tables. Text and attributes use `HtmlWriter` escaping.
Alternating ordinary/nonbreaking spaces preserve significant whitespace;
tabs become four nonbreaking spaces. Empty paragraphs get explicit breaks.
Field results become links only when `parse_rtf_hyperlink` accepts the target;
rejected or unsupported fields still render their visible results.

Document mode wraps the body in `<html><body>` and closing tags. Generated block
separators use CRLF by default; `with_new_line_format(NewLineFormat::Unix)` selects
LF, while `Mixed` is rejected. Optional `header` and `footer` use
`HeaderFooterFormat::Text` by default; `Html` inserts trusted markup.
`convert_with_callbacks` takes diagnostics and a placeholder callback receiving
`(zero_based_index, &mut HtmlWriter)` in the current paragraph/inline context.
Only visible placeholders count; their immediately following literal or escaped
space is consumed. Without a callback placeholders produce no markup.

`convert_with_tag_callback(reader, callback)` customizes generated body tags.
The callback receives `(&mut HtmlTagContext, &mut HtmlWriter)` for `div`, `span`,
`a`, `br`, `table`, `tr`, and `td`. It is responsible for writing each start
tag: call `context.write_tag(writer, true)` to preserve it, pass `false` and
write replacement attributes yourself, write a replacement tag, or write nothing.
Source names and attributes remain read-only.

```rust
use mimecraft::text::{HtmlTagId, RtfToHtml};

let mut converter = RtfToHtml::new();
converter.output_html_fragment = true;
let html = converter.convert_with_tag_callback(
    &b"{\\rtf1 {\\b bold} plain}"[..],
    |context, writer| {
        if context.tag_id() == HtmlTagId::Span {
            context.delete_tag = true;
            context.delete_end_tag = true;
        } else {
            context.write_tag(writer, true).unwrap();
        }
    },
)?;
assert_eq!(html, "<div>bold plain</div>\r\n");
# Ok::<(), std::io::Error>(())
```

End tags are automatic unless `invoke_callback_for_end_tag` requests a callback
or `delete_end_tag` omits both the tag and its callback. End callbacks retain the
start context's attributes and flags. To replace a tag name, write the new start
tag and request an end callback to write the matching closing tag. Empty `br`
elements never receive end callbacks. `delete_tag` records the callback's
decision but does not undo writes or imply `delete_end_tag`.

`suppress_inner_content` hides an element's text, descendant tags, and descendant
tag/placeholder callbacks, but not the element's own end tag. Interpretation
and diagnostics continue, and suppressed placeholder indices still advance.
`convert_with_all_callbacks(reader, report, tag_callback, placeholder)` combines
all three hooks. The existing `convert_with_callbacks` signature is unchanged.
Document wrappers, headers, footers, and callback-written markup bypass tag
callbacks. Generated contexts remain bounded to six open elements regardless
of callback decisions; custom markup is neither tracked nor validated.

The nonzero `max_group_depth`, `max_font_table_entries`, and
`max_color_table_entries` limits each default to 4096. Automatic color entries
count toward the color limit. Folded saved states compare hyperlink identity in
constant time; excess groups are skipped, not rendered in the wrong state.
The renderer retains at most six generated open elements and caches at most 256
CSS formats. Field instructions retain at most **2048 UTF-8 bytes**, rather than
MimeKit's UTF-16 units, without splitting a scalar. Once an instruction exceeds
this bound, all later instruction text is discarded. The retained prefix is still
parsed, so an unterminated quoted target can produce a truncated link. No bounds
are imposed on the output string or caller-owned callback data.

`convert_with_issues` adds color-table overflow, undefined-color, and
field-instruction-limit diagnostics to the text converter's checks.
`max_issues_per_violation` defaults to zero (unlimited). Positive quotas apply
independently to all nineteen kinds with one first-suppression marker: at most
nineteen times the limit plus one issues per conversion. All interpretation and
reporting state starts fresh per call. Source I/O errors propagate without
returning partial HTML; failed conversions cannot resume and callback side
effects are not rolled back.

This is **RTF rendering, not encapsulated-HTML extraction**: `\fromhtml` does not
switch modes, `\htmltag` groups are skipped, and `\htmlrtf` content is rendered.
Use `RtfHtmlExtractor` separately for raw, unfiltered encapsulated HTML.
HTML tokenization/filtering and streaming-output conversion remain deferred.
This is not a sanitizer:
HTML decorations and callback markup are trusted, and the hyperlink scheme
allowlist does not validate addresses or make targets trustworthy.

### Extracting raw HTML from RTF

`text::RtfHtmlExtractor` recovers HTML encapsulated in original RTF bytes without
rendering it. `extract(reader)` returns `io::Result<Option<String>>`: `None`
means no recognized HTML declaration, while `Some("")` means recognized empty
HTML. It never falls back to RTF-to-HTML rendering.

```rust
use mimecraft::text::RtfHtmlExtractor;

let extractor = RtfHtmlExtractor::new();
let rtf = br"{\rtf1\fromhtml1 {\*\htmltag <p>}Hello &amp; goodbye{\*\htmltag </p>}}";
assert_eq!(
    extractor.extract(&rtf[..])?.as_deref(),
    Some("<p>Hello &amp; goodbye</p>")
);
assert_eq!(extractor.extract(&b"{\\rtf1 ordinary RTF}"[..])?, None);
# Ok::<(), std::io::Error>(())
```

Recognition follows MimeKit's first-ten-token window: tokens must be group
starts or control words. A bare or nonzero `\fromhtml` enables extraction and
zero disables it within that window. Other token kinds, emitted content, or
the eleventh token close the window; later declarations cannot change modes.
Ignored physical RTF newlines do not consume token slots. Recognition is
tolerant and does not prove the RTF header or group structure is valid.

HTMLTAG contents use the document code page, regardless of the selected font;
ordinary text uses the current font's code page. The numeric HTMLTAG parameter
is ignored. Both HTMLTAG contents and unsuppressed surrounding text are already
HTML and are copied without HTML encoding. `\htmlrtf` suppresses RTF-only text
but still updates group-scoped properties, including font selection. Ordinary
hidden text is skipped; like MimeKit, HTMLTAG destinations are copied even
when hidden/HTMLRTF flags are set.

`\par` and `\line` become CRLF and `\tab` becomes a tab. Inside HTMLTAG, `\_`
becomes a soft hyphen instead of a nonbreaking hyphen. Formatting and table
boundaries add no markup. Attachments are not rendered; a visible placeholder
still consumes its immediately following literal or hex-escaped space.
Charset decoding, Unicode fallback, surrogate repair, and destination recovery
reuse the existing interpreter.

**Extracted HTML is untrusted and unfiltered.** Scripts, event attributes,
arbitrary URLs, comments, and malformed tags remain. No HTML parsing, tag
callbacks, wrappers, decorations, or fragment conversion occur. Use a separate
sanitization policy before displaying it; `RtfToHtml`'s tag callbacks do not
filter this extractor's output.

`max_group_depth` and `max_font_table_entries` default to 4096 and use the
same bounded-state recovery as `RtfToText`. Color-table and hyperlink-field
collection are disabled. Output size is not capped.
`extract_with_issues(reader, report)` reports lexical, structural, and decoding
issues even for non-HTML input. `max_issues_per_violation` defaults to zero
(unlimited) and uses the shared report's single first-suppression marker.
Every invocation starts fresh and consumes the entire source unless an I/O
error occurs. Source errors return neither partial HTML nor `None`; already
delivered diagnostics are not rolled back.

### Parsing RTF hyperlink fields

`text::parse_rtf_hyperlink(&str) -> Option<&str>` extracts an external target
from already-decoded `\fldinst` text. It borrows the target from the input
without allocating and scans in linear time with constant additional storage.
It does not extract instructions from a raw RTF document or interpret groups.

```rust
use mimecraft::text::parse_rtf_hyperlink;

assert_eq!(
    parse_rtf_hyperlink(r#"HYPERLINK \o "tooltip" "https://example.com/""#),
    Some("https://example.com/"),
);
assert_eq!(parse_rtf_hyperlink(r#"HYPERLINK \l "bookmark""#), None);
```

The field name and allowed schemes (`http`, `https`, `mailto`, `ftp`, and `tel`)
use ASCII-insensitive matching. Switches are case-sensitive: `\l` before a
target rejects a local bookmark; `\o` and `\t` skip one quoted or unquoted
argument, while other switches only skip themselves.

MimeKit's recovery is preserved: keyword separators are optional, switches
are recognized by their first letter, and an unterminated quoted target uses
the rest of the instruction. Quotes are not backslash-unescaped. The first
target ends parsing, so subsequent switches (including `\l`) are ignored.
Whitespace separation and target trimming use Unicode whitespace. Controls
remaining inside the trimmed target are rejected, as are missing or disallowed
schemes. A rejected first target does not cause a search for later candidates.

**This is not full URL validation or HTML sanitization.** Scheme-only targets
such as `https:` are accepted; addresses, escapes, and host names are not
validated or normalized. Use `html_attribute_encode` when writing the result
into HTML and apply any application-specific URL restrictions separately.
Unlike the HTML converter's bounded field collector, this helper does not
truncate its borrowed input. `RtfToHtml` collects field instructions and tracks
group-scoped hyperlink identity; plain-text conversion renders only field results.

### Converting plain text to HTML

`text::TextToHtml` converts decoded plain text into an HTML document or fragment,
escaping input markup and adding line breaks, quote blocks, and automatic links.

```rust
use mimecraft::text::TextToHtml;

let converter = TextToHtml {
    output_html_fragment: true,
    ..TextToHtml::new()
};
assert_eq!(
    converter.convert("> Hello & goodbye"),
    "<blockquote>Hello &amp; goodbye<br/></blockquote>"
);
assert_eq!(
    converter.convert("Visit www.example.com"),
    "Visit <a href=\"http://www.example.com\">www.example.com</a><br/>"
);
```

By default, output includes `<html><body>` and `</body></html>`. Fragment mode
omits only those wrappers. Each logical line produces `<br/>`, including an
unterminated final line; empty input adds no break. CRLF, LF, and bare CR are
accepted. Leading `>` markers, optionally separated by one ASCII space each,
become nested blockquotes. Outstanding quotes close at the end of each fragment.
Whitespace otherwise follows `html_encode`; spaces and tabs are not expanded,
so browsers may collapse them.

The optional `header` and `footer` fields are `Option<String>`.
`header_format` and `footer_format` default to `HeaderFooterFormat::Text`,
converting each decoration as an independent fragment with the same quote/link
rules. `HeaderFooterFormat::Html` instead copies trusted markup verbatim.
No separators are inserted between fragments, and decorations are emitted even
with an empty body. This is not HTML sanitization; never use the HTML format for
untrusted decorations without a separate sanitization policy.

Automatic links recognize MimeKit's fixed `http://`, `https://`, `ftp://`,
`sftp://`, `file://`, `news://`, `nntp://`, `telnet://`, `webcal://`, `callto:`,
`h323:`, `sip:`, and `mailto:` prefixes, ignoring ASCII case. `www.` and `ftp.`
gain their implied schemes, and bare email addresses gain `mailto:`.
Bare-address domains require at least two labels and a final label containing
at least two Unicode letters or a validated IDNA A-label (`xn--...`), using
the existing `mimecraft::idna` helpers. Whitespace, controls, and Unicode format
characters terminate unquoted components. Local-part, label, and domain limits count 64, 63, and 255
UTF-8 bytes, respectively, instead of the source's UTF-16 units.

Trailing prose punctuation and enclosing delimiters are excluded, while balanced
inner brackets in paths, queries, and fragments remain part of the URL. Queries
and fragments can follow a host directly, including an empty `#`. Schemes ending
in `://` support userinfo and bracketed IPv6 hosts. Address literals follow SMTP
syntax, while URI IPv6 hosts use URI compression and embedded IPv4 rules;
IPvFuture and scoped IPv6 hosts are not recognized. These are source heuristics,
not full URL/address validation: partial-word matches and unrestricted numeric
ports are preserved.
Targets are escaped, not canonicalized, resolved, or checked for trustworthiness.

`convert_with_tag_callback(text, callback)` passes generated body `blockquote`,
`a`, and `br` tags to a closure receiving
`(&mut HtmlTagContext, &mut HtmlWriter)`. The callback must write each start tag
it wants to keep; nothing is written automatically after it returns.

```rust
use mimecraft::text::{HtmlTagId, TextToHtml};

let converter = TextToHtml {
    output_html_fragment: true,
    ..TextToHtml::new()
};
let html = converter.convert_with_tag_callback("Visit www.example.com", |context, writer| {
    if context.tag_id() == HtmlTagId::A {
        // Keep the label but omit both link tags.
        context.delete_end_tag = true;
    } else {
        context.write_tag(writer, true).unwrap();
    }
});
assert_eq!(html, "Visit www.example.com<br/>");
```

The context exposes the original tag identity and read-only attributes,
including the decoded `href`. Use `write_tag(writer, false)` followed by writer
attribute methods to replace attributes. End tags are automatic unless
`invoke_callback_for_end_tag` requests a callback or `delete_end_tag` omits both
the end tag and its callback. Requested end callbacks retain the original
attributes and flags, including when quotes close at EOF. Empty `br` elements
never receive end callbacks. `delete_tag` is bookkeeping only for plain text;
it does not undo written markup or omit an end tag.

`suppress_inner_content` hides a link's label or all content and descendant
callbacks inside a quotation, without hiding the element's own end tag.
The renderer stores one context per callback-visible open quote level through
the first suppressing quote; hidden descendants require no additional contexts.
There is no quote-depth cap. Callback-free conversion retains its constant-space
quote counter and unchanged output.

Wrappers, text/HTML decorations, and callback-written markup bypass callbacks.
Custom markup is trusted, not parsed, stack-tracked, or sanitized.
Conversion does not change MIME parts or decode charsets. `HtmlToHtml` handles
HTML rewriting; streaming adapters for plain/flowed HTML output remain deferred.

### Converting flowed text to HTML

`text::FlowedToHtml` converts decoded `format=flowed` text into HTML paragraphs.
It removes one stuffing space after contiguous quote markers, joins soft-broken
lines, and lets quote-depth changes take precedence over continuation.
Nonempty paragraphs become `<p>...</p>`, empty hard paragraphs become `<br/>`,
and quotation levels become nested `<blockquote>` elements. Links are detected
after unwrapping, using the same scanner and escaping rules as `TextToHtml`.

```rust
use mimecraft::text::FlowedToHtml;

let mut converter = FlowedToHtml::new();
converter.output_html_fragment = true;
assert_eq!(
    converter.convert("> hello \n> world"),
    "<blockquote><p>hello world</p>\r\n</blockquote>"
);
converter.delete_space = true;
assert_eq!(converter.convert("soft \nbreak"), "<p>softbreak</p>\r\n");
```

Set `delete_space` only for `delsp=yes` (ignoring ASCII case); it removes
exactly one trailing soft-break space. Tabs and Unicode whitespace do not mark
continuation. CRLF, LF, and bare CR input endings are accepted. A final nonempty
flowed paragraph is emitted even at EOF, with its closing tag and markup newline.
Empty pending paragraphs are omitted, including on quote-depth changes. Like
MimeKit, `-- ` is treated as a soft break, not a signature separator.

The default output is a complete document with CRLF paragraph markup newlines
on every platform. `with_new_line_format(NewLineFormat::Unix)` selects LF;
`Mixed` is rejected. `new_line_format()` reports the setting. This affects only
paragraph markup newlines, not wrappers, quotation tags, or decorations.
`output_html_fragment` omits the outer `<html><body>` wrapper.

`header`, `footer`, `header_format`, and `footer_format` work like `TextToHtml`:
text decorations are converted as independent **plain-text**, not flowed,
fragments, while HTML decorations are trusted markup copied verbatim.
Each fragment closes its own quotes; no separators are inserted between them.

`convert_with_tag_callback(text, callback)` uses the same `HtmlTagContext` and
`HtmlWriter` contract as plain-text conversion, adding `p` contexts for nonempty
paragraphs. Links are detected after flowed joining and optional space deletion.
Suppressing a paragraph also skips its link callbacks; its own end tag is still
handled normally. Quote suppression and callback-visible context storage follow
the same rules as `TextToHtml`. Wrappers, decorations, and custom markup bypass
callbacks.

Unlike plain-text conversion, `delete_tag` suppresses a flowed paragraph's
generated newline, including for an empty `br` paragraph. It is checked **after**
any end-tag callback. Deleting only the end tag does not suppress the newline,
and neither flag removes markup already written by a callback. To rename a
paragraph, write a replacement start tag and set `invoke_callback_for_end_tag`
so the callback can write its matching closing tag.

Conversion does not modify MIME parts, decode charsets, sanitize trusted HTML,
or expose stream APIs.

### Enriched text to HTML

`text::EnrichedToHtml` converts decoded RFC 1896 `text/enriched` into an HTML
document or fragment. It does not interpret the older `text/richtext` format.

```rust
use mimecraft::text::EnrichedToHtml;

let converter = EnrichedToHtml {
    output_html_fragment: true,
    ..EnrichedToHtml::new()
};
let html = converter.convert("<bold>Hello</bold> & goodbye").unwrap();
assert_eq!(html,
    "<div style=\"white-space: pre-wrap;\"><div><span style=\"font-weight: bold;\">Hello</span> &amp; goodbye</div></div>");
```

Character formatting includes bold, italic, underline, fixed-width and validated
font families, named or 16-bit RGB colors, language attributes, and relative font
sizes in one-sixth steps bounded to 1/6 through 12 times the base size. Block
formatting includes alignment, nofill, excerpts, and four-character indentation
steps. Nested state restores when commands close. Spaces and tabs are preserved.
CRLF, CR, and LF are normalized: outside nofill, a single newline becomes a space
and N consecutive newlines become N-1 breaks; inside nofill, every newline is
retained. Block transitions avoid introducing duplicate implicit breaks.

Unknown commands retain their text, but parameter content is always suppressed.
Only immediately adjacent parameters can affect the preceding eligible command.
Nested commands invalidate a parameter. Invalid parameters preserve inherited
formatting; malformed command syntax is literal text. Command names are ASCII
case-insensitive and limited to 60 characters. Unmatched ends are ignored,
mismatched ends close intervening commands, and EOF closes remaining formatting.

`max_element_depth` and `max_parameter_length` default to 4096;
`max_input_length` and `max_output_length` default to 16 MiB and 64 MiB.
All use `NonZeroUsize`. Lengths count **UTF-8 bytes**, not UTF-16 code units.
Parameter lengths include suppressed content and nested command markup after
newline normalization; `<<` counts both input bytes. Original input is checked
before conversion. Output limits include wrappers, escaped text, and decorations.
Limit failures return a specific `EnrichedToHtmlError` and no partial HTML.
All conversion state is local, so the converter remains reusable after an error.

The `header`, `footer`, and corresponding format fields follow `TextToHtml`:
plain-text decorations use its link/quotation conversion; HTML decorations are
trusted caller-supplied markup. Decorations do not count toward the input limit.
Plain-text decorations are rendered before checking their resulting size, so
their intermediate allocations are not bounded by the output limit.
Body text never generates automatic links or fetches external resources.

Font validation uses the existing `unicode-general-category` dependency for
letters and decimal digits, plus spaces, hyphens, underscores, and periods.
Rust validates complete scalars, so supplementary letters/digits are accepted
where MimeKit's UTF-16-character check rejects their surrogate halves.
No dependency or bundled Unicode-table changes are required.
Stream adapters, charset conversion, tag callbacks, and automatic MIME-body
replacement are not part of this converter.

### Delivery-status groups

`MessageDeliveryStatus<R>` wraps a `MimePart<R>` whose first Content-Type is
message/delivery-status or message/global-delivery-status. `new()` creates an
empty message/delivery-status body; `from_part` checks an existing leaf without
reading its source. MIME tree loading still represents these as ordinary leaves,
and converting the wrapper into `MimeEntity` preserves that representation.

```rust
use mimecraft::{Header, HeaderList, MessageDeliveryStatus};

let mut status = MessageDeliveryStatus::new();
let message: HeaderList = vec![
    Header::parse(b"Reporting-MTA: dns; example.com").unwrap(),
].into();
let recipient: HeaderList = vec![
    Header::parse(b"Final-Recipient: rfc822; user@example.com").unwrap(),
    Header::parse(b"Action: failed").unwrap(),
    Header::parse(b"Status: 5.1.1").unwrap(),
].into();
status.set_status_groups(&[message, recipient]).unwrap();
let groups = status.status_groups().unwrap();
assert_eq!(groups.len(), 2);
```

`status_groups()` returns detached `Vec<HeaderList>` data, parsed afresh from the
stored transfer encoding. The first group contains per-message fields; later
groups contain per-recipient fields. Extra blank CRLF/LF lines are skipped,
and an unterminated final record is accepted. Ordered raw fields, folds,
duplicates, malformed records, and non-UTF-8 bytes follow `HeaderList` behavior.
Required fields, status codes, and recipient addresses are not validated.
Group parsing now uses the shared incremental `MimeParser` path. Header offsets
count from the start of the outer decoded body, including preceding groups and
blank lines, not from each group's start or from encoded wire bytes. A separately
decoded remainder has its own zero-based origin. To process already decoded
input one group at a time, use `MimeParser::parse_status_group` directly;
`status_groups()` still buffers the decoded body and returns all groups.

For Office365 compatibility, an active group's Base64/QP/UU transfer-encoding
field decodes all remaining content once. This may occur after the first group;
7bit/8bit/binary do not trigger it, and encoding fields within the decoded
remainder are ordinary data. Unknown active encodings return errors rather
than silently truncating groups. Existing tolerant transfer codecs are retained.

`status_groups_with_options` accepts `DeliveryStatusParseOptions`. Defaults
allow 64 MiB of total decoding output (outer decoded body plus any separately
decoded remainder) and 16 KiB per folded header record. Zero limits are rejected
before I/O; exceeding a limit fails rather than presenting a truncated success.
These bounds do not cap encoded input consumed by tolerant codecs or allocation
overhead. The entire outer decoded body is buffered and its source rewound
before parsing; metadata errors precede I/O, while failed copying follows
`MimeContent::decode_to`'s source-position rules.

Editing a returned vector does not change the part. For owned in-memory bodies,
`set_status_groups` serializes completely before replacing content; its
`with_new_line_format` variant supports CRLF or LF. Each group gets a blank
separator. An empty slice clears content, but empty member groups are rejected
because they cannot survive parsing. If a retained group declares an encoded
remainder, later groups are re-encoded accordingly, including UU wrappers,
so their metadata remains consistent on rereading. Outer headers remain
unchanged, including requested transfer encoding and any stale Content-Length;
callers must update descriptive fields themselves. Stored content becomes
`Unspecified` encoding at position zero. C# caches, change events, async parsing,
and typed delivery-status field models remain deferred.

### Disposition-notification fields

`MessageDispositionNotification<R>` wraps a leaf with Content-Type
message/disposition-notification or message/global-disposition-notification.
`new()` creates an empty notification; `from_part` checks the first media-type
header without source I/O. Tree loading keeps these parts as ordinary leaves,
and conversion back into `MimeEntity` preserves that representation.

```rust
use mimecraft::{Header, HeaderList, MessageDispositionNotification};

let mut notification = MessageDispositionNotification::new();
let fields: HeaderList = vec![
    Header::parse(b"Final-Recipient: rfc822; user@example.com").unwrap(),
    Header::parse(b"Disposition: manual-action/MDN-sent-manually; displayed").unwrap(),
].into();
notification.set_fields(&fields).unwrap();
assert_eq!(notification.fields().unwrap(), fields);
```

`fields()` returns a detached `HeaderList` for the first field block only.
It preserves raw values, spelling, order, duplicates, folds, malformed records,
and non-UTF-8 bytes using the existing header parser. An unterminated final
record is accepted. An initial blank line produces an empty list rather than
skipping to later fields. Unlike delivery-status parsing, field-level
Content-Transfer-Encoding is ordinary data and never decodes a remainder.
Required fields, disposition values, recipients, and message IDs are not
validated or synthesized; typed notification-field models remain deferred.

`fields_with_options` accepts `DispositionNotificationParseOptions`, defaulting
to 64 MiB of decoded body bytes and 16 KiB per folded record. Zero limits fail
before I/O. Decoding uses stored encoding, buffers the entire body, and rewinds
the source before parsing. Bytes after the first blank separator are not
returned as fields, but still count toward the byte limit and can produce
read errors. The original content remains unchanged. Limits do not bound
encoded input consumed by tolerant codecs or allocation overhead.

Field edits have no effect until `set_fields` is called on an owned in-memory
notification. The setter serializes completely before replacing content;
`set_fields_with_new_line_format` selects CRLF or LF and rejects Mixed.
Each record is terminated and one blank separator is appended, even for an
empty list. Replacement discards the entire old body, including any trailing
non-field bytes. Stored encoding becomes `Unspecified`, position is zero, and
the newline hint is cleared. Outer headers, including requested transfer
encoding and any stale Content-Length, remain untouched. Other metadata and
serialization stay available through `part`/`part_mut`; later field operations
revalidate the media type after raw edits.

### Feedback-report fields

`MessageFeedbackReport<R>` wraps a message/feedback-report leaf, typically the
second child of `MultipartReport` with report type `feedback-report`. `new()`
creates empty unencoded content without synthesizing mandatory fields.
`from_part` checks the first Content-Type ignoring ASCII case, without source
I/O. Unlike disposition notifications, there is no global subtype alias.
Tree loading and conversion to `MimeEntity` retain ordinary leaf representation.

```rust
use mimecraft::{Header, HeaderList, MessageFeedbackReport};

let mut report = MessageFeedbackReport::new();
let fields: HeaderList = vec![
    Header::parse(b"Feedback-Type: abuse").unwrap(),
    Header::parse(b"User-Agent: SomeGenerator/1.0").unwrap(),
    Header::parse(b"Version: 1").unwrap(),
].into();
report.set_fields(&fields).unwrap();
assert_eq!(report.fields().unwrap(), fields);
```

`fields()` returns a detached raw `HeaderList` for the first field block.
Spelling, order, duplicate fields such as Reported-Uri, folds, malformed
records, and non-UTF-8 bytes survive. Initial blank lines yield empty fields;
EOF without a final newline is accepted. Inner Content-Transfer-Encoding
fields are ordinary data. Mandatory fields, feedback types, versions,
addresses, URIs, and field multiplicities are not validated.

Field-block I/O is shared with disposition notifications.
`fields_with_options` accepts `FeedbackReportParseOptions`, defaulting to
64 MiB of decoded body bytes and 16 KiB per folded header. Zero limits fail
before I/O. The entire body is decoded using stored encoding, so trailing
bytes count toward the byte limit and trailing I/O failures propagate even
though only the first block is returned. Successful decoding rewinds the
source before parsing; failed-copy positions follow `MimeContent::decode_to`.
Limits do not bound encoded input consumed by tolerant codecs or allocation
overhead. Reads are uncached and leave stored bytes and metadata unchanged.

Detached edits take effect only after `set_fields` on an owned in-memory
report. `set_fields_with_new_line_format` selects CRLF or LF; Mixed is rejected.
Serialization terminates every record and appends a blank separator, even
for empty fields. Replacement is atomic and discards the entire old body,
including any trailing data. Stored encoding becomes `Unspecified`, position is
zero, and the newline hint is cleared. Outer headers, including requested
transfer encoding and any stale Content-Length, remain unchanged.
Use `part`/`part_mut` for metadata and serialization or `into_part` to recover
ownership; field operations recheck the media type after raw edits.

### Multipart reports

`MultipartReport<R>` composes `Multipart<R>` with checked `report-type` access.
Construction requires an explicit boundary, and creates no children or other
metadata beyond Content-Type. Like MimeKit, it does not enforce a child count,
layout, or agreement between the report type and the second child's subtype.
Empty and unknown report types are accepted.

```rust
use mimecraft::{MessageDeliveryStatus, MimeEntity, MultipartReport, TextPart};

let mut report: MultipartReport =
    MultipartReport::new("delivery-status", "report-boundary").unwrap();
report.multipart_mut().children_mut().push(
    TextPart::new("plain", "Delivery failed.").unwrap().into(),
);
report.multipart_mut().children_mut().push(MessageDeliveryStatus::new().into());
assert_eq!(report.report_type().unwrap().as_deref(), Some("delivery-status"));
let entity: MimeEntity = report.into();
```

Loading and conversion use the ordinary `MimeEntity::Multipart` variant.
`from_multipart` checks the first Content-Type, ignoring media-type case,
without requiring a boundary, report-type parameter, or specific children.
It preserves the container unchanged; a later duplicate Content-Type cannot
substitute for a missing or invalid first value. Access the underlying tree
through `multipart`/`multipart_mut`, or recover ownership with `into_multipart`.
Existing writing, traversal, and body selection remain ordinary multipart
operations.

`report_type()` reads the authoritative raw header on every call and returns a
detached decoded value: `None` means absent, while `Some("")` means empty.
Reads do not trim or change case. `set_report_type` trims outer Unicode
whitespace, preserving case and internal whitespace. An exact match to either
the supplied or trimmed value is a no-op, retaining raw spelling, folds, and
duplicates without validating boundary or formatting options.

A changed value requires a valid existing boundary. The setter formats the
complete Content-Type before replacing its duplicates at the first slot;
failures leave the container unchanged. Other parameters, unrelated headers,
children, preamble, and epilogue are preserved. Use
`set_report_type_with_options` for explicit newline, quoting, and line-length
options. All report-type operations recheck the media type after raw edits,
and neither read nor reposition child sources.

### Partial-message splitting and reassembly

`MessagePartial<R>` composes a message/partial `MimePart<R>`.
`new(id, number, total)` creates empty owned content; `with_content` adopts a
seekable source without I/O, preserving its position and stored encoding.
Both constructors require `1 <= number <= total <= i32::MAX` and generate only
Content-Type. IDs are opaque, case-sensitive strings, including empty strings.
`from_part` validates only the first Content-Type, not parameters or content.
Loading and conversion use the ordinary `MimeEntity::Part` representation.

```rust
use std::io::Cursor;
use mimecraft::{Header, HeaderList, MessagePartial, MimeContent};

let envelope: HeaderList =
    vec![Header::parse(b"From: sender@example.com").unwrap()].into();
let mut fragments = vec![
    MessagePartial::with_content(
        "shared-id", 2, 2, MimeContent::new(Cursor::new(b"body".as_slice())),
    ).unwrap(),
    MessagePartial::with_content(
        "shared-id", 1, 2,
        MimeContent::new(Cursor::new(b"Subject: Original\r\n\r\n".as_slice())),
    ).unwrap(),
];
let joined = MessagePartial::join(&envelope, &mut fragments).unwrap().unwrap();
assert!(joined.headers().contains("From"));
assert!(joined.headers().contains("Subject"));
```

`id()`, `number()`, and `total()` reparse authoritative headers without source
I/O; `part_mut()` permits raw edits. Only absent parameters return `None`.
Numbers accept ASCII digits, surrounding ASCII space or tab-through-CR
whitespace, and values through `i32::MAX`, including zero when reading.
Unlike MimeKit's nullable getters, malformed present numbers return explicit
errors. No sign, Unicode whitespace/digits, or trailing NUL is accepted.

`join` takes the **first fragment's enclosing message headers**, not the
fragment's own Content-* headers. It leaves the caller's slice order unchanged
and checks all metadata before I/O, including the single-fragment case.
Every fragment needs a matching ID and a number; sorted numbers must be
exactly 1 through the supplied count. The highest-numbered fragment's total
must match that count. Earlier totals are ignored, as in MimeKit.
Unlike MimeKit, missing IDs are rejected rather than matching each other.
An empty collection returns `None`.

Each source is opened at zero and independently decoded according to its
stored encoding, not its current outer transfer header. Decoded readers are
chained without collecting all fragment bytes into an intermediate buffer;
the parsed message owns its body streams. Headers and source bytes remain
unchanged, but cursors are not restored after success or failure.

`join_with_options` applies `MimeEntityParseOptions` to the combined decoded
wire message, with shared input/header/depth limits and the usual structured
message opt-out. Zero limits fail before source I/O, even for an empty
collection. Limits do not bound encoded bytes consumed by tolerant decoders,
read-ahead, enclosing headers, fragment metadata, or allocation overhead.
Incomplete MIME structure and I/O failures propagate without a partial result.

Reassembly retains enclosed Subject, Message-ID, Encrypted, and MIME-Version
fields and discards other enclosed non-Content headers. Other enclosing
non-Content fields are copied in order; the four special fields take matching
enclosed values at their enclosing slots, with remaining enclosed special
fields appended. The enclosed body's Content-* headers are preserved.
Raw spelling, duplicates, folds, and values are retained. Stream offsets on
combined message and root-body headers are cleared: offsets from the enclosing
and enclosed streams cannot define their new order. Ordinary and masked writers
therefore emit combined message headers before root-body Content-* headers.
Descendant offsets and embedded-message header interleaving remain intact;
the enclosing and input fragment headers are unchanged.
`MessagePartial::split(message, max_size)` consumes an in-memory `MimeMessage`
and returns `Vec<MimeMessage>`. `split_with_options` also accepts
`&MimePartWriteOptions`. It serializes the enclosed root's Content-* fields,
Subject, Message-Id, Encrypted, and MIME-Version, followed by the body. Root
Content-* extensions and all descendant headers are retained. When this
filtered serialization fits, the original message is returned without copying
its content or changing its headers, framing, IDs, or cursor positions.

```rust
use mimecraft::{MessagePartial, MimeMessage, TextPart};

let message = MimeMessage::with_body(
    TextPart::new("plain", &"hello\r\n".repeat(100)).unwrap().into()
);
let fragments = MessagePartial::split(message, 256).unwrap();
assert!(fragments.len() > 1);
```

Otherwise, each fragment owns between one and `max_size` unencoded bytes.
Whole-line boundaries are preferred; oversized lines are split at the limit,
keeping CRLF together when possible. Unlike MimeKit's scan, an LF immediately
past the limit is not included. Every fragment declares its one-based number
and total, and shares the original Message-Id or a generated ID if absent.
Enclosing messages clone the original non-Content headers with cleared offsets
and receive distinct generated Message-Ids. Generated IDs use the configured
`MessageId::default_domain()`. A missing original Message-Id is not inserted
into the enclosed serialization.

The limit excludes enclosing headers and later serialization changes. No
transfer-encoding header or automatic seven-bit preparation is applied to the
fragments. Ordinary serializers still normalize newlines and may append a
final newline if requested; fragment bytes must remain unchanged for exact
reassembly. A one-byte limit can divide a CRLF pair, so such fragments should
be passed directly to `join`, which reads stored bytes without normalization.
The entire enclosed serialization is buffered before splitting, with no
aggregate memory budget or lazy/stream-backed return type. Zero size, invalid
writer options or original IDs, excessive fragment counts, and serialization
or ID-generation failures return errors, dropping the consumed message and
intermediate buffers rather than returning a partial vector.

Explicit-domain IDs can also be created with `MessageId::generate_with_domain`.
Fragment leaves can use ordinary part/tree transport preparation; no
splitting-specific preparation
is performed.

### Returned-message headers

`TextRfc822Headers<R>` composes `MessagePart<R>` for text/rfc822-headers and
message/global-headers. `new()` creates text/rfc822-headers with no contained
message; `with_message(message)` adopts a message without copying or accessing
its sources. Only the wrapper Content-Type is generated.

```rust
use mimecraft::{Header, MimeEntity, MimeMessage, TextRfc822Headers};

let mut original: MimeMessage = MimeMessage::new();
original.headers_mut().add(Header::parse(b"Subject: Original message").unwrap());
let returned = TextRfc822Headers::with_message(original);
let mut entity: MimeEntity = returned.into();
let mut output = Vec::new();
entity.write_to(&mut output).unwrap();
```

Loading and conversion use `MimeEntity::Message`, not a new enum variant.
`from_message_part` checks the first Content-Type, accepting either supported
type without case sensitivity. Missing, malformed, and other media types are
errors; later duplicate headers never replace an invalid first value.
Wrapping does not validate transfer encoding or the contained message.
`message_part`/`message_part_mut` expose the underlying wrapper, and
`into_message_part` recovers ownership. No source traits are needed for these
ownership and metadata operations.

As in MimeKit's MessagePart-based implementation, an existing message body is
retained and serialized, not silently stripped. Supply a message with no body
when authoring returned headers. Present messages recover interrupted inner
headers without a separator by default.
Loading partitions inner Content-* headers onto the inner body as usual.
An absent message remains distinct from a present empty message.

Transparent text/rfc822-headers now loads structurally by default under
`parse_message_parts`; set that option to false to retain opaque bytes.
Base64, quoted-printable, and UU wrappers remain opaque regardless.
Structured writing accepts message/* or text/rfc822-headers with transparent
encoding, shares the iterative message writer, and retains tree-wide boundary
checks. Loading retains shared input, header, and depth limits. Body-part
enumeration stops at the wrapper; leaf traversal enters any contained body.

### Attached messages

`MessagePart::new(message)` wraps an owned `MimeMessage` in message/rfc822.
`with_subtype(subtype, message)` uses the existing Content-Type compatibility
rules to select another message subtype, such as global or news. It does not
validate subtype-specific semantics; use `MessageDeliveryStatus` for delivery
reports and opaque `MimePart` leaves for partial message fragments.

```rust
use mimecraft::{MessagePart, MimeMessage, Multipart, TextPart};

let original = MimeMessage::with_body(
    TextPart::new("plain", "attached message").unwrap().into(),
);
let attachment = MessagePart::new(original);
let mut mixed: Multipart = Multipart::with_boundary("mixed", "outer-boundary").unwrap();
mixed.children_mut().push(attachment.into());
let mut output = Vec::new();
mixed.write_to(&mut output).unwrap();
assert_eq!(mixed.leaf_parts().count(), 1);
```

`headers_mut` edits wrapper metadata, while `message_mut` edits the contained
message. `set_message` returns the previous message without source I/O;
`from_parts` and `into_parts` preserve raw components without copying payloads.
The internal optional box breaks the recursive layout, but accessors expose
ordinary borrowed or owned messages. A missing message differs from a present
bodyless message: the latter still writes its own headers and separator.
Checked Content-Type setters require message/* and format before replacing
duplicates; raw edits may preserve missing or invalid metadata.

`write_to` and `write_to_with_options` share the multipart writer's explicit
stack, not recursive message serialization or whole-message buffering. Leaf
iteration descends through attached messages; derived debugging and ordinary
drop still recurse, so callers must bound programmatic depth. Active ancestor
boundaries remain checked inside messages, and independent delimiter framing
preserves binary suffixes. Root newline assurance flows through message
wrappers to the body or root epilogue; multipart children do not force it.

`write_content_to` and its `_with_options` variant omit only the wrapper's
headers and separator. The enclosed message still includes its message headers,
root-body headers, separator, and body, without generating metadata or hiding
Bcc. A missing message writes no bytes but flushes the destination; a present
empty message writes its own separator. Standalone wrapper metadata checks
still apply, including rejection of unsupported encoded wrappers.

The wrapper requires a valid message/* or text/rfc822-headers Content-Type,
except for an implicit message/rfc822 child written through a multipart/digest
parent, and an absent, 7bit, 8bit, or binary transfer encoding. Byte-range
restrictions are not checked.
Parsed empty or unknown transfer-encoding headers retain the reader's
`Unspecified` interpretation while their first raw value is unchanged, using
the same recovery guard as ordinary leaf writers. Unrelated wrapper or message
edits preserve that interpretation. Authored or changed invalid values still
fail before wrapper output, including content-only writes; replacing the header
with Base64, quoted-printable, or UU still rejects a structured wrapper.
Base64/quoted-printable/UU wrapper encoding, including for message/global,
is not implemented and returns an error rather than misleading output.
Use opaque encoded leaves for these representations. Inner leaf transfer
conversion remains supported. No metadata or Content-Disposition is generated
beyond the constructor's Content-Type, and Bcc fields are not hidden.

`MimeEntity::load` and `MimeMessage::load` recognize message/rfc822, news,
global, global-headers, external-body, and rfc2822, plus text/rfc822-headers,
ignoring case, when the wrapper encoding is absent, 7bit, 8bit, or binary.
Empty or unknown encoding values also recover as unspecified, with their raw
headers and compliance diagnostics retained.
Each becomes a `MessagePart`
whose inner message owns its non-Content headers and whose body owns its
Content-* headers. Input, header, and depth limits apply throughout the tree;
each message-part wrapper and embedded body root counts toward entity depth.
An enclosing marker with no payload means `message: None`; standalone EOF
constructs an empty embedded message. Interrupted headers recover without a
separator. Delimiter-only blank lines do not create messages.

Set `MimeEntityParseOptions { parse_message_parts: false, ..Default::default() }`
to preserve the previous opaque behavior. Encoded wrappers, unrecognized
message subtypes remain opaque regardless of this flag. Returned-header parts
share the same recovery described above. To interpret an
opaque encoded attachment explicitly, open
its decoded `MimeContent` and pass the reader to `MimeMessage::load`; that
separate load has its own limits rather than sharing the outer parse budget.

Immediate multipart/digest children without Content-Type default to
message/rfc822 when structured message parsing is enabled. The default does
not extend into their embedded bodies, nested non-digest containers, or
siblings outside the digest. Explicit Content-Type always wins; invalid
metadata still fails instead of falling back. Encoded children remain opaque.

```rust
use mimecraft::MimeEntity;

let raw = b"Content-Type: multipart/digest; boundary=d\r\n\r\n\
--d\r\n\r\nSubject: attached\r\n\r\nbody\r\n--d--\r\n";
let MimeEntity::Multipart(mut digest) = MimeEntity::load(&mut &raw[..]).unwrap() else {
    unreachable!()
};
let MimeEntity::Message(part) = &digest.children()[0] else {
    unreachable!()
};
assert!(part.content_type().unwrap().is_none());
assert!(part.message().unwrap().headers().contains("Subject"));
let mut output = Vec::new();
digest.write_to(&mut output).unwrap();
assert_eq!(output, raw);
```

No default header is inserted: `MessagePart::content_type()` still returns
`None` for an implicit digest child. The tree writer accepts that missing type
only under the immediate digest parent. Before writing the child independently,
moving it under another container, or changing the parent's subtype, assign an
explicit message/rfc822 Content-Type with `set_content_type`; otherwise writing
fails explicitly. Raw header edits and ownership operations do not silently
materialize defaults.

### Bounded source streams

`io::BoundedStream::new(source, start, end)` exposes a range without copying its
bytes. `start` and the exclusive `end: Option<u64>` are absolute offsets in the
supplied source; `None` leaves the end unbounded. Positions and seeks through
the view are relative to `start`. Construction performs no I/O and rejects an
end before the start, but does not check whether the boundaries exist yet.

```rust
use std::io::Cursor;
use mimecraft::{ContentEncoding, MimeContent, io::BoundedStream};

let mut message = Cursor::new(b"header\r\nQ2F0\r\nsuffix");
let body = BoundedStream::new(&mut message, 8, Some(14)).unwrap();
let mut content = MimeContent::with_encoding(body, ContentEncoding::Base64);
let mut decoded = Vec::new();
content.decode_to(&mut decoded).unwrap();
assert_eq!(decoded, b"Cat");
assert_eq!(content.get_ref().position(), 0);
```

The source must implement `Seek`. The view implements `Read` when the source
also supports reading, and `Write` when it supports writing. Unlike MimeKit's
nonseekable fallback, the start boundary always identifies a seekable offset.
Owned streams and mutable references both work; borrowing replaces
`leaveOpen`. Nested views use their immediate source's coordinate system.
The adapter does not discover MIME ranges itself; `MimeParser` with `Source`
storage discovers them and constructs read-only bounded shared views.

`io::SharedStream` owns a seekable input (or a mutable borrow), with an
independent absolute cursor per clone and no source-mutation API. A shared
mutex keeps each seek/read pair atomic. Construction queries the input
position; the source is released when the last handle drops. Source errors,
incorrect absolute seek results, and poisoned locks are explicit I/O errors.

Reads are clipped to the remaining range and return zero at a fixed end
without accessing the source. Earlier source EOF does not shorten the declared
range. Empty reads/writes perform no I/O. Writes follow MimeKit's whole-request
boundary check: a request crossing the end fails before writing even a prefix.
An underlying short write is still reported normally. Writes may grow the
underlying source within the configured range. Ordinary read/write errors,
including `Interrupted`, propagate; standard `read_exact`/`write_all` provide
their usual retries. A zero write becomes `WriteZero` through `write_all`.

`position()` and `stream_position()` report the logical cursor without I/O.
`get_ref`/`get_mut` expose the source, and `into_inner` returns it without I/O.
Direct source access deliberately bypasses the range. Reads/writes that reach
the source resynchronize its cursor first, so moving it through `get_mut` does
not change the view's logical position. An explicit seek, including
`SeekFrom::Current(0)`, also restores the corresponding source offset.
Logical position advances only on success; failures may move the underlying
cursor, which is resynchronized on the next operation.

`len()` reports `end - start` without I/O for a fixed range, even if the source
is shorter. For an unbounded range it queries the source end and restores the
source's original cursor on success; query or restoration errors propagate.
An end before the start is an explicit `InvalidInput` error, not a negative
length. `is_empty()` uses the same length rules.

Seeks cannot precede the view's start or pass a configured end. Unlike
MimeKit's nonnegative end-relative special case, **all** `SeekFrom::End`
offsets use the view's end, not the full source's end. An unbounded view can
seek beyond EOF if its source permits it. EOF and source length are not cached,
allowing later reads to observe growth. Offset arithmetic is checked; invalid
seeks, oversized writes, and offset overflow return `InvalidInput`. A source
that reports a different absolute seek result returns `InvalidData` before
the view reads or writes at that unexpected position.

`flush()` delegates to the source. Dropping the view neither flushes nor restores
position; borrowed sources remain available. Resizing (`SetLength`),
nonseekable sources, timeout forwarding, and async I/O are not implemented.

### Chained streams

`io::ChainedStream<R>` presents seekable children as one logical stream.
Create an empty chain and `add` children, convert a `Vec<R>`, or collect an
iterator. These operations perform no I/O. Child buffers are not copied;
owned streams and mutable references both work. Reads/writes additionally
require the corresponding trait on `R`.

```rust
use std::io::{BufReader, Cursor, Read};
use mimecraft::{HeaderList, io::ChainedStream};

let mut chain = ChainedStream::new();
chain.add(Cursor::new(&b"Content-Type: text/plain\r\n\r\n"[..]));
chain.add(Cursor::new(&b"Hello, world!\r\n"[..]));
let mut reader = BufReader::new(chain);
let headers = HeaderList::load(&mut reader).unwrap();
assert_eq!(headers.get("content-type").unwrap().decode_value().text, "text/plain");
let mut body = String::new();
reader.read_to_string(&mut body).unwrap();
assert_eq!(body, "Hello, world!\r\n");
```

Every child is addressed from its own offset zero, **not its initial cursor**.
This differs from MimeKit's initial sequential reads but keeps reading and
seeking in the same coordinate system. Wrap a child in `BoundedStream` to
select only a range. Child lengths must remain stable except for writes through
the chain; explicitly seek again after any external length change.

Reads skip empty/exhausted children in insertion order. EOF is not latched,
and appending after EOF makes the new child readable. A child that returns zero
before its declared length produces `UnexpectedEof`, rather than silently
changing the offsets of later children. This detects truncated bounded ranges.
An empty chain reads as EOF.

Writes fill each non-final child's remaining length and then spill into the
next. Only the final child may grow, subject to its own writer; a bounded final
child still rejects overlong requests. The previous full writer is flushed
before advancing. A failed boundary flush leaves the chain ready to retry
without duplicating bytes accepted by earlier calls. `flush()` flushes only the
current child, matching MimeKit. Explicit seeks do not flush the old child;
flush before seeking if its writer requires it. A nonempty write to an empty
chain returns `WriteZero`. Empty reads/writes perform no I/O.

A single read/write returns immediately after transferring bytes from/to one
child. It can therefore be short at a boundary, unlike MimeKit's fill-buffer
loop. This preserves Rust's I/O contract: an error in a later child cannot hide
already-transferred progress. `read_exact`, `read_to_end`, and `write_all`
traverse boundaries normally, with their usual handling of interruptions.
Other read/write/flush failures propagate, and source zero writes remain zero.
Earlier successful calls are never rolled back.

`position()` and `stream_position()` report the combined logical cursor without
I/O. `len()` queries and sums current child lengths, restoring child cursors on
success; `is_empty()` tests that byte length, not the child count.
`SeekFrom::Start`, `Current`, and `End` address the full concatenation, including
its exact end, but cannot seek outside it. Seeking takes O(number of children)
time and temporary length storage. The selected child's cursor is synchronized
immediately; other children are positioned lazily when reached, rather than
eagerly rewound. Query/seek failures leave logical position unchanged even if
they move child cursors; subsequent I/O resynchronizes the active child.
Lengths and offsets are checked for overflow.

`get_ref()` exposes children in order; `into_inner()` returns them without I/O.
Dropping the chain neither flushes nor rewinds its children and leaves borrowed
streams open. It can be used inside `MimeContent` or contain bounded body ranges.
Nonseekable chaining and async I/O remain unported; resizing is not supported.

### Block-backed memory streams

`io::MemoryBlockStream` stores bytes in separate 2048-byte allocations rather
than repeatedly growing and copying a single contiguous buffer. It implements
`Read`, `Write`, and `Seek`, and can back `MimeContent`, bounded ranges, or
chained streams. Wrap it in `BufReader` for buffered header loading.

```rust
use std::io::{Seek, SeekFrom, Write};
use mimecraft::io::MemoryBlockStream;

let mut stream = MemoryBlockStream::new();
stream.write_all(b"body").unwrap();
stream.set_len(8).unwrap();
stream.seek(SeekFrom::End(-1)).unwrap();
stream.write_all(b"!").unwrap();
assert_eq!(stream.to_vec().unwrap(), b"body\0\0\0!");
assert_eq!(stream.position(), 8);
```

`new()` and `default()` allocate nothing. Writes overwrite existing bytes and
can extend the end; reads fill the requested buffer across block boundaries up
to EOF. Empty I/O does nothing, and `flush()` is a no-op. `len()`, `is_empty()`,
and `position()` query state without I/O.

Like MimeKit, seeks must stay within `0..=len()`; unlike `Cursor`, seeking past
EOF is rejected. To create a zero-filled gap, first grow with `set_len()`, then
seek within the new length. Resizing clamps position to the new end when needed.
Shrinking releases unneeded blocks and clears the retained block's unused tail,
so regrowth never exposes truncated bytes. All newly allocated blocks are
zero-initialized; MimeKit's global buffer pool is not ported.

`to_vec()` copies all logical bytes into an independent contiguous allocation
without changing position. It does not include spare block capacity. There is
no direct contiguous-buffer accessor.

Lengths and positions use `u64`. `MAX_LEN` preserves MimeKit's capacity ceiling
of `i32::MAX * 2048` bytes, with an additional platform `usize` limit. Writes and
resizing consistently accept the inclusive ceiling, rather than MimeKit's
exclusive write check. Invalid lengths/seeks return `InvalidInput`.
Allocation reservation failures return `OutOfMemory`, leaving existing bytes,
length, and position unchanged, although successfully reserved spare blocks may
remain. Contiguous `to_vec()` allocation can fail even when segmented storage
fits. Drop releases owned blocks; pooling, async I/O, and timeouts are not ported.

### Measuring output

`io::MeasuringStream` implements `Write` and `Seek` without retaining or
inspecting written bytes. Use it as a destination for header serialization,
MIME content copying, or a filtered writer to measure actual encoded output.

```rust
use std::io::Write;
use mimecraft::{
    encodings::Base64Encoder,
    filters::EncoderFilter,
    io::{FilteredWriter, MeasuringStream},
};

let mut writer =
    FilteredWriter::new(MeasuringStream::new(), EncoderFilter::new(Base64Encoder::new()));
writer.write_all(b"Cat").unwrap();
writer.finish().unwrap();
assert_eq!(writer.get_ref().len(), 6); // Q2F0 plus CRLF
```

Always finalize filters before inspecting the measured length: `flush()` alone
does not include pending padding or terminators. The sink itself allocates no
storage for output; filters and serializers may still use their own buffers.

`len()` reports the logical extent, not the cumulative bytes submitted.
Sequential writes increase it; overwriting after a backward seek only increases
it when the write passes the previous end. `position()` tracks the cursor,
and `is_empty()` tests logical length. Seeks must stay within `0..=len()`.
`set_len(u64)` is infallible, allocates nothing, and clamps position on shrink;
`set_len(0)` resets the sink. Empty writes and `flush()` are no-ops.

Offsets and lengths use the full `u64` range. Overflowing writes and invalid
seeks return `InvalidInput` without changing length or position. Reading is
unsupported at compile time, rather than through runtime capability checks.
Async operations and timeouts are not ported.

### Raw header parsing

`HeaderParser` accepts byte slices and appends `Header` records to a caller-owned
`Vec`. It has no dependency on `Read`, async traits, or a runtime. Each `parse`
call returns the number of bytes consumed, including the blank line separating
headers from the body. Preserve the unconsumed suffix for body processing:

```rust
use mimecraft::HeaderParser;

let mut parser = HeaderParser::new();
let mut headers = Vec::new();
parser.parse(b"Subject: hello\r\n\tworld\r", &mut headers).unwrap();
let input = b"\n\r\nbody";
let consumed = parser.parse(input, &mut headers).unwrap();
assert_eq!(&input[consumed..], b"body");
assert!(parser.is_finished());
assert!(headers[0].matches_field("SUBJECT"));
assert_eq!(headers[0].raw_value(), b" hello\r\n\tworld\r\n");
```

At source EOF, call `finish` to emit the last pending record. An empty input
slice does not signal EOF. After a body separator or `finish`, parsing consumes
zero further bytes and repeated finishes do nothing. `reset` discards pending
state and permits a new block; cloning preserves all current state. Records
already emitted remain in the caller's vector.

On streaming-parser records, `Header::raw()` contains the entire original record.
`field()` returns its ASCII name without whitespace before the colon; `matches_field` compares it
case-insensitively. `raw_field()` retains whitespace before the colon, while
`raw_value()` retains everything after it, including folds and the final
newline if present. Duplicate fields remain in input order. No decoding,
unfolding, trimming, newline normalization, or repair of truncated input occurs.

The parser accepts LF and CRLF, including mixed and split endings. Space/tab
continuation lines, even whitespace-only lines, belong to the preceding
record. Bare CR is preserved inside records; a lone CR at the start of the
final line is a body separator, matching MimeKit. `body_separator()` returns
the exact separator, or `None` when EOF occurs without one. Concatenating
record `raw()` bytes and this separator reproduces the consumed input.

Malformed field/colon syntax is explicitly exposed by `is_invalid()`, with
`field() == None`, the whole record in `raw_field()`, and an empty `raw_value()`.
Like MimeKit, a colon with an empty field name is accepted; this is not a
compliance validator. This lower-level scanner preserves an invalid *first*
record too, rather than applying MimeKit's top-level message parser rejection
rules. The current Rust message loader also retains these records.
The scanner does not interpret mbox markers or multipart boundaries.

The default per-record size limit is 16 KiB, matching MimeKit.
Set a positive limit with `HeaderParser::with_max_header_length(NonZeroUsize)`. Field, colon, value,
folded lines, and newline bytes count toward it; the body separator does not.
An oversized record produces `HeaderTooLong` before buffering beyond the
limit. Its `offset()` reports the record's start relative to the parser input,
including bytes supplied in earlier chunks; `limit()` reports the maximum.
Already-emitted records remain available, but the block must be abandoned:
errors are latched until `reset`, not retryable. Reset retains the configured
limit. Callers must separately limit total bytes and record counts.

This is the raw header foundation, not yet complete header handling.
Decoded display access is described below; general decoded-value editing and
formatting and additional structured-field grammars remain unported.

### Standalone headers and decoded display values

`Header::parse(&[u8])` and `str::parse::<Header>()` accept a standalone header,
following MimeKit's public header parser. They require a printable ASCII field
name, optional spaces/tabs, and a colon. Empty names are accepted for source
compatibility. Malformed field syntax returns `HeaderParseError`, whose
`offset()` identifies where a colon was expected in the supplied input slice.

Unlike the byte-preserving streaming parser, standalone parsing appends CRLF
when the input does not end in LF. Existing LF/CRLF endings are retained, and
an existing bare terminal CR is retained before the appended CRLF. The appended
newline is always CRLF, consistent with this crate's defaults, rather than
MimeKit's platform-dependent default.

```rust
use mimecraft::{Header, HeaderList};

let mut headers = HeaderList::new();
headers.add(Header::parse(b"Subject: =?utf-8?q?caf=C3=A9?=").unwrap());
headers.add("X-Trace: original".parse::<Header>().unwrap());

let subject = headers.get("subject").unwrap();
let decoded = subject.decode_value();
assert_eq!(decoded.text, "caf\u{e9}");
assert!(decoded.issues.is_empty());
assert_eq!(subject.raw_value(), b" =?utf-8?q?caf=C3=A9?=\r\n");
```

**Standalone parsing is not a header-block parser or a safe authoring
validator.** Everything after the first field's colon is retained as its value,
including embedded header-like lines, blank lines, controls, and arbitrary
bytes. Folding and value grammar are not validated. Isolate and size-limit a
single header before calling it; this API does not apply the streaming parser's
record limit. Use `HeaderParser` to find header/body boundaries and preserve
unterminated or malformed records exactly.

`decode_value()` follows MimeKit's `Value` getter: decode RFC 2047 text, trim
surrounding Unicode whitespace, then remove remaining CR/LF. Internal spaces,
tabs, quotes, and other controls are preserved, including NUL. Decode-before-
unfold ordering also removes line breaks encoded inside encoded words. This is
display decoding, not structured parsing, control sanitization, or encoding for
safe insertion into a new header. Malformed records have an empty raw value
and return empty decoded text; inspect `is_invalid()` separately.

`decode_value_with_options(&rfc2047::DecodeOptions)` selects strict/loose
encoded-word boundaries and an optional fallback charset for this call.
Valid UTF-8 still takes precedence over the fallback, as do recognized
encoded-word labels. Both methods return `rfc2047::DecodedHeader`: `issues`
retains recovery diagnostics with byte ranges relative to `raw_value()`, not
the complete field or transformed text. Results are uncached; decoding never
changes raw bytes, header identifiers, or collection indices.

`utils::unfold(&str)` exposes the same display unfolding for already-decoded
text. It returns a borrowed subslice when trimming suffices and an owned string
when internal CR/LF must be removed. It does not insert spaces at line breaks,
collapse internal whitespace, decode words, or validate folding syntax.

### Typed values and header records

`Header::parse_addresses()`, `parse_content_type()`, and
`parse_content_disposition()` parse **raw value bytes**, without calling
`decode_value()` first. This keeps encoded punctuation inside names/parameters
from being reinterpreted as structural syntax. Each has a `_with_options`
variant accepting the corresponding `AddressParseOptions` or
`ParameterParseOptions`.

These methods explicitly select a value grammar; they do not dispatch on or
validate the field name, so extension fields can use them too. Syntax-error
offsets and parameter diagnostic ranges refer to `raw_value()`, including its
leading whitespace. Address-name diagnostics retain the address parser's
preprocessed-name coordinates. No fallback value or mutable cache is installed,
and the raw record never changes.

The existing complete-value parsing rules still apply: empty/CFWS-only address
values fail, including an empty Bcc field. Malformed raw records expose empty
values and also fail typed parsing. Field-specific allowances and restrictions
belong to the higher-level message model, not these grammar adapters.

`AddressList::to_header(field)`, `ContentType::to_header()`, and
`ContentDisposition::to_header()` return immutable `Header` records using their
existing checked ASCII formatters. Their `_with_options` variants preserve the
selected folds and LF/CRLF endings exactly. Cached `HeaderId` and field/value
boundaries are initialized without reparsing or decoding the value.

```rust
use mimecraft::{Header, HeaderList};

let mut headers = HeaderList::from(vec![
    Header::parse(b"Subject: unchanged\n")?,
    Header::parse(b"Content-Type: text/plain; charset=iso-8859-1")?,
]);
let mut content_type = headers.get("Content-Type").unwrap().parse_content_type()?;
content_type.set_charset(Some("utf-8"));
headers.replace_all(content_type.to_header()?)?;
assert_eq!(headers.get("Subject").unwrap().raw(), b"Subject: unchanged\n");
assert_eq!(
    headers.get("Content-Type").unwrap().parse_content_type()?.charset(),
    Some("utf-8")
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Constructing the replacement before `replace_all` makes formatting failures
leave the collection untouched. Existing replacement semantics retain the first
matching position, collapse later duplicates, and preserve every unrelated
record's bytes and order; absent fields are appended. Parsed typed values are
independent snapshots: editing one does not update its source header until an
explicit replacement. Encoding does not clear the source model's diagnostics,
though a newly parsed canonical replacement may no longer need recovery.

### Ordered header collections

`HeaderList` owns raw `Header` records in order, including duplicate fields and
malformed records. Move the parser's output vector into a list without copying,
collect an iterator of headers, or `extend` a list with successive parser batches:

```rust
use mimecraft::{HeaderList, HeaderParser};

let mut parser = HeaderParser::new();
let mut records = Vec::new();
let input = b"Received: first\r\nSubject: hello\r\nreceived: second\r\n\r\nbody";
let consumed = parser.parse(input, &mut records).unwrap();
let mut headers = HeaderList::from(records);
assert_eq!(&input[consumed..], b"body");
assert_eq!(headers.get_all("RECEIVED").count(), 2);
assert_eq!(headers.index_of("received"), Some(0));
assert_eq!(headers.last_index_of("received"), Some(2));
assert_eq!(headers.remove("Received").unwrap().raw_value(), b" first\r\n");
assert_eq!(headers.get("received").unwrap().raw_value(), b" second\r\n");
assert_eq!(headers.get("Subject").unwrap().raw_value(), b" hello\r\n");
```

Name queries ignore ASCII case but do not trim whitespace or interpret values.
`get` returns the first occurrence, `get_all` visits all occurrences in order
(or in reverse), and `contains`, `index_of`, and `last_index_of` use the same
matching rule. A `HashMap` indexes each field's first occurrence, giving `get`,
`contains`, and `index_of` expected O(1) access in the number of headers, plus
the cost of hashing/normalizing the query name. Dictionary keys are ASCII
lowercase; already-lowercase queries are borrowed without allocation, while
queries containing uppercase ASCII letters allocate a normalized key. Original
record bytes are unchanged. `get_all` and `last_index_of` still scan the list;
`get_all` remains allocation-free. Use a known `HeaderId`'s `as_str()` for a
canonical query name; `HeaderId::Unknown` is not a wildcard for extension fields.

`add` appends and `insert` inserts before a position, allowing duplicates.
`replace(index, header)` changes only that slot, even if its name changes.
`replace_all(header)` matches MimeKit's `Replace(Header)`: replace at the first
matching field's position and discard later duplicates, or append when absent.
It returns the previous first record, not the discarded duplicates. All supplied
replacement bytes, including field spelling, are retained. `remove` returns
the first matching record and exposes the next duplicate to lookup;
`remove_all` returns the number removed. `remove_at` removes by position.
Other records retain their bytes and relative order.
Appends and extensions update the dictionary incrementally. Positional edits
shift indices and promote the next duplicate when necessary; bulk removals and
duplicate-collapsing replacements rebuild the index once after compaction.

Positional edits report `HeaderListEditError::IndexOutOfBounds`; insertion at
`len()` is allowed, replacement/removal there is not. A field-based replacement
without a parsed name returns `InvalidHeader`. Failed edits leave the list
unchanged. Malformed records remain accessible by position and iteration, but
never match a field-name query. Empty names accepted by the raw parser do match
`""`; collection edits are not strict header-authoring validation.

`iter`, borrowed/owned iteration, `as_slice`, and `into_vec` expose ordered
records; `clear` retains list and dictionary capacity and `with_capacity` can
reserve both up front. Rust ownership replaces shared mutable headers and
change notifications; collection methods keep the dictionary synchronized.
There are no decoded-string setters. Keep separator metadata separately;
collection edits do not repair missing newlines or turn arbitrary raw records
into a valid serialized header block.

### Header collection I/O

`HeaderList::load(&mut impl BufRead)` loads through the first blank line or EOF,
building the case-insensitive index without changing record bytes. It consumes
the separator but leaves body bytes available through the **same buffered
reader**. The reader can prefetch beyond the separator: do not discard its
buffer or switch to its underlying source when reading the body. Seeking is
not required, and neither endpoint is closed.

```rust
use std::io::{BufReader, Read};
use mimecraft::HeaderList;

let mut reader = BufReader::new(&b"Subject: hello\n\nbody"[..]);
let headers = HeaderList::load(&mut reader).unwrap();
let mut body = Vec::new();
reader.read_to_end(&mut body).unwrap();
assert_eq!(body, b"body");

let mut output = Vec::new();
headers.write_to(&mut output).unwrap();
assert_eq!(output, b"Subject: hello\r\n");
// The collection does not own the header/body separator.
output.extend_from_slice(b"\r\n");
output.extend_from_slice(&body);
```

Loading uses the parser's default 16 KiB per-record limit.
`load_with_max_header_length(reader, NonZeroUsize)` selects a different limit;
neither method bounds total header bytes or record count. Empty input yields an
empty list, and an unterminated final record is preserved. Unlike MimeKit's
higher-level parser, malformed first records are retained rather than rejected.
Read errors propagate; an oversized record returns `io::ErrorKind::InvalidData`
containing `HeaderTooLong`. On failure the temporary list is discarded, and
the source is not rewound.

For separator metadata, incremental output, or retryable read errors, use
`HeaderParser::read_from(reader, &mut records)` instead, then construct a
`HeaderList` from the records. It retains parser state and completed records
on I/O failure, so retry with the same parser, reader, and output vector.
Interrupted reads retry automatically. A record-limit failure is terminal:
abandon the block; resetting the parser does not restore the reader position.
On success, `body_separator()` distinguishes LF, CRLF, a lone CR at EOF, and
missing separators. An already-finished parser performs no further I/O.

`write_raw_to` writes each stored record verbatim and does not flush the sink.
`write_to` converts newlines to CRLF and flushes;
`write_to_with_new_line_format` selects CRLF or LF. `Mixed` is rejected before
any output or flush. These methods preserve field spelling, encoded values,
folding whitespace, bare CR bytes, and malformed records without refolding or
re-encoding. The newline-normalizing writer preserves an invalid CR-only first
line verbatim: otherwise repeated Unix conversion could turn it into a blank
line and move later headers into the body. Continuations still get normalized.

**None of these writers adds a header/body separator or a missing record
newline.** This deliberately leaves separator ownership outside `HeaderList`,
unlike MimeKit's separator-tracking collection. Before appending a separator
yourself, ensure the final record has a terminating newline. For byte-exact
round trips, retain the parser's separator and write it after `write_raw_to`.
Serialization does not validate arbitrary records as safe authored headers.

Short and interrupted writes are handled; zero writes, other write failures,
and requested flush failures propagate. Errors may leave partial or complete
output. Retrying a collection writer starts at the beginning, not at the
failure position. The collection itself remains unchanged.

### Header identifiers

`HeaderId` identifies all 131 known field names from MimeKit's current
`HeaderId.cs`. `HeaderId::from_name` uses allocation-free ASCII case-insensitive
lookup; pass just the name, without whitespace, a colon, or a value.
`as_str()` and `Display` return MimeKit's canonical spelling, including
`MIME-Version`, `Message-Id`, and `Content-Md5`.

```rust
use mimecraft::{HeaderId, HeaderParser};

let mut parser = HeaderParser::new();
let mut headers = Vec::new();
parser.parse(b"content-TYPE \t: text/plain\r\n\r\n", &mut headers).unwrap();
assert_eq!(headers[0].id(), HeaderId::ContentType);
assert_eq!(headers[0].field(), Some("content-TYPE"));
assert_eq!(HeaderId::from_name("mime-version"), HeaderId::MimeVersion);
assert_eq!(HeaderId::MimeVersion.as_str(), "MIME-Version");
```

Each `Header` caches its `id()` when emitted by the parser. Looking up an
identifier does not alter field spelling or raw bytes and does not validate
the header value. Unknown extension fields, empty names, and malformed records
all have `HeaderId::Unknown`; use `field()` and `is_invalid()` to distinguish
them. `HeaderId::Unknown.as_str()` returns the diagnostic label `"Unknown"`,
not the original extension name. Use raw bytes for lossless reserialization.

Rust uses idiomatic acronym casing for `ReceivedSpf`, `TlsRequired`,
`X400MtsIdentifier`, and `XMsMailPriority`. Enum numeric values are not a
compatibility contract. `AutoSubmitted` (`Auto-Submitted`) and the legacy
`Autosubmitted` are distinct identifiers. DKIM, ARC, and Autocrypt names are
recognized as ordinary metadata only; no cryptographic support is added.

### Filename MIME-types and preferred extensions

`MimeTypes` provides 562 filename-extension mappings and 409 independent
preferred-extension mappings, based on MimeKit with selected IANA updates.
Built-in tables are immutable;
`new()` and `Default` start with no overrides. Registrations belong to an
instance, not a global registry, and cloning copies overrides independently.

```rust
use mimecraft::MimeTypes;

let mut types = MimeTypes::new();
assert_eq!(types.mime_type("photo.JPEG"), "image/jpeg");
assert_eq!(types.extension("IMAGE/JPEG"), Some(".jpg"));
assert_eq!(types.mime_type("unknown"), "application/octet-stream");

types.register("application/vnd.example", "Example").unwrap();
assert_eq!(types.mime_type("file.example"), "application/vnd.example");
assert_eq!(types.extension("application/vnd.example"), Some(".Example"));
```

Filename lookup uses the final dot in the final path segment, treating both
slash and backslash as separators on every platform. A leading-dot filename
such as `.txt` has an extension; a trailing dot does not. `archive.tar.gz` uses
only `.gz`, returning `application/gzip`.
Unknown or absent suffixes yield `application/octet-stream`. Preferred-extension
lookup returns `None` for unknown names. Neither lookup strips whitespace,
Content-Type parameters, URI queries, or fragments, or decodes percent escapes.
These are filename guesses, not content inspection or validation; no filesystem,
OS registry, or network access is involved.

The IANA update checked on October 23, 2026 adds WebAssembly, YAML (`.yaml`
preferred, `.yml` also recognized), TOML, JPEG XL, HEIC/HEIF images and sequences,
and Zstandard. It also adds missing preferred extensions for AVIF and JSON-LD.
Unlike MimeKit's original mappings, `.gz`/`.gzip` now return `application/gzip`
and `.flac` returns `audio/flac`. Reverse lookup still accepts
`application/x-gzip` and `audio/x-flac` for compatibility. This is a targeted
update, not exhaustive coverage of the IANA registry.

`register` adds a leading dot when absent, overwrites just the two supplied
lookup keys, and preserves other aliases and old reverse mappings. Returned
values retain spelling. Only empty MIME-type/extension strings are errors,
leaving the registry unchanged; nonempty values are not syntax-validated.
Multi-dot extensions can be registered but cannot match final-suffix lookup.
Lookup keys ignore ASCII case; non-ASCII characters are literal, unlike
MimeKit's Unicode-aware case comparer. Portable path separators and local
registration deliberately replace MimeKit's platform-dependent path handling
and global mutable dictionaries.

### Basic header values

`ContentEncoding::parse(&str)` and `"base64".parse::<ContentEncoding>()` follow
MimeKit's tolerant Content-Transfer-Encoding value parsing. They skip leading
Unicode whitespace and recognize the first token case-insensitively. Whitespace
or a semicolon ends the token; everything following it is ignored. This retains
MimeKit's workaround for Google Docs appending a semicolon.

The accepted tokens are `7bit`, `8bit`, `binary`, `base64`, `quoted-printable`,
`x-uuencode`, `uuencode`, and `x-uue`. The last three all select `UuEncode`.
`ContentEncoding::as_str()` returns the canonical token, using `x-uuencode`
for UU, or `None` for `Unspecified`. Empty or unrecognized input returns
`UnknownContentEncoding`, never a success-shaped `Unspecified`.

This is not strict validation: `base64; junk` and `base64 other text` are
accepted, while `(comment) base64` and `base64(comment)` are rejected. It does
not decode encoded words or charsets. When using raw headers, explicitly
convert the value to UTF-8 with `std::str::from_utf8` rather than silently
replacing undecodable bytes.

`MimeVersion::parse(&[u8])` and `str::parse::<MimeVersion>()` port MimeKit's
version utility. They accept two through four ASCII decimal components in
`0..=i32::MAX`, with nested comments and MIME whitespace around numbers and dots.
Unlike encoding-value parsing, whitespace here means only space, tab, CR, and
LF. Comments are byte-oriented, may contain non-UTF-8 bytes, and respect escaped
parentheses and backslashes without recursive parsing.

```rust
use mimecraft::{ContentEncoding, MimeVersion};

assert_eq!(" BASE64;\r\n".parse(), Ok(ContentEncoding::Base64));
assert_eq!(ContentEncoding::UuEncode.as_str(), Some("x-uuencode"));
let version = MimeVersion::parse(b" 1 (comment) .\t0\r\n").unwrap();
assert_eq!(version, MimeVersion::V1_0);
assert_eq!(version.major(), 1);
assert_eq!(version.minor(), 0);
assert_eq!(version.to_string(), "1.0");
```

`components()` retains all parsed components; `build()` and `revision()` expose
the optional third and fourth values. Formatting removes comments, whitespace,
and leading zeros. `1.0`, `1.0.0`, and `1.0.0.0` remain distinct. Acceptance of
other versions does not imply support for them. For compatibility, MimeKit's
exact-end trailing-dot quirk is retained: `1.0.` succeeds, but `1.0. ` fails.
Malformed versions return `MimeVersionParseError` with a reason and zero-based
byte offset. Neither value parser modifies the raw header record.

### Message identifiers

`MessageId::parse(&[u8])` and `str::parse::<MessageId>()` interpret Message-Id or
Content-Id values using MimeKit's tolerant grammar. They require a complete
value, allowing leading/trailing comments and MIME whitespace but rejecting
additional tokens after the identifier. `MessageId::parse_prefix` instead
returns `(identifier, bytes_consumed)`, matching `MimeUtils.ParseMessageId`'s
prefix extraction without silently discarding the remainder.

```rust
use mimecraft::MessageId;

let id = MessageId::parse(b" <local@@example.com.> (comment)").unwrap();
assert_eq!(id.as_str(), "local@example.com");
let input = b"<one@example.com> <two@example.com>";
let (first, consumed) = MessageId::parse_prefix(input).unwrap();
assert_eq!(first.as_str(), "one@example.com");
assert_eq!(&input[consumed..], b" <two@example.com>");
assert!(MessageId::parse(input).is_err());
```

`as_str()`, `into_string()`, and `Display` expose the normalized identifier
without outer angle brackets. Quoted local parts retain quotes and escapes but
lose CR/LF; unquoted local-part whitespace is removed. Domain comments and
whitespace are removed, including whitespace inside domain literals. Case and
UTF-8 are preserved without IDN conversion. Invalid UTF-8 in local parts or
domain atoms is an error; comments may contain opaque non-UTF-8 bytes.

Compatibility includes missing domains (`<local@>`), local-only identifiers,
repeated initial `@` signs, multiple domains, Exchange square-bracket wrappers,
and stripped trailing domain dots. Domain literals are parsed as tokens, not
validated as IP addresses. `MessageIdParseError` exposes a reason and a
zero-based input byte offset. Parsing never changes the raw header record.

**The reading APIs are not header-authoring validators.** Even complete parsing
retains MimeKit's loose token grammar: it may accept controls, collapse unquoted
whitespace, or tolerate a closing `>` without an opening `<`. Do not insert the
result into a header without separate authoring validation. Use
`MessageId::from_value` for MimeKit's setter-style checks, or
`MessageId::generate` to author new identifiers.
Prefix parsing begins at the first non-comment token; it does not search for
an embedded `<id>` and can absorb whitespace-separated text in a loose local
part. Both APIs take a complete input slice, independent of I/O or chunking.

`MessageId::from_value(&str)` ports the assignment rules used by MimeKit's
message-ID, content-ID, and reference-list setters. It trims outer Unicode
whitespace, rejects NULs, and requires every remaining CRLF, bare CR, or bare LF
to be followed by ASCII space or tab. Accepted folds lose the line break but
retain the following whitespace. Because trimming happens first, outer line
endings are removed rather than rejected.

```rust
use mimecraft::MessageId;

let id = MessageId::from_value("\"x\r\n y\"@example.com").unwrap();
assert_eq!(id.as_str(), "\"x y\"@example.com");
assert!(MessageId::from_value("a b@example.com").is_err());
assert!(MessageId::from_value("a@b\r\nBcc: other@example.com").is_err());
```

Already-bracketed values are parsed as complete tokens. Unbracketed values are
wrapped for parsing, but first reject unquoted space/tab so implicit wrapping
cannot turn `a b@c` into `ab@c`. Explicit `<a b@c>` retains MimeKit's tolerant
normalization. Trailing extra identifiers, a missing closing `>`, and a stray
closing `>` after a bare ID are rejected. `<a@b> (comment)` is accepted, while
`a@b (comment)` fails the bare-whitespace check. Parsed getter-style values such
as `(paren@example.com` are accepted.

Failures use `MessageIdParseError`, including `NulCharacter`,
`InvalidLineBreak`, and `UnquotedWhitespace`. Offsets refer to the original
string before trimming, unfolding, or inserting brackets; errors on synthetic
brackets map to the trimmed value's boundary. No per-byte offset map is retained.
Successful values contain no NUL, CR, or LF. This is still not strict RFC
validation: unusual compatibility IDs and other local-part controls remain
possible. `MessageId` itself imposes no length limit and supplies no header
formatter. `FromStr` remains the tolerant reading API; it does not implicitly
opt into assignment validation.
`MimeMessage`'s identifier and reference setters use this assignment validation
before formatting complete headers; standalone `MessageId` remains a value type.

`MessageId::generate()` uses a process-wide domain, defaulting to
`localhost.localdomain`. Call `MessageId::set_default_domain(domain)` once at
startup to configure it. The first `generate()` or `default_domain()` call
permanently initializes the default if it was not configured, even if generation
subsequently fails. Later configuration attempts return
`MessageIdDomainError::AlreadyInitialized`, including attempts to set the same
value. Invalid domains return `InvalidDomain` without initializing the setting.
Concurrent initialization has exactly one winner.

`MessageId::default_domain()` returns the normalized domain.
`MessageId::generate_with_domain(domain)` is an independent per-call override:
it neither reads nor initializes the default. No hostname discovery or DNS
lookup is performed, and there is no hostname dependency.
Generation combines UTC ticks since year 1 and eight OS-random bytes, using
MimeKit's uppercase, least-significant-digit-first base36 representation and
big-endian random integer interpretation. It returns a `MessageId` without angle
brackets; callers explicitly assign the result to Message-Id or Content-Id.

```rust
use mimecraft::{MessageId, MimeMessage};

MessageId::set_default_domain("example.com").unwrap();
let id = MessageId::generate().unwrap();
let mut message: MimeMessage = MimeMessage::new();
message.set_message_id(id.as_str()).unwrap();
```

Generation uses `getrandom` for OS entropy, not a time-seeded PRNG. Uniqueness
is probabilistic; these identifiers are not authentication tokens.
`MessageIdGenerationError` distinguishes invalid explicit domains, clock values
outside years 1 through 9999, and entropy failures,
retaining underlying error sources.
Domain and clock checks precede the entropy request; failures never substitute
a weak random source or fallback identifier. Unlike MimeKit's platform IDN
wrapper, generation uses the existing MIME-safe IDNA validation, lowercases
ASCII domains, and removes an optional final DNS root dot. This validation also
applies to the configured default domain. Build targets must support a
`getrandom` backend; targets without a default randomness backend require its
explicit platform-specific configuration.

### References and In-Reply-To

`References::new(&[u8])` lazily enumerates bracketed message identifiers from a
complete raw header value. For text, pass `value.as_bytes()`. The iterator
borrows its input and yields `Result<MessageId, MessageIdParseError>`; identifiers
are allocated on demand, not collected in advance. Order and duplicates are
preserved, and the iterator supports cloning and fused iteration.

```rust
use mimecraft::{MessageIdParseErrorKind, References};

let mut references = References::new(b"prose <first@x> <> <last@x>");
assert_eq!(references.next().unwrap().unwrap().as_str(), "first@x");
assert_eq!(references.next().unwrap().unwrap_err().kind(),
           MessageIdParseErrorKind::EmptyId);
assert_eq!(references.next().unwrap().unwrap().as_str(), "last@x");
assert!(references.next().is_none());
```

Scanning follows `MimeUtils.EnumerateReferences`: comments, quoted prose, bare
words, and non-candidate punctuation are skipped. Only a `<` outside those
constructs starts an identifier; an unbracketed `local@example.com` is not
returned. Candidates use the same compatibility grammar as `MessageId`.

Unlike MimeKit's silent recovery, malformed candidates and unterminated comments
or quoted strings yield explicit errors. Offsets are relative to the entire
supplied slice, including when an error occurs after earlier valid entries.
Calling `next()` again resumes from the parser's stopping point, not necessarily
the diagnostic offset or the next `>`. Unterminated comments or quotes consume
the remaining input; apparent identifiers inside them are not recovered.
Collecting into `Result<Vec<_>, _>` stops at the first error; manual iteration
can handle an error and continue.

This is a lazy iterator over complete input, not a chunk-fed parser or a mutable
message-ID list. It does not change raw header bytes or validate IDs for authoring.

### Address specifications

`AddrSpec` represents the `local-part@domain` portion of a mailbox, independently
of a display name or route. `parse` and `FromStr` consume a complete value;
`parse_prefix` returns the address and consumed byte count for an enclosing
address parser. Leading/trailing comments and whitespace count as consumed.
Comma, `>`, and `;` delimiters, and other suffixes after a domain, remain untouched.
An opening `<`, display-name phrase, group, or route is not accepted here.

```rust
use mimecraft::{AddrSpec, AddrSpecParseOptions, AddressComplianceMode};

let address = AddrSpec::parse(b"user (note) . name @ Example.COM")?;
assert_eq!(address.local_part(), "user.name");
assert_eq!(address.domain(), Some("Example.COM"));
assert_eq!(address.as_str(), "user.name@Example.COM");

let options = AddrSpecParseOptions {
    compliance_mode: AddressComplianceMode::Strict,
    allow_local_only: false,
};
assert!(AddrSpec::parse_with_options(b"local", &options).is_err());
# Ok::<(), mimecraft::AddrSpecParseError>(())
```

Quoted words and their backslash escapes are preserved, including embedded `@`
characters and tabs. Comments/whitespace outside words are removed; even
obsolete quoted-word local parts such as `a."b c".d` are supported. ASCII controls
other than TAB are rejected inside quoted words, including escaped controls.
Domain parsing shares the message-ID/route grammar: UTF-8 atoms, literals with
whitespace removed, and tolerated trailing domain dots stripped. Case and ACE
label spelling are retained; parsing never implicitly IDNA-decodes domains.
Literal content is not checked as an IP address.

`AddressComplianceMode::Loose` is the default. Malformed UTF-8 local-part words
are recovered as Latin-1, with `had_latin1_fallback()` reporting the recovery.
The **whole offending word** is decoded as Latin-1, not just invalid bytes.
`Strict` rejects this recovery. `Looser` additionally accepts repeated local
dots, a local dot before `@`, and unquoted escaped `\@` sequences, replacing those
escapes with `%40` as MimeKit does. Domain atoms always require UTF-8.
Local-only addresses are accepted by default and have `domain() == None`;
`allow_local_only` controls this independently of the compatibility mode.

An ASCII contiguous-token fast path avoids separate local/domain allocations;
comments, quotes, international text, and repairs use the full grammar. Errors
identify the original byte offset before normalization or Latin-1 expansion.
Unclosed comments, quotes, and literals point to their opening delimiter;
other errors point to the offending byte or EOF. No partial value is returned.

`ascii_domain()` and `unicode_domain()` return an optional converted domain.
`with_ascii_domain()` and `with_unicode_domain()` return the whole address with
only its domain converted. These methods use the explicit IDNA policy below,
including validation and lowercase normalization of ASCII domain names. They
return conversion errors rather than silently preserving invalid names.
Bracketed literals are always preserved, even when they contain ACE-looking
text, and the stored address is never changed.

The local part is **never** IDNA-encoded, case-normalized, or downgraded.
Consequently, `with_ascii_domain()` can still return a non-ASCII address.
`is_international()` inspects the currently stored spelling; an ASCII ACE domain
alone does not make it true. These are compatibility parsing/conversion APIs,
not SMTP authoring validation: strict mode still accepts obsolete syntax, and
parsing enforces neither SMTP length limits nor DNS validity. Empty input is
rejected rather than representing a null reverse path. `MailboxAddress` adds display
names and routes around this address component; `GroupAddress` and `AddressList`
add groups and lists. `AddressList::to_header_string` provides checked, folded
ASCII address headers separately from compatibility parsing.

### Single mailboxes

`MailboxAddress` adapts the current MimeKit address parser for one mailbox, reusing
`AddrSpec`, `DomainList`, and RFC 2047 name decoding rather than maintaining
separate local-part/domain grammars.

```rust
use mimecraft::MailboxAddress;

let mailbox: MailboxAddress =
    "=?utf-8?q?Ren=C3=A9e?= <@relay.example:user@example.com>".parse()?;
assert_eq!(mailbox.display_name(), "Ren\u{e9}e");
assert_eq!(mailbox.address().local_part(), "user");
assert_eq!(mailbox.route().as_slice(), &["relay.example"]);

let mailbox = MailboxAddress::parse(b"user@example.com (Display Name)")?;
assert_eq!(mailbox.display_name(), "Display Name");
# Ok::<(), mimecraft::MailboxParseError>(())
```

`parse`/`FromStr` require a complete mailbox. `parse_prefix` returns the mailbox
and consumed bytes, leaving separators or other suffixes for an enclosing
grammar; trailing comments/whitespace are consumed. Groups, multiple mailboxes,
and empty/null reverse paths are not accepted by complete parsing. Input is a
complete value or token, not incremental chunks.

`MailboxParseOptions::address` uses the existing addr-spec compatibility
settings. Loose modes accept excess/unbalanced angle brackets, a missing
display-name closing quote, an addr-spec used as an unquoted display name, and
square brackets/unbalanced closing parentheses in names. Strict mode rejects
these repairs; it is still a compatibility parser, not full RFC validation.
As in MimeKit, disabling `address.allow_local_only` rejects bare local-only
addresses but still permits angle-bracketed forms such as `<local>`.

Unquoted commas in names are enabled by default, even in strict mode, so names
such as `Worthington, Warren <warren@example.com>` can be read. Set
`allow_unquoted_commas` to false when this would be ambiguous with separators
between local-only addresses. Dots in names are tolerated in all modes.

Phrase names are unquoted **before** RFC 2047 decoding, preserving quotes and
backslashes encoded inside encoded words. Raw phrase tabs become spaces and
raw CR/LF are removed. For bare addresses the first trailing comment supplies
the name; comment tabs become spaces, but quotes, escapes, nested parentheses,
and internal line breaks remain. Comments after angle-bracketed mailboxes do
not supply a name. All decoded names are trimmed.

`name_decoding` independently configures RFC 2047 boundary recognition and a
preferred fallback charset. Charset recovery is exposed through
`display_name_issues()`; its ranges refer to the **preprocessed name bytes**,
before decoding/trimming, not offsets in the original mailbox. Addr-spec
Latin-1 recovery remains available through `address().had_latin1_fallback()`.
Syntax errors retain original mailbox byte offsets, including rebased errors
from the shared address/route parsers; no partial mailbox is returned.

Names containing only ASCII without `=` bypass encoded-word decoding. Domain
spelling is retained and IDNA conversion remains explicit through `address()`
and `route()`. No DNS/SMTP validity or length checks are added by the mailbox
parser. Decoded names can contain controls and line breaks, so these values
are not sanitized header output. Sorting and folded mailbox/header
serialization remain deferred; `Display` provides unencoded text for display only.

### Groups and address lists

`Address` represents either a `MailboxAddress` or an `GroupAddress`. `GroupAddress`
retains its decoded name, name-decoding diagnostics, and ordered `members()`.
`AddressList` stores either kind of address, preserving duplicates and empty
groups. Its `len()` counts top-level addresses, while `mailboxes()` traverses
all mailboxes depth-first in source order, skipping empty groups.

```rust
use mimecraft::{Address, AddressList};

let list: AddressList =
    "Empty:;, Friends: Alice <alice@example.com>, bob@example.com;, carol@example.com"
        .parse()?;
assert_eq!(list.len(), 3);
assert!(matches!(&list.as_slice()[0], Address::Group(group) if group.members().is_empty()));
let mailboxes: Vec<_> = list.mailboxes().map(|m| m.address().as_str()).collect();
assert_eq!(mailboxes, ["alice@example.com", "bob@example.com", "carol@example.com"]);
# Ok::<(), mimecraft::AddressParseError>(())
```

`Address` and `GroupAddress` provide complete-value and prefix parsing; a prefix
consumes trailing comments/whitespace but leaves enclosing list separators.
`AddressList::parse` always parses a complete list. All three also implement
`FromStr`, and their `_with_options` entry points accept `AddressParseOptions`.
Its `mailbox` field reuses mailbox grammar/name-decoding options, including
unquoted-comma ambiguity and local-only address handling. Group names use the
same unquoting and RFC 2047 decoding as mailbox phrase names; name diagnostic
ranges refer to the preprocessed name bytes, not the whole address input.

`max_group_depth` defaults to **1**, counting the outermost group. Use **1** for
strict RFC 5322 section 3.4 compliance: groups may contain mailboxes, but not
other groups. Zero disables groups; the `u8` setting caps parsed nesting at
**255**. Parsing and mailbox traversal use explicit stacks. Higher limits allow
non-standard nested groups for compatibility, independently of the mailbox
compliance mode. Loose modes tolerate a missing final semicolon after members,
but `Name:` followed only by comments/whitespace is an error in every mode.
Use `Name:;` for an empty group.

Strict mode requires commas between addresses; loose modes tolerate missing
separators. Repeated/trailing commas after a valid address are accepted in all
modes, but leading commas are not. Parsing is **fail-fast**: invalid members are
never silently dropped or replaced with empty groups, and errors carry byte
offsets in the complete input. MimeKit's internal malformed-header
resynchronization is not implemented here.

Empty or whitespace/comment-only input is an error, while `AddressList::new()`
creates an empty collection. Lists expose ordinary edits through `as_mut_vec()`,
vector conversions, collection, and owned/borrowed iteration. These models also
support construction and editing as described below; authoring validation,
sorting, and folded address-header serialization remain deferred.

### Constructing and editing addresses

`MailboxAddress::new(name, address)` takes Unicode text and an already parsed `AddrSpec`,
starting with an empty route. `set_address` replaces the typed address and its
diagnostics without changing its spelling, display name, or route. Parsing a
replacement first keeps failures explicit and leaves the mailbox unchanged on
parse errors. Unlike MimeKit's string constructor, empty reverse-path addresses
are not representable by `AddrSpec`. Local-only addresses remain supported.

`GroupAddress::new(name, members)` takes an `AddressList`, including an empty
one. `route_mut()` and `members_mut()` expose ordinary collection editing.
`From<MailboxAddress>` and `From<GroupAddress>` convert either model into `Address`.

```rust
use mimecraft::{GroupAddress, AddressList, MailboxAddress};

let mut alice = MailboxAddress::new(" Alice ", "alice@example.com".parse()?);
alice.set_address("alice@example.net".parse()?);
let mut friends = GroupAddress::new(" Friends ", AddressList::new());
friends.members_mut().as_mut_vec().push(alice.into());
assert_eq!(friends.display_name(), "Friends");
assert_eq!(
    friends.members().mailboxes().next().unwrap().address().as_str(),
    "alice@example.net"
);
# Ok::<(), mimecraft::AddrSpecParseError>(())
```

Both constructors and `set_display_name` trim leading/trailing Unicode
whitespace but do **not** unquote, decode encoded words, or sanitize text.
An empty string represents an absent name. Replacing a name clears its previous
decoding diagnostics even when the text is unchanged; unrelated fields and
member diagnostics remain intact. `Address::set_display_name` handles either
variant. Clones own independent names, routes, and members.

Editing does not impose parsing limits or validate header output. In particular,
`max_group_depth` is a parse-time limit only; callers must bound programmatically
assembled trees themselves. Route edits can contain invalid domains, and names
can retain internal controls or line breaks. These APIs do not yet serialize
addresses into headers.

### Address equality and hashing

`MailboxAddress`, `GroupAddress`, `Address`, and `AddressList` implement
`PartialEq`, `Eq`, and `Hash`. Mailbox equality follows MimeKit: the stored
display name and addr-spec text must match exactly, including case. It does
not perform IDNA conversion or Unicode normalization. Consequently, Unicode
and ASCII IDN spellings can differ even when explicit domain conversion would
produce the same result.

Source routes and decoding diagnostics do **not** affect mailbox equality.
Groups compare their exact names and ordered members, ignoring group-name
decoding diagnostics. Lists compare ordered addresses rather than flattened
mailboxes: duplicates, empty groups, group boundaries, and member order all
matter. A mailbox and a group are always different variants.

```rust
use std::collections::HashSet;
use mimecraft::MailboxAddress;

let plain: MailboxAddress = "User <user@example.com>".parse()?;
let routed: MailboxAddress = "User <@relay.example:user@example.com>".parse()?;
assert_eq!(plain, routed);
assert_ne!(plain.to_string(), routed.to_string());
assert_eq!(HashSet::from([plain, routed]).len(), 1);
# Ok::<(), mimecraft::MailboxParseError>(())
```

Hashing uses the same fields as equality, not display strings; equal values
therefore hash equally despite differing routes or diagnostics. This deliberately
differs from MimeKit's display-based hash calculation. Hash values are not a
stable serialization format. These comparisons describe stored address models,
not SMTP recipient equivalence. Sorting remains deferred: MimeKit's
case-insensitive comparison must not become a Rust `Ord` implementation that
disagrees with case-sensitive `Eq`.

### Address display formatting

`MailboxAddress`, `GroupAddress`, `Address`, and `AddressList` implement
`Display`/`to_string()` using MimeKit's non-encoded display conventions.
A named mailbox always quotes its name and wraps its route/address in angle
brackets. An unnamed mailbox uses angles only when its route has nonblank
domains; otherwise it displays the bare addr-spec. Quotes and backslashes in
mailbox names are escaped with the existing quoting helper. Domain spelling
and Unicode remain unchanged; display performs no IDNA conversion.

Groups display their name **verbatim**, followed by `: `, their comma-separated
members, and `;`. Empty groups therefore display as `Name: ;` (or `: ;` with no
name). Lists join address displays with `, `, preserving duplicates, empty
groups, and nesting; an empty list displays as an empty string. Group/list
formatting uses an explicit stack instead of recursive display calls.

```rust
use mimecraft::AddressList;

let addresses: AddressList = "Friends: Alice <alice@example.com>;, bob@example.com".parse()?;
assert_eq!(
    addresses.to_string(),
    "Friends: \"Alice\" <alice@example.com>;, bob@example.com"
);
# Ok::<(), mimecraft::AddressParseError>(())
```

Display is **not safe header serialization**: it does not encode words, fold
lines, validate domains, or remove controls/line breaks. Group-name punctuation
is not quoted, so arbitrary display strings are not guaranteed to parse back
into the same tree. Name-decoding diagnostics and stored values are unchanged.
Use `to_ascii()` for separately checked, encoded value syntax, or
`AddressList::to_header_string()` for a complete folded ASCII field.

### ASCII address-value encoding

`MailboxAddress`, `GroupAddress`, `Address`, and `AddressList` provide
`to_ascii()` and `to_ascii_with_options(&rfc2047::EncodeOptions)`. Display names
reuse the RFC 2047 phrase encoder: ordinary ASCII names remain plain or quoted,
while Unicode, controls, and encoded-word lookalikes become encoded words.
UTF-8 is the default name charset; the options select another charset and an
encoded-word limit, not a physical-line limit.

```rust
use mimecraft::MailboxAddress;

let mailbox = MailboxAddress::new("Ren\u{e9}e", "user@b\u{fc}cher.example".parse()?);
assert_eq!(
    mailbox.to_ascii()?,
    "=?utf-8?b?UmVuw6ll?= <user@xn--bcher-kva.example>"
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Domains and nonblank route entries use checked IDNA conversion; domain literals
retain their spelling and are not checked as IP addresses. Local parts retain
their spelling, but international local parts are explicitly rejected rather
than silently downgraded. Compatibility-only malformed addr-spec syntax, such
as consecutive local-part dots, is rejected by strict reparsing. Local-only
mailboxes use angle brackets so they remain parseable when bare local-only
addresses are disabled.

Group names use phrase syntax too, with `""` for unnamed groups. Lists preserve
order, duplicates, empty groups, and nesting; empty lists encode to an empty
string. Iterative group encoding imposes no additional nesting limit, so
programmatically built trees remain caller-bounded. Nested groups remain a
compatibility extension, not strict RFC 5322 group syntax.

`AddressEncodeError` reports the failure kind and a path of zero-based member
indexes. List paths start at the top-level list; standalone-group paths start
at its members. An empty path denotes the standalone address/group itself or
invalid options checked before traversal. Route errors also retain the original
route index, including skipped blanks. No partial output is returned, and
encoding never mutates stored values or decoding diagnostics.

These methods return **unfolded value syntax**, not complete transport headers.
They add no field name or terminal newline and do not enforce physical-line or
SMTP address-length limits. Use `AddressList::to_header_string` when a complete
folded field is needed. Name controls are encoded, not sanitized: decoded names
can still contain controls. These value APIs remain ASCII-only; international
output is available through the complete header formatter.

### Folded address headers

`AddressList::to_header_string(field)` returns a complete ASCII header with
the supplied field name, colon, encoded addresses, folds as needed, and one
terminating CRLF. An empty list produces `field:\r\n`, useful for an empty Bcc.
The field must be nonempty printable ASCII without a colon or whitespace;
its spelling is preserved. Callers choose an appropriate field and enforce
field-specific mailbox-count and group restrictions.

```rust
use mimecraft::{AddressList, Header};

let list: AddressList = "Alice <alice@example.com>, bob@example.com".parse()?;
let encoded = list.to_header_string("To")?;
assert_eq!(encoded, "To: Alice <alice@example.com>, bob@example.com\r\n");
let header = Header::parse(encoded.as_bytes())?;
assert_eq!(AddressList::parse(header.raw_value())?, list);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`to_header_string_with_options(field, &AddressHeaderFormatOptions)` selects
the physical-line limit (`60..=998`, default 78), DOS/Unix newline format,
and RFC 2047 name-encoding options. Mixed newlines are rejected. The limit
counts bytes excluding the newline, with each continuation tab counting as
one byte. Unlike MimeKit's soft target, this bound is enforced.

Folding replaces only syntactic spaces outside quoted strings, addr-specs,
source routes, and domain literals. Quoted whitespace and escapes remain
unchanged. Long names use encoded words, whose limit is additionally capped
at the physical limit minus two bytes to leave room for indentation and a
group's colon. Invalid name options are rejected before this adjustment.

`AddressHeaderFormatError` reports invalid fields/options, wraps address
encoding errors with their member paths, or reports the required byte count
when an indivisible piece cannot fit. Attached commas and semicolons stay with
their preceding piece; exceptionally long routes, quoted local parts, or runs
of closing nested-group semicolons may therefore fail rather than be rewritten.
No partial header is returned, and source values/diagnostics remain unchanged.
International local parts are rejected by default. This does not perform
SMTP address-length validation or decoded-name sanitization.

#### International address headers

Set `AddressHeaderFormatOptions::international` to allow UTF-8 display names
and local parts. The formatter retains stored domain and source-route spelling,
including Unicode, letter case, and ACE labels, after the same MIME-safe IDNA
validation used for ASCII output. It does not decode ACE labels or normalize
raw names to the selected encoding charset. Routes still skip whitespace-only
entries and normalize domain-literal whitespace without validating IP addresses.

```rust
use mimecraft::{AddressList, AddressHeaderFormatOptions};

let list: AddressList = "Ren\u{e9}e <caf\u{e9}@b\u{fc}cher.example>".parse()?;
let options = AddressHeaderFormatOptions {
    international: true,
    ..Default::default()
};
assert_eq!(list.to_header_string_with_options("To", &options)?,
    "To: Ren\u{e9}e <caf\u{e9}@b\u{fc}cher.example>\r\n");
# Ok::<(), Box<dyn std::error::Error>>(())
```

Raw names are quoted when phrase specials or repeated spaces require it.
Individual raw name pieces must fit with a continuation tab and a group's
colon. Oversized names, ASCII controls, and literal encoded-word lookalikes
use whole-name RFC 2047 encoding instead of inserting whitespace into the
decoded name. Encoded fallback honors the chosen charset and encoded-word
limit, returning an error if needed; raw UTF-8 ignores the charset, but invalid
word limits are still rejected. Line limits count UTF-8 bytes, not characters.

Strict addr-spec syntax, domain/route checks, indivisible addr-specs and routes,
and nested error paths still apply. Message address and Sender/Resent-Sender
`_with_options` setters support this mode and remain atomic on failure.
ASCII-only value APIs and default header output are unchanged. For SMTP delivery,
generally set `international` to match whether the server advertises `SMTPUTF8`
in its `EHLO` response. Keep it false if the extension is unavailable; enabling
it is optional when supported. This option does not negotiate SMTPUTF8; callers
must ensure the destination accepts internationalized headers and addresses.

### SMTP envelope address validation

`utils::validate_smtp_address` checks exact addr-spec text for non-SMTPUTF8
envelope use. This is separate from tolerant MIME header parsing and header
formatting: display names, comments, angle brackets, whitespace around the
address, local-only addresses, and the empty reverse path are not accepted.
It performs no IDNA conversion, DNS lookup, delivery check, or normalization.

```rust
use mimecraft::{MailboxAddress, SmtpAddressError, SmtpAddressValidationOptions, utils};

assert_eq!(utils::validate_smtp_address("user@example.com"), Ok(()));
assert_eq!(utils::validate_smtp_address("caf\u{e9}@example.com"),
    Err(SmtpAddressError::NonAscii));

let options = SmtpAddressValidationOptions {
    enforce_length_limits: true,
    ..Default::default()
};
let long = format!("{}@example.com", "a".repeat(65));
assert_eq!(utils::validate_smtp_address(&long), Ok(()));
assert_eq!(utils::validate_smtp_address_with_options(&long, &options),
    Err(SmtpAddressError::LocalPartTooLong));

let mailbox: MailboxAddress = "Ren\u{e9}e <user@example.com>".parse()?;
assert_eq!(mailbox.validate_smtp_with_options(&options), Ok(()));
# Ok::<(), mimecraft::MailboxParseError>(())
```

Errors are reported in syntax, non-ASCII, local-part length, label length,
domain length, then whole-address length order. Length checking is optional:
when enabled, limits are 64, 63, 253, and 254 bytes respectively. Quotes and
escapes count toward local-part length. Domain/label limits do not apply to
address literals, but the whole-address limit does.

Literal syntax supports IPv4 (including decimal leading zeros), `IPv6:`
(SMTP compression must replace at least two groups), and general `tag:content`
literals. General tags and ASCII ACE-looking labels are checked syntactically,
not against a registry or IDNA semantics. `NonAscii` distinguishes otherwise
syntactically valid Unicode text; it does not imply full SMTPUTF8 policy
validation. For Unicode domains with ASCII local parts, explicitly convert
with `AddrSpec::with_ascii_domain` and validate the resulting address.

`allow_iso2022_jp_local_part`, also false by default, permits legacy ISO-2022-JP
escapes inside quoted local parts. This is a nonstandard compatibility exception:
only ASCII, JIS Roman, and paired JIS X 0208 runs are accepted, with a required
return to ASCII before the closing quote. Quotes/backslashes still need wire
escaping; character assignments are not decoded or checked.

Mailbox methods validate only the stored addr-spec, ignoring display names,
source routes, and recovery diagnostics. Parsing may already have normalized
the original text; use the string utility to validate original input. These
checks neither change header parsing/formatting nor negotiate SMTP capabilities.
Raw UTF-8 headers can require SMTPUTF8 even when the envelope is ASCII.

### SMTPUTF8 Unicode address policy

`utils::validate_smtp_utf8_address` ports MimeKit's
`draft-ietf-mailmaint-smtputf8-syntax` policy, which is stricter than RFC 6531
transport syntax alone. It requires bare SMTP mailbox syntax, PRECIS
IdentifierClass code points (plus quoted SPACE), NFC, contextual character
rules, and compatible non-ASCII scripts across the address. It uses the same
literal syntax as the ASCII validator, without its ISO-2022-JP exception.
It neither proves spoof safety nor negotiates SMTPUTF8.

```rust
use mimecraft::{MailboxAddress, SmtpUtf8AddressError, SmtpUtf8AddressValidationOptions, utils};

assert_eq!(utils::validate_smtp_utf8_address("caf\u{e9}@example.com"), Ok(()));
assert_eq!(utils::validate_smtp_utf8_address("cafe\u{301}@example.com"),
    Err(SmtpUtf8AddressError::NotNormalized));
assert_eq!(utils::validate_smtp_utf8_address("user@xn--gr-zia.example.com"),
    Err(SmtpUtf8AddressError::ALabel));

let options = SmtpUtf8AddressValidationOptions {
    decode_idn: true,
    ascii_is_latin: true,
    enforce_length_limits: true,
};
let mailbox: MailboxAddress = "User <user@xn--gr-zia.example.com>".parse()?;
assert_eq!(mailbox.validate_smtp_utf8_with_options(&options), Ok(()));
// Opt-in decoding validates without changing the stored address.
assert_eq!(mailbox.address().as_str(), "user@xn--gr-zia.example.com");
# Ok::<(), mimecraft::MailboxParseError>(())
```

All three options default to false:

- `decode_idn` permits domain A-labels after canonical raw RFC 3492 decoding,
  not UTS #46 mapping. Even without length enforcement, encoded labels must be
  ASCII, at most 63 bytes, contain a non-ASCII scalar after decoding, obey the
  decoded hyphen restrictions, and round-trip ignoring ASCII case. Decoded
  text still undergoes all Unicode checks. Local atoms beginning with `xn--`
  remain rejected; quoted local parts are exempt from this prefix check.
- `ascii_is_latin` counts ASCII letters as Latin separately within the whole
  local part and each domain label, detecting additional mixed-script forms.
  It does not relax the global non-ASCII script policy.
- `enforce_length_limits` measures the local part and original address as
  UTF-8 bytes (64 and 254), and ordinary domain labels/domain in raw Punycode
  A-label form (63 and 253). Quotes and escapes count; domain literals bypass
  label/domain limits. Opt-in decoding does not change the measured spelling.

Errors precede one another in syntax, optional local/label/domain/address
lengths, A-labels, disallowed code points, NFC, context, then mixed-script order.
Common and Inherited scripts are neutral. Han/Hiragana/Katakana, Han/Hangul,
and Han/Bopomofo are compatible families, not one unrestricted CJK group.
Default A-label rejection deliberately differs from the non-SMTPUTF8
validator's syntactic acceptance of ASCII ACE-looking labels.

The bundled policy and normalization tables share MimeKit's Unicode `18.0.0`
snapshot, exposed as `SMTP_UTF8_UNICODE_VERSION`; this identifies the imported
data, not a claim about a finalized Unicode release. The NFC conformance data
comes from MimeKit's `UnitTests/TestData/unicode/NormalizationTest.txt.gz`.
The standalone importer regenerates the tables and 277 upstream address
fixtures (four lone-UTF-16-surrogate cases cannot be represented by Rust strings):

```powershell
rustc --edition 2024 tools\import-smtp-unicode.rs -o target\debug\import-smtp-unicode.exe
.\target\debug\import-smtp-unicode.exe C:\src\MimeKit
```

Mailbox methods check the stored addr-spec only; names, source routes, and
recovery diagnostics are ignored. These checks do not normalize or repair
addresses, validate headers, look up DNS, or establish deliverability.

### Domain routes

`DomainList` represents the obsolete route portion of a mailbox, not a full
address or a general comma-separated domain list. `parse` and `FromStr` require
a complete route; `parse_prefix` returns the route and consumed byte count,
leaving the mailbox's separating colon untouched.

```rust
use mimecraft::DomainList;

let input = b"@relay (note) . example,,@gateway.example:user@example.com";
let (mut route, consumed) = DomainList::parse_prefix(input)?;
assert_eq!(route.as_slice(), &["relay.example", "gateway.example"]);
assert_eq!(&input[consumed..], b":user@example.com");
route.as_mut_vec().push("another.example".into());
assert_eq!(route.to_string(), "@relay.example,@gateway.example,@another.example");
# Ok::<(), mimecraft::DomainListParseError>(())
```

Parsing requires an explicit first `@` after optional comments/whitespace, and
a domain immediately after each `@`. Comments and whitespace around domain
dots and route separators are removed. Case, duplicate domains, and UTF-8 atoms
are preserved; domain literals have whitespace removed but are not validated as
IP addresses. A trailing domain dot before a comma, colon, or EOF is stripped.
Like MimeKit, repeated/trailing commas and adjacent `@domain` entries are
tolerated. Whitespace-only Unicode domains are omitted. Errors expose their
kind and original byte offset; no partial list is returned on failure.

Unlike MimeKit's public prefix-style wrapper, complete parsing rejects trailing
non-route input. Prefix parsing consumes trailing comments, whitespace, and
commas, but leaves other suffixes for the enclosing parser. Neither API is
incremental.

`as_slice` provides indexing, iteration, and case-sensitive string lookup;
`as_mut_vec` provides ordinary vector edits without validation. Vector conversion,
collection, and owned/borrowed iteration preserve all entries. Display skips
empty/whitespace-only entries without modifying storage, joins the remaining
strings with commas and `@` prefixes, and does no escaping or folding. Arbitrary
edited strings are **not safe header syntax**.

`to_ascii()` separately formats validated ASCII route value syntax without
modifying the list. It skips blank entries, converts names through the IDNA
helpers below, and validates bracketed literals through the existing MIME
domain parser. Internal literal whitespace is removed; literals are still
not validated as IP addresses. Output has no enclosing angle brackets, mailbox
colon, folding, or terminating newline.

```rust
use mimecraft::DomainList;

let route: DomainList = [" \t ", "b\u{fc}cher.example", "[IPv6: ::1]"]
    .into_iter().collect();
assert_eq!(route.to_ascii()?, "@xn--bcher-kva.example,@[IPv6:::1]");
# Ok::<(), mimecraft::DomainListEncodeError>(())
```

Unlike MimeKit's route encoder, all emitted names are validated and normalized,
including ASCII names; their case becomes lowercase. `DomainListEncodeError`
identifies the original stored index, including skipped blank entries, and
distinguishes IDNA failures from malformed literals. Failure returns no partial
output and never silently passes through Unicode. Raw display and tolerant
parsing keep their previous behavior. `MailboxAddress` integrates route parsing,
including within groups/lists; mailbox header formatting remains unported.

### International domain names

`mimecraft::idna::to_ascii` and `to_unicode` adapt MimeKit's `Punycode` wrapper
using the `idna` crate and its Unicode-data dependencies rather than implementing
Unicode normalization and IDNA tables locally. Both return
`Result<Cow<str>, IdnaError>` and borrow unchanged input when possible.

```rust
use mimecraft::idna;

assert_eq!(idna::to_ascii("b\u{fc}cher.example")?, "xn--bcher-kva.example");
assert_eq!(idna::to_unicode("xn--bcher-kva.example")?, "b\u{fc}cher.example");
assert!(idna::to_ascii("example\u{ff20}evil.com").is_err());
# Ok::<(), idna::IdnaError>(())
```

The fixed policy is nontransitional UTS #46 processing with hyphen, bidi,
and joiner checks enabled. Names are case-normalized, canonical Unicode
composition and compatibility mappings are applied, and valid ACE labels are
decoded by `to_unicode`. Unicode dot variants map to ASCII dots; an optional
final root dot is preserved. Both directions enforce nonempty labels of at most
63 **encoded ASCII bytes** and a maximum name length of 253 bytes excluding the
root dot. STD3's letters/digits/hyphens-only rule is not enabled: underscores
and other MIME atom characters remain allowed.

ASCII results are additionally checked against the MIME domain alphabet.
Unicode characters such as THIN SPACE and FULLWIDTH COMMERCIAL AT must not
introduce ASCII whitespace or address delimiters. Invalid IDNA/lengths produce
`InvalidDomain`; otherwise unsafe mapped ASCII produces `UnsafeAscii`.
`to_unicode` first validates the ASCII form under the same policy and never
returns partially decoded or replacement-character error output. The backend
does not expose per-rule diagnostics or input byte offsets.

These are domain-only APIs, not mailbox/URL/literal parsers. Pass a Rust string
slice instead of using C# index/count overloads. No platform `IdnMapping` options
or implicit unchanged-input fallback are reproduced. Unicode versions, mapping,
and case behavior can differ from .NET. Conversion does not perform DNS lookup,
check mailbox deliverability, or detect visually confusable Unicode names.

### Character sets

`mimecraft::charsets` provides `Charset` and `decode_best`. This is the shared
character-conversion layer used by encoded-word and parameter decoding and
intended for future text-part support, not itself an RFC 2047 or RFC 2231 parser.
Convenience constants are limited to `Charset::ASCII`, `Charset::UTF8`, and
`Charset::LATIN1`; other charsets, including UTF-16LE/BE, use `Charset::from_label()`.

```rust
use mimecraft::charsets::{Charset, decode_best};

let latin1 = Charset::from_label("ISO_8859_1").unwrap();
assert_eq!(latin1.name(), "iso-8859-1");
assert_eq!(latin1.decode(b"Ren\xe9e").unwrap(), "Ren\u{e9}e");
assert_eq!(latin1.encode("Ren\u{e9}e").unwrap().as_ref(), b"Ren\xe9e");

let (text, selected) = decode_best(b"\x80", None);
assert_eq!(text, "\u{80}");
assert_eq!(selected, Charset::LATIN1);

let windows = Charset::from_label("windows-cp1252").unwrap();
let (text, selected) = decode_best(b"\x80", Some(windows));
assert_eq!(text, "\u{20ac}");
assert_eq!(selected, windows);
```

Labels are ASCII and case-insensitive; surrounding whitespace is not removed.
Supported MimeKit aliases include `unicode` for UTF-8, `iso-10646` for UTF-16BE,
numeric code-page spellings, and legacy Japanese, Korean, and Chinese labels.
`name()` reports the canonical lowercase label of the actual codec: for example,
GB2312 aliases resolve to the backend's GBK superset.

ASCII and ISO-8859-1 are deliberately separate from Windows-1252: high bytes
are invalid ASCII, while Latin-1 maps every byte directly to the corresponding
Unicode scalar, including C1 controls. UTF-16 encoding emits the requested byte
order rather than substituting UTF-8. Explicit conversions neither sniff nor
strip BOMs and never insert a BOM automatically.

`decode` and `encode` return `Result<Cow<_>, CharsetError>`, borrowing unchanged
input where possible. Invalid bytes and unencodable text are errors, not empty
strings, best-fit substitutions, or HTML numeric references. `decode_lossy`
explicitly replaces malformed sequences with U+FFFD and returns a
`had_errors` flag. This uniform replacement policy differs from .NET's
charset-dependent defaults.

`decode_best` ports MimeKit's unlabelled-text preference: valid UTF-8 first,
then a supplied charset if it decodes without errors, then Latin-1. It returns
the selected charset alongside the text, so fallback is observable. This is
not general charset detection. Every call processes a complete buffer;
stateful encodings do not carry state across calls.

Legacy codec mappings come from `encoding_rs`, not .NET, and are not claimed
to match all .NET code pages. UTF-7, UTF-32, ISO-2022-KR, HZ, and unsupported
DOS/Mac encodings return `UnsupportedLabel`. ISO-8859-9/11, TIS-620, and related
aliases are rejected rather than silently upgraded to Windows encodings.
Browser-only replacement and user-defined encodings are also rejected.
Unsupported labels never silently fall back; guessing is a separate operation.

### Date headers

`DateTimeOffset::parse(&[u8])` and `"value".parse::<DateTimeOffset>()` adapt MimeKit's
`DateUtils` reader. They try standard mail-date ordering first, then recover
reordered fields, numeric dates, missing times, and other legacy formats.
`Display` produces an English date with a recalculated weekday, seconds, and
a numeric zone. The original spelling and comments remain available only in
the raw `Header`.

```rust
use mimecraft::DateTimeOffset;

let date = DateTimeOffset::parse(b"Tue, 9 Jun 92 03:45:24 JST").unwrap();
assert_eq!(date.to_string(), "Tue, 09 Jun 1992 03:45:24 +0900");
assert_eq!(date.offset_minutes(), 540);

let epoch = DateTimeOffset::new(1970, 1, 1, 1, 0, 0, 60).unwrap();
assert_eq!(epoch.unix_timestamp(), 0);
```

The reader intentionally retains MimeKit's permissive behavior. Incorrect
weekdays and extra tokens are ignored; absent, unknown, and invalid zones
normally become UTC. Missing or invalid times can become midnight through
recovery. `23:59:60` is normalized to `23:59:59`, and `-0000` loses its distinct
unknown-zone meaning. Names for months and weekdays are case-insensitive;
named zones and AM/PM markers must be uppercase. The reader recognizes the
source's US, military, UT/UTC/GMT, JST, and KST mappings, not a timezone database.
Unterminated comments stop tokenization, but an already complete date can
still succeed. Ignored bytes need not be valid UTF-8.

Standard-order years below 100 map to 2000-2069 or 1970-1999. Recovery's short
year handling differs: for example, `Jan 1 69` is 1969, while
`1 Jan 69 00:00:00 +0000` is 2069. Parsing recognizes years from 1969 through
9999 and zones from -12 through +14 hours, rejecting zone minutes above 59.
When no interpretation succeeds, it returns `DateTimeOffsetError::InvalidSyntax`;
the heuristic reader does not provide byte-offset diagnostics.

For authoring, `DateTimeOffset::new(year, month, day, hour, minute, second,
offset_minutes)` checks the supplied values without recovery. It supports
Gregorian years 1-9999, seconds 0-59, and offsets within 14 hours of UTC, and
requires the UTC date to fit the same calendar range. This deliberately matches
the wider range accepted by MimeKit's date formatter rather than its reader;
dates outside the reader's range need not round-trip. Component getters and
`unix_timestamp()` support interoperability without an external dependency.
Equality includes the local components and offset; compare timestamps when
comparing instants. This is not date arithmetic or strict RFC syntax validation.

### Received trace headers

`Received::parse(&[u8])` and `"value".parse::<Received>()` interpret a raw
Received field value, excluding its field name. Use `Header::raw_value()` when
starting from a parsed header; no preliminary unfolding or encoded-word decoding
is required. The result is detached; edits do not change the original header.

```rust
use mimecraft::Received;

let received = Received::parse(
    b"from source.example ([192.0.2.1]) by target.example with ESMTP; Tue, 30 Apr 2019 19:10:19 +0000"
).unwrap();
assert_eq!(received.from(), Some("source.example"));
assert_eq!(received.from_tcp_info(), Some("[192.0.2.1]"));
assert_eq!(received.by(), Some("target.example"));
assert_eq!(received.with(), Some("ESMTP"));
assert_eq!(received.date_time().unwrap().year(), 2019);
```

`clauses()` retains source order and returns `ReceivedClause` values with `id()`,
`keyword()`, `value()`, and `comments()`. The recognized `ReceivedClauseId`
variants are `From`, `By`, `Via`, `With`, `Id`, and `For`; leading comments form
a `None` clause with empty keyword/value strings. Unknown keywords are retained
as `Unknown`. `clause(id)` returns the first match. Convenience accessors include
`from`, `by`, `via`, `with`, `id`, and `for_recipient`; `from_tcp_info` and
`by_tcp_info` return the corresponding clause's first comment.

During parsing, known clauses are ASCII case-insensitive, may appear in any order,
and cannot repeat. Keyword spelling is preserved. Unknown clauses may repeat. Values may
be empty or contain multiple tokens joined with single spaces, retaining
MimeKit's compatibility with `Microsoft SMTP Server` and `Frontend Transport`.
A recognized keyword starts a new clause even where a value might be expected.
Comments can precede or follow values. Nested parentheses survive, outer
parentheses and escape backslashes are removed, CR/LF are dropped, and tabs
become spaces, including escaped whitespace.

`date_time()` reuses the tolerant `DateTimeOffset` parser. `date_time_text()`
retains decoded date spelling, including named zones and comments, with outer
ASCII whitespace trimmed, CR/LF removed, and tabs replaced by spaces. It does
not canonicalize dates or collapse remaining spaces. Missing dates and an empty
suffix after `;` are accepted. Empty/whitespace-only input also succeeds.

`ReceivedParseOptions::fallback_charset` selects an optional preferred charset.
Each keyword, value token, complete comment, and date independently uses the
existing UTF-8 / preferred-charset / Latin-1 decoding order.
`had_charset_fallback()` reports whether any segment used non-UTF-8 decoding.
RFC 2047 encoded words and IDNA domains are not decoded.

`ReceivedParseErrorKind` distinguishes `IncompleteComment`,
`DuplicateClause(ReceivedClauseId)`, and `InvalidDateTime`. `token_offset()` marks
the offending token's start; `offset()` marks EOF, the duplicate keyword's end,
or the invalid date's trimmed end. Offsets are zero-based bytes relative to the
supplied slice, including when parsing a subslice of a larger buffer.

Parsing is a compatibility inspection API, not strict RFC validation or a trust
decision. Parsed host, protocol, message-ID, and recipient values are not validated;
quotes and address literals do not shield comment/date delimiters.
Other control bytes can survive parsing, so these strings must not be inserted
directly into authored headers. The underlying date parser may recover dates
from nonstandard suffixes, including an unterminated trailing date comment.
Comment nesting is iterative and allocation is proportional to input size.
Complete-header formatting is explicit; automatic message integration remains
deferred. Existing raw header storage and serialization are unchanged.

#### Constructing and editing Received values

`Received::new()` / `Default` creates an empty value. `with_hosts(from, from_ip,
by, by_ip, date_time)` creates checked `from`/`by` clauses with bracketed
`std::net::IpAddr` comments and a checked `DateTimeOffset`.

```rust
use mimecraft::{DateTimeOffset, Received};

let mut received = Received::new();
received.set_from(Some("smtp.source.com")).unwrap();
received.set_from_tcp_info(Some("[192.168.1.1]"));
received.set_by(Some("smtp.target.com")).unwrap();
received.set_with(Some("ESMTPS")).unwrap();
received.set_for_recipient(Some("unit-tests@mimekit.net")).unwrap();
received.set_date_time(Some(
    DateTimeOffset::new(2026, 4, 18, 20, 5, 38, -240).unwrap()
));
assert_eq!(received.from_tcp_info(), Some("[192.168.1.1]"));
```

`set_from`, `set_by`, `set_via`, `set_with`, `set_id`, and `set_for_recipient`
accept `Option<&str>`. `Some` validates and updates the first matching clause,
retaining keyword spelling and comments; `None` removes that clause and its
comments. A new clause uses a lowercase keyword and sorts the collection in
`None` / `From` / `By` / `Via` / `With` / `Id` / `For` / `Unknown` order.
Equal classifications keep their relative order. Updating/removing an existing
clause does not sort. Failed assignments leave the whole object unchanged.

`ReceivedClause::new(keyword, value)` accepts ASCII-letter/hyphen keywords
beginning with a letter. `with_comment` adds one unmodified comment.
`set_value` applies the same clause-specific checks as the convenience setters.
Values retain their exact spelling and must be nonempty with no ASCII controls.
Host checks follow MimeKit's permissive hostname rules, not DNS/IDNA validation;
bracketed literals use Rust's `IpAddr` grammar, excluding legacy abbreviated or
octal/hex IPv4 forms and scoped IPv6 addresses accepted by .NET.
Identifiers use the existing complete, tolerant `MessageId` parser; recipients
use `MailboxAddress`. Unlike MimeKit's recipient setter, controls and folds are
rejected before parsing, because the original mailbox spelling is retained.
Other clause values reject parentheses and semicolons but allow spaces and
Unicode, including multi-word transport/protocol descriptions.
`ReceivedEditError` distinguishes invalid keywords, empty values, invalid
hosts, illegal value bytes, and wrapped identifier/mailbox parser failures.

`set_from_tcp_info` and `set_by_tcp_info` replace all comments on the first
matching host clause. They remove CR/LF and replace tabs with spaces without
trimming or validating comment syntax. `Some` creates a missing clause with an
empty host value; `None` clears comments without creating or removing a clause.
`clauses_mut()` and `comments_mut()` expose ordinary vector edits, including
duplicates and unvalidated comments. Convenience getters/setters/removals act
on the first matching clause if direct edits introduce duplicates.

`set_date_time` always clears `date_time_text`, even when assigning the same
date. Other edits retain the cached date spelling. `had_charset_fallback`
continues to describe the original parse, not the current edited strings;
newly constructed objects start with no recovery flag.
These assignment checks do not make arbitrary parsed or edited values safe
to insert into headers without the formatter's additional output checks.

#### Formatting Received headers

`to_header_string()` emits a complete `Received:` header with CRLF, an enforced
78-byte physical-line limit, and ASCII output. `to_header_string_with_options`
accepts `ReceivedFormatOptions` with `max_line_length` in `60..=998`,
`new_line_format` (`Dos` or `Unix`, not `Mixed`), and `international`.
Unlike MimeKit's `ToString`, output includes the field name and colon.
It always ends with one newline; an empty object becomes `Received:\r\n`.

```rust
use mimecraft::{Header, Received, ReceivedFormatOptions};

let mut received = Received::new();
received.set_from(Some("caf\u{e9}.example")).unwrap();
received.set_by(Some("target.example")).unwrap();
let options = ReceivedFormatOptions {
    international: true,
    ..Default::default()
};
let output = received.to_header_string_with_options(&options).unwrap();
let header = Header::parse(output.as_bytes()).unwrap();
assert_eq!(Received::parse(header.raw_value()).unwrap().from(),
    Some("caf\u{e9}.example"));
```

The formatter ports MimeKit's clause grouping: keep a keyword, value, and first
comment together when they can fit on a continuation line; otherwise keep the
keyword/value pair together, or split between them. Other comments and the date
are appended as separate pieces. Folds use a continuation tab. Complete values,
comments, and date suffixes are never split internally, even at spaces, so a
piece that cannot fit fails instead of exceeding the limit. Measurements count
UTF-8 bytes, indentation, comment parentheses/escapes, and the field prefix.
A semicolon may move to a continuation line when the preceding line is full.

Current clause order, keyword/value spelling, and cached date spelling survive
formatting. Authored or reassigned dates use `DateTimeOffset`'s canonical output.
Leading-comment clauses emit only comments, even if their stored value was
edited. Missing values and duplicate clauses are not removed or repaired.
This is not strict SMTP trace validation, and freely edited values are not
guaranteed to round-trip through the tolerant clause parser.

All emitted components reject ASCII controls. Comments alone normalize CR/LF
away and tabs to spaces, matching MimeKit's protection against injected lines.
Backslashes are escaped. Balanced nested parentheses are preserved; unlike
MimeKit, an unbalanced comment escapes all parentheses so edited text cannot
close the enclosing comment. Normalization and escaping do not mutate the model.

Non-ASCII text fails unless `international` is enabled; no RFC 2047 encoding,
IDNA conversion, or legacy output charset is selected. For SMTP, generally
mirror the server's advertised `SMTPUTF8` capability. The option does not
negotiate transport support, authenticate trace data, or validate hostnames.
Decoded legacy input is emitted as UTF-8 when international output is enabled.

`ReceivedFormatError` reports invalid options, oversized pieces, controls, or
non-ASCII text without opt-in. Text errors include a `ReceivedFormatPart`
(keyword/value clause index, clause/comment indexes, or date) and a UTF-8 byte
offset in the stored component, before comment normalization. Failure returns
no partial string and leaves the original object unchanged.

### Quoted-string utilities

`mimecraft::utils` ports MimeKit's shared quoted-string transformations:
`quote` / `append_quoted`, `unquote` / `append_unquoted`, and
`unquote_bytes` / `append_unquoted_bytes`. These are building blocks for MIME
parameters and addresses, not complete Content-Type, Content-Disposition, or
address parsers.

```rust
use mimecraft::utils::{append_unquoted_bytes, quote, unquote};

assert_eq!(quote("a \"quoted\" \\ path"), "\"a \\\"quoted\\\" \\\\ path\"");
assert_eq!(unquote("\"a\\\"b\r\n\tc\"", false), "a\"b\tc");
assert_eq!(unquote("\"a\\\"b\r\n\tc\"", true), "a\"b c");

let mut output = b"prefix: ".to_vec();
append_unquoted_bytes(b"\"\xff\\\"\"", false, &mut output);
assert_eq!(output, b"prefix: \xff\"");
```

Quoting adds outer double quotes and escapes backslashes and double quotes.
Everything else, including controls and CR/LF, is preserved. It is **not
header-authoring validation** and must not be used to sanitize untrusted input.

Unquoting removes unescaped double quotes anywhere, removes backslash quoting,
drops all CR/LF, and discards a final lone backslash. It does not require balanced
quotes or reject unknown escapes. Tabs and line breaks reset escape state;
the explicit `convert_tabs_to_spaces` argument controls whether each tab becomes
one space. Other characters and whitespace are preserved.

`unquote` returns `Cow<str>` and `unquote_bytes` returns `Cow<[u8]>`: unchanged
input is borrowed, while transformed input is owned. Byte variants preserve
non-ASCII bytes exactly, including invalid UTF-8; there is no charset or
encoded-word decoding. Append variants leave existing output untouched.
Each call processes one complete value, with no escape state carried between
calls. Consequently, these are not streaming decoders, and quoting/unquoting is
not a lossless round trip for input containing CR/LF.

### Quoted-printable

`QuotedPrintableEncoder` and `QuotedPrintableDecoder` follow the same
`new`/`encode`/`finish` and `new`/`decode` patterns, with `Clone` and `reset`.
Both encoders support `with_line_length` and return `InvalidLineLength` for
requests outside 60..=998. Quoted-printable caps the requested width at 76
without Base64's rounding to a multiple of four.

The quoted-printable encoder normalizes both CRLF and bare LF input to the
selected ending and escapes lone CR bytes. It encodes trailing spaces and tabs;
when content does not end in a newline, `finish` adds a soft break (`=` plus the
selected ending) rather than an extra decoded newline. Hard line breaks remain
in the decoded content in the selected format; lone CR bytes round-trip unchanged.

The decoder accepts mixed-case hex, removes `=\n` and `=\r\n`, and passes
malformed escapes through literally. An incomplete final escape (`=`, `=A`,
or `=\r`) stays pending until more input arrives or `reset` discards it.
`QuotedPrintableDecoder::for_encoded_words()` additionally maps literal
underscores to spaces for RFC 2047 Q payloads; it does not parse encoded-word
wrappers or convert character sets. Quoted-printable uses safe scalar code.

### UU encoding

`UuEncoder` writes payload lines of up to 45 input bytes, using backticks for
zero sextets. It does not write `begin`/`end` wrappers. `new()` defaults to
`NewLineFormat::Dos`; `with_new_line_format` also accepts `Unix` and rejects
`Mixed`. There is no configurable line width.

Unlike Base64 and quoted-printable, **every `finish` call writes a zero-length
terminator line**, including for empty input or repeated calls. This matches
MimeKit's UU encoder. `reset` discards pending data without emitting anything,
and both `reset` and `finish` retain the selected new-line format.

`UuDecoder::new()` scans for a case-sensitive `begin ` marker at a line start,
then skips the rest of that header without validating its mode or filename.
Use `UuDecoder::for_payload()` for unwrapped encoder output:

```rust
use mimecraft::encodings::{UuDecoder, UuEncoder};

let mut encoded = Vec::new();
let mut encoder = UuEncoder::new();
encoder.encode(b"Cat", &mut encoded);
encoder.finish(&mut encoded);
assert_eq!(encoded, b"#0V%T\r\n`\r\n");

let mut decoded = Vec::new();
UuDecoder::for_payload().decode(&encoded, &mut decoded);
assert_eq!(decoded, b"Cat");
```

The decoder follows MimeKit's tolerant scalar behavior: CR is ignored, LF marks
a new line, all other bytes map to sextets, and space and backtick both represent
zero. It accepts length prefixes up to 63 bytes and retains incomplete quartets,
even across malformed line breaks. A zero-length prefix ends decoding; `end`
trailers and further bodies are ignored until `reset`, which retains the
original scanning mode. Both UU codecs support cloning and caller-owned output
buffers. The default `simd` feature accelerates complete encoding lines,
well-formed decoding lines, and complete quartets within partially received
lines. Other input retains the tolerant scalar behavior.

### yEnc

`YEncoder` writes yEnc payload without `=ybegin`, `=ypart`, or `=yend` wrappers.
It defaults to 128 encoded octets per line and CRLF. Its `with_line_length`,
`with_new_line_format`, and `with_options` constructors follow the other
configurable codecs, but widths of 60..=998 are retained without capping or
rounding. `Mixed` is rejected. Escapes remain together, so a line can exceed the
requested width by one octet, matching MimeKit.

`finish` terminates a partial line without adding another newline to an empty
or already terminated line. Unlike the other encoders, it **retains the running
checksum**; call `reset` before encoding an independent body.

`YDecoder::new()` scans for a case-sensitive `=ybegin ` at a line start and
skips an optional `=ypart ` header. `YDecoder::for_payload()` decodes unwrapped
encoder output. Both discard CR/LF, cancel a pending escape at a line break,
and stop at `=y` at the start of a payload line without requiring a complete
`=yend` marker. Subsequent input is ignored until `reset`. Headers, sizes,
filenames, and trailer checksums are not validated or returned; multipart
assembly is not implemented.

The optional-part scan preserves a leading payload escape when the first line
after `=ybegin` is not a `=ypart` header, matching MimeKit's corrected decoder.
All possible initial bytes are covered across input splits and cloned states.

Both codecs expose `checksum() -> u32`: MimeKit's **uncomplemented** running
CRC-32 state, initially `u32::MAX`. Use `!codec.checksum()` for the finalized
value used in a yEnc trailer. Only original/decoded bytes affect it, not line
endings, escapes, or headers. Cloning preserves the checksum and all streaming
state; resetting clears them while retaining configuration or scanning mode.
The implementation is safe scalar Rust with a static CRC table.

#### Complete yEnc block metadata and writing

`encodings::YEncHeader` supplies checked immutable metadata. `new(file_name, size)`
creates a single-part header with a nominal payload line length of 128.
`with_line_length` and `with_part(part_number, total_parts, range)` return updated
metadata; multipart ranges are one-based and inclusive, and `None` leaves the
total part count unspecified. `size()` always describes the entire file, while
`decoded_size()` describes just this block. Empty single-part files are valid.
Empty filenames, surrounding ASCII spaces, NUL/CR/LF, zero multipart numbers,
inconsistent totals, and empty/out-of-file ranges are rejected. Filenames remain
untrusted metadata, not filesystem paths.

`encodings::YEncWriter<W: Write>` wraps an owned or borrowed destination.
`write_block(&header, &mut source, file_checksum)` writes `=ybegin`, optional
`=ypart`, encoded payload, and `=yend`, returning the **finalized** CRC-32 of this
block. It reuses `YEncoder` rather than duplicating its payload algorithm. Multiple
successful blocks can be written consecutively. Sources need only `Read`, begin
at their current positions, and must end at exactly `decoded_size()`; an excess
probe consumes at most one byte beyond that size. Multipart sources contain only
their declared range: the writer does not seek into whole files or assemble parts.

`YEncWriterOptions` defaults to CRLF, exact Latin-1 filenames, a 16384-byte
physical-line limit, and a 1 GiB per-block decoded-size limit. Options are copied
on construction. LF and explicit filename `Charset` values are supported without
replacement fallback; yEnc does not identify the charset on the wire. Encoded
filenames are checked again for NUL/CR/LF. Control-line limits count encoded
bytes, not characters. Metadata accepts line lengths from 1 through
`i32::MAX - 2`, but the writer requires 60 through 998 and reserves one extra
physical-line byte for an escape pair, even for an empty block.

Single-part trailers contain `crc32`; multipart trailers contain `part`,
optional `total`, `pcrc32`, and an optional supplied whole-file `crc32`.
A supplied whole-file checksum is verified for single-part blocks and multipart
ranges covering the entire file. For partial ranges it is emitted without
verification. Checksum mismatches suppress the trailer.

Preflight validates metadata-dependent limits, filename encoding, and all
control lines (including the future trailer) before any source/destination I/O.
These failures leave the writer reusable. Once I/O begins, any source, destination,
size, or checksum failure permanently faults the writer, and output may be partial.
`YEncWriteError` distinguishes these failures and exposes underlying I/O/charset/
encoder errors through `Error::source`. Interrupted reads and writes are retried,
short writes are completed, and zero-length writes fail explicitly.
Neither writing nor dropping flushes the sink; `get_ref`, `get_mut`, and
`into_inner` provide destination access. Dropping owned streams follows normal Rust
ownership; borrowed streams remain available.

```rust
use mimecraft::encodings::{YEncHeader, YEncWriter};

let header = YEncHeader::new("test.bin", 20)?
    .with_part(2, Some(3), 10..=18)?;
let mut writer = YEncWriter::new(Vec::new());
let crc = writer.write_block(&header, &mut &b"123456789"[..], Some(0x12345678))?;
assert_eq!(crc, 0xcbf43926);
let encoded_block = writer.into_inner();
# Ok::<(), Box<dyn std::error::Error>>(())
```

Unsigned `u64` byte counts/ranges and `u32` part counts replace MimeKit's signed
integer APIs; `Option` replaces zero sentinels for multipart metadata. This also
allows positive values beyond MimeKit's signed integer ranges.

#### Complete yEnc block reading

`encodings::YEncReader<R: Read>` supplies strict streaming block validation.
Call `read_next()` to find and validate the next header, then
`decode_to(&mut destination)` to decode and validate its payload and trailer.
Decode to `std::io::sink()` to discard a block while still validating it.
Surrounding text is skipped, and consecutive blocks are supported.
Sources need not seek; neither decoding nor dropping flushes the destination.

`YEncReaderOptions` copies the same physical-line limit, per-block decoded-size
limit, and strict filename charset defaults as the writer. Readers accept both
CRLF and LF, including a final unterminated line. Line scanning uses bounded
buffers, not an unbounded delimiter read or allocations based on advertised
file sizes. The physical limit applies to surrounding text and control/payload
lines. The declared width cannot exceed that limit; payload lines can exceed
their declared width by one byte only to complete a final escape pair.
Unlike writing, reading permits nominal widths below 60 or above 998.

Control fields are case-sensitive. Unknown fields are allowed, but malformed,
empty, or duplicate fields are rejected. Header `name=` consumes the remainder
of its line and trims only ASCII spaces. Multipart blocks require an immediate
`=ypart` header with a valid range. Payload validation rejects unexpected control
lines, unescaped NUL/CR, incomplete escapes, and excess decoded bytes before
writing the offending line. Header, decoded, and trailer sizes must agree;
multipart trailer numbers and any advertised total must match the header.

Checksums are optional, but advertised values must contain exactly eight hex
digits. Part checksums are always verified; whole-file checksums are verified
only for single-part blocks or multipart ranges covering the entire file.
`checksum()`, `expected_checksum()`, and `expected_part_checksum()` return
finalized CRC values only after successful decoding. A partial range's
`expected_checksum()` is explicitly unverified metadata.

Output remains provisional until `decode_to` succeeds and must be discarded on
failure. `decoded_size()` counts decoded bytes, not necessarily bytes delivered
successfully to a failing destination. Syntax, limit, checksum, or I/O failures
permanently fault the reader; `YEncReadError` exposes structured failures and
underlying error sources. Calling in the wrong phase leaves it reusable.
Metadata/checksums reset when advancing to another block; repeated EOF calls do
not read again. Invalid headers never publish partially validated metadata.
Interrupted I/O and short writes are handled.

The reader can read ahead, so callers must not bypass it to read the source.
`into_parts()` returns the source and unread buffered bytes; consume those bytes
before reading the source again. Dropping never validates a pending block.

```rust
use mimecraft::encodings::YEncReader;

let input = b"=ybegin line=128 size=9 name=test.bin\n\
              [\\]^_`abc\n=yend size=9 crc32=cbf43926\n";
let mut reader = YEncReader::new(input.as_slice());
assert!(reader.read_next()?);
assert_eq!(reader.header().unwrap().file_name(), "test.bin");
let mut decoded = Vec::new();
reader.decode_to(&mut decoded)?;
assert_eq!(decoded, b"123456789");
assert_eq!(reader.checksum(), Some(0xcbf43926));
# Ok::<(), Box<dyn std::error::Error>>(())
```

Synchronous yEnc support is complete. Async/cancellation entry points remain
deferred consistently with the rest of the synchronous I/O API. As in MimeKit,
these block APIs do not add MIME headers, NNTP framing/dot-unstuffing, or multipart
assembly. The tolerant `YDecoder` remains available independently; it is not a
validated reader.

### Header payload encodings

`Rfc2231Encoder` and `Rfc2231Decoder` match MimeKit's names and use `%HH`
escapes rather than plain hexadecimal.
They encode/decode RFC 2231 MIME parameter value payloads, not complete
parameters or general-purpose URLs. The encoder leaves MIME attribute
characters literal and writes uppercase escapes; space becomes `%20`, while
`+` stays literal. No charset/language prefix, continuation, or charset
conversion is added.

Percent decoding is incremental and accepts either case of hex. Invalid
three-byte escapes pass through literally, without reinterpreting embedded
percent signs. Incomplete `%` or `%X` sequences stay pending, even if X is
not hex, until more input arrives or `reset` discards them.

`Rfc2047Base64Encoder` writes a complete, independently padded **B payload per
call**, without MIME line wrapping or a final newline. It reuses the existing
runtime-dispatched Base64 SIMD backends. It is not an incremental encoder:
provide all bytes for one payload together rather than streaming arbitrary
chunks into successive calls.

`Rfc2047QuotedPrintableEncoder::new(QEncodeMode::Phrase)` writes a **Q payload**
for contexts such as address display names; `QEncodeMode::Text` selects MimeKit's
text alphabet. Both map spaces to underscores and escape literal underscores,
unsafe characters, and non-ASCII bytes as `=HH`. Unlike body quoted-printable,
they do not normalize newlines, apply trailing-whitespace rules, or wrap lines.
Use `QuotedPrintableDecoder::for_encoded_words()` to decode Q payloads and
`Base64Decoder` to decode B payloads.

All three header payload encoders are stateless and append to caller-owned
buffers, with no `finish` or `reset`. They do not write `=?charset?B?...?=` or
`=?charset?Q?...?=` wrappers, convert character sets, split characters or words,
enforce encoded-word length limits, or fold headers. Complete-value encoding
and decoding are available separately in `mimecraft::rfc2047`. Complete folded
headers are available through `Header::from_unstructured` and
`AddressList::to_header`.

### RFC 2047 header decoding

`mimecraft::rfc2047::decode_text` decodes encoded words in unstructured values
such as Subject. `decode_phrase` uses phrase atom boundaries for display names,
but does not parse addresses, remove comments, or unquote strings. Both return
a `DecodedHeader` containing `text` and charset recovery `issues`.

```rust
use mimecraft::rfc2047::{decode_text, DecodeIssue};

let decoded = decode_text(b"=?utf-8?q?Ren=C3?=\r\n =?UTF8?q?=A9e?=");
assert_eq!(decoded.text, "Ren\u{e9}e");
assert!(decoded.issues.is_empty());

let recovered = decode_text(b"=?unknown?q?=E9?=");
assert_eq!(recovered.text, "\u{e9}");
assert!(matches!(recovered.issues[0], DecodeIssue::UnsupportedCharset { .. }));
```

The `_with_options` variants accept `DecodeOptions` with a `compliance_mode`
and optional `fallback_charset`. Loose mode is the default, recovering words
embedded in ordinary text, literal spaces in payloads, and valid words after
unterminated candidates. Strict mode recognizes complete whitespace-delimited
text tokens or ASCII phrase atoms. It is not a strict RFC validator: both modes
use tolerant Base64/Q codecs and do not enforce word-length limits.

Whitespace between recognized encoded words is discarded; other whitespace,
including leading/trailing CR/LF, is preserved. No automatic unfolding or
trimming is performed. Malformed wrappers remain ordinary text. Decoded control
characters and NUL are retained; this is not a header-injection sanitizer.
Pass `Header::raw_value()` to decode a parsed header without modifying its bytes.

Adjacent words with the same resolved charset and B/Q encoding share payload
decoder state and are converted to Unicode together. This recovers split Q
escapes, Base64 quartets, multibyte characters, and stateful charset sequences.
Aliases and encoding-letter case do not split a run; different unsupported
charset labels share one unknown-charset identity, as in MimeKit. Language
suffixes are accepted but do not affect decoding or appear in the result.

Recognized charsets use explicit U+FFFD replacement with `InvalidBytes` issues.
Unsupported labels use UTF-8, the preferred charset, then Latin-1 fallback and
produce `UnsupportedCharset` issues, including empty runs. Raw non-UTF-8 tokens
produce `UnlabelledText` issues. Diagnostics report the charset and half-open
source byte range of the token or whole encoded-word run, including wrappers
and suppressed inter-word whitespace. They are not precise bad-byte offsets
or a complete malformed-payload report. Charset mapping/replacement differences
from .NET are those documented by the charset layer.

### RFC 2047 text and phrase encoding

`rfc2047::encode_text` encodes an unstructured value such as a Subject, while
`encode_phrase` encodes a literal display name. Both return `Result<String,
EncodeError>` containing **ASCII value syntax only**: no field name, folding,
or terminating newline. As with MimeKit's `EncodeText`/`EncodePhrase`, callers
must separately format and fold complete fields before writing headers.

```rust
use mimecraft::charsets::Charset;
use mimecraft::rfc2047::{encode_text, encode_phrase, encode_text_with_options, EncodeOptions};

assert_eq!(encode_text("Hello world")?, "Hello world");
assert_eq!(
    encode_text("Hello Ren\u{e9}e")?,
    "=?utf-8?q?Hello_Ren=C3=A9e?="
);
assert_eq!(encode_phrase("Doe, Jane")?, "\"Doe, Jane\"");

let options = EncodeOptions {
    charset: Charset::LATIN1,
    max_word_length: 60,
};
let encoded = encode_text_with_options("caf\u{e9}", &options)?;
assert!(encoded.starts_with("=?iso-8859-1?"));
# Ok::<(), mimecraft::rfc2047::EncodeError>(())
```

The defaults are UTF-8 and a 75-byte encoded-word limit, **including wrappers**.
`max_word_length` accepts `1..=75`; limits too small for a complete character and
wrapper return `WordTooLong`, including the original UTF-8 byte offset.
Each word contains complete scalar values and self-contained B/Q payloads;
stateful charsets are finalized independently for every word. Automatic B/Q
selection uses actual payload sizes, preferring Q on ties; ISO-2022-JP always
uses B. Phrase Q output uses the more restrictive phrase alphabet.

Ordinary printable ASCII text remains literal. Non-ASCII characters, controls,
literal `=?` sequences, or tokens longer than the word limit cause the entire
value to be encoded. Short ASCII phrases use atoms separated by single spaces
or an escaped quoted string; an empty phrase is `""`. Long quoted phrases use
encoded words instead of inserting spaces into the display name.
Original whitespace, including leading/trailing spaces, tabs, and CR/LF, stays
in the payloads. The single spaces inserted between encoded words are syntax
and disappear when decoded. Controls and NUL remain part of the decoded value;
encoding does not sanitize that value.

Input to `encode_phrase` is a decoded, literal display name, not pre-quoted
address syntax. When reading its result, parse/unquote the ASCII syntax
**before** decoding encoded words. The existing `decode_phrase` helper does not
remove syntactic quotes; unquoting after decoding can wrongly strip literal
quotes or backslashes from the name.

This deliberately simpler encoder does not reproduce MimeKit's mixed
plain/quoted/encoded-word merging, its 17-percent B/Q heuristic, or implicit
UTF-8 fallback. It preserves complete input whitespace and consistently uses
the requested charset for encoded words. Unmappable characters return a charset
error, not replacement text or a silent change of charset; legacy mapping
limitations remain those documented by `Charset::encode`. Errors return no
partial output.

The word limit is **not** a physical-line limit; the returned value can be
longer than one header line. Use `Header::from_unstructured` or
`AddressList::to_header` for complete folded fields; comment encoding remains
unported. Do not use the text/phrase helpers for addr-specs or MIME parameters;
Content-Type and Content-Disposition
serialization uses the separate RFC 2231 path.

### MIME parameters

`ParameterList::parse` decodes the parameter portion of a Content-Type or
Content-Disposition value, optionally including its leading semicolon. Do not
include the media type or disposition token.

```rust
use mimecraft::ParameterList;

let parameters = ParameterList::parse(
    b"; filename*1*=%A9.txt; filename*0*=utf-8''caf%C3; size=123"
)?;
assert_eq!(parameters.get("FILENAME"), Some("caf\u{e9}.txt"));
assert_eq!(parameters.get("size"), Some("123"));
assert!(parameters.issues().is_empty());
# Ok::<(), mimecraft::ParameterParseError>(())
```

Names retain their first spelling and position, with ASCII case-insensitive
lookup. Values are owned Unicode strings. Ordinary quoted values are unescaped;
ordinary values also support RFC 2047 encoded words. Unencoded continuation
sections do not interpret encoded words.

`parse_with_options` accepts `ParameterParseOptions`. Default loose syntax
accepts unquoted spaces/folding and missing semicolons after quoted values.
Strict syntax requires token or quoted values and separators, but still uses
tolerant payload decoding and continuation recovery. Unterminated comments or
quotes fail in either mode, following MimeKit's throwing parse path. Errors
carry byte offsets; no partial list is returned. Empty entries are ignored;
`name=;` is accepted, while `name=` at end of input is incomplete.

RFC 2231 sections are sorted numerically and joined, including gaps and
duplicate section numbers. Duplicate sections retain stable encounter order,
unlike MimeKit's unspecified sort order for ties. Extended/continued definitions
override ordinary duplicates; otherwise the first ordinary or last extended
definition wins, with continuation groups positioned at their first encounter.
Adjacent encoded sections share percent-decoder state and charset conversion,
allowing escapes and multibyte characters to span sections. Incorrectly quoted
extended values have only their outer quotes removed, not quoted-pair escapes.

Missing or unsupported RFC 2231 charsets use Latin-1, **not** UTF-8-first
guessing. An incomplete declaration remains part of the payload; a complete
unsupported declaration is stripped. The preferred fallback charset applies
only to raw text and RFC 2047 decoding. Invalid declared-charset bytes use U+FFFD
replacement, rather than MimeKit's RFC 2231 `"?"` replacement. Other charset
mapping differences are those documented above.

`issues()` reports recovered separators, duplicates, section gaps, charset
fallback/replacement, and discarded incomplete percent escapes. Spans are
half-open original-input byte ranges for whole values or continuation regions,
not exact invalid-byte locations. Interleaved sections can make a span include
unrelated parameters. Diagnostics are not exhaustive payload or RFC validation.

`Parameter::new` and `set_value` support decoded values without interpreting
quotes, percent escapes, or encoded words. Names must be nonempty ASCII
attribute characters, without RFC 2231 suffixes; name-validation errors report
the first invalid byte in the name. Values may be empty or contain Unicode,
controls, or line breaks and are not ready-to-send header syntax.

`ParameterList::new` creates an empty list. `set` updates a value while retaining
its name's first spelling and position, or appends a new name; it returns the
previous value if present. `add` and indexed `insert` reject duplicate names
case-insensitively. Indexed `replace` can change a name, but cannot duplicate
another entry. `remove`, `remove_at`, `clear`, `contains`, `index_of`, and borrowed
iteration provide ordered collection operations. Failed validation, duplicate,
or index checks leave the list unchanged. Mutable entries are not exposed in a
way that could bypass name uniqueness.

Parse diagnostics are **historical**, not validation of current contents: edits,
including `clear`, retain the original input ranges and recovery events.
Language, original charset, and formatting/encoding preferences are not retained.
Typed content headers can serialize these values with quoting and RFC 2231
encoding; preserve the raw header separately for lossless output. Decoded values
are not sanitized for filenames, paths, controls, or NUL.

### Content-Type and Content-Disposition

`ContentType::parse` and `ContentDisposition::parse` accept complete header
**values**, excluding field names. They also implement `FromStr`. Use
`Header::raw_value()` to interpret a parsed header without altering its bytes.

```rust
use mimecraft::{ContentDisposition, ContentType};

let content_type = ContentType::parse(b"Text/Plain; charset=utf-8; format=flowed")?;
assert!(content_type.is_mime_type("text", "*"));
assert_eq!(content_type.format(), Some("flowed"));

let disposition = ContentDisposition::parse(
    b"attachment; filename*=utf-8''caf%C3%A9.txt; size=123"
)?;
assert!(disposition.is_attachment());
assert_eq!(disposition.file_name(), Some("caf\u{e9}.txt"));
assert_eq!(disposition.size()?, Some(123));
# Ok::<(), Box<dyn std::error::Error>>(())
```

Both types expose `parameters()` and accept `ParameterParseOptions` through
`parse_with_options`. Options affect parameters, not the top-level tokens.
All syntax-error offsets and parameter diagnostic spans refer to the complete
input value, including leading whitespace and the type/disposition token.
Syntax failures return no partial result.

Comments and folding around tokens are accepted, and spelling is preserved.
`is_mime_type` compares case-insensitively and supports a whole-component `*`
in either argument. `mime_type()` returns only `type/subtype`, not a serialized
header. MimeKit's permissive subtype handling, including extra slashes in
`multipart/multipart/mixed`, is retained; parsing is not strict RFC validation.
Unknown ASCII disposition atoms such as `form-data` are preserved.

Disposition parsing follows MimeKit's throwing `Parse` behavior: quoted or
missing disposition atoms are errors, rather than the attachment recovery of
`TryParse`. Non-ASCII disposition bytes are rejected explicitly instead of
MimeKit's lossy ASCII question-mark replacement.

Content-Type accessors expose `boundary`, `charset`, `format`, and `name`;
`charset_encoding()` resolves the label using the shared charset layer.
Content-Disposition exposes `file_name`, `creation_date`, `modification_date`,
`read_date`, and `size`. Charset, date, and size conversion returns
`Result<Option<_>, _>`: absence is `Ok(None)`, while an unsupported or invalid
value, including an empty value, is an error rather than silent absence.
Dates use the existing tolerant `DateTimeOffset` parser. Size requires ASCII
digits only, with no sign or surrounding whitespace, and retains MimeKit's
`i64::MAX` limit despite returning `u64`. These conversions are lazy; their
failure does not prevent access to the decoded parameter strings.

### Constructing and editing content headers

```rust
use mimecraft::{ContentDisposition, ContentType, Parameter};

let mut content_type = ContentType::new("text", "plain")?;
content_type.set_charset(Some("utf-8"));
content_type.set_format(Some("flowed"));
content_type.parameters_mut().add(Parameter::new("x-custom", "value")?)?;

let mut disposition = ContentDisposition::default(); // attachment
disposition.set_file_name(Some("caf\u{e9}.txt"));
disposition.set_size(Some(123))?;
disposition.set_is_attachment(false); // inline, retaining parameters
assert_eq!(disposition.file_name(), Some("caf\u{e9}.txt"));
disposition.set_size(None)?; // remove the size parameter
# Ok::<(), Box<dyn std::error::Error>>(())
```

`ContentType::new`, `set_media_type`, and `set_media_subtype` validate complete
components using the same ASCII alphabets as the parser, including its
extra-slash subtype compatibility. This deliberately tightens MimeKit's
unchecked Content-Type assignments: empty values, comments, whitespace, and
other malformed components are rejected instead of stored. It is not strict
RFC validation. `ContentDisposition::new` and `set_disposition` require a
nonempty ASCII atom, including extension dispositions such as `form-data`.
Failures report the first invalid byte in the affected component and do not
change the object.

`parameters_mut()` exposes the ordered parameter-editing API. Convenience
setters accept `Some(value)` to set a parameter and `None` to remove it.
`set_charset` stores a literal label; `set_charset_encoding` uses a supported
charset's canonical name. Date setters take checked `DateTimeOffset` values
and store their canonical date-header formatting. `set_size` takes `Option<u64>`
and rejects values above `i64::MAX` without changing the previous value.

Edits retain original parse diagnostics and never modify a separate raw
`Header`. Clones have independent owned values. Boundary values and filenames
are not validated or sanitized, and size is declared metadata, not measured
content.

### Serializing content headers

`ContentType::to_header_string` and `ContentDisposition::to_header_string`
return a complete ASCII header: **field name, colon, encoded value, and one
terminating CRLF**. They do not add the blank line separating headers from a
message body. Use `to_header_string_with_options` to select formatting:

```rust
use mimecraft::{ContentDisposition, HeaderFormatOptions};
use mimecraft::encodings::NewLineFormat;

let mut disposition = ContentDisposition::default();
disposition.set_file_name(Some("caf\u{e9}.txt"));
assert_eq!(
    disposition.to_header_string()?,
    "Content-Disposition: attachment; filename*=iso-8859-1''caf%E9.txt\r\n"
);

let options = HeaderFormatOptions {
    max_line_length: 78,
    new_line_format: NewLineFormat::Unix,
    always_quote_parameters: true,
    ..Default::default()
};
let header = disposition.to_header_string_with_options(&options)?;
assert!(header.ends_with('\n'));
assert!(!header.contains('\r'));
# Ok::<(), mimecraft::HeaderFormatError>(())
```

Ordinary values are tokens or escaped quoted strings; empty values are `""`.
Unicode, ASCII controls (including CR, LF, tab, DEL, and NUL), and literal `=?`
sequences use RFC 2231 percent encoding by default. This keeps authored
encoded-word text literal when reparsed instead of accidentally interpreting it
as RFC 2047.
Long values use numbered continuations. Escaped quotes, whitespace, complete
percent triplets, and Unicode scalar boundaries are preserved. Encoding does
not remove controls from the decoded value or make a filename safe to use.

Set `HeaderFormatOptions::international` to `true` to allow raw UTF-8 in quoted
parameter values and unencoded numbered continuations. These values remain
UTF-8 regardless of preferred charsets; scalars and quoted-pair escapes are never
split. Controls and literal `=?` sequences still require encoding. Selecting
RFC 2047 also retains encoded-word output for values too long for an ordinary
assignment. Short safe Unicode values use quoted UTF-8 even with an explicit
encoding-method preference. The default remains ASCII output.

This option applies only to Content-Type and Content-Disposition parameters,
not international addresses or other headers, and does not negotiate transport
capabilities. For SMTP delivery, generally set `international` to match whether
the server advertises `SMTPUTF8` in its `EHLO` response. Keep it false if the
extension is unavailable; enabling it is optional when supported. The caller
must ensure that the destination accepts UTF-8 headers.

For encoded output, charset selection follows MimeKit's automatic choice:
printable ASCII uses `us-ascii`, controls and text entirely within U+00FF use
`iso-8859-1`, and other Unicode uses `HeaderFormatOptions::parameter_charset`
(UTF-8 by default).
`Parameter::set_charset(Some(charset))` overrides that preference; `None`
restores inheritance. These are preferences, not forced encodings: ASCII and
Latin-1 selection still takes precedence, and a preferred charset that cannot
represent every character falls back to UTF-8 for the **entire value**, never
replacement characters or mixed charsets. Supported encodings and legacy
mapping limitations are those of `charsets::Charset`.

This selection is independent of a Content-Type `charset` parameter, which
describes the body rather than parameter text. Extended continuations
consistently use percent encoding; long ordinary ASCII values use unencoded,
optionally quoted sections. Legacy/stateful and UTF-16 chunks are encoded as
complete strings, preserving scalar boundaries and including state-reset bytes
in their line budgets. Parameter and encoded-word decoding restart character
conversion at explicit ISO-2022-JP ASCII-reset boundaries, while preserving
split transfer escapes and charset state elsewhere. Original language tags,
charset alias spelling, and unnecessary quoting are not retained.

Set `HeaderFormatOptions::parameter_encoding_method` to
`ParameterEncodingMethod::Rfc2047` for legacy clients that require encoded words
inside quoted parameter values. This compatibility convention is not standard
MIME parameter encoding; RFC 2231 remains the default. RFC 2047 output uses the
shared B/Q encoder, with complete characters and words of at most 75 bytes.
Original whitespace stays inside the encoded payloads, so folding cannot change
the decoded value. Short ASCII values still use ordinary tokens or quoted strings.

`Parameter::set_encoding_method(Some(method))` overrides the header-wide choice;
`None` restores inheritance. `Parameter::set_always_quote(true)` requests quoting
of that parameter's ordinary values and unencoded continuation sections, even
when header-wide quoting is disabled. False does not override header-wide quoting.
Charset, encoding-method, and quoting preferences survive cloning and value
edits, including `ParameterList::set`
and `ContentDisposition::set_file_name`. Add an authored `Parameter` with
`ParameterList::add` or replace an existing entry with `ParameterList::replace`;
names remain checked for case-insensitive uniqueness.

Parsing now recovers per-parameter encoding preferences. Extended values and
continuations select RFC 2231; the first encoded section supplies the charset,
including Latin-1 recovery for missing or unsupported declarations. Entirely
unencoded continuations inherit the charset. Ordinary values containing
recognized encoded words select RFC 2047 and the most common word charset,
with ties resolved by first encounter and charset aliases counted together.
As in MimeKit, RFC 2047 UTF-8 inherits the header-wide charset; a predominant
unsupported label leaves both preferences unset rather than remembering a
guessed charset. Unlike upstream's default codepage result for plain text,
ordinary values without recognized encoded words keep default preferences.

Recovered preferences take precedence over header-wide formatting choices.
Clear them with `set_encoding_method(None)` and `set_charset(None)` on an owned
parameter and replace its list entry to restore inheritance. These preferences
do not force short safe values to be encoded or disable automatic Latin-1
selection. Formatting still normalizes syntax rather than reproducing the wire.

The default line limit is 78 **bytes**, counting the field name, tabs, quotes,
and separators but excluding newline bytes. Continuation lines start with one
tab. Limits outside `60..=998` and `NewLineFormat::Mixed` return
`HeaderFormatError`. Unlike MimeKit's soft target, the requested limit is
enforced: section-number growth and separators are included, and oversized
indivisible names, primary values, or encoded characters return `LineTooLong`
rather than exceeding it. Consequently, exact folding and section packing can
differ from MimeKit. A charset-only initial section is allowed when the
declaration fits but the first character must go in the following section.

Serialization preserves parameter order and spelling without mutating values,
historical diagnostics, or separately stored raw headers. Failures return no
partial output. These APIs normalize headers, not reproduce their original
bytes; use `Header::raw()` for the latter. Unstructured and address-header UTF-8
output have their own independent formatting options.

## Scope and compatibility

The target is MimeKit.Core: transfer encodings, headers and addresses, MIME
entities, message parsing and writing, and the core's supporting utilities.
Base64, quoted-printable, UU, yEnc, and pass-through codecs, common incremental
codec traits, plus parameter percent and RFC 2047 payload codecs, are implemented
so far, along with newline, pass-through, codec-adapter, `From ` escaping/armoring,
trailing-whitespace filters, incremental charset conversion, transfer-encoding
analysis, and synchronous
filtered reading and writing, seekable MIME content streams, bounded source
ranges, chained streams, block-backed memory storage, and measuring sinks, plus incremental
raw header-block parsing, standalone header parsing and display decoding,
ordered header collections, known header identifiers,
basic transfer-encoding/version value parsing, message identifiers and
references, address specifications, mailboxes, groups and address lists,
domain routes and IDNA conversion,
quoted-string helpers, date-header handling,
charset conversion, RFC 2047 text/phrase encoding and decoding, MIME parameter parsing, and
typed Content-Type/Content-Disposition parsing, construction, editing, and
folded serialization with RFC 2231 parameters and opt-in UTF-8 parameters.
Single/text/multipart ownership and writing, complete entity-tree loading, a message wrapper
with raw header partitioning and load/write APIs, and explicit attached-message
ownership and writing are also implemented. A synchronous event reader and
owned-model parser share default malformed-message recovery and explicit limits.
Source-backed storage, explicit mbox iteration, and header-only EOF loading
are implemented.
MimeKit.Cryptography, including DKIM, ARC, and DMARC, is excluded.
`LegacyMimeParser` and its associated event-argument types will not be ported;
parser behavior is being adapted from the current `MimeReader` implementation.

Recovery coverage includes frozen topology/raw-content snapshots generated from
the current MimeKit reader/parser sources, exercised with and without logging
across small and large refills. The ported scanning tests cover randomized and
exhaustive kernel inputs, generated MIME/mbox messages, and all 202 original
files in `tests/data/messages` and `tests/data/mbox`. Original compliance fixtures
in `tests/data/compliance` exercise LF/CRLF issue kinds, ordering, and positions.
Body null/8-bit diagnostics deliberately identify the offending byte rather
than MimeKit's buffer-dependent line-fragment start.
Header diagnostics follow MimeKit's exact positions for the first null byte
and first invalid UTF-8 sequence in each header, including continuation lines
and valid multibyte sequences preceding malformed bytes. Valid RFC 6532 UTF-8
remains accepted. Generic byte diagnostics still cover address headers until
the address-specific compliance validators are ported.
Additional ports cover reader content-callback ranges, physical line positions,
empty multipart/message containers, consecutive mbox markers, and truncated
header diagnostics. Parser cases cover truncated/raw invalid headers, bare-CR
endings, status-group EOF behavior, and embedded-message boundary lookahead
across short reads, including bounded probing of long mbox markers.
The `issue1189_mbox` and `jwz_mbox` reader tests compare full nested offset trees
against MimeKit's original Unix and DOS JSON goldens across one-byte, short,
and full-buffer refills. They check MIME types, message/part nesting, marker and
header offsets, octet counts, and content-line counts. The recorder adapts
MimeKit's end-callback clamping of empty-body header separators to Rust's
physical header-end positions reported at entity begin, including ancestors
ending at the same delimiter. Closing-marker newlines shared with an immediate
parent or mbox delimiter are excluded from entity-end positions; EOF retains them.
Both corpora also check parsed model topology, raw body bytes, source ranges,
and exact message serialization with and without logging in default, Memory,
and Source modes.
Tests retain MimeKit's names in
snake_case and applicable inline comments; synchronous cases also cover the
input-fragmentation scenarios from its asynchronous variants.
Parser round-trip coverage now includes all 21 upstream delimiter-newline cases
in LF and CRLF, with repeated `ensure_new_line` serialization, and the 69-file
synchronous fixture corpus across one-byte, short, and full refills, using
both copying and source-backed storage as well as the original default API.
The corpus includes opaque signed MIME from the OpenPGP, DKIM, and S/MIME test
directories, without implementing cryptographic operations. Additional cases
exercise raw padded/mixed-newline markers, mutation invalidation, binary tails,
logged parsing, and output-failure retries.
Lifecycle regressions cover failure after partially reading a message body,
nested callback failures followed by source replacement, and owned messages
remaining readable after their sources and parser are dropped. Extreme-nesting
reader cases use MimeKit's 10,000 multipart and 100,000 message-wrapper depths,
checking balanced events and exact innermost content without building a model.
Ordinary header lines are rejected as boundary candidates before ancestor
scanning, following MimeKit's boundary-prefix check.
The double-boundary parser case also checks the recovered empty first child
and exact serialization with and without logging.
Five further upstream parser cases cover invalid Content-Type values, an empty
header name followed by a colon, a doubled colon in Content-Transfer-Encoding,
and rejection of ordinary garbage before message or entity headers. They run
with Unix/DOS input, one-byte/short/full refills, logging, default/Memory/Source
storage, and direct slices. Successful recovery retains header offsets,
parameters, raw body bytes/source ranges, and repeatable serialization after
the parser is dropped. Unlike MimeKit's recovered typed properties, Rust's raw
model keeps invalid Content-Type and transfer-encoding getters fallible; the
parser still recovers a leaf with unspecified stored encoding, without
rewriting the malformed headers or decoding an invalid `: base64` value.
Six subsequent cases cover invalid headers after mbox From-lines, streams
containing only markers, truncated markers with and without trailing CR,
standalone quoted-printable MIME leaves, and mixed body line endings.
Marker EOF errors consume the available input, report `UnexpectedEof`, and
leave the failed source unusable until replacement, rather than inventing an
empty message. The trailing-CR case retains MimeKit's nontermination regression
rationale. Leaf cases check metadata, header offsets, retained body ranges,
and serialization after dropping the parser across buffered/storage/slice and
logging variants. Mixed-newline bodies keep their `Mixed` hint and original
decoded text even after repeated Unix/DOS output normalization; unlike
MimeKit's Text getter, Rust does not normalize newlines when reading text.
Five structure-preservation cases cover CR-only invalid header lines, empty
multipart alternatives, textual multipart epilogues, badly folded Exchange
diagnostic headers, and header-only messages with no body. They check header
offsets, container/leaf topology, and stable output through three reparses,
using Unix/DOS output, fragmented reads, default/Memory/Source storage, direct
slices, and logging. Models remain usable after their parsers are dropped.
The CR-only fuzzer cases retain MimeKit's rationale and verify that invalid
records never become header/body separators or lose carriage returns.
Fixture round trips preserve the complete wire input; empty alternatives
keep their following sibling and its content,
and epilogue bytes remain separate from the closing delimiter's newline.
Parsed delimiter lines retain their original endings, so ordinary fixtures
use matching input/output newlines; the CR-only cases also exercise conversion.
Further upstream parser regressions cover folded recipient headers with 1,
100, and 50,000 addresses, preserving raw folds and following-header offsets;
Japanese ISO-2022-JP subject/body decoding; and unmunged mbox `From ` lines.
These run across short reads and default, Memory, and Source storage.
Leading-mbox-BOM cases check offsets, exact input limits, malformed/truncated
marks, interrupted and failing reads, source replacement, and nonzero backing
origins while preserving byte order marks inside bodies.
Standalone-message BOM cases port MimeKit's partial-prefix and BOM-only EOF
regressions and cover the same buffered/storage/slice paths. Additional checks
exercise every prefix split in the byte-fed API, exact byte/header budgets,
original header/body ranges, source replacement after failure, and nested/body
BOM preservation. Initialization does not read again after observing EOF or
delay the early callback that can reject a message without I/O.
Nine further upstream cases cover garbage-only and empty message/entity/mbox
streams, an empty message with a header/body separator, 4097-byte mbox markers,
and long garbage lines before the simple mbox corpus. They exercise Unix/DOS
newlines, logging, direct slices, fragmented reads, and default/Memory/Source
storage. Empty message/entity streams fail with `UnexpectedEof`; empty mbox
iteration returns `None`, the intentional Rust adaptation of MimeKit's EOF
exception. Garbage is rejected with `InvalidData`, and failed sources cannot be
reused. The garbage-prefix cases retain MimeKit's refill/midline rationale and
verify all three nested multipart trees, original header offsets, decoded
content, and serialization after dropping the parser. Long markers retain their
full bytes in reader events, not stored message headers, and exact marker-length
limits hold across buffered and byte-fed pauses. Empty-message serialization
stays stable through repeated reparsing. No production changes were needed.
Further reader cases cover folded transfer-encoding value positions,
colonless metadata records at EOF, embedded-message first-header diagnostics,
and overlong or space-terminated multipart boundaries. The short-refill
byte-position matrix includes malformed UTF-8, folded and nested headers,
multiple offending bytes, EOF, and LF/CRLF input. Representative cases also
exercise every two-way byte-fed split and one-byte feeds.
Invalid field-name diagnostics identify non-ASCII bytes as well as controls.
A first embedded header retains its newline when immediately followed by a
MIME or mbox delimiter, preserving the raw record and the missing-separator
position; a framing-only blank line still cannot create an absent message.
Logged and unlogged reads retain identical events and final positions.
Three further parser cases cover growing invalid leading headers inside an
embedded message, munged mbox-style lines before ordinary message headers,
and a leading marker followed by either an EOF-truncated Subject or an empty
message. Unlike MimeKit's separate Message.MboxMarker, the Rust model retains
these records as invalid raw headers and serializes them unchanged; actual
mbox iteration still consumes its marker separately. Coverage checks original
offsets, header order, payload/source ranges, and repeated serialization after
the parser is dropped across LF/CRLF, short refills, slices, logging, and
default/Memory/Source storage. Growing preheaders cross 4 KiB and 8 KiB refill
boundaries; representative messages also exercise every two-way byte-fed split.
EnsureNewLine does not append a newline to an EOF-truncated header when there
is no body. Existing production behavior satisfies these cases.
Deep-nesting parser cases now cover 64 message/rfc822 wrappers and 64 multipart
containers, checking iterator depths, retained headers, innermost content/source
ranges, and repeated serialization after the parser is dropped. A five-level
sweep compares reader event types and model structure across the full depth
boundary, retaining MimeKit's off-by-one regression rationale.
Rust's positive, root-inclusive depth budget remains a hard limit: 64
containers plus their innermost leaf require a budget of 65, while a budget of
64 fails rather than returning MimeKit's opaque truncated leaf. Rejected parses
return no partial tree and cannot be reused without replacing the source.
Coverage includes LF/CRLF, short refills, slices, logging, default/Memory/Source
storage, exact depth-issue positions, and byte-fed splits for homogeneous and
mixed nesting. No production changes were needed.
Four further upstream cases cover a missing Content-Type subtype, a truncated
header name, a lone CR between status groups, and consecutive mbox markers.
They retain the established raw-model differences: malformed standalone
headers and status-group records remain available instead of throwing, and
the typed Content-Type getter rejects the missing subtype while preserving
the raw filename parameter. A lone CR is not silently treated as a separator.
Adjacent mbox markers yield an empty message followed by the populated message,
with physical positions accounting for the five-byte next-marker lookahead.
Coverage includes LF/CRLF, short refills, slices, logging, retained header
offsets, and repeated serialization after parser drop; message cases also
exercise default/Memory/Source storage and every two-way byte-fed split.
No production changes were needed.
Header-limit regressions accept an exact-sized folded header, reject one extra
byte or an oversized field name, and verify the failing header's offset,
including enclosing multipart/message and mbox framing, and early termination
under the default limit. MimeKit's long-line mbox cases place fake midline
markers at offset 4096 and span buffer refills; they verify
message IDs, positions, raw bodies, and serialization in all storage modes.
Mixed-nesting cases compare entity/message hierarchy, offsets, and line/octet
counts against MimeKit's original recursive-reader event goldens. They cover
following siblings, ancestor-boundary unwinding, multipart/digest defaults,
empty message wrappers, and missing boundary parameters, with Unix/DOS input,
fragmented reads, and logging. Owned trees, raw bodies/source ranges, and
repeated byte-exact serialization are checked in all storage modes after the
parser is dropped. Golden comparisons account for MimeKit's logical end
offsets: Rust retains the delimiter-preceding newline in a truncated multipart's
physical preamble range, just as the upstream preamble callbacks do.
Ten additional upstream multipart cases distinguish EOF at the end of an
opening delimiter from EOF after its newline: the former retains the marker
without inventing a child, while the latter retains an empty child. Coverage
also includes truncated child header values/names, a closing delimiter in place
of the header/body separator, and invalid `--`-prefixed headers that are not
boundaries. Unix/DOS input, one-byte/short/full refills, logging, and default,
Memory, and Source storage all preserve raw headers, body bytes, and source
ranges. Repeated serialization with `ensure_new_line` toggled retains the
original framing after the parser is dropped. Empty recovered leaves keep a
missing Content-Type absent even though MimeKit classifies them as TextPart
instances.
Nine more upstream multipart recovery cases cover missing closing delimiters,
legal boundary-line padding, false delimiters with non-whitespace suffixes,
and repeated opening delimiters followed by content, a closing delimiter, EOF,
or a parent boundary. Nested message/rfc822 EOF recovery preserves the final
text without inventing closing delimiters or a trailing newline. A multipart
without a boundary parameter keeps its entire body as preamble rather than
guessing delimiters. These cases check exact topology, headers, decoded bytes,
source ranges, and repeatable serialization after dropping the parser, across
Unix/DOS newlines, short refills, logging, and all storage modes. Applicable
MimeKit serialization comments are retained. Existing parsing and serialization
behavior satisfies these cases without production changes.
Eight additional message/rfc822 cases cover wrappers truncated after their
headers or followed immediately by a closing delimiter; complete, EOF-only,
and truncated munged From-markers; transparent wrappers with absent or empty
transfer encoding; and opaque Base64 wrappers decoded and reparsed explicitly.
They run with Unix/DOS newlines, one-byte/short/full refills, logging, and
default/Memory/Source storage, checking topology, metadata, raw bodies/source
ranges, and repeated serialization after the parser is dropped. The munged
message reuses the exact upstream inline fixture from the existing mbox corpus.
Rust keeps embedded From-markers as invalid raw headers rather than a separate
message marker property. EOF-only markers remain unterminated even with
`ensure_new_line`, matching MimeKit's verbatim marker and absent-content rules.
The empty-encoding regression exposed and fixed wrapper serialization rejecting
unchanged recovered encoding headers. Focused writer cases also enforce strict
validation of authored/changed values without invalidating recovery on unrelated
edits. Applicable upstream comments and compatibility rationale are retained.
The applicable synchronous MimeReader/MimeParser regression batch is complete.
Source-storage cases also cover shared
mbox cursors, owned/borrowed lifetime, nonzero source origins, buffered source
replacement, concurrent decoding, backing-source reads, and seek/read failures.
Opt-in Content-Length-aware mbox parsing compares the original `content-length`
Unix/DOS offset goldens across one-byte, short, and full refills. Parser cases
check the same corpus's topology, raw bodies/source ranges, and exact message serialization
with and without logging. Additional regressions cover MimeKit's embedded
From-marker case, numeric-prefix and duplicate-header rules, thresholds inside
prefixes and buffered chunks, nested lengths, EOF, resource limits, source
replacement, and fallible callbacks/loggers.
No LegacyMimeParser tests are included.

The closeout adds canonical short-read multipart and mixed-newline mbox cases,
split-CRLF body diagnostics, parser option/reporting/position forwarding, and
mixed-container and implied `multipart/digest` depth boundaries. The exact
root-inclusive budget retains the complete tree without diagnostics; lower
budgets report one precise depth issue and fail without a partial model.
The implied message wrapper charges a level at its opening delimiter, and its
embedded root charges another at its first header. Reader cases add exact
bare-LF body positions, binary null/8-bit acceptance, invalid raw-header byte
positions, and missing required fields at separators and EOF. Missing-field
checks distinguish Message/Mbox from Entity/Headers and embedded messages;
Content-Type or transfer encoding requires MIME-Version, but disposition alone
does not. Byte-fed counterparts cover short-read mbox input, CRLF splits,
binary bodies, required-field diagnostics, and implied-depth failures.
Applicable upstream comments are retained. No production changes were needed.

The final name audit reconciles 123 unique synchronous parser names and 63
reader names from the local upstream sources. These are names, not expanded
parameterized executions; the reader count includes an inactive historical
case. Of the 32 previously unmatched names, 14 now have direct regression
ports; the remaining 18 map to existing coverage or deliberate exclusions:

| Upstream family (`Test` prefix omitted) | Rust coverage or disposition |
| --- | --- |
| Parser `ShortReads`, `CrLfSplitAcrossReadsIsNotBareLinefeed` | Direct snake_case ports across slices, short reads, logging, and default/Memory/Source storage. |
| Parser `MimeComplianceMimeDepthLimitExceeded`, `MimeComplianceMimeDepthLimitExceededImpliedContentType` | Direct ports adapted to fatal, root-inclusive limits rather than MimeKit's flattened leaves. |
| Parser `MimeParserForwardsReaderProperties` | Direct port of options, compliance policy, logger effects, positions, and EOF; nullable setters are not a Rust API. |
| Reader `BareLinefeedsInBodyPositions`, `BinaryContentTransferEncodingAcceptsNullAnd8BitBytes`, `InvalidHeaderFieldReportsNullByte` | Direct ports; standalone raw headers remain available, including invalid records. |
| Reader `MimeComplianceMissingHeaders`, `MimeComplianceMissingHeadersMbox`, `MimeComplianceMissingHeadersIncompleteHeader`, `MimeComplianceMissingMimeVersion`, `MimeComplianceMissingHeadersIgnoresEmbeddedMessages`, `MimeComplianceMissingHeadersIgnoresEntitiesAndHeaders` | Direct ports with exact issue order, positions, scope, and logged/unlogged event parity. |
| Parser/reader `ArgumentExceptions` | Rust types rule out null/wrong-type arguments; `option_validation_precedes_io_and_limits_remain_strict` and source/storage error tests cover runtime validation. |
| Parser `ContentStorageLifetime` | `storage_loaders_keep_owned_sources_until_content_is_released`, borrowed-source/failure counterparts, and `storage_keeps_ranges_or_copies_and_releases_last_owner`. |
| Parser `JwzPersistentMbox` | `jwz_mbox` checks upstream offset goldens, model topology, bodies, and serialization in all storage modes. |
| Parser `SimpleMbox`, `SimpleMboxWithByteOrderMark` | Simple-mbox corpus/garbage-prefix cases and `mbox_byte_order_mark_keeps_source_ranges_and_replacement_origins`. |
| Parser `StatusGroupParserNoStatusGroups` | Existing no-content, blank-line, LF, and lone-CR status-group cases. |
| Parser `RejectedSetStream` | `rejected_replacement_preserves_mbox_state_and_old_bodies`. |
| Parser `SetStreamContentStorage`, `SetStreamPersistentOverloads` | Storage constructors and replacement/lifetime cases. Rust replacement retains storage/options; changing storage requires a new parser, not a C# overload or implicit reset to Memory. |
| Parser `MaxMimeDepthMatchesLegacyMimeParser` | `max_mime_depth_matches_reader` compares shared reader/model behavior across the full boundary; LegacyMimeParser itself is excluded. |
| Parser `MimeParserForwardsAllCallbacks`, `MimeParserIsNotAMimeReader` | C# protected virtual forwarding/inheritance checks do not apply. Rust reader events and private model assembly are covered by nested topology/offset goldens. |
| Parser `LegacyPersistentNonseekableCompatibility` | Excluded Legacy-only fallback behavior; source-backed Rust constructors require seeking. |
| Parser `MboxMarkerAsyncCallbacksNotUsedBySyncParse` | Excluded async virtual-dispatch API; synchronous callbacks and byte-feed pauses are covered separately. |
| Reader `ReadToEosReturnsBufferedAndUnreadInput` | `standalone_headers_preserve_raw_blocks_separators_and_unread_bodies` checks `into_inner().read_to_end()` across every buffer capacity. |
| Reader `NextGrowSize` | C# array-growth formula is not a Rust API; Vec-backed header growth, refill boundaries, and limits have behavioral coverage. |
| Reader `MimeComplianceInvalidMimeVersion` | Inactive upstream `CAN_DETECT_MIME_VERSION_ISSUES` block, not an active parity requirement; the active missing-version case is ported. |

This closes the bounded parser/reader batch, not every MimeKit feature.
Async/Legacy APIs remain excluded from this batch, and further compliance
reporting implementation/performance work remains paused.

### Sans-I/O MIME parsing

The MIME scanner and tree engine now preserve their state between byte-fed
calls without owning or reading a source. This includes partial header and
delimiter lines, CRLF lookahead, multipart/message frames, newline observations,
initial message BOMs, mbox prefixes, Content-Length protection, and input
budgets. The existing `MimeReader` and model-building `MimeParser` APIs use a
synchronous driver over that same core; source-backed storage remains outside it.

`MimeEventParser::new(MimeEventFormat::{Entity, Message, Mbox})` exposes the
core to callers. `parse(bytes, callback)` consumes the supplied slice on success;
empty slices are no-ops. `finish(callback)` explicitly signals EOF, applies
existing truncation recovery, and is idempotent after success. Parsing and
callback/logger failures poison the parser rather than allowing unsafe retries.
Mbox mode streams all entries and resets input/reporting budgets per message.
`with_options` and `with_logger` retain the reader's limits and reporting policy.
`HeaderParser` already provides separate byte-fed parsing and I/O adapters and
does not need a replacement.

Ordinary body bytes still borrow input buffers; only headers and ambiguous
lookahead are retained. The parser can move between threads between calls when
its logger is `Send`; local-only loggers remain supported. Callbacks themselves
are synchronous and borrowed content must be copied if retained for downstream
async processing. Drivers can pause between bounded input chunks without
finalizing; a callback error is not a backpressure signal. Runtime-specific
async drivers and async model/storage APIs remain deferred.

Direct byte-fed regressions exercise every two-way split of representative
nested/recovered messages and mbox archives, one-byte feeds, empty feeds, explicit
EOF, exact input limits, padded delimiters, long lines, terminal callback/logger
failures, borrowed content, and thread migration. Corpus and generated-message
checks compare byte-fed and synchronous events (including positions and errors)
with and without logging. Existing model/source-storage and byte-preserving
recovery regressions continue to cover the synchronous adapter.

The source baseline for this initial port is the local MimeKit 5.0.0 development
tree. The Base64 implementation follows `MimeKit/Encodings/Base64Encoder.cs` and
`Base64Decoder.cs`; tests adapt cases from the corresponding
`UnitTests/Encodings` files and add chunk-boundary coverage. SIMD optimizations
are adapted from MimeKit's SSSE3, AVX2, and AdvSimd kernels, which credit the .NET
runtime Base64 helpers, Wojciech Mula's base64simd project, and aklomp/base64.
The quoted-printable port follows `QuotedPrintableEncoder.cs` and
`QuotedPrintableDecoder.cs`, with compatibility vectors and line-boundary
regressions adapted from their corresponding unit tests.
The UU port follows `UUEncoder.cs` and `UUDecoder.cs`, including their SSSE3,
AVX2, and AdvSimd kernels, begin-line scanning, terminator handling, and
malformed-input behavior.
The yEnc port follows `YEncoder.cs`, `YDecoder.cs`, and `Utils/Crc32.cs`,
including their checksum lifetime and permissive header/payload transitions.
Complete block reading and writing follow `Encodings/YEncHeader.cs`,
`YEncReader.cs`, `YEncWriter.cs`, `YEncFormat.cs`, and `YEncOptions.cs`, with
applicable comments retained.
`UnitTests/Encodings/YEncTests.cs` supplies exact empty/single-part/multipart wire
fixtures, chunked round trips, source-size/checksum failures, charset behavior,
limits, and faulted-state cases. Rust tests additionally cover all 939 supported
payload widths with both newline formats, UTF-8 byte limits, interrupted and
short I/O, failures during each output stage, and independent bitwise CRC checks.
Reader coverage includes all seven valid and 39 invalid upstream block fixtures,
bounded scanning and EOF behavior, control-field grammar, multipart checksums,
provisional output, buffered-source recovery, and faulted-state behavior.
The five upstream `TestData/yenc` fixtures cover strict and tolerant multipart
JPEG decoding, MIME-body decoding and byte-for-byte re-encoding, and permissive
decoder state transitions. The initial-payload-escape correction also matches
the current upstream decoder, with exhaustive initial-byte/split/clone coverage.
Asynchronous entry points remain deferred.
The header payload codecs follow `Rfc2231Encoder.cs`, `Rfc2231Decoder.cs`,
`Rfc2047Base64Encoder.cs`, and `Rfc2047QuotedPrintableEncoder.cs`, using the
character classifications in `Utils/ByteExtensions.cs`.
The common interfaces and pass-through codecs follow `IMimeEncoder.cs`,
`IMimeDecoder.cs`, `PassThroughEncoder.cs`, `PassThroughDecoder.cs`, and
`ContentEncoding.cs`, adapted to caller-owned Rust buffers and traits.
Filename MIME-type and preferred-extension mappings follow `MimeTypes.cs` and
`UnitTests/MimeTypeTests.cs`, using both tables independently and retaining the
source's registry comment. Sorted static tables plus
instance-local overrides replace global dictionaries; registration keeps the
source's independent overwrite and alias-retention rules. ASCII-only key
folding and portable slash/backslash handling are explicit compatibility
differences. Selected IANA registrations checked on October 23, 2026 supplement
the original tables: WebAssembly, YAML, TOML, JPEG XL, HEIC/HEIF images and
sequences, Zstandard, and AVIF/JSON-LD preferred extensions. Registered gzip and
FLAC names replace the original filename guesses while retaining legacy
reverse aliases. Other MimeKit mappings are unchanged.
The initial filter layer follows `IO/Filters/IMimeFilter.cs`,
`EncoderFilter.cs`, `DecoderFilter.cs`, `PassThroughFilter.cs`,
`Dos2UnixFilter.cs`, and `Unix2DosFilter.cs`, retaining source state transitions
without inheriting MimeKit's buffer-management base class.
Incremental charset conversion follows `IO/Filters/CharsetFilter.cs` and
`UnitTests/IO/Filters/FilterTests.cs`, retaining independent decoder/encoder
state and the source's all-characters encoding loop comment. Empty final input
is passed to the decoder to resolve buffered incomplete characters before the
target is finalized. Shared charset resolution, bounded UTF-8 intermediates,
explicit replacement flags, and uniform U+FFFD/`?` fallback adapt the source to
the existing Rust filter trait without changing complete-buffer strict APIs.
The text filters follow `MboxFromFilter.cs`, `ArmoredFromFilter.cs`, and
`TrailingWhitespaceFilter.cs`, with whitespace-run storage adapted from
`Utils/PackedByteArray.cs` and source vectors from `UnitTests/IO/Filters/FilterTests.cs`.
Transfer-encoding analysis follows `BestEncodingFilter.cs` and
`EncodingConstraint.cs`, including source threshold and final-line behavior.
Filtered reading follows the synchronous read path in `IO/FilteredStream.cs`
and its tests, adapted to Rust's `Read`/`BufRead` traits and nested readers.
Filtered writing adapts the same source's write and one-time flush behavior,
separating I/O flush from filter finalization and retaining short-write progress
to meet Rust's `Write` contract.
MIME content streams follow `MimeContent.cs` and `IMimeContent.cs`, with source
rewind and destination-flush behavior adapted from their synchronous APIs.
`Read + Seek` bounds and exclusive reader borrows replace runtime capability
and disposal checks. Existing decoder filters are reused, copy methods return
byte counts, and ordinary I/O failures propagate without cancellation-specific
cleanup or silent recovery. Async and cancellation APIs are not ported.
The initial single-part model follows `MimePart.cs` and `MimeEntity.cs`,
including the default media type, first-header metadata lookup, and independent
stored/requested encodings. Raw headers and detached typed values replace
change events and mutable caches. Checked setters reuse header formatting and
duplicate-collapsing replacement; missing content metadata stays absent and
invalid values return errors rather than silently selecting defaults.
Content-Id follows `MimeEntity.ContentId`, sharing the existing loose raw-prefix
parser and checked value validator across all entity kinds. Explicit accessor
errors replace the upstream invalid-value null fallback; raw edits need no
cache invalidation. Assignment always canonicalizes and collapses duplicates,
without implicit ID generation or changes to related references.
Attachment flag editing follows `MimeEntity.IsAttachment`: setting true creates
or promotes a disposition; setting false only demotes an attachment to inline.
Matching flags preserve raw headers, including duplicates, while actual changes
preserve parsed parameters and use atomic duplicate-collapsing formatting.
Explicit parse errors replace implicit malformed-header fallback behavior;
checked formatting failures preserve the original metadata.
Filename metadata follows `MimePart.FileName`: disposition-first lookup,
Unicode trimming on read, and updates to both MIME parameter collections.
Detached parsing and formatting make the two-header update atomic. Unlike
MimeKit's always-present ContentType object, missing raw Content-Type prevents
non-null assignments instead of silently supplying a media type. Removal
preserves missing fields; equal assignments use canonical duplicate replacement.
UU writing shares untrimmed lookup so its existing control checks remain intact.
Content-Description and Content-Duration follow the same source's nullable
properties: tolerant decoded descriptions and nonnegative integer seconds,
with explicit errors replacing the duration getter's implicit null fallback.
Unstructured formatting is shared with Subject; the numeric parser retains
the source runtime's trailing-NUL compatibility behavior. Cache-dependent
setter no-ops are replaced by deterministic canonical header replacement.
Content-MD5 metadata follows `MimePart.ContentMd5` and its generic header-value
handling, not checksum computation or verification. Tolerant decoded reads and
checked unstructured formatting preserve arbitrary caller-supplied text, while
canonical assignment replaces the source's cache-dependent no-op. Formatting
and atomic replacement are shared with Content-Description.
Part-level encoding recommendation follows `MimePart.GetBestEncoding`, reusing
the existing analyzer through a bounded streaming writer over a discard sink.
It retains decoded text/message analysis and the non-text media-type shortcut,
but validates line limits before any source access, including shortcut cases.
Missing raw Content-Type is an explicit error rather than an implicit default.
Leaf preparation follows `MimePart.Prepare`, preserving safe-encoding shortcuts,
the unspecified/SevenBit no-op, and equal-encoding header preservation. Selection
succeeds before any header mutation. Explicit serialization options replace the
source's private encoder-width state; callers use the same options for preparation
and writing. Invalid first transfer encodings and invalid options produce errors
instead of implicit fallback; preparation is not whole-message validation.
Single-part writing follows `MimePart.WriteContent` and `MimeWriter.cs`, reusing
the existing transfer codecs, newline filters, and normalized header writer.
Stored/requested encoding comparisons, binary pass-through, UU wrappers, and
empty-body newline handling are retained. Options and required metadata errors
are explicit and UU filename controls are rejected. Authored records are
terminated; parsed EOF records, separator absence, and no-content delimiter
ownership are retained until edits require new framing. Signature verification
and configurable message-level formatting are not implemented. Writing does not
prepare implicitly. Explicit tree preparation adapts `Multipart.Prepare`,
`MessagePart.Prepare`, and `MimeMessage.Prepare` using a depth-proportional
iterator stack instead of recursive calls. It retains pre-descent MIME-Version
insertion and traversal-order partial changes on failure. Invalid versions
produce typed errors rather than being treated as absent. Container metadata
and framing remain unchanged, as in the source.
Text parts adapt `TextPart.cs`'s subtype construction, charset detection,
automatic and explicit text access, and text/charset replacement.
Composition over `MimePart<R>` replaces
inheritance, while in-memory-only setters respect Rust source types. Existing
strict charset conversion and checked header formatting make failed edits
atomic. Automatic decoding ports MIME/BOM/HTML precedence and recovery;
explicit overrides remain strict. Detection uses the shared HTML tokenizer and
preserves the source's prescan comments. Newlines/BOMs remain unchanged, codec
replacement uses U+FFFD, and trailing incomplete sequences are finalized.
`TextEncodingConfidence` represents Certain/Tentative; `Option` replaces
Undefined and the always-present stream model has no null-content Irrelevant
state. Text-format conversion remains separate, as does MIME previewing.
Format-based construction maps the supported `TextFormat` variants to the
source's text subtypes and flowed parameter. Classification and predicates
reuse body-selection matching, retaining the unknown-text Plain fallback while
using MimeKit's case-insensitive subtype comparison. Enriched
`text/richtext` stays distinct from RTF. The text wrapper's existing `text/*`
validation remains stricter than MimeKit's support for `application/rtf`.
Multipart ownership adapts `Multipart.cs` and `MimeEntity.cs` using an enum and
owned child vectors instead of inheritance and parent/event machinery.
Raw headers remain authoritative; checked edits implement the source's boundary
validation TODO using RFC 2046 section 5.1.1. The default constructor ports
`Multipart.GenerateBoundary` using the existing OS-randomness dependency and
shared unwrapped Base64 encoder; `with_boundary` accepts caller-supplied values.
Raw preamble/epilogue bytes avoid implicit string
decoding, and iterative leaf traversal avoids recursive enumeration. Private
parser provenance preserves original delimiters and truncation until structural
or framing-metadata edits require regeneration.
Text setters adapt `Multipart.FoldPreambleOrEpilogue` and its upstream folding
and setter-side-effect tests. UTF-8 byte widths replace UTF-16 character counts;
explicit strict charset getters replace implicit detection and fallback.
Epilogue text neither adds nor strips the closing delimiter's separately stored
newline. Byte-identical folded assignments preserve parsed framing without
depending on cached decoded strings.
Multipart writing follows `MimeWriter.cs`'s explicit frames and
`MimeEntity.EndsWithDelimiterNewLine` ownership rules, including truncated and
empty-body cases. Rust retains an empty stream plus private no-content metadata
instead of nullable content. Epilogues exclude the closing marker's line ending,
which is stored with that marker unless owned by the enclosing delimiter.
Authored invalid container encodings and overlapping ancestor boundaries
produce explicit errors. Unchanged recovered metadata follows reader semantics.
Existing leaf serializers and newline filters are reused; body collision
scanning and signature-specific canonicalization remain deferred.
Entity loading uses a shared event reader and owned-model parser, adapting
`MimeReader.cs`'s iterative traversal, line-anchored boundary recognition,
content callbacks, and withheld delimiter-newline semantics, plus
`MimeParser.cs`'s construction stack, raw headers, and stored-encoding content.
Rust callbacks receive owned headers and borrowed content chunks with original
byte/line positions. Mbox iteration adapts the source's five-byte marker
recognition, initial-line skipping, envelope events, and outer-boundary newline
ownership. Explicit method selection replaces a format property, and limits
reset per message. Truncated multipart frames unwind; opt-in Content-Length
handling adapts the outer-header length parsing and `CheckBoundary` /
`IsPartialBoundary` offset checks, including `MessagePartBegin` lookahead.
Borrowed buffered content and small withheld-newline storage adapt
`ScanContent` without copying every ordinary line; ambiguous padded
delimiter candidates still require buffering under the input-size limit.
`MimeContentStorage::Memory` / `Source` follows MimeKit's enum-based storage
contract. In-memory cursors implement copied bodies; bounded shared streams
implement source-backed bodies. Rust ownership replaces caller-managed stream
lifetime, and independent synchronized cursors adapt the parser-position
restoration in `MimeParser.Initialize` without discarding buffered read-ahead.
Preamble endings are retained; nested epilogues exclude parent framing and
start after their own closing marker's newline. Recovery preserves malformed
metadata and truncated containers while resource/source failures remain fatal.
Empty content always has a stream; private framing metadata distinguishes its
delimiter-newline ownership and retains raw markers. LF serialization adds a CRLF delimiter prefix
after a bare body/epilogue CR to avoid losing that byte on reload.
Message ownership and I/O adapt `MimeMessage.cs`'s separate message/body header
lists, optional body, and combined header output, plus `Header.cs`'s
case-insensitive Content-* classification. Existing entity I/O is reused
without copying payloads during header partitioning. Explicit constructors
replace clock/random-ID defaults, and serialization does not insert
MIME-Version or hide Bcc. Parsed stream byte offsets are retained during
partitioning and used in the source's original-order merge,
shared by ordinary and masked writers. Other typed conveniences, change
events and automatic preparation are deferred.
Message importance and priority adapt `MessageImportance.cs`,
`MessagePriority.cs`, `XMessagePriority.cs`, the corresponding `MimeMessage.cs`
properties, and `UnitTests/MimeMessageTests.cs`. `Utils/ParseUtils.cs` supplies
X-Priority's checked 32-bit numeric-prefix rules; the shared MIME whitespace
helper is reused. Overflow and absent-Normal no-op comments are preserved or
adapted inline. Raw-authoritative getters replace lazy caches and change events.
Canonical setters compare newly parsed values instead of stale cached defaults;
equal values preserve raw records only without duplicates, and explicit edits
repair invalid metadata.
Missing headers still mean Normal, but present invalid values return errors
rather than silently selecting Normal. Priority fields remain independent.
Attached-message ownership adapts `MessagePart.cs` with optional owned messages
and an internal box instead of inheritance and disposal tracking.
`MimeWriter.cs`'s iterative descent into message bodies is integrated with the
existing tree writer, retaining active multipart boundaries across wrappers.
Checked raw metadata and transparent wrapper encodings prevent writing an
unencoded message under an encoded-transfer header. Leaf traversal includes
attached bodies. Automatic loading adapts `MimeReader.cs`'s recognized message
subtypes, transparent-wrapper classification, and absent-message handling.
A shared iterative stack retains multipart ancestors through message frames;
header partitioning reuses the root-message loader's helper. Delimiter
lookahead keeps delimiter framing out of embedded header separators. Explicit
errors replace depth-limit fallback; incomplete messages recover by default.
Returned-header containers adapt `TextRfc822Headers.cs`,
`UnitTests/TextRfc822HeadersTests.cs`, and the text/rfc822-headers and
message/global-headers classification in `ParserOptions.cs` and `MimeReader.cs`.
Composition replaces inheritance and visitor dispatch while reusing
`MimeWriter.cs`'s ordinary message-body traversal: existing bodies are not
discarded. The text media-type exception is explained inline and shares
validation between metadata edits and writing. Transparent parsing uses the
same bounded message frames; encoded wrappers remain opaque. Present messages
recover missing inner separators at EOF and enclosing boundaries by default.
Partial-message metadata and joining adapt `MessagePartial.cs`,
`UnitTests/MessagePartialTests.cs`, and the RFC 2046 fixtures in
`UnitTests/TestData/partial`. Number-validation, stream-chaining, and RFC header
merging comments are preserved/adapted. Composition and decoded borrowed
readers replace inheritance and disposable chained streams. Explicit errors
distinguish malformed numbers from missing parameters, and missing IDs are
rejected. Shared parser limits bound the reassembled decoded message; no
intermediate concatenated message buffer is required. As in MimeKit, combined
message and root-body offsets are cleared without changing descendant offsets.
Visitor dispatch and disposal comments do not apply to the Rust model.
Automatic splitting adapts the same source's `Split` and `CloneMessage`,
including its whole-line preference comment and eleven-fragment photo test.
The consuming in-memory API preserves small-message storage and cursor positions,
uses configured-domain IDs, and enforces the fragment size even when LF lies
just outside the boundary. Root-only header filtering retains Content-* extensions
and attached-message headers. Writer-error coverage replaces the source's
throwing-stream test because the approved API uses owned byte buffers.
`MimeReader.cs`'s multipart/digest default is scoped to the immediate parent
and resets at embedded message roots. Unlike cached effective metadata, the
raw API keeps missing headers absent; the writer uses its parent frame to
allow implicit message/rfc822 children only inside a digest.
Body-part and attachment enumeration adapt `MimeMessage.cs`'s
`EnumerateMimeParts`, `BodyParts`, and `Attachments`: skip multipart nodes and
stop at attached-message wrappers. Shared iterative traversal replaces
`MimeIterator` for these views while preserving recursive-message leaf
enumeration. Raw Content-Disposition parsing and explicit errors replace
cached attachment flags; malformed fields are not silently treated as matches.
Text-body selection adapts `MimeMessage.cs`, `Multipart.cs`,
`MultipartAlternative.cs`, `MultipartRelated.cs`, and `TextPart.cs`, with cases
from `UnitTests/MultipartAlternativeTests.cs` and
`UnitTests/MultipartRelatedTests.cs`. Reverse alternative preference,
related-root selection, format matching, and mixed stopping rules are retained.
Borrowed part access and an iterative path search replace recursive selection
and automatic string getters. Explicit errors replace silently ignored invalid
metadata; related IDs are compared directly rather than through .NET URI
canonicalization.
Decoded body-text access adapts `MimeMessage.GetTextBody` and
`MultipartAlternative.GetText`, sharing `TextPart`'s automatic charset
detection and recovery. `Text/FlowedToText.cs` and
`UnitTests/Text/FlowedToTextTests.cs` provide the paragraph, stuffing,
quote-depth, delsp, and final-soft-line rules. Deterministic CRLF/LF options
replace platform-dependent writer newlines; whole-string conversion omits
the C# converter hierarchy, stream adapters, and header/footer decoration.
The source's signature-separator handling is preserved and documented.
Verbatim plain-text conversion follows `Text/TextToText.cs` and
`UnitTests/Text/TextToTextTests.cs`. Optional owned headers and footers replace
nullable C# properties; copying whole strings retains all characters and
decoration boundaries without the source's buffered reader/writer loop.
This standalone converter does not add decoration to the flowed converters.
Flowed-text generation follows `Text/TextToFlowed.cs` and
`UnitTests/Text/TextToFlowedTests.cs`, retaining word-boundary preference,
space-stuffing, deep-quote limits, and the source's explanatory inline comments.
UTF-8 byte counts and character-safe splitting replace UTF-16 slicing; a cached
word boundary avoids repeatedly scanning long unbroken words. Both flowed
converters share logical-line scanning and deterministic CRLF/LF output.
Trailing-space trimming, signature handling, quote normalization, and the
absence of stream adapters and header/footer decoration are explicit.
Plain-text previews adapt the string overload in `Text/PlainTextPreviewer.cs`,
the default and checked length range in `Text/TextPreviewer.cs`, and
`UnitTests/Text/PlainTextPreviewerTests.cs`. Whitespace classification and the
trim-before-ellipsis ordering are retained; Unicode scalar counts replace
UTF-16 code-unit counts. Strict UTF-8 reader overloads share these normalization
rules. `text::TextPreviewer` additionally adapts the MIME-part overload in
`Text/TextPreviewer.cs`, BOM selection from `Utils/CharsetUtils.cs`, and all
applicable `UnitTests/Text/TextPreviewerTests.cs` cases. It preserves the
16-KiB transfer-decoded cap and charset fallback, with explicit differences
for replacement policy and preview-reader lookahead.
HTML previews adapt `Text/HtmlTextPreviewer.cs` and
`UnitTests/Text/HtmlTextPreviewerTests.cs`, including their document fixtures
and constant-time open-element regression workloads. The reader contract is
strict UTF-8 with tokenizer read-ahead; length limits count Unicode scalars.
HTML encoding adapts `Text/HtmlUtils.cs` and
`UnitTests/Text/HtmlUtilsTests.cs`, preserving the unchanged-prefix fast path,
quote-specific escaping, control handling, and decimal-reference boundaries.
The shared numeric writer retains the source's stack-buffer layout and inline
comments without allocating a temporary string per Unicode character.
Typed attribute quotes replace invalid-character errors, and UTF-8 scalar
iteration replaces surrogate-pair handling. Name validation uses the source's
delimiter checks for the in-memory writer; stream overloads remain deferred.
The in-memory `HtmlWriter` adapts `Text/HtmlWriter.cs`,
`Text/HtmlWriterState.cs`, and `UnitTests/Text/HtmlWriterTests.cs`.
Checked operations and an owned string replace exceptions and external writer
lifetime management; `finish()` consumes the writer after flushing a pending
start tag. Applicable inline comments are retained. Name-based operations reject
unchecked names; attribute-object writes preserve tokenizer-originated names
using the source's previous-valueless-attribute workaround for leading `=`.
`HtmlToHtml` adapts `Text/HtmlToHtml.cs`, `HtmlNoScriptHandling.cs` and
`HtmlTagContextStack.cs`, with the shared context extended to retain source names
and end attributes. Indexed per-name flag vectors replace linked context objects;
the writer drains UTF-8 buffers without resetting pending tag state.
HTML identifiers and attributes adapt `Text/HtmlTagId.cs`,
`Text/HtmlAttributeId.cs`, `Text/HtmlAttribute.cs`, and
`Text/HtmlAttributeCollection.cs`, with regression coverage from their
corresponding `UnitTests/Text` tests. Sorted static tables replace runtime
dictionaries; immutable names allow identifiers to be resolved at construction
instead of through a lazy sentinel cache. Ordered vectors, `Option` lookups,
and checked constructors replace nullable lists, out parameters, and exceptions.
Applicable comments are retained or adapted; ASCII name matching deliberately
avoids .NET-specific Unicode ordinal folding.
HTML tokens adapt `Text/HtmlToken.cs`, `Text/HtmlTokenKind.cs`, and the
`HtmlWriter.WriteToken` path, with fixtures from `UnitTests/Text/HtmlTokenTests.cs`,
`HtmlTokenizerTests.cs`, and `HtmlWriterTests.cs`. An owned enum replaces
inheritance, and explicit data constructors replace internal encoding/context
flags. Applicable comments preserve the bogus-comment slash restoration,
trailing-less-than protection, and DOCTYPE name/quote rationale. Checked
DOCTYPE setters retain keyword transitions and reject unrepresentable values
atomically. String output replaces `TextWriter`; tokenization is a separate
engine and reusable token objects are not included.
The structural token controller ports `Text/HtmlTreeConstruction.cs`,
`Text/HtmlOpenElementStack.cs`, and `Text/HtmlNamespace.cs`, with all direct
upstream stack/controller regressions and selected structural sequences from
`HtmlTokenizerTests.cs`. A public token-consumer API replaces the internal
helper; checked positive limits, terminal post-overflow behavior, `Option`
lookups, ASCII matching, and amortized vector growth are explicit adaptations.
Applicable namespace, scope, form-pointer, folding, and adoption-agency
comments are retained.
HTML character scanning ports `Text/HtmlTokenizer.cs` and its lexical states,
using `UnitTests/Text/HtmlTokenizerTests.cs` and the nine non-charset
`UnitTests/TestData/html` document/token/output fixtures. A strict-UTF-8
feed/poll engine replaces scanner-owned `TextReader` I/O, with a thin `BufRead`
adapter sharing all lexical state. Explicit needs-input/EOF outcomes preserve
partial tokens across arbitrary byte partitions and recoverable reader errors.
Applicable raw-end-tag reconsumption, reference, declaration, DOCTYPE, and
depth-overflow comments are retained; charset/BOM detection and compliance
reporting are not part of this port.
RTF tokenization ports `Text/RtfTokenizer.cs` and `Text/RtfKeyword.cs`, with
canonical tokens, malformed recovery, and lexical positions from
`UnitTests/Text/RtfTokenizerTests.cs` and `RtfInterpreterComplianceTests.cs`.
`BufRead` and owned token payloads replace `TextReader` and reusable character
buffers. Reading original bytes preserves raw high bytes and makes `\binN`
lengths unambiguous without a Latin-1 decoding layer. A resumable state machine
preserves source errors for retry; fixed lexical bounds, binary skipping,
case-sensitive aliases, and applicable inline rationale are retained.
`RtfTokenizerOptions` and capped callback delivery adapt the lexical subset of
`Text/RtfComplianceOptions.cs` and `Text/RtfCappedComplianceLogger.cs`.
Per-violation counters and the first-suppressed-issue marker preserve the
upstream rationale, with coverage from `RtfCappedComplianceLoggerTests.cs`
and `RtfConverterComplianceTests.cs`. Copied constructor options and stream-local
counters replace global defaults and the converter's logger wrapper; calls
without a callback do not spend the reporting budget.
`RtfValidator` ports the initial-header, group-balance, and trailing-content
checks from `Text/RtfInterpreter.cs`, with structural fixtures and issue ordering
from `UnitTests/Text/RtfInterpreterComplianceTests.cs`. A token-preserving
`BufRead` adapter exposes these checks independently of the rendering
interpreter. The tokenizer and validator share the adapted capped-reporting
helper; the validator applies one budget layer to the combined report.
Applicable comments retain the header grammar, harmless-padding rule, lenient
brace recovery, and first-root closure behavior. Structural state advances even
on callback-free reads so enabling reporting does not recheck a consumed header.
`RtfToText` ports `Text/RtfToText.cs` and the plain-text interpretation paths in
`Text/RtfInterpreter.cs`, with fixtures from `RtfToTextTests.cs` and
`RtfInterpreterTests.cs`. Applicable inline comments preserve folded-state
limits, destination skipping, font-table boundaries, decoder-cache bounds,
Unicode fallback/surrogate handling, delayed table separators, and placeholder
spacing. Original byte input and the existing incremental charset filter replace
the Latin-1 reader and platform encodings. Color storage and hyperlink collection
are disabled for plain-text conversion; charset fallback and decode errors have
explicit diagnostics.
`RtfToHtml` ports the rendering handler from `Text/RtfToHtml.cs`, sharing the
interpreter's bounded color table, field collector, and group-scoped hyperlink
state. Fixtures come from `UnitTests/Text/RtfToHtmlTests.cs`; additional cases
cover UTF-8 field limits, decoder boundaries, state bounds, and reporting quotas.
Applicable comments preserve automatic-color handling, CSS formatting/cache
bounds, empty paragraphs, whitespace, table recovery, field truncation, and link
nesting. Reference-counted targets replace object identity without comparing
long strings during state folding; the field cap uses UTF-8 bytes instead of
UTF-16 units. Existing `HtmlWriter` and decoration helpers replace the C# writer
plumbing. `HtmlTagContext` and generated-tag callback handling adapt
`Text/HtmlTagContext.cs`, `Text/HtmlTagCallback.cs`, and the rendering callbacks
in `Text/RtfToHtml.cs`. Closures are supplied per conversion rather than stored
in converter options. Read-only source attributes, explicit end-tag opt-in,
suppression after popping a context, and placeholder numbering follow the source.
Comments explain why suppressed contexts stay tracked and why custom markup
does not affect generated nesting.
`RtfHtmlExtractor` adapts raw de-encapsulation from `Text/RtfInterpreter.cs`
and the extraction-mode handler in `Text/RtfToHtml.cs`, with canonical cases
from `UnitTests/Text/RtfInterpreterTests.cs`. Applicable comments retain the
recognition window, HTMLTAG document-code-page rule, HTMLRTF state tracking,
CRLF conversion, and soft-hyphen exception. A separate `Option<String>` result
distinguishes non-HTML RTF from empty HTML without changing the rendering APIs.
Unlike MimeKit's full `RtfToHtml` extraction pipeline, this helper does not
tokenize/filter HTML or provide a streaming extracted-character reader.
RTF hyperlink-field parsing ports `RtfInterpreter.ParseHyperlink` and its
canonical fixtures in `UnitTests/Text/RtfInterpreterTests.cs`. Borrowed UTF-8
slices replace `StringBuilder` and substring allocations. The switch/argument
and control-character rationale, allowed schemes, and malformed-quote recovery
are preserved; keyword and scheme folding deliberately use ASCII matching.
This standalone helper does not implement the interpreter's field collector.
Character-reference decoding follows `Text/HtmlEntityDecoder.cs`,
`Text/HtmlEntityDecoder.g.cs`, and `HtmlUtils.HtmlDecode`. The standalone Rust
generator adapts HtmlKit's `CodeGenerator/Program.cs` and bundles its
`HtmlEntities.json` input. The generated named table is cross-checked against
MimeKit's `UnitTests/TestData/html/HtmlEntities.json`;
`UnitTests/Text/HtmlEntityDecoderTests.cs` supplies numeric recovery cases.
Bounded longest-prefix searches replace the named-entity state machine, while
numeric references consume unlimited digits with saturating arithmetic.
Applicable recovery and compatibility comments are retained. `HtmlEntityDecoder`
ports character-at-a-time consumption and reset semantics with bounded retained
input; its attribute accessor and `html_attribute_decode` also apply MimeKit's
historical attribute exception, including lookahead inside a pushed suffix.
`UnitTests/Text/HtmlTokenizerTests.cs` supplies the attribute-reference fixture.
Opt-in reference diagnostics port `HtmlEntityDecoder.GetComplianceViolations`
and `HtmlTokenizer.ReadAmbiguousAmpersand`, using the character-reference subset
of `HtmlComplianceViolation`. `HtmlComplianceOptions` and
`HtmlCappedComplianceLogger` supply the per-kind budget and single-marker
semantics. `HtmlTokenizerComplianceTests` and `HtmlCappedComplianceLoggerTests`
supply recovery, ordering, and reporting-limit cases. Explicit Rust results
replace callbacks, and source locations use UTF-8 byte offsets rather than
UTF-16 code units. General HTML compliance logging and tokenization remain
deferred.
Plain-text HTML rendering adapts `Text/TextToHtml.cs`, `Text/UrlScanner.cs`,
the fixed patterns in `Text/TextConverter.cs`, and
`Text/HeaderFooterFormat.cs`, with cases from `UnitTests/Text/TextToHtmlTests.cs`
and `UnitTests/Text/UrlScannerTests.cs`. Applicable scanner rationale and
compatibility comments are retained. Fixed, ordered pattern tables replace
runtime registration; bounded UTF-8 scanning replaces UTF-16 indexing.
The `unicode-general-category` dependency supplies the letter and format
classifications used by the scanner; existing IDNA helpers validate A-label
top-level domains. HTML encoding reuses the existing append helper, and
plain-text quote-prefix parsing is shared with
`TextToFlowed`. Document/fragment output and independent text/HTML decorations
are retained without stream APIs. Generated-tag callbacks share `HtmlTagContext`
and `HtmlWriter` with RTF rendering, with per-conversion Rust closures replacing
stored C# delegates. A shared renderer handles plain/flowed link callbacks,
quotation contexts, end-tag decisions, and content suppression. Applicable
comments and canonical regressions cover deleted quote endings at EOF and
suppressed nested quotations. Absolute input quote depths let the Rust renderer
avoid storing contexts for hidden descendants.
Flowed HTML rendering adapts `Text/FlowedToHtml.cs` and
`UnitTests/Text/FlowedToHtmlTests.cs`, including applicable inline comments.
It shares HTML link/decoration/quotation output with `TextToHtml` and
space-unstuffing with `FlowedToText`, but retains its own paragraph state machine:
the source converters differ when a pending flowed paragraph is empty.
Paragraph markup newlines default to CRLF rather than the platform newline,
with the same checked `NewLineFormat` constructor pattern as `FlowedToText`.
Enriched HTML rendering adapts `Text/EnrichedToHtml.cs` and
`UnitTests/Text/EnrichedToHtmlTests.cs`, preserving the applicable scalar-buffer
comment and the name-indexed stack rationale from `Text/HtmlTagContextStack.cs`.
An iterative command stack, shared immutable formatting strings, and a bounded
text buffer replace the C# reader/writer machinery. The existing HTML encoders
escape text and attributes. All 79 upstream formatting cases, the RFC example,
invalid parameters, exact resource thresholds, Unicode/buffer boundaries, and
large/malformed command sequences have direct regressions. Stream-only tests
are outside this whole-string milestone.
Delivery-status groups adapt `MessageDeliveryStatus.cs`,
`MimeParser.ParseStatusGroup`, `MimeReader.SkipBlankLines`, and
`UnitTests/MessageDeliveryStatusTests.cs`; `ParserOptions.cs` supplies the
global-delivery-status classification. Detached `HeaderList` values and explicit
atomic replacement replace mutable cached collections and event-driven writes.
Resource limits and explicit errors replace swallowed parse failures, while
retained inner encoding declarations cause re-encoding on replacement rather
than stale metadata over plain recipient groups.
The reader/parser share a bounded header scanner for standalone blocks and
incremental status groups. Empty blocks implement the source's blank-line
skipping and trailing-CR behavior without a second lexical parser. Cumulative
positions and budgets survive between groups; switching to a separately
decoded remainder starts a new source origin.
Disposition-notification fields adapt `MessageDispositionNotification.cs`,
`HeaderList.cs`, and `UnitTests/MessageDispositionNotificiationTests.cs`
(source spelling), with global-subtype classification from `ParserOptions.cs`.
Detached raw fields and explicit owned-body replacement replace cached mutable
fields and change events. Reads share bounded decoding with delivery-status
parts, but parse only one header block and never interpret embedded encoding
fields. Serialization explicitly terminates the block, rather than retaining
the C# header list's separator flag.
Feedback-report fields adapt `MessageFeedbackReport.cs`,
`UnitTests/MessageFeedbackReportTests.cs`, and its `feedback-report.txt` data.
The bounded single-block parser and atomic serializer are shared with
disposition notifications. Explicit detached edits replace change events;
the source's event-handler non-null invariant does not apply to Rust ownership.
Serialization and separator handling are explained inline, while repeated
Reported-Uri fields remain independently accessible.
Common body assembly adapts `BodyBuilder.cs` and `UnitTests/BodyBuilderTests.cs`,
including configurable body encoding and the single-attachment shortcut.
Owned vectors and consuming construction replace shared mutable attachment
collections. Existing text and multipart helpers supply strict encoding,
generated boundaries, and related-root metadata; upstream default, charset,
and attachment cases accompany combination and serialization regressions.
Standalone attachment helpers adapt `AttachmentCollection.cs` and its synchronous
`UnitTests/AttachmentCollectionTests.cs` cases, using the original `girl.jpg` and
`lorem-ipsum.txt` fixtures in `tests/data/attachments` and existing message fixtures.
They preserve the applicable charset-detection TODO and filename, disposition,
encoding-selection, and embedded-message behavior. Ordinary vectors replace
collection wrappers; malformed inferred messages intentionally return errors,
with explicit binary attachment as the opt-in alternative. Bounded nonseekable
reads, escaped resource locations, and source-lifetime regressions accompany
the upstream byte/reader/file cases.
Alternative multipart bodies adapt `MultipartAlternative.cs` and
`UnitTests/MultipartAlternativeTests.cs`. Composition replaces inheritance;
one format-taking read method covers the source's plain/HTML properties and
`GetTextBody`, while borrowed selection replaces `TryGetValue`. The shared
iterative selector retains last-to-first ordering and nested related/mixed
rules, with applicable source comments and flowed/nesting tests. Decoding
reuses the existing charset and flowed helpers without discarding parsed
framing. Typed construction and enum conversion replace dynamic constructor
arguments and visitor dispatch.
Multipart reports adapt `MultipartReport.cs`, `Parameter.Value`, and
`UnitTests/MultipartReportTests.cs`. Composition and conversion to the ordinary
multipart variant replace inheritance and visitor dispatch; explicit boundaries
replace generated ones. Raw-header reads and atomic edits replace cached
parameters and change events. Exact-before-trim and equal-after-trim no-ops
are retained and explained inline, without adding child-layout validation.
Related multipart root management adapts `MultipartRelated.cs` and
`UnitTests/MultipartRelatedTests.cs`. Root reads share the existing text-body
selection helper; root replacement distinguishes an actual selector match
from the getter's first-child fallback. The source's Thunderbird bug 471402
comment is retained/adapted where first-child roots remove start. Missing
Content-Ids can be generated using the configured default domain; atomic checked
metadata edits replace cached parameter updates, and replaced children are
returned to the caller.
Content-Id resource lookup adapts the CID branch of `IndexOf` and the
`TestReferenceByContentId` regression using typed IDs and shared matching logic.
Decoded opening adapts `Open` and `TestOpenNonMimePartThrows`, with borrowed
incremental readers replacing disposable streams. Internal read-only-content
access preserves parsed framing rather than triggering editing-accessor invalidation.
URI lookup adapts the remaining `IndexOf` cases with `fluent-uri` resolution
and literal comparison instead of .NET URI equality. Contains helpers, direct
preferred-body lookup, and metadata-returning opening adapt `Contains`,
`TryGetValue`, and the metadata `Open` overload. Tests include the upstream
MHTML fixture, generated root IDs, lookup/base cases, and media-type/charset
outputs. Visitor dispatch, dynamic constructor arguments, and disposal invariants
are represented by enum dispatch, typed construction, and ownership rather than
additional runtime APIs.
Bounded stream views follow `IO/BoundedStream.cs`, with range and seek cases
adapted from `UnitTests/IO/BoundedStreamTests.cs`. Relative positions, source
cursor resynchronization, and whole-request write boundary checks are retained,
including the source's explanatory comments. Rust trait bounds require seeking,
`Option<u64>` replaces the negative unbounded sentinel, and borrowed ownership
replaces `leaveOpen`. End-relative seeking consistently uses the view's end;
EOF-derived length caching is omitted so source growth remains observable.
Generic resizing and nonseekable fallback behavior are not ported.
Chained streams follow `IO/ChainedStream.cs` and
`UnitTests/IO/ChainedStreamTests.cs`, including cross-child writes, combined
seeking, and current-child flushing. Source seek explanations are retained
while child repositioning is lazy and logical state commits only on success.
Zero-based children and explicit premature-EOF errors keep range offsets
consistent; one-child transfers preserve Rust's partial-I/O guarantees.
Bounded and chained streams share checked seek and length-query helpers.
Block-backed memory storage follows `IO/MemoryBlockStream.cs` and
`UnitTests/IO/MemoryBlockStreamTests.cs`, retaining block sizing, bounded seeking,
resize cursor clamping, and explanatory shrink/seek comments. Owned,
zero-initialized allocations replace pooled buffers; reservation errors are
explicit, empty construction is lazy, and read/write/resize cases are also
compared against a contiguous model.
Measuring sinks follow `IO/MeasuringStream.cs` and
`UnitTests/IO/MeasuringStreamTests.cs`, preserving logical-extent measurement,
bounded seeking, resize clamping, and the source's seek sanity-check comment.
Checked `u64` arithmetic replaces signed counters; unsupported reads are
excluded by traits. Header and transfer-encoding measurements are compared with
the corresponding serialized bytes.
Raw header parsing follows `Header.cs`, the current `MimeReader.cs` header
scanner, and `ParserOptions.MaxHeaderLength`, with regression cases adapted from
`UnitTests/HeaderListTests.cs` and `UnitTests/MimeParserTests.cs`. It retains raw
bytes like the stream parser rather than adding the newline that MimeKit's
standalone `Header.TryParse` API inserts for unterminated input.
Standalone header parsing and display decoding follow `Header.cs`'s public
`Parse`/`TryParse`, `Value`, `GetValue`, and `Unfold` paths, with vectors from
`UnitTests/HeaderTests.cs`. Parsing adds a missing terminal CRLF without
changing the streaming scanner's behavior. Byte-offset errors and explicit
RFC 2047 recovery diagnostics replace boolean/out-parameter parsing and
string-only display results. The decoded string buffer is unfolded in place;
the borrowed-string utility avoids allocation when trimming alone suffices.
Ordered header collections follow the duplicate lookup, insertion, removal,
positional replacement, and duplicate-collapsing replacement rules in
`HeaderList.cs` and `UnitTests/HeaderListTests.cs`. A case-insensitive dictionary
indexes first occurrences, while owned records and direct index maintenance
replace the change-event machinery. Raw records remain immutable;
malformed records have no queryable field name and are edited by position.
Synchronous loading uses the existing header scanner with `BufRead` consumption
to retain body read-ahead. Writing follows `HeaderList.WriteTo` and
`WriteRawField`, including their explanatory CR-only-invalid-line comment and
round-trip regression vectors. Existing newline filters and `FilteredWriter`
handle conversion and short writes. The Rust collection does not own or emit
the body separator; parser metadata and explicit caller writes replace that
source behavior. Unstructured decoded-value authoring and async I/O remain
unported.
Typed integration adapts `MimeMessage.AddAddresses`' raw-value parsing and
`MimeEntity`'s content-header parsing, serialization, and replacement paths.
Explicit parser results replace internal silent failures/defaults. Owned typed
snapshots and checked `to_header` conversions replace mutable cached values and
change-event synchronization; existing collection edits handle replacement only
after encoding succeeds. Raw-value offsets and diagnostics are retained rather
than decoding/unfolding structured syntax first.
Header identifiers follow the complete `HeaderId.cs` mapping, with round-trip
coverage adapted from `UnitTests/HeaderTests.cs`. A sorted static lookup table
replaces the C# dictionary without allocating or changing raw header names.
Transfer-encoding and version values follow `Utils/MimeUtils.cs`, with canonical
encoding tokens from `MimePart.cs` and comment/number rules from
`Utils/ParseUtils.cs`. Version vectors are adapted from
`UnitTests/Utils/MimeUtilsTests.cs`; failures use Rust `Result` values instead
of C# boolean/out-parameter APIs.
Message identifiers follow `Utils/ParseUtils.cs` (`TryParseMsgId` and
`TryParseCompleteMsgId`). The separate assignment API follows
`TryParseMsgIdValue` and `ContainsUnquotedWhiteSpace` in the same file.
Compatibility vectors are adapted from `UnitTests/Utils/ParseUtilsTests.cs`,
`UnitTests/Utils/MimeUtilsTests.cs`, and `UnitTests/MimeMessageTests.cs`.
Reference scanning follows `Utils/MimeUtils.cs` and its `SkipWord` behavior from
`Utils/ParseUtils.cs`, with explicit Rust error items added to tolerant recovery.
Address specifications follow the current `AddressParser.cs` local-part,
quoted-string, contiguous-token, and addr-spec grammar, with component and domain
conversion behavior adapted from `MailboxAddress.cs`. Vectors come from
`UnitTests/MailboxAddressTests.cs`, `InternetAddressTests.cs`, and
`InternetAddressListTests.cs`. Rust byte offsets replace UTF-16 indices, and
Latin-1 fallback is explicitly reported. Complete-value and prefix parsing are
separate; full mailbox parsing is not implied by the `AddrSpec` type.
Single mailboxes follow `AddressParser.cs` phrase, angle-mailbox, bare-mailbox,
and local-mailbox productions, plus name preprocessing in `AddressParserInput.cs`.
Vectors come from `UnitTests/MailboxAddressTests.cs` and the escaped-at case in
`InternetAddressListTests.cs`. The Rust model exposes a replaceable typed
`AddrSpec` and editable `DomainList`, retains charset recovery diagnostics rather than silently
guessing, and shares the existing raw-byte unquoting and RFC 2047 decoders.
Syntax offsets are original-input byte offsets; name decoding diagnostics use
the preprocessed name buffer.
Groups and address lists follow the group/list productions in `AddressParser.cs`,
with models and traversal adapted from `GroupAddress.cs` and
`InternetAddressList.cs`. Vectors come from `UnitTests/GroupAddressTests.cs` and
`InternetAddressListTests.cs`. Explicit stacks replace recursive parsing and
traversal; the depth option is bounded to 255 rather than an unrestricted integer.
Malformed members return errors instead of internal TryParse-style dropping or
empty-group recovery. Malformed-header resynchronization remains deferred.
Construction and editing adapt `MailboxAddress.cs`, `GroupAddress.cs`, and
`InternetAddress.Name`, including Unicode name trimming. Typed addr-spec
assignment replaces string reparsing; ordinary Rust mutable borrows replace
change events. Replacing a name explicitly clears only its decoding diagnostics.
Equality follows `MailboxAddress.Equals`, `GroupAddress.Equals`, and
`InternetAddressList.Equals`: routes and decoding diagnostics are excluded,
while names/address text, variants, ordered members, and group boundaries are
retained. Rust hashing uses those same equality fields rather than display text.
Display formatting follows these models' and `InternetAddressList.cs`'s
`ToString(..., false)` paths, reusing quote and route formatting helpers.
An explicit stack replaces recursive group formatting; raw group names and
controls remain display text, not transport-safe output.
ASCII value encoding adapts these models' `Encode` paths, reusing RFC 2047
phrase encoding and checked IDNA/route formatting. Local-only angle brackets
and quoted empty group names retain the source rationale. Explicit stacks and
member-index error paths replace recursive encoding; strict addr-spec checks
and international-local-part rejection prevent malformed or non-ASCII output.
Complete address headers additionally adapt the source's whitespace folding,
using a syntax-aware scan of our encoded output to preserve quoted whitespace,
routes, and addr-specs. Physical limits are hard bounds rather than soft
targets; name words are bounded to fit, while indivisible oversized pieces
fail explicitly. Field validation, LF/CRLF selection, and retained member-error
paths complete the ASCII path. International output also adapts
`InternetAddress.EncodeInternationalizedPhrase` and the international mailbox,
group, and route branches. Raw output retains checked address/domain spelling;
whole-name encoding preserves controls, literal encoded-word lookalikes, and
oversized words or quoted names without introducing decoded whitespace.
Domain routes follow `DomainList.cs` and its `Utils/ParseUtils.cs` domain helpers,
with vectors from `UnitTests/DomainListTests.cs`. Domain normalization is shared
with message IDs while retaining each enclosing grammar's trailing-dot sentinels.
Rust exposes ordinary vector edits instead of C# collection notifications, and
separates checked complete-value parsing from prefix parsing.
International domain conversion follows `Encodings/Punycode.cs`, with vectors
from `UnitTests/Encodings/PunycodeTests.cs`. The `idna` crate supplies UTS #46
processing in place of .NET `IdnMapping`; MIME ASCII safety checks and explicit
errors replace the original wrapper's unchanged-input fallback. ASCII route
formatting adapts `DomainList.Encode`, additionally validating ASCII names and
literals and reporting the failed stored index.
Quoted-string transformations follow `Quote`, `AppendQuoted`, `Unquote`, and
`UnquoteTo` in `Utils/MimeUtils.cs`, with vectors from
`UnitTests/Utils/MimeUtilsTests.cs`. String and raw-byte paths share one escape
transform; Rust `Cow` avoids allocating when no transformation is required.
Date parsing and formatting follow `Utils/DateUtils.cs`, with compatibility
vectors from `UnitTests/Utils/DateParserTests.cs`. The dependency-free `DateTimeOffset`
stores checked Gregorian components and a minute offset in place of
`DateTimeOffset`; formatting and timestamp conversion use Gregorian day counts.
Received trace parsing follows `Received.cs`, preserving applicable inline
comments for missing values, multi-token server strings, nested/escaped comments,
and date spelling. All 19 enabled valid parser fixtures and 11 invalid cases from
`UnitTests/ReceivedTests.cs` are ported, including original error offsets.
Additional cases cover clause ordering, charset recovery, subslices, all fixture
prefixes, arbitrary bytes, and deeply nested comments. Construction and editing
adapt `Received`/`ReceivedClause` constructors and setters from the same source,
retaining comments about parser bypasses and verbatim identifier validation.
Constructor, invalid-assignment, valid-ID, property-update/removal, comment
unfolding, and TCP-info-without-value tests are ported; additional cases verify
atomic failure, stable ordering, duplicate edits, control rejection, IPv6,
date-cache invalidation, and persistent parse diagnostics.
The existing charset/date/message-ID/mailbox helpers are reused.
Formatting adapts `AppendClause`, comment formatting, and `ToString` from the
same source, retaining grouping, editable-comment, and cached-date rationale.
All 19 enabled formatting fixtures are covered in CRLF and LF modes: 17 match
upstream exactly after adding `Received:`, while two 80-byte identifier cases
correctly fail the enforced 78-byte limit and round-trip at 81 bytes.
Constructor/removal/comment formatting expectations also match upstream.
Additional cases cover every supported line limit, exact group/piece thresholds,
UTF-8 byte counting, errors with component offsets, unbalanced comments,
date-cache updates, and deep nesting without recursion.
The upstream disabled malformed-literal case remains excluded.
Charset alias resolution and unlabelled-text fallback follow
`Utils/CharsetUtils.cs`, with compatibility cases from
`UnitTests/Utils/CharsetUtilsTests.cs`. Legacy conversion uses `encoding_rs`;
ASCII and Latin-1 are handled directly, and UTF-16 output uses Rust's UTF-16
iterator. This is a supported charset subset with documented mapping and
replacement differences, not a complete port of .NET's encoding registry.
Encoded-word decoding follows `TokenDecoder`, `TokenizeText`, `TokenizePhrase`,
and encoded-word recognition in `Utils/Rfc2047.cs`, with vectors from
`UnitTests/Utils/Rfc2047Tests.cs`. Shared safe tokenization replaces pointer
scanning; charset recovery diagnostics are an explicit Rust API addition.
Complete-value encoding adapts `GetRfc822Words`, `CharsetConvert`,
`AppendEncodedWord`, and `Encode` from the same source, with tests from the same
suite. It reuses the B/Q payload codecs, enforces self-contained words of at most
75 bytes, and preserves relevant source rationale in inline comments. Whole-value
encoding, exact B/Q size comparisons, preserved boundary whitespace, and explicit
charset errors replace the source's merging heuristics and silent fallback.
Parameter parsing follows `ParameterList.cs` and the attribute/token classes
in `Utils/ByteExtensions.cs`, with vectors adapted from
`UnitTests/ParameterListTests.cs`, `UnitTests/ContentTypeTests.cs`, and
`UnitTests/ContentDispositionTests.cs`. Rust adds recovery diagnostics and
stable ordering for duplicate continuation numbers. Parameter construction and
editing follow `Parameter.cs` and `ParameterList.cs`, with vectors from their
unit tests. Parameter serialization follows RFC 2231 encoding in `Parameter.cs`
and the relevant `FormatOptions.cs` defaults, with encoding vectors from
`UnitTests/ParameterTests.cs` and `UnitTests/ContentTypeTests.cs`. Rust enforces
the requested physical-line limit and accounts for growing continuation numbers;
extended sections consistently use percent encoding. Global/per-parameter
encoding-method selection, per-parameter quoting, and RFC 2047 compatibility
output adapt `GetEncodeMethod` and `EncodeRfc2047` from `Parameter.cs`, reusing the
shared complete-word B/Q encoder rather than the source's size estimates.
Global/per-parameter charset preferences adapt `GetBestEncoding`, `CanEncode`,
and `Rfc2231GetNextValue`, including whole-value UTF-8 fallback and complete
legacy/stateful chunks. ASCII, Latin-1, and UTF-8 keep their scalar fast path.
GB18030 and unmappable-charset vectors come from `UnitTests/ParameterTests.cs`;
Rust additionally checks UTF-16 surrogate boundaries, independent state resets,
and exact physical-line budgets. Parsed encoding preferences follow
`ParameterList.TryParse` and `Rfc2047.TokenDecoder.GetMostCommonCodePage`,
including first-encounter ties, unknown-label grouping, and RFC 2047 UTF-8
inheritance. Plain values keep default preferences rather than adopting
RFC 2047 solely because of the source decoder's default codepage result.
Typed content-header parsing and accessors follow `ContentType.cs` and
`ContentDisposition.cs`, with vectors from their corresponding unit tests.
Their parameter parser shares original-input positions with standalone parameter
parsing. Explicit typed-conversion errors and rejection of non-ASCII disposition
atoms replace the source's silent missing-value or lossy-conversion behavior.
Typed construction and setters follow the same classes, with checked
Content-Type components, explicit size bounds, and Rust ownership instead of
change events. Parameter edits preserve historical parse diagnostics separately
from the current decoded values.

## Acceleration

The default `simd` feature selects AVX2 or SSSE3 on x86/x86-64 and NEON on
AArch64 after CPU feature detection, for both Base64 and UU codecs.
MIME content scanning uses SSE2 or AVX2 with POPCNT on x86/x86-64 and NEON on
AArch64 to find boundary candidates and count/classify newlines in one pass.
CPU selection is retained across messages and source replacement. These
non-validating scans do not implement MimeKit's body compliance diagnostics.
Other CPUs use scalar fallbacks.
32-bit ARM is not accelerated; AVX-512 is intentionally omitted.

The Base64 NEON encoder uses 48-byte deinterleaved loads for bulk encoding and a
12-byte/16-byte-output path for smaller blocks. Its NEON decoder processes
16-byte input vectors, handing off non-alphabet characters to the scalar
decoder. Both kernels use byte-wise operations without word-endianness
assumptions.

Base64 vector decoding handles runs of complete, non-padding quartets. Padding,
non-alphabet bytes, incomplete quartets, and chunk boundaries retain the scalar
decoder's behavior. Encoding preserves MIME line boundaries. Small input chunks
use the scalar path without CPU detection.

UU encoding processes full 45-byte lines, overlapping the final vector so every
load stays inside the line. Decoding validates whole lines into bounded scratch
storage before appending exactly the declared byte count. In partial lines it
vectorizes only complete quartets before CR/LF, preserving the scalar decoder's
length counts and malformed-input state. Header scanning, short tails, and
terminators remain scalar. Both CRLF and LF formats use the same kernels.

Mimecraft's own unsafe code is restricted to private architecture-specific SIMD modules.
Their safe entry points require a privately constructed, CPU-feature-checked
capability token. Loads are
bounds-checked, stores target fixed-size local arrays, and appending decoded or
encoded bytes uses safe `Vec` operations. Disabling default features removes the
SIMD modules and forbids unsafe code in mimecraft itself. This setting does not
disable acceleration or unsafe code inside dependencies such as `encoding_rs`
and `memchr`:

```toml
[dependencies]
mimecraft = { path = "../mimecraft", default-features = false }
```

## Development

When porting code, preserve MimeKit's explanatory inline comments, especially
algorithm rationale, bit-layout diagrams, and boundary conditions. Adapt them
to the Rust implementation when the mechanics differ rather than dropping the
explanation or retaining comments that no longer apply.

```text
cargo test
cargo test --no-default-features
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
```

Unit tests explicitly exercise every supported backend, comparing outputs and
pending decoder state with the scalar implementation. Cases cover every byte
value inserted or replaced across vector boundaries, unaligned input, line
wrapping, chunk splitting, padding, random noise, and cloned/reset codec state.

### Regenerating HTML entities

`tools\html-entities` is a standalone Rust port of HtmlKit's
`CodeGenerator`. It generates `src\text\html_entities.rs` from the bundled
`HtmlEntities.json`, emitting a sorted lookup table rather than C# transition
tables. Unicode scalar escapes replace UTF-16 surrogate escapes, and the
maximum name length is derived from the input. Update the JSON snapshot and
regenerate instead of editing the generated Rust file.

```text
cargo run --manifest-path tools\html-entities\Cargo.toml
cargo run --manifest-path tools\html-entities\Cargo.toml -- --check
cargo test --manifest-path tools\html-entities\Cargo.toml
cargo fmt --manifest-path tools\html-entities\Cargo.toml --check
cargo clippy --manifest-path tools\html-entities\Cargo.toml --all-targets -- -D warnings
```

Default paths are relative to the tool's manifest directory, not the current
working directory. To use another snapshot or output file, pass both paths:

```text
cargo run --manifest-path tools\html-entities\Cargo.toml -- INPUT.json OUTPUT.rs
cargo run --manifest-path tools\html-entities\Cargo.toml -- --check INPUT.json OUTPUT.rs
```

The generator checks entity-name syntax, nonempty values, Unicode scalar
validity, and agreement between `codepoints` and `characters`. Malformed JSON,
duplicate names/fields, and invalid entries fail before output is written.
Output is deterministic ASCII with LF newlines, independent of JSON property
order. `--check` compares the exact generated bytes without writing and exits
unsuccessfully for stale or missing output. The tool has its own manifest and
lockfile; its JSON dependencies are not dependencies of the MIME library and
generation is not part of normal library builds.

### Benchmarks

```text
cargo bench --bench base64 --no-default-features
cargo bench --bench base64
cargo bench --bench mime_reader
cargo bench --bench mime_reader --no-default-features
cargo bench --bench html
cargo bench --bench html --no-default-features
cargo bench --bench rtf
cargo bench --bench rtf --no-default-features
cargo bench --bench headers
cargo bench --bench filters
cargo bench --bench message
```

The base64 benchmark, which needs no extra benchmarking framework, compares scalar-only and runtime-dispatched
builds. It also retains the original match-based decoder as a baseline for the
rank table. Workloads use a deterministic 1 MiB payload, reused output buffers,
and 1-byte, line-sized, and 4 KiB chunks; decoding includes unwrapped, MIME-wrapped,
and noisy input. MIME-wrapped input uses CRLF; unwrapped input removes both CR
and LF. Rates count input bytes and report the median of five timed
samples after warm-up. These are in-memory microbenchmarks, not message-parser or
end-to-end I/O throughput, and results depend on the CPU and compiler.

The `mime_reader` benchmark ports the synchronous reader and
parser workloads from MimeKit's `Benchmarks/MimeParserBenchmarks.cs`, retaining
their benchmark names: `StarTrekMessage`, `ContentLengthMbox`, `JwzMbox`,
`HeaderStressTest`, and `MixedNested64`. `NestedRfc822x1000` is reader-only, as
upstream. These replace the generated attachment/long-line/mbox microbenchmarks.
`benches/data/messages/startrek.eml` is the original benchmark message;
`header-stress.eml` contains the exact ASCII bytes of MimeKit's header-stress
string, including its line endings. The two mbox cases reuse the original
archives in `tests/data/mbox` (29 and 152 messages). The Content-Length archive
uses default mbox framing, matching upstream's benchmark, not its optional
Content-Length-aware parsing mode. Nesting generators preserve the upstream
wire format; only those cases raise the depth limit to include the final leaf.

Each parser workload reports the original `MimeParser_*` default-copying
baseline plus `MimeParser_Source_*` and `MimeParser_Slice_*` variants. The
buffered source-backed variant uses the explicit-storage constructor with
borrowed seekable `Cursor` input; the slice variant uses `from_slice_with_options`
without an input staging buffer. Both include shared-source setup in their
timing and retain body ranges without cloning the input. Reader baselines
remain `MimeReader_*`, with `MimeReader_Slice_*` passing the fixture slice
directly to the reader. The 1,000-level workload stays reader-only.

Fixtures are loaded/generated outside timing. Each iteration creates a fresh
reader/parser; buffered variants use a 4 KiB input buffer, while slice variants
read directly from the fixture. Reader callbacks do no work,
and parser timing includes source setup, model construction, and drop.
Bodies are not read or decoded after parsing: `Source` measures scanning and
retaining body ranges, whereas `Memory` also copies encoded bodies. Each mbox
message is dropped before parsing the next, rather than retaining the archive's
entire model. Message counts,
full input consumption, and EOF are checked before timing. After warm-up, five
samples of at least 100 ms report median input MiB/s and messages/s. The existing
timing harness is retained; it does not reproduce BenchmarkDotNet's allocation
diagnostics or statistical machinery. Async and
LegacyMimeParser variants are not included.

The `html` benchmark ports `HtmlTokenizer_ReadAllTokens` from MimeKit's
`Benchmarks/Text/HtmlBenchmarks.cs`. Both original documents are preserved
byte-for-byte under `benches/data/html`: `test.html` and
`Gimhae_Kim_clan.utf-8.html`. Stable harness IDs are `test_html` and
`Gimhae_Kim_clan_utf_8_html`. Like upstream's `File.ReadAllText` setup,
loading removes a UTF-8 BOM and replaces malformed UTF-8 before measurement.
This differs from MimeKit's Windows-1252 tokenizer-test setup: the benchmark
decodes 32 malformed bytes in `test.html` as replacement characters, while
the Korean page has a UTF-8 BOM. Neither line endings nor original fixture
bytes are normalized.

The tokenizer operation creates a fresh `HtmlTokenizerReader` over the decoded
string and counts and drops all owned tokens. Scanner/source setup, token
allocation, and destruction are included; fixture loading/decoding is not.
Canonical counts (7,972 and 1,911 tokens), complete byte consumption, and EOF
are checked outside measurement. The standalone harness reports median
decoded-UTF-8 MiB/s and documents/s over five warmed-up samples, matching the
existing timing harness.

Both HTML harnesses also provide `HtmlToText_ConvertToString` and
`HtmlToText_ConvertToTextWriter`. There is no upstream HTML-to-text benchmark
yet: these mirror the upstream HTML-to-HTML converter workload shapes using
the same preloaded documents. Each iteration constructs a fresh converter;
the string result is dropped and the writer uses a null sink. Conversion and
allocation/destruction are timed, but input loading/decoding and output
fingerprint/adapter-agreement checks are outside measurement. Expected CRLF
output is checked against MimeKit-generated byte counts and FNV-1a fingerprints.
Both harnesses also port all three upstream `HtmlToHtml` workloads:
`HtmlToHtml_ConvertToString`, `HtmlToHtml_ConvertToTextWriter`, and
`HtmlToHtml_ConvertWithCallback`. Each uses a fresh converter over predecoded
input. The callback case filters comments and suppresses script/style content
and end tags, exactly as upstream; it is not a complete sanitizer.
String allocation/destruction and conversion are timed; writer variants discard
output. Default/filtered output lengths and FNV-1a fingerprints generated with
MimeKit are checked outside timing, along with string/writer agreement.

#### RTF benchmarks and pre-sans-I/O baseline

The `rtf` harness ports all thirteen converter workloads from MimeKit's
`Benchmarks/Text/RtfConverterBenchmarks.cs` and adds seven
`RtfTokenizer_ReadAllTokens` cases over the same generated RTF documents.
There is no separate upstream tokenizer benchmark class. The generators preserve
the upstream inline rationale and exact input bytes: Word-like formatting,
Unicode/DBCS, encapsulated HTML, deep identical/formatted groups, an oversized
font table, and binary data. `LargeBinary` uses raw `0xff` bytes rather than
UTF-8-encoding the upstream StringReader's U+00FF characters, so `\binN` consumes
exactly the intended payload.

Each iteration constructs a fresh synchronous tokenizer/converter with a 4 KiB
RTF input buffer. Tokenizer timing includes counting and dropping owned tokens,
the final EOF probe, and reader destruction. Converters return and drop complete
strings, unlike upstream's null-writer measurements; no writer API was added
just for benchmark parity. `RtfToHtml_Render` always renders RTF.
`RtfToHtml_Extract/EncapsulatedHtml` composes `RtfHtmlExtractor` and `HtmlToHtml`,
matching upstream's extraction/rewrite behavior while materializing both the
intermediate and final strings. `HtmlToHtml/ExtractedHtml` measures rewriting the
pre-generated HTML alone. These API/allocation differences preclude direct
cross-runtime timing comparisons.

Generation and assertions run outside measurement. The integration test
`rtf_benchmarks` and both harnesses check original input fingerprints, canonical
token counts, complete consumption, repeated EOF, and CRLF output fingerprints.
References were generated using the unmodified MimeKit generators/converters
at upstream revision `20ab449ec5ee334efd6c5d909424ee61971267d5`; raw extracted HTML
also matches the independently generated HTML string exactly.

The following baseline was recorded on October 29, 2026, before changing the
I/O-coupled tokenizer, using the library at `1a6a57d` plus these harnesses:
Windows x86-64, Intel Core Ultra 7 268V, rustc 1.99.1
(`58cf8f9c3`, LLVM 23.1.1), MSVC target, default `simd` feature,
`-C target-cpu=x86-64-v3`, fat LTO, and one codegen unit. Throughput is the
standalone median of five samples of at least 100 ms after three warm-ups.
Rates count original RTF bytes, or UTF-8 bytes for the extracted HTML baseline.
Allocation counts/bytes are per operation from the separate metabench allocator
run; allocated bytes are cumulative, not peak memory.

| Operation | Document | MiB/s | Documents/s | Allocations | Allocated bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| RtfTokenizer_ReadAllTokens | WordDocument | 121.2 | 121.2 | 89,837 | 771,591 |
| RtfToText | WordDocument | 35.6 | 35.6 | 89,858 | 2,247,394 |
| RtfToHtml_Render | WordDocument | 24.9 | 24.9 | 138,248 | 9,104,713 |
| RtfTokenizer_ReadAllTokens | UnicodeDocument | 68.8 | 68.8 | 159,731 | 184,742 |
| RtfToText | UnicodeDocument | 41.7 | 41.7 | 159,750 | 1,005,006 |
| RtfToHtml_Render | UnicodeDocument | 37.9 | 37.8 | 159,756 | 3,331,909 |
| RtfTokenizer_ReadAllTokens | EncapsulatedHtml | 109.6 | 109.6 | 83,194 | 815,981 |
| RtfToText | EncapsulatedHtml | 50.8 | 50.8 | 83,213 | 1,554,415 |
| RtfToHtml_Render | EncapsulatedHtml | 44.0 | 44.0 | 97,758 | 3,459,919 |
| RtfToHtml_Extract | EncapsulatedHtml | 35.8 | 35.8 | 117,698 | 4,360,481 |
| RtfTokenizer_ReadAllTokens | DeepNesting | 60.0 | 60.0 | 4 | 4,132 |
| RtfToText | DeepNesting | 24.5 | 24.5 | 8 | 5,160 |
| RtfTokenizer_ReadAllTokens | DeepFormattedNesting | 39.8 | 67.4 | 190,651 | 194,779 |
| RtfToText | DeepFormattedNesting | 27.6 | 46.7 | 190,672 | 660,943 |
| RtfToHtml_Render | DeepFormattedNesting | 27.3 | 46.1 | 194,777 | 1,001,500 |
| RtfTokenizer_ReadAllTokens | LargeFontTable | 114.2 | 114.4 | 103,514 | 647,005 |
| RtfToText | LargeFontTable | 74.8 | 75.0 | 103,536 | 796,653 |
| RtfTokenizer_ReadAllTokens | LargeBinary | 6,193.4 | 6,193.5 | 9 | 4,159 |
| RtfToText | LargeBinary | 5,337.5 | 5,337.5 | 14 | 5,203 |
| HtmlToHtml | ExtractedHtml | 104.1 | 276.7 | 34,480 | 2,429,172 |

Reproduce with `cargo bench --bench rtf` and
`cargo bench --features bench-metabench --bench rtf_metabench -- --allocations`.
Keep compiler, CPU flags, features, buffering, and output policies fixed for
before/after comparisons, and rerun the old revision on the same machine to
control for environmental noise. The standalone harness is intentionally short;
use `--criterion` on the metabench target for longer statistical timing.
The RTF tokenizer remains I/O-coupled in this batch. The TNEF reader still needs
its own benchmark baseline before its planned sans-I/O refactor.

#### Address, codec/filter, and message benchmarks

The `headers`, `filters`, and `message` targets port the remaining upstream
workloads that have matching public Rust entry points, adding 118 cases:

| Target | Upstream benchmark classes | Cases |
| --- | --- | ---: |
| `headers` | `AddressParserBenchmarks`, `Utils/Rfc2047Benchmarks` | 84 |
| `filters` | All seven `IO/Filters/*Benchmarks` classes | 31 |
| `message` | `MimeMessageBenchmarks` | 3 |

`headers` preserves all 41 address-generator inputs and both `ParseBytes` and
`ParseString`, including malformed brackets, unknown charsets, UTF-8/encoded
names, quoted local-parts, folding, long atoms, and long comments. Every input
and complete parsed name/address/group sequence is checked against MimeKit
FNV-1a references, not just address counts. Both Rust entry points consume UTF-8;
`ParseString` does not incur MimeKit's UTF-16-to-UTF-8 conversion. Results and
their decoding diagnostics are allocated and dropped within each operation.
The two RFC 2047 cases preserve the original `ILoveMimeSoMuch` input and assert
the fully decoded Unicode text outside measurement.

`filters` includes direct/streaming Base64 encode at widths 60, 64, 68, 72, and
76, streaming quoted-printable/UU encode, direct/streaming decode for all three
codecs, both From-line filters, both BestEncoding inputs, trailing whitespace,
and all eight newline-filter cases. Upstream's `photo.b64`, `photo.uu`,
`wikipedia.txt`, and `wikipedia.qp` are preserved byte-for-byte in
`benches/data/encoders`, with Git text normalization disabled. Its `photo.jpg`
is identical to the existing `tests/data/attachments/girl.jpg`, which is reused
rather than duplicated; lorem ipsum is also reused from the attachment corpus.
Fixture and transformed-output lengths/hashes come from the upstream files and
converters. BestEncoding recommendations and reset behavior are checked too.

Fresh codecs, filters, and output buffers are created inside each operation.
Direct encoding reserves the calculated output capacity; direct decoding
reserves input length as an upper bound, since Rust has no output-estimate API.
These allocations and destruction are timed. Stream cases use `FilteredWriter`
and explicitly call `finish`, not merely Rust's non-finalizing `flush`.
From-line filters retain their full output vector and 1 KiB submissions;
other stream filters write to a measuring sink with 4 KiB submissions.
This intentionally uses Rust's adapter, which itself accepts at most 4 KiB
per write, rather than trying to reproduce .NET's `MemoryStream.CopyTo` fast
path or pooled input buffers. BestEncoding uses the original 100 reset passes
per operation and 4 KiB chunks, with reusable output storage; its byte rate
counts all 100 passes. Unlike MimeKit's pass-through slice, Rust's filter API
copies those bytes into the output buffer.

Upstream's newline setup converts *both* `LoremIpsumDos` and `LoremIpsumUnix`
with `Unix2DosFilter`. That quirk is preserved, rather than silently changing
the benchmark inputs. The quoted-printable and UU encoder cases do not use
the upstream `MaxLineLength` parameter, so their five identical copies are
collapsed into one case each. Hardware-enabled/disabled Base64 pairs use one
Rust identity: compare default features with `--no-default-features` for
dispatched/scalar builds. The .NET `System.Buffers.Text.Base64` comparison
baselines are omitted, not relabeled as Rust MIME codecs.

`message` constructs the original two-leaf multipart body outside timing,
then reuses it and resets both leaves to `Unspecified` before every `prepare`,
as upstream does. It measures all three constraints, including reset/header
updates and a `RefCell` mutable borrow, but not message construction or payload
loading. Content uses memory-backed cursors, not upstream's file-backed JPEG
stream. The exact leaf encoding choices are checked on consecutive operations.
Reported MiB/s counts logical payload size, not actual bytes read: unrestricted
preparation can choose binary without scanning that content.

`tests/core_benchmarks.rs` exercises the shared setups and correctness checks.
Address/filter references were generated against MimeKit revision
`20ab449ec5ee334efd6c5d909424ee61971267d5`. All three targets use the existing
three-warm-up/five-sample standalone policy; the optional metabench targets
below register the same 118 identities for Criterion and allocation tracking.
These measure the current Rust API costs, not cross-runtime speed ratios.

Deferred upstream benchmark classes are `AddressValidatorBenchmarks` (the
separate address compliance validator is not ported) and
`Encodings/EncodingValidatorBenchmarks` (Rust's validators are private to
compliance reporting). No library APIs are exposed solely for benchmarking,
and full MIME-reader validation is not mislabeled as isolated validator work.
The existing MIME benchmark target continues to omit upstream async and legacy
parser variants. There is no TNEF benchmark class in this upstream checkout;
representative TNEF workloads and a pre-sans-I/O baseline remain queued.

#### Optional metabench targets

The `base64`, `mime_reader`, `html`, `rtf`, `headers`, `filters`, and `message`
targets remain available without any benchmark framework. Separate targets
with the `_metabench` suffix are enabled by `--features bench-metabench`;
their optional dependencies
are not compiled by ordinary library builds or the original benchmark commands.
These harnesses use the same fixtures, generators, and operations under
`benches/support`, covering all 21 Base64, 27 MIME, twelve HTML, twenty RTF,
84 address/RFC 2047, 31 codec/filter, and three message-preparation cases.
The MIME metabench target also includes six `headers/HeaderList_Get` cases:
lowercase, mixed-case, and uppercase hits, a mixed-case miss, and long-name
lookups. Header lists are prepared outside measurement; each operation performs
1,000 lookups without constructing or parsing headers.

```text
cargo bench --features bench-metabench --bench mime_reader_metabench -- --criterion
cargo bench --features bench-metabench --bench mime_reader_metabench -- --allocations
cargo bench --features bench-metabench --bench base64_metabench -- --allocations
cargo bench --features bench-metabench --bench base64_metabench --bench mime_reader_metabench -- --test
cargo bench --features bench-metabench --bench html_metabench -- --criterion
cargo bench --features bench-metabench --bench html_metabench -- --allocations
cargo bench --features bench-metabench --bench html_metabench -- --test
cargo bench --features bench-metabench --bench rtf_metabench -- --criterion
cargo bench --features bench-metabench --bench rtf_metabench -- --allocations
cargo bench --features bench-metabench --bench rtf_metabench -- --test
cargo bench --features bench-metabench --bench headers_metabench -- --criterion
cargo bench --features bench-metabench --bench filters_metabench -- --allocations
cargo bench --features bench-metabench --bench message_metabench -- --criterion
cargo bench --features bench-metabench --bench headers_metabench --bench filters_metabench --bench message_metabench -- --test
```

Criterion timing and allocation tracking work on Windows as well as Linux.
Instruction counting uses Gungraun/Callgrind and requires Linux (including WSL),
Valgrind, and the runner version matching metabench's locked Gungraun dependency:

```text
cargo install gungraun-runner --version 0.19.4 --locked
cargo bench --features bench-metabench --bench mime_reader_metabench -- --gungraun
cargo bench --features bench-metabench --bench base64_metabench -- --gungraun
```

Install Valgrind using the Linux distribution's package manager. In WSL, use
Linux Rust and a Linux build, not a Windows benchmark executable. Keep compiler
flags and features fixed for comparisons; `--no-default-features` selects the
library's scalar paths. The local `.cargo/config.toml` may select CPU instructions
unsupported by the installed Valgrind. In that case explicitly use
`RUSTFLAGS="-C target-cpu=x86-64"` for both builds on x86-64 Linux; do not compare
against a baseline built with different flags. Instruction counts complement
throughput measurements, rather than predicting elapsed time or guaranteeing
identical counts across compilers, CPUs, SIMD dispatch, and allocators.

Native cases and Criterion registrations use matching identities. Fixture I/O,
generation, correctness checks, and reusable Base64 output allocation occur
outside measurement. MIME measurements still include fresh reader/source setup,
model construction and destruction; source-backed bodies are not decoded.
HTML measurements include fresh scanner setup and owned token creation/drop,
preserving upstream's emphasis on per-document allocation as well as throughput.
The metabench allocator adds a tracking-flag check even when not collecting
allocations, and workload entry points are non-inlined. Compare metabench results
against other metabench runs, not absolute timings from the original harness.

For a focused allocation run, append a Criterion-style name filter:

```text
cargo bench --features bench-metabench --bench mime_reader_metabench -- --allocations --criterion-arg StarTrekMessage
```

Gungraun accepts its own filter via `--gungraun-arg`; use `--gungraun --list`
to inspect native names. `--test` is a smoke run, not a performance measurement.
Reports are written under `target/metabench/<target>/`; preserve a report and
select it with `--baseline <report.json>` for an explicit comparison. The previous
report is otherwise the automatic baseline. Use separate output paths with
`--output <path>` for engine/feature configurations you want to retain.

These are cached in-memory workloads without storage or network I/O.
Reader/parser measurements do not decode MIME bodies; converter and codec cases
do process decoded content. Results are not a throughput guarantee or a direct
cross-runtime comparison against MimeKit; measure server workloads too.

The UU backend comparison is an ignored unit test so it can force scalar,
SSSE3, AVX2, or NEON independently on supported hardware:

```text
cargo test --release encodings::uu::tests::backend_throughput -- --ignored --nocapture
```

It measures 1 MiB payloads with 4 KiB chunks and reused output buffers, reporting
the median of five samples of at least 100 ms for each backend and new-line
format. Throughput counts input bytes, and output is checked against scalar.

The AArch64 implementation has been cross-compiled and its equivalence tests
executed under QEMU user-mode emulation. Native ARM throughput has not been
measured; run the same benchmarks on ARM hardware for meaningful performance
comparisons.

Library code is licensed under MIT, retaining the original MimeKit copyright
notice in `LICENSE`. The bundled Unicode tables and normalization conformance
data are covered by `LICENSE-Unicode` (Unicode License v3).
