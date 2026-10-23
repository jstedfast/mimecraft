# mimecraft

An idiomatic Rust port of MimeKit's core MIME library. Behavioral compatibility
with MimeKit is the goal, not a one-to-one translation of its C# API.

This is an early, incomplete implementation, not yet a usable replacement for
MimeKit. The crate is not configured for publication.

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
- Text parts with strict charset-aware reading, borrowed or owned source
  streams, and atomic in-memory text/charset replacement.
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
- Buffered loading of complete MIME entity trees into owned body streams,
  with raw headers, stored transfer encodings, and input/header/depth limits.
- Message ownership, buffered loading, and synchronous writing, with separate
  message/body headers, optional bodies, and no generated metadata.
- Structured attached-message ownership and iterative writing, with nested
  message traversal and boundary checks across message wrappers.
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
- Complete ASCII address headers with checked field names, CRLF/LF endings,
  grammar-aware folding, and enforced physical-line limits.
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

`mimecraft::ContentEncoding` mirrors MimeKit's `Default`, `SevenBit`, `EightBit`,
`Binary`, `Base64`, `QuotedPrintable`, and `UuEncode` variants. It is metadata,
not an encoder factory or content validator. Its `parse` method recognizes
Content-Transfer-Encoding values; it does not parse an entire header line.

`PassThroughEncoder::new(encoding)` and `PassThroughDecoder::new(encoding)`
append input bytes unchanged. Their `default()` selects `ContentEncoding::Default`.
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
As in MimeKit, yEnc codecs and `Rfc2231Decoder` report `ContentEncoding::Default`;
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

`encoder_filter(encoding)` and `decoder_filter(encoding)` return
`Box<dyn MimeFilter>`. Base64, quoted-printable, and UU select their codecs;
`Default`, `SevenBit`, `EightBit`, and `Binary` pass through without validation
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
stored bytes. `new(source)` labels unencoded content as `ContentEncoding::Default`;
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
assert_eq!(part.content().encoding(), ContentEncoding::Default);
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
missing means `Default`, but empty or unknown values return explicit errors.
Setting `Default` removes all transfer-encoding headers; other encodings write
one canonical token with CRLF termination.

The stored encoding belongs to `MimeContent`, independently of the requested
transfer-encoding header. Metadata edits do no I/O or transcoding, do not move
the source cursor, and do not validate body structure or transport suitability.
`content_mut()` provides body operations; `set_content()` returns the old body
without changing headers or either source's position. `into_parts()` extracts
both components. Sources may be owned or borrowed; use an empty stream for an
empty body.

Complete entity loading is available through `MimeEntity::load`, and message
loading through `MimeMessage::load`, including transparent attached messages.
Encoded message-wrapper interpretation and truncated-entity recovery remain deferred.

### Single-part serialization

`MimePart::write_to(&mut output)` writes normalized headers, one blank
separator, and the body using CRLF and the first Content-Transfer-Encoding
header (or `Default` when absent). It does not change headers or stored bytes.
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
assert_eq!(part.content().encoding(), ContentEncoding::Default);
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
when enabled, nonbinary output ends with the selected newline, including empty
Base64/QP bodies. New encoders already terminate nonempty output, and UU always
emits a zero-length line.

New UU output includes `begin 0644 filename` and `end` lines. The filename uses
Content-Disposition, then Content-Type's `name`, then `unknown`, with surrounding
whitespace trimmed. Control characters are rejected rather than introducing
extra wrapper lines. Already UU-encoded content retains its existing wrapper.

Header normalization preserves field spelling, folding, duplicates, malformed
records, and value bytes using `HeaderList`'s rules. Unlike standalone list
writing, part writing terminates truncated records in output before writing
the separator, without changing the stored records. No automatic MIME-Version
header, content-type fallback, duplicate removal, or transport validation occurs.

Options and required metadata are checked before source I/O or destination
writes/flushes. Invalid options return `io::ErrorKind::InvalidInput`; an unknown
transfer encoding or invalid required UU filename metadata returns `InvalidData`.
Unneeded content metadata is not parsed. I/O errors propagate and can leave
partial output. Successful body copies rewind the source before filter
finalization; subsequent write or flush failures may therefore leave it at zero.
The borrowed destination is flushed, never closed. Retrying restarts the entire
part, not the failed write; clear or replace the destination first. Neither the
source body nor the output is materialized in its entirety.

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
text operations and `is_plain()`/`is_html()` recheck them, reporting missing,
malformed, or non-text values rather than using a cache or later duplicate.

`text()` transfer-decodes using the stored content encoding, then strictly
decodes the declared charset, using UTF-8 only when the charset is absent.
`text_with_charset(charset)` overrides the label without editing headers.
Unsupported labels and invalid byte sequences are explicit `TextPartError`s.
Unlike MimeKit's automatic getter, this initial API does not sniff BOMs or HTML
metadata, fall back to Latin-1, replace characters, or normalize newlines.
Explicit BOMs remain characters. Whole-text reads allocate the decoded body and
resulting string; use lower-level content/filter APIs for incremental processing.
Successful copies rewind the source before charset decoding, including when
charset decoding fails. I/O failures follow `MimeContent`'s existing rules.

Text setters are available for the default `TextPart<Cursor<Vec<u8>>>` storage,
not arbitrary file or borrowed-stream types. `set_text(text)` selects UTF-8;
`set_text_with_charset(text, charset)` uses the specified codec. Both encode and
format before committing the body and charset change. Other Content-Type
parameters survive, duplicates collapse at the first slot, and unrelated headers
remain untouched. In particular, the requested transfer encoding is preserved.
The new body stores unencoded bytes at position zero with no newline hint.
Failure leaves all old bytes, headers, and source position unchanged.

Automatic charset detection/recovery and text-format conversion remain deferred.

### Multipart ownership and MIME trees

`MimeEntity<R>` is `Part(MimePart<R>)`, `Multipart(Multipart<R>)`, or
`Message(MessagePart<R>)`.
Converting a `TextPart` into an entity retains the same leaf headers and body
without copying; extracting that leaf and using `TextPart::from_part` restores
text-specific APIs. All leaves in a tree share one source type, defaulting to
`Cursor<Vec<u8>>`; owned or borrowed seekable sources are supported.

```rust
use mimecraft::{MimeEntity, Multipart, TextPart};

let mut alternatives: Multipart = Multipart::new("alternative", "alt-boundary").unwrap();
alternatives.children_mut().push(TextPart::new("plain", "hello").unwrap().into());
alternatives.children_mut().push(TextPart::new("html", "<b>hello</b>").unwrap().into());

let mut mixed: Multipart = Multipart::new("mixed", "outer-boundary").unwrap();
mixed.set_preamble(b"This message has multiple parts.\r\n".to_vec());
mixed.children_mut().push(alternatives.into());
let root: MimeEntity = mixed.into();
assert_eq!(root.leaf_parts().count(), 2);
```

`Multipart::new(subtype, boundary)` creates an empty container with one checked
Content-Type. Boundaries are explicit, not randomly generated. Checked creation,
Content-Type replacement, and boundary edits require the RFC 2046 boundary
alphabet, 1..=70 bytes, and no trailing space. Input is not trimmed.
Boundary errors report the first invalid byte. Model edits do not scan sources
or check collisions: callers must choose values that do not collide with body
data or ancestor/descendant delimiters. Serialization checks ancestor overlap.

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
Preamble and epilogue setters take raw byte vectors and return the previous
bytes, without decoding, folding, or newline conversion. These vectors exclude
delimiter lines; epilogue begins after the closing delimiter's line ending.
`from_parts`/`into_parts` transfer all four components without validation,
default insertion, source I/O, or copying. Empty vectors represent absent text.

`leaf_parts()` visits leaves depth-first using an explicit stack, descending
through attached messages as well as multiparts. Empty containers and absent
messages/bodies contribute no leaves, and a leaf root visits itself. Traversal does
not inspect metadata or source bytes. Programmatic depth is unrestricted:
callers must bound it because derived debugging and ordinary drop still recurse.
Editing headers does not change an enum variant or reinterpret its body.
`MimeEntity::body_parts()` instead stops at attached-message wrappers;
`attachments()` filters those non-multipart nodes by their own dispositions.

Content collision scanning, random boundary generation, and
preservation of parsed/truncated delimiter spellings remain deferred.

### Multipart serialization

`Multipart::write_to` and `MimeEntity::write_to` serialize complete trees with
default CRLF output. Their `write_to_with_options` variants reuse
`MimePartWriteOptions`; a leaf entity root behaves exactly like `MimePart`.

```rust
use mimecraft::{Multipart, TextPart};

let mut mixed: Multipart = Multipart::new("mixed", "message-boundary").unwrap();
mixed.children_mut().push(TextPart::new("plain", "hello").unwrap().into());
let mut output = Vec::new();
mixed.write_to(&mut output).unwrap();
assert!(output.ends_with(b"\r\n\r\nhello\r\n--message-boundary--\r\n"));
```

Writing uses an explicit stack proportional to nesting depth, not recursive
calls or whole-body buffering. Headers use the leaf writer's normalization and
truncated-record repair. Preamble and epilogue newlines are normalized without
charset decoding or folding; nonempty preambles are terminated before the
first delimiter. Empty containers still emit a closing delimiter. The closing
delimiter always has its own line ending, followed by the stored epilogue.

Every complete child is followed by a **separate delimiter-owned newline**,
even when its body is empty or already ends in a newline. No binary body suffix
is consumed or rewritten to provide framing. Children reuse the leaf writer's
transfer conversion with `ensure_new_line` disabled; the root option affects
only its epilogue. Encoder-generated endings are retained.
With LF output, a child ending in bare CR receives a CRLF delimiter prefix:
otherwise its CR and the delimiter's LF would look like one CRLF, losing the
body byte when parsed. The boundary marker line still uses LF. This also
protects nested epilogues ending in bare CR.

Container Content-Type and boundary must be valid. Equal or prefix-overlapping
ancestor boundaries are rejected, including across attached-message wrappers,
but siblings may reuse boundaries. Multipart
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
unchanged. Transport preparation and signature-specific handling are deferred.

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

The loader uses iterative traversal and incremental buffered reads. It buffers
up to two physical lines and retains the resulting headers, preambles,
epilogues, and encoded leaf bodies; this is not constant-memory or
persistent-source parsing.
The wire-byte limit does not describe total allocation overhead or buffered
reader read-ahead. Sources need `BufRead`, not seeking; use `BufReader` around a
plain `Read` source. The borrowed source is never closed.

Headers retain raw bytes, order, folds, duplicates, and malformed records.
The first Content-Type selects multipart, attached message, or leaf; a missing
type remains absent and normally selects an opaque leaf. Immediate
multipart/digest children instead default to message/rfc822. Invalid present
Content-Type and unknown transfer encodings are explicit errors. Stored encoding is initialized from
the first Content-Transfer-Encoding header, or `Default` when absent. Bodies
are neither decoded nor normalized and start at position zero. Newline hints
describe their stored LF/CRLF sequences, excluding boundary framing.

Boundaries match whole case-sensitive lines, with optional trailing space/tab.
LF, CRLF, and a closing delimiter at EOF are accepted. Leaf bodies and nested
epilogues exclude exactly the newline immediately before their parent delimiter;
at EOF their trailing bytes are retained. Preambles retain their final newline.
Epilogues begin after the closing delimiter's newline, matching the owned model.
Empty containers and empty child bodies are supported.

Every entity requires a header/body separator. Empty input, header-only parts,
missing closing markers, and prematurely encountered ancestor delimiters fail
instead of producing a truncated tree. Multipart encoding, boundary syntax,
and ancestor-overlap checks match serialization. Raw delimiter spelling,
absent-versus-empty body distinctions, and separator spelling are not retained:
load/write may normalize framing, so it is not a lossless raw-wire API.
Recognized transparent message wrappers become structured attached messages;
encoded wrappers and other non-multipart types remain opaque leaves.
Header-only message interpretation, recovery diagnostics, mbox handling,
zero-copy source ranges, and async loading remain deferred.

Zero limits return `InvalidInput` before I/O. Exceeded limits, malformed
metadata, and misplaced boundaries return `InvalidData`; incomplete structure
returns `UnexpectedEof`. Header-length errors retain `HeaderTooLong`.
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
list, but original message/body header interleaving is not retained. Ordinary
descendant entity headers are not repartitioned.

Loading always produces a body, even for empty content, and requires a complete
header/body separator. Missing From/Date and invalid non-content metadata are
not rejected. Recognized attached messages are parsed by default, sharing the
same limits across all message and multipart nesting. Raw constructors and
edits do not repartition misplaced headers; callers must keep Content-* fields on the
body and manage any duplicates themselves.

`write_to` and `write_to_with_options` write message headers followed by body
headers, one separator, and body content, reusing `MimePartWriteOptions` and
the tree writer. An absent body still gets a separator; an entirely empty
message writes one newline. Options fail before I/O; later metadata or I/O
errors can leave partial output. Sinks are flushed, not closed, and successful
body copying rewinds sources. Headers and stored bytes are not mutated.

No Date, Message-Id, From, Subject, or MIME-Version is generated. This is not
transport preparation: **Bcc and Resent-Bcc are written as supplied**.
Message-specific typed convenience setters, safe complete unstructured-header
authoring, offset-based header merging, and automatic
transport preparation remain deferred.

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
use mimecraft::{Header, MessagePart, MimeEntity, MimeMessage, TextPart};

let inner = MimeMessage::with_body(TextPart::new("plain", "attached").unwrap().into());
let mut part = MessagePart::new(inner);
part.headers_mut().add(Header::parse(b"Content-Disposition: attachment").unwrap());
let message = MimeMessage::with_body(part.into());
let attachments = message.attachments().collect::<Result<Vec<_>, _>>().unwrap();
assert_eq!(attachments.len(), 1);
assert!(matches!(attachments[0], MimeEntity::Message(_)));
assert_eq!(message.leaf_parts().count(), 1);
```

`MimeEntity::is_attachment()` parses only that node's first Content-Disposition
and compares its disposition with `attachment`, ignoring ASCII case. Missing
disposition is false; filenames, media types, message-level headers, and inner
attachments do not imply attachment status. Raw header edits are immediately
visible, and later duplicates do not override the first record.

Malformed dispositions produce errors, not silent skips or attachment
fallbacks. `Attachments` yields the error and can resume at the next body part;
collecting into `Result<Vec<_>, _>` stops at the first failure. Multipart nodes
themselves are excluded from both body-part and attachment enumeration, but
their children are visited regardless of the container's disposition.
Attached-message contents are not inspected; enumerate the contained
`MimeMessage` explicitly when needed.

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

let mut alternative = Multipart::new("alternative", "alt").unwrap();
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

### Related multipart roots

`MultipartRelated<R>` composes `Multipart<R>`, with an explicit boundary in
`new(boundary)` and checked wrapping through `from_multipart`. Wrapping checks
only the first Content-Type for multipart/related; it does not require a valid
boundary, selectors, or children. Loading and conversion keep the ordinary
`MimeEntity::Multipart` variant. No source traits or I/O are required.

```rust
use mimecraft::{MimeEntity, MultipartRelated, TextFormat, TextPart};

let mut related: MultipartRelated = MultipartRelated::new("related-boundary").unwrap();
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
the new root's Content-Id. A non-first replacement requires a valid existing
Content-Id and stores its bracketed ID in start. Unlike MimeKit, missing IDs
are errors rather than automatically generated values. Child headers are
unchanged; duplicate IDs are not rejected, and subsequent lookup picks the
first matching child.

`set_root_with_options` accepts `HeaderFormatOptions`. Both setters validate
the existing boundary and serialize the complete parent Content-Type before
editing headers or children. They collapse Content-Type duplicates at the
first slot while preserving other parameters, unrelated headers, other
children, preamble, epilogue, and source positions. Metadata and formatting
errors leave the container unchanged; the supplied root is dropped on failure,
following ownership-taking API conventions. Borrowed sources stay open.

URI resource lookup, Content-Location/base resolution, resource opening, and
automatic Content-Id generation remain deferred. Body selection continues to
follow nested multipart roots, including alternatives, through the existing
`MimeEntity` and `MimeMessage` APIs.

### Decoded body text and flowed conversion

`MimeMessage::text_body(format)` and `MimeEntity::text_body(format)` return
`Result<Option<String>, TextBodyError>`. They use the same selection rules as
`text_body_part`, then read only the selected source. No match returns `None`;
an empty matching body returns `Some(String::new())`.

Transfer decoding uses the stored content encoding, not a newly edited
Content-Transfer-Encoding header. Charset decoding reuses `TextPart`'s strict
decoder: the declared charset wins, absent charset means UTF-8, and unsupported
labels or invalid bytes are errors. No BOM/HTML sniffing, Latin-1 fallback, or
replacement occurs. Selection and charset-label errors happen before source
I/O. Successful byte copying rewinds the source before charset decoding;
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
Streaming text converters, flowed-text generation, and automatic HTML/RTF
conversion remain deferred.

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
`Default` encoding at position zero. C# caches, change events, async parsing,
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
non-field bytes. Stored encoding becomes `Default`, position is zero, and
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
including any trailing data. Stored encoding becomes `Default`, position is
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

### Partial-message reassembly

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
Raw spelling, duplicates, folds, and values are retained; ordinary message
serialization still groups message headers before body headers.
Automatic splitting, generated IDs, and transport preparation remain deferred.

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
when authoring returned headers. Present messages use the existing strict
parser and require an inner header/body separator even without body bytes;
header-only payloads lacking that separator are not recovered automatically.
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
let mut mixed: Multipart = Multipart::new("mixed", "outer-boundary").unwrap();
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

The wrapper requires a valid message/* or text/rfc822-headers Content-Type,
except for an implicit message/rfc822 child written through a multipart/digest
parent, and an absent, 7bit, 8bit, or binary transfer encoding. Byte-range
restrictions are not checked.
Base64/quoted-printable/UU wrapper encoding, including for message/global,
is not implemented and returns an error rather than misleading output.
Use opaque encoded leaves for these representations. Inner leaf transfer
conversion remains supported. No metadata or Content-Disposition is generated
beyond the constructor's Content-Type, and Bcc fields are not hidden.

`MimeEntity::load` and `MimeMessage::load` recognize message/rfc822, news,
global, global-headers, external-body, and rfc2822, plus text/rfc822-headers,
ignoring case, when the wrapper encoding is absent, 7bit, 8bit, or binary.
Each becomes a `MessagePart`
whose inner message owns its non-Content headers and whose body owns its
Content-* headers. Input, header, and depth limits apply throughout the tree;
each message-part wrapper and embedded body root counts toward entity depth.
No payload means `message: None`; a present message requires its own complete
header/body separator. Delimiter-only blank lines do not create messages.

Set `MimeEntityParseOptions { parse_message_parts: false, ..Default::default() }`
to preserve the previous opaque behavior. Encoded wrappers, unrecognized
message subtypes remain opaque regardless of this flag. Returned-header parts
share the strict inner separator requirement described above. To interpret an
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
This supplies range-backed content storage, not automatic MIME-body discovery
or a complete persistent message parser.

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

The default per-record size limit is 16 KiB, rather than MimeKit's 16 MiB.
Set a positive limit with `HeaderParser::with_max_header_length(NonZeroUsize)`. Field, colon, value,
folded lines, and newline bytes count toward it; the body separator does not.
An oversized record produces `HeaderTooLong` before buffering beyond the
limit. Already-emitted records remain available, but the block must be
abandoned: errors are latched until `reset`, not retryable. Reset retains the
configured limit. Callers must separately limit total bytes and record counts.

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
for UU, or `None` for `Default`. Empty or unrecognized input returns
`UnknownContentEncoding`, never a success-shaped `Default`.

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
`MessageId::from_value` for MimeKit's setter-style checks. ID generation is not
implemented.
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
possible, and no length limit or header formatter is supplied. `FromStr` remains
the tolerant reading API; it does not implicitly opt into assignment validation.

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
can still contain controls. SMTPUTF8 output remains deferred.

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
International local parts are still rejected; this is not SMTPUTF8 output,
SMTP address-length validation, or decoded-name sanitization.

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

For compatibility, the header-scanning decoder preserves MimeKit's quirk of
discarding a leading `=` when the first line after `=ybegin` begins with `=`
followed by a byte other than `y`. This can misdecode an escaped first payload
byte when there is no intervening `=ypart` header. Payload-only mode does not
have this quirk.

Both codecs expose `checksum() -> u32`: MimeKit's **uncomplemented** running
CRC-32 state, initially `u32::MAX`. Use `!codec.checksum()` for the finalized
value used in a yEnc trailer. Only original/decoded bytes affect it, not line
endings, escapes, or headers. Cloning preserves the checksum and all streaming
state; resetting clears them while retaining configuration or scanning mode.
The implementation is safe scalar Rust with a static CRC table.

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
and decoding are available separately in `mimecraft::rfc2047`; folding
unstructured fields and address headers remains unported.

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
longer than one header line. Complete unstructured/address-field folding
and comment encoding are still unported. Do not use these
helpers for addr-specs or MIME parameters; Content-Type and Content-Disposition
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
};
let header = disposition.to_header_string_with_options(&options)?;
assert!(header.ends_with('\n'));
assert!(!header.contains('\r'));
# Ok::<(), mimecraft::HeaderFormatError>(())
```

Ordinary values are tokens or escaped quoted strings; empty values are `""`.
Unicode, ASCII controls (including CR, LF, tab, DEL, and NUL), and literal `=?`
sequences use RFC 2231 percent encoding. This keeps authored encoded-word text
literal when reparsed instead of accidentally interpreting it as RFC 2047.
Long values use numbered continuations. Escaped quotes, whitespace, complete
percent triplets, and Unicode scalar boundaries are preserved. Encoding does
not remove controls from the decoded value or make a filename safe to use.

Charset selection follows MimeKit's automatic choice with UTF-8 as its Unicode
default: printable ASCII uses `us-ascii`, controls and characters through U+00FF
use `iso-8859-1`, and other Unicode uses `utf-8`. The selection considers the
entire parameter value and never substitutes unrepresentable characters.
It is independent of a Content-Type `charset` parameter, which describes the
body rather than parameter text. Extended continuations consistently use
percent encoding; long ordinary ASCII values use unencoded, optionally quoted
sections. Original language tags and declared charsets are not reproduced.

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
bytes; use `Header::raw()` for the latter. RFC 2047 parameter output, raw
internationalized headers and explicit/per-parameter encoding preferences
remain unported.

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
folded ASCII serialization with RFC 2231 parameters. Single/text/multipart
ownership and writing, complete entity-tree loading, a message wrapper
with raw header partitioning and load/write APIs, and explicit attached-message
ownership and writing are also implemented.
MimeKit.Cryptography, including DKIM, ARC, and DMARC, is excluded.
`LegacyMimeParser` and its associated event-argument types will not be ported;
parser behavior is being adapted from the current `MimeReader` implementation.

Async adapters remain deferred while message parsing and serialization APIs
stabilize. Parsing and
serialization state should remain independent of blocking I/O so synchronous
and future optional async drivers can share the same processing logic.

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
Single-part writing follows `MimePart.WriteContent` and `MimeWriter.cs`, reusing
the existing transfer codecs, newline filters, and normalized header writer.
Stored/requested encoding comparisons, binary pass-through, UU wrappers, and
empty-body newline handling are retained. Options and required metadata errors
are explicit, UU filename controls are rejected, and truncated header records
are terminated in output to keep the separator distinct. Signature verification,
configurable message-level formatting, and transport preparation are not implemented.
Text parts adapt `TextPart.cs`'s subtype construction, explicit charset text
access, and text/charset replacement. Composition over `MimePart<R>` replaces
inheritance, while in-memory-only setters respect Rust source types. Existing
strict charset conversion and checked header formatting make failed edits
atomic. Unsupported labels and malformed bytes remain errors, and newlines/BOMs
are preserved rather than applying MimeKit's detection, fallback, or newline
normalization. Automatic detection and text-format conversion are not ported.
Multipart ownership adapts `Multipart.cs` and `MimeEntity.cs` using an enum and
owned child vectors instead of inheritance and parent/event machinery.
Raw headers remain authoritative; checked edits implement the source's boundary
validation TODO using RFC 2046 section 5.1.1. Caller-supplied boundaries replace
random generation for now. Raw preamble/epilogue bytes avoid implicit string
decoding, and iterative leaf traversal avoids recursive enumeration. Parsed
delimiter preservation is not ported.
Multipart writing follows `MimeWriter.cs`'s explicit frames and
`MimeEntity.EndsWithDelimiterNewLine` ownership rules for complete children.
Mandatory leaf content and the absence of truncated containers mean every
child receives an independent delimiter newline. Rust epilogues exclude the
closing delimiter's line ending, so that ending is always emitted explicitly.
Nonempty preambles are terminated even for empty containers; invalid container
encodings and overlapping ancestor boundaries produce explicit errors instead
of ambiguous output. Existing leaf serializers and newline filters are reused;
body collision scanning and signed/truncated-container special cases are deferred.
Entity loading adapts `MimeReader.cs`'s iterative construction, whole-line
boundary recognition, and withheld delimiter-newline semantics, plus
`MimeParser.cs`'s raw headers and stored-encoding content construction.
In-memory cursors replace the source's memory-block/persistent-stream choice.
Preamble endings are retained; nested epilogues exclude parent framing and
start after their own closing marker's newline. Explicit limits and errors
replace recovery for truncated or invalid containers in this first loader.
Empty content always has a stream; absent-content and raw-delimiter metadata
are not modeled. LF serialization adds an independent CRLF delimiter prefix
after a bare body/epilogue CR to avoid losing that byte on reload.
Message ownership and I/O adapt `MimeMessage.cs`'s separate message/body header
lists, optional body, and combined header output, plus `Header.cs`'s
case-insensitive Content-* classification. Existing entity I/O is reused
without copying payloads during header partitioning. Explicit constructors
replace clock/random-ID defaults, and serialization does not insert
MIME-Version or hide Bcc. Header offsets are not stored, so grouped output
replaces the source's original-order merge. Cached typed properties, change
events and automatic preparation are deferred.
Attached-message ownership adapts `MessagePart.cs` with optional owned messages
and an internal box instead of inheritance and disposal tracking.
`MimeWriter.cs`'s iterative descent into message bodies is integrated with the
existing tree writer, retaining active multipart boundaries across wrappers.
Checked raw metadata and transparent wrapper encodings prevent writing an
unencoded message under an encoded-transfer header. Leaf traversal includes
attached bodies. Automatic loading adapts `MimeReader.cs`'s recognized message
subtypes, transparent-wrapper classification, and absent-message handling.
A shared iterative stack retains multipart ancestors through message frames;
header partitioning reuses the root-message loader's helper. Bounded line
lookahead keeps delimiter framing out of embedded header separators. Explicit
errors replace depth-limit fallback and incomplete-message recovery.
Returned-header containers adapt `TextRfc822Headers.cs`,
`UnitTests/TextRfc822HeadersTests.cs`, and the text/rfc822-headers and
message/global-headers classification in `ParserOptions.cs` and `MimeReader.cs`.
Composition replaces inheritance and visitor dispatch while reusing
`MimeWriter.cs`'s ordinary message-body traversal: existing bodies are not
discarded. The text media-type exception is explained inline and shares
validation between metadata edits and writing. Transparent parsing uses the
same bounded message frames; encoded wrappers remain opaque. Unlike the
source's recovery paths, present messages still require an inner separator.
Partial-message metadata and joining adapt `MessagePartial.cs`,
`UnitTests/MessagePartialTests.cs`, and the RFC 2046 fixtures in
`UnitTests/TestData/partial`. Number-validation, stream-chaining, and RFC header
merging comments are preserved/adapted. Composition and decoded borrowed
readers replace inheritance and disposable chained streams. Explicit errors
distinguish malformed numbers from missing parameters, and missing IDs are
rejected. Shared parser limits bound the reassembled decoded message; no
intermediate concatenated message buffer is required. Source-offset clearing,
visitor dispatch, and disposal comments do not apply to the Rust model.
Automatic splitting and ID generation are not yet ported.
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
`MultipartAlternative.GetText`, reusing strict `TextPart` charset decoding
rather than MimeKit's sniffing/fallback behavior. `Text/FlowedToText.cs` and
`UnitTests/Text/FlowedToTextTests.cs` provide the paragraph, stuffing,
quote-depth, delsp, and final-soft-line rules. Deterministic CRLF/LF options
replace platform-dependent writer newlines; whole-string conversion omits
the C# converter hierarchy, stream adapters, and header/footer decoration.
The source's signature-separator handling is preserved and documented.
Delivery-status groups adapt `MessageDeliveryStatus.cs`,
`MimeParser.ParseStatusGroup`, `MimeReader.SkipBlankLines`, and
`UnitTests/MessageDeliveryStatusTests.cs`; `ParserOptions.cs` supplies the
global-delivery-status classification. Detached `HeaderList` values and explicit
atomic replacement replace mutable cached collections and event-driven writes.
Resource limits and explicit errors replace swallowed parse failures, while
retained inner encoding declarations cause re-encoding on replacement rather
than stale metadata over plain recipient groups.
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
comment is retained/adapted where first-child roots remove start. Explicit
Content-Ids replace automatic generation, atomic checked metadata edits replace
cached parameter updates, and replaced children are returned to the caller.
URI-based resource lookup/opening and generated IDs are not yet ported;
visitor dispatch and disposal invariants do not apply to the Rust model.
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
paths complete the ASCII path. Internationalized output remains deferred.
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
extended sections consistently use percent encoding. Explicit/per-parameter
encoding preferences and RFC 2047 parameter output are not yet ported.
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
Other CPUs use the scalar implementation.
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
SIMD module and forbids unsafe code in mimecraft itself. This setting does not
disable acceleration or unsafe code inside dependencies such as `encoding_rs`:

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

### Benchmarks

```text
cargo bench --bench base64 --no-default-features
cargo bench --bench base64
```

The benchmark, which needs no extra benchmarking framework, compares scalar-only and runtime-dispatched
builds. It also retains the original match-based decoder as a baseline for the
rank table. Workloads use a deterministic 1 MiB payload, reused output buffers,
and 1-byte, line-sized, and 4 KiB chunks; decoding includes unwrapped, MIME-wrapped,
and noisy input. MIME-wrapped input uses CRLF; unwrapped input removes both CR
and LF. Rates count input bytes and report the median of five timed
samples after warm-up. These are in-memory microbenchmarks, not message-parser or
end-to-end I/O throughput, and results depend on the CPU and compiler.

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

Licensed under MIT, retaining the original MimeKit copyright notice in
`LICENSE`.
