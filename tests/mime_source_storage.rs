// Adapted from MimeKit's MimeParserTests.cs storage and stream-lifetime tests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, BufRead, BufReader, Cursor, Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use mimecraft::{
    MimeContentStorage, MimeContentStream, MimeEntity, MimeEntityParseOptions, MimeMessage,
    MimeParser, io::SharedStream,
};

#[derive(Default)]
struct Observed {
    reads: AtomicUsize,
    seeks: AtomicUsize,
    drops: AtomicUsize,
    read_error: AtomicBool,
    seek_error: AtomicBool,
    wrong_seek: AtomicBool,
}

struct Source {
    bytes: Cursor<Vec<u8>>,
    observed: Arc<Observed>,
    chunk: usize,
    failure_at: Option<u64>,
}

impl Source {
    fn new(bytes: impl Into<Vec<u8>>, chunk: usize) -> (Self, Arc<Observed>) {
        let observed = Arc::new(Observed::default());
        (
            Self {
                bytes: Cursor::new(bytes.into()),
                observed: observed.clone(),
                chunk,
                failure_at: None,
            },
            observed,
        )
    }
}

impl Read for Source {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.observed.reads.fetch_add(1, Ordering::SeqCst);
        if self.observed.read_error.load(Ordering::SeqCst) {
            return Err(io::Error::other("source read failure"));
        }
        let mut count = bytes.len().min(self.chunk);
        if let Some(offset) = self.failure_at {
            if self.bytes.position() >= offset {
                return Err(io::Error::other("partial source read failure"));
            }
            count = (count as u64).min(offset - self.bytes.position()) as usize;
        }
        self.bytes.read(&mut bytes[..count])
    }
}

impl Seek for Source {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.observed.seeks.fetch_add(1, Ordering::SeqCst);
        if self.observed.seek_error.load(Ordering::SeqCst) {
            return Err(io::Error::other("source seek failure"));
        }
        let position = self.bytes.seek(from)?;
        if self.observed.wrong_seek.load(Ordering::SeqCst) && matches!(from, SeekFrom::Start(_)) {
            Ok(position + 1)
        } else {
            Ok(position)
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        self.observed.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn body<R: Read + Seek>(message: &mut MimeMessage<R>) -> &mut R {
    let Some(MimeEntity::Part(part)) = message.body_mut() else {
        panic!("expected leaf body");
    };
    part.content_mut().get_mut()
}

fn read_body<R: Read + Seek>(message: &mut MimeMessage<R>) -> Vec<u8> {
    let body = body(message);
    body.rewind().unwrap();
    let mut bytes = Vec::new();
    body.read_to_end(&mut bytes).unwrap();
    bytes
}

#[derive(Clone, Copy, Debug)]
enum StorageLoader {
    Message,
    MessageOptions,
    Entity,
    EntityOptions,
}

const STORAGE_LOADERS: [StorageLoader; 4] = [
    StorageLoader::Message,
    StorageLoader::MessageOptions,
    StorageLoader::Entity,
    StorageLoader::EntityOptions,
];

impl StorageLoader {
    fn load<R: Read + Seek>(
        self,
        source: R,
        storage: MimeContentStorage,
        options: &MimeEntityParseOptions,
    ) -> io::Result<MimeMessage<MimeContentStream<R>>> {
        match self {
            Self::Message => MimeMessage::load_with_storage(source, storage),
            Self::MessageOptions => {
                MimeMessage::load_with_storage_and_options(source, storage, options)
            }
            // Wrap entities only to reuse the same leaf-content assertions.
            Self::Entity => {
                MimeEntity::load_with_storage(source, storage).map(MimeMessage::with_body)
            }
            Self::EntityOptions => {
                MimeEntity::load_with_storage_and_options(source, storage, options)
                    .map(MimeMessage::with_body)
            }
        }
    }
}

#[test]
fn storage_loaders_keep_owned_sources_until_content_is_released() {
    let prefix = b"prefix";
    let headers = b"Content-Type: text/plain\r\n\r\n";
    let bytes = b"original body";
    for api in STORAGE_LOADERS {
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let input = [prefix.as_slice(), headers, bytes].concat();
            let end = input.len() as u64;
            let (mut source, observed) = Source::new(input, 7);
            source.bytes.set_position(prefix.len() as u64);
            let mut message = api
                .load(source, storage, &MimeEntityParseOptions::default())
                .unwrap();
            assert_eq!(
                observed.drops.load(Ordering::SeqCst),
                usize::from(storage == MimeContentStorage::Memory),
                "{api:?}/{storage:?}",
            );
            match body(&mut message) {
                MimeContentStream::Memory(cursor) => {
                    assert_eq!(storage, MimeContentStorage::Memory);
                    assert_eq!(cursor.position(), 0);
                    assert_eq!(cursor.get_ref(), bytes);
                }
                MimeContentStream::Source(range) => {
                    assert_eq!(storage, MimeContentStorage::Source);
                    assert_eq!(
                        range.start_boundary(),
                        (prefix.len() + headers.len()) as u64
                    );
                    assert_eq!(range.end_boundary(), Some(end));
                    assert_eq!(range.position(), 0);
                }
            }
            let reads = observed.reads.load(Ordering::SeqCst);
            assert_eq!(read_body(&mut message), bytes);
            assert_eq!(
                observed.reads.load(Ordering::SeqCst) > reads,
                storage == MimeContentStorage::Source,
            );
            drop(message);
            assert_eq!(observed.drops.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn storage_loaders_leave_borrowed_sources_open() {
    let input = b"Content-Type: text/plain\r\n\r\noriginal body";
    for api in STORAGE_LOADERS {
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let (mut source, observed) = Source::new(input.to_vec(), 1);
            let mut message = api
                .load(&mut source, storage, &MimeEntityParseOptions::default())
                .unwrap();
            assert_eq!(read_body(&mut message), b"original body");
            drop(message);
            assert_eq!(observed.drops.load(Ordering::SeqCst), 0);
            source.rewind().unwrap();
            let mut original = Vec::new();
            source.read_to_end(&mut original).unwrap();
            assert_eq!(original, input);
            drop(source);
            assert_eq!(observed.drops.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn copying_loaders_remain_independent_of_borrowed_input() {
    for entity in [false, true] {
        let mut source = Cursor::new(b"Content-Type: text/plain\r\n\r\noriginal body".to_vec());
        let mut message = if entity {
            MimeMessage::with_body(MimeEntity::load(&mut source).unwrap())
        } else {
            MimeMessage::load(&mut source).unwrap()
        };
        let start = source.get_ref().len() - b"modified body".len();
        source.get_mut()[start..].copy_from_slice(b"modified body");
        drop(source);
        assert_eq!(read_body(&mut message), b"original body");

        let mut nonseekable = b"\r\nbody".as_slice();
        let mut message = if entity {
            MimeMessage::with_body(MimeEntity::load(&mut nonseekable).unwrap())
        } else {
            MimeMessage::load(&mut nonseekable).unwrap()
        };
        assert_eq!(read_body(&mut message), b"body");
    }
}

#[test]
fn storage_loader_failures_release_owned_but_not_borrowed_sources() {
    for api in STORAGE_LOADERS {
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            for seek_failure in [false, true] {
                let (source, observed) = Source::new(b"\r\nbody".to_vec(), 1);
                observed.seek_error.store(seek_failure, Ordering::SeqCst);
                observed.read_error.store(!seek_failure, Ordering::SeqCst);
                let error = api
                    .load(source, storage, &MimeEntityParseOptions::default())
                    .err()
                    .unwrap();
                assert_eq!(
                    error.to_string(),
                    if seek_failure {
                        "source seek failure"
                    } else {
                        "source read failure"
                    }
                );
                assert_eq!(observed.drops.load(Ordering::SeqCst), 1);
            }
            let (mut source, observed) = Source::new(b"\r\nbody".to_vec(), 1);
            source.failure_at = Some(4);
            let error = api
                .load(&mut source, storage, &MimeEntityParseOptions::default())
                .err()
                .unwrap();
            assert_eq!(error.to_string(), "partial source read failure");
            assert_eq!(observed.drops.load(Ordering::SeqCst), 0);
            assert_eq!(source.bytes.position(), 4);
            source.failure_at = None;
            source.rewind().unwrap();
            let mut recovered = api
                .load(&mut source, storage, &MimeEntityParseOptions::default())
                .unwrap();
            assert_eq!(read_body(&mut recovered), b"body");
            drop(recovered);
            drop(source);
            assert_eq!(observed.drops.load(Ordering::SeqCst), 1);

            let (source, observed) = Source::new(Vec::new(), 1);
            assert_eq!(
                api.load(source, storage, &MimeEntityParseOptions::default())
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::UnexpectedEof,
            );
            assert_eq!(observed.drops.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn storage_loaders_validate_and_apply_explicit_options() {
    for api in [StorageLoader::MessageOptions, StorageLoader::EntityOptions] {
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let (source, observed) = Source::new(Vec::new(), 1);
            let options = MimeEntityParseOptions {
                max_header_length: 0,
                ..Default::default()
            };
            assert_eq!(
                api.load(source, storage, &options).err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(observed.reads.load(Ordering::SeqCst), 0);
            assert_eq!(observed.seeks.load(Ordering::SeqCst), 0);
            assert_eq!(observed.drops.load(Ordering::SeqCst), 1);
            for limit in [5, 6] {
                let options = MimeEntityParseOptions {
                    max_input_bytes: limit,
                    ..Default::default()
                };
                assert_eq!(
                    api.load(Cursor::new(b"\r\nbody"), storage, &options)
                        .is_ok(),
                    limit == 6
                );
            }
            let options = MimeEntityParseOptions {
                parse_message_parts: false,
                ..Default::default()
            };
            let mut message = api
                .load(
                    Cursor::new(b"Content-Type: message/rfc822\r\n\r\nSubject: inner\r\n\r\nbody"),
                    storage,
                    &options,
                )
                .unwrap();
            assert_eq!(read_body(&mut message), b"Subject: inner\r\n\r\nbody");
        }
    }
}

#[test]
fn storage_loaders_preserve_header_partition_and_nested_content() {
    let input = b"Subject: outer\r\nContent-Type: message/rfc822\r\n\r\n\
        Subject: inner\r\nContent-Type: text/plain\r\n\r\nbody";
    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
        for api in STORAGE_LOADERS {
            let mut message = api
                .load(
                    Cursor::new(input),
                    storage,
                    &MimeEntityParseOptions::default(),
                )
                .unwrap();
            let body = message.body().unwrap();
            assert!(body.headers().contains("Content-Type"));
            let is_message = matches!(api, StorageLoader::Message | StorageLoader::MessageOptions);
            assert_eq!(message.headers().contains("Subject"), is_message);
            assert_eq!(body.headers().contains("Subject"), !is_message);
            let MimeEntity::Message(part) = body else {
                panic!("expected attached message");
            };
            let nested = part.message().unwrap();
            assert!(nested.headers().contains("Subject"));
            assert!(!nested.headers().contains("Content-Type"));
            assert!(!nested.body().unwrap().headers().contains("Subject"));
            let MimeEntity::Part(part) = nested.body().unwrap() else {
                panic!("expected leaf");
            };
            assert_eq!(
                matches!(part.content().get_ref(), MimeContentStream::Source(_)),
                storage == MimeContentStorage::Source
            );
            let mut output = Vec::new();
            message.write_to(&mut output).unwrap();
            assert_eq!(output, input);
        }
    }
}

#[test]
fn storage_keeps_ranges_or_copies_and_releases_last_owner() {
    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
        let bytes = vec![b'x'; 128 * 1024];
        let mut input = b"Subject: large\r\n\r\n".to_vec();
        let start = input.len() as u64;
        input.extend_from_slice(&bytes);
        let (source, observed) = Source::new(input, 4096);
        let mut parser = MimeParser::with_storage(source, storage).unwrap();
        assert_eq!(parser.storage(), storage);
        let mut message = parser.parse_message().unwrap();
        match body(&mut message) {
            MimeContentStream::Memory(cursor) => {
                assert_eq!(storage, MimeContentStorage::Memory);
                assert_eq!(cursor.get_ref(), &bytes);
            }
            MimeContentStream::Source(range) => {
                assert_eq!(storage, MimeContentStorage::Source);
                assert_eq!(range.start_boundary(), start);
                assert_eq!(range.end_boundary(), Some(start + bytes.len() as u64));
                assert_eq!(range.position(), 0);
            }
        }
        let reads = observed.reads.load(Ordering::SeqCst);
        drop(parser);
        assert_eq!(
            observed.drops.load(Ordering::SeqCst),
            usize::from(storage == MimeContentStorage::Memory)
        );
        assert_eq!(read_body(&mut message), bytes);
        assert_eq!(
            observed.reads.load(Ordering::SeqCst) > reads,
            storage == MimeContentStorage::Source
        );
        drop(message);
        assert_eq!(observed.drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn mbox_cursors_survive_interleaved_reads_replacement_and_drop() {
    for chunk in [1, 7, 4096] {
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let (source, observed) = Source::new(
                b"From first\nSubject: one\n\nfirst\nFrom second\nSubject: two\n\nsecond\nFrom third\n\nthird\n".to_vec(),
                chunk,
            );
            let mut parser = MimeParser::with_storage(source, storage).unwrap();
            let mut first = parser.parse_mbox_message().unwrap().unwrap();
            let mut byte = [0];
            body(&mut first).read_exact(&mut byte).unwrap();
            assert_eq!(&byte, b"f");
            let position = parser.position();
            let mut second = parser.parse_mbox_message().unwrap().unwrap();
            assert!(parser.position().offset > position.offset);
            body(&mut second).seek(SeekFrom::Start(2)).unwrap();
            body(&mut second).read_exact(&mut byte).unwrap();
            assert_eq!(&byte, b"c");
            body(&mut first).read_exact(&mut byte).unwrap();
            assert_eq!(&byte, b"i");
            let mut third = parser.parse_mbox_message().unwrap().unwrap();
            assert!(parser.parse_mbox_message().unwrap().is_none());
            assert_eq!(read_body(&mut third), b"third\n");
            assert_eq!(read_body(&mut second), b"second");

            let (mut replacement, replacement_observed) =
                Source::new(b"prefix\r\nnew".to_vec(), chunk);
            replacement.bytes.set_position(6);
            drop(parser.replace_storage_source(replacement).unwrap());
            assert_eq!(parser.position().offset, 0);
            assert!(!parser.is_end_of_stream());
            let mut new = parser.parse_message().unwrap();
            drop(parser);
            assert_eq!(read_body(&mut new), b"new");
            assert_eq!(read_body(&mut first), b"first");
            drop((first, second));
            assert_eq!(
                observed.drops.load(Ordering::SeqCst),
                usize::from(storage == MimeContentStorage::Memory)
            );
            drop(third);
            assert_eq!(observed.drops.load(Ordering::SeqCst), 1);
            drop(new);
            assert_eq!(replacement_observed.drops.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn borrowed_nonzero_buffered_source_and_generic_replacement() {
    let (mut source, observed) = Source::new(b"prefixSubject: one\r\n\r\nbody".to_vec(), 7);
    source.bytes.set_position(6);
    {
        let mut buffered = BufReader::with_capacity(11, &mut source);
        assert!(!buffered.fill_buf().unwrap().is_empty());
        let mut parser = MimeParser::with_storage(buffered, MimeContentStorage::Source).unwrap();
        let mut message = parser.parse_message().unwrap();
        assert_eq!(parser.position().offset, 20);
        drop(parser);
        assert_eq!(read_body(&mut message), b"body");
    }
    assert_eq!(observed.drops.load(Ordering::SeqCst), 0);
    source.rewind().unwrap();
    drop(source);
    assert_eq!(observed.drops.load(Ordering::SeqCst), 1);

    let mut parser =
        MimeParser::with_storage(Cursor::new(b"\r\nold".to_vec()), MimeContentStorage::Source)
            .unwrap();
    let mut old = parser.parse_message().unwrap();
    let mut replacement = Cursor::new(b"skip\r\nnew".to_vec());
    replacement.set_position(4);
    let mut buffered = BufReader::with_capacity(7, SharedStream::new(replacement).unwrap());
    assert!(!buffered.fill_buf().unwrap().is_empty());
    drop(parser.replace_source(buffered));
    let mut new = parser.parse_message().unwrap();
    drop(parser);
    assert_eq!(read_body(&mut old), b"old");
    assert_eq!(read_body(&mut new), b"new");
}

#[test]
fn decoded_bodies_keep_independent_cursors_across_threads() {
    let input = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n\
        --b\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nw=6Frld\r\n--b--\r\n";
    let (source, _) = Source::new(input.to_vec(), 1);
    let mut parser = MimeParser::with_storage(source, MimeContentStorage::Source).unwrap();
    let MimeEntity::Multipart(mut multipart) = parser.parse_entity().unwrap() else {
        panic!("expected multipart");
    };
    drop(parser);
    let mut threads = Vec::new();
    for expected in [b"hello", b"world"] {
        let MimeEntity::Part(mut part) = multipart.children_mut().remove(0) else {
            panic!("expected leaf");
        };
        threads.push(std::thread::spawn(move || {
            for _ in 0..50 {
                let mut decoded = Vec::new();
                part.content_mut().decode_to(&mut decoded).unwrap();
                assert_eq!(decoded, expected);
            }
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn rejected_replacement_preserves_mbox_state_and_old_bodies() {
    let (source, _) = Source::new(b"From first\n\none\nFrom second\n\ntwo".to_vec(), 7);
    let mut parser = MimeParser::with_storage(source, MimeContentStorage::Source).unwrap();
    let mut first = parser.parse_mbox_message().unwrap().unwrap();
    let position = parser.position();
    let pending = parser.pending_input().to_vec();
    let (replacement, observed) = Source::new(Vec::new(), 7);
    observed.seek_error.store(true, Ordering::SeqCst);
    assert!(parser.replace_storage_source(replacement).is_err());
    assert_eq!(parser.position(), position);
    assert_eq!(parser.pending_input(), pending);
    assert!(!parser.is_end_of_stream());
    assert_eq!(read_body(&mut first), b"one");
    let mut second = parser.parse_mbox_message().unwrap().unwrap();
    assert_eq!(read_body(&mut second), b"two");
}

#[test]
fn source_errors_and_limits_propagate_without_copy_fallback() {
    for logged in [false, true] {
        let (source, observed) = Source::new(b"\r\nbody".to_vec(), 1);
        let mut parser = MimeParser::with_storage(source, MimeContentStorage::Source).unwrap();
        observed.read_error.store(true, Ordering::SeqCst);
        let error = if logged {
            parser.parse_message_with_logger(&mut |_| Ok(()))
        } else {
            parser.parse_message()
        }
        .err()
        .unwrap();
        assert_eq!(error.to_string(), "source read failure");
        assert_eq!(
            parser.parse_message().err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
        let (replacement, observed) = Source::new(b"\r\nbody".to_vec(), 1);
        drop(parser.replace_storage_source(replacement).unwrap());
        let mut message = parser.parse_message().unwrap();
        observed.seek_error.store(true, Ordering::SeqCst);
        assert_eq!(
            body(&mut message).read(&mut [0]).unwrap_err().to_string(),
            "source seek failure"
        );
        assert_eq!(body(&mut message).stream_position().unwrap(), 0);
        observed.seek_error.store(false, Ordering::SeqCst);
        observed.wrong_seek.store(true, Ordering::SeqCst);
        assert!(body(&mut message).read(&mut [0]).is_err());
        assert_eq!(body(&mut message).stream_position().unwrap(), 0);
        observed.wrong_seek.store(false, Ordering::SeqCst);
        assert_eq!(read_body(&mut message), b"body");
        observed.read_error.store(true, Ordering::SeqCst);
        body(&mut message).rewind().unwrap();
        assert_eq!(
            body(&mut message).read(&mut [0]).unwrap_err().to_string(),
            "source read failure"
        );
        assert_eq!(body(&mut message).stream_position().unwrap(), 0);
    }

    let (source, observed) = Source::new(Vec::new(), 1);
    let options = MimeEntityParseOptions {
        max_header_length: 0,
        ..Default::default()
    };
    assert!(
        MimeParser::with_storage_and_options(source, MimeContentStorage::Source, options).is_err()
    );
    assert_eq!(observed.seeks.load(Ordering::SeqCst), 0);

    for limit in [5, 6] {
        let mut parser = MimeParser::with_storage_and_options(
            Cursor::new(b"\r\nbody"),
            MimeContentStorage::Source,
            MimeEntityParseOptions {
                max_input_bytes: limit,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(parser.parse_message().is_ok(), limit == 6);
    }
}

#[test]
fn source_logger_failure_recovers_by_replacing_source() {
    let mut parser = MimeParser::with_storage(
        Cursor::new(b"From first\n\none\nFrom second\nContent-Type: broken\n\ntwo".to_vec()),
        MimeContentStorage::Source,
    )
    .unwrap();
    let mut first = parser.parse_mbox_message().unwrap().unwrap();
    let error = parser
        .parse_mbox_message_with_logger(&mut |_| Err(io::Error::other("logger failed")))
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "logger failed");
    assert!(parser.parse_mbox_message().is_err());
    drop(
        parser
            .replace_storage_source(Cursor::new(b"\r\nnew".to_vec()))
            .unwrap(),
    );
    assert_eq!(read_body(&mut parser.parse_message().unwrap()), b"new");
    assert_eq!(read_body(&mut first), b"one");
}

#[test]
fn shared_stream_cursor_validation_and_poisoning() {
    let mut first = SharedStream::new(Cursor::new(b"abcdef")).unwrap();
    let mut second = first.clone();
    first.seek(SeekFrom::Start(2)).unwrap();
    second.seek(SeekFrom::End(-1)).unwrap();
    assert!(first.seek(SeekFrom::Current(-3)).is_err());
    assert_eq!(first.position(), 2);
    let mut byte = [0];
    first.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"c");
    second.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"f");
    first.seek(SeekFrom::Start(u64::MAX)).unwrap();
    assert!(first.seek(SeekFrom::Current(1)).is_err());
    assert!(first.read(&mut byte).is_err());
    assert_eq!(first.read(&mut []).unwrap(), 0);

    struct Panics(Cursor<Vec<u8>>);
    impl Seek for Panics {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.0.seek(from)
        }
    }
    impl Read for Panics {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("source panic")
        }
    }
    let mut source = SharedStream::new(Panics(Cursor::new(Vec::new()))).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.read(&mut [0]))).is_err()
    );
    assert_eq!(
        source.read(&mut [0]).unwrap_err().to_string(),
        "shared source lock poisoned"
    );
}

#[test]
fn partial_mbox_failure_preserves_previous_source_backed_message() {
    for logged in [false, true] {
        let mut input = b"From first\n\none\nFrom second\n\n".to_vec();
        let failure_at = input.len() as u64 + 64;
        input.extend_from_slice(&vec![b'x'; 8192]);
        let (mut source, observed) = Source::new(input, 7);
        source.failure_at = Some(failure_at);
        let mut parser = MimeParser::with_storage(source, MimeContentStorage::Source).unwrap();
        let mut first = parser.parse_mbox_message().unwrap().unwrap();
        let error = if logged {
            parser.parse_mbox_message_with_logger(&mut |_| Ok(()))
        } else {
            parser.parse_mbox_message()
        }
        .err()
        .unwrap();
        assert_eq!(error.to_string(), "partial source read failure");
        assert!(parser.parse_mbox_message().is_err());
        let (replacement, _) = Source::new(b"\r\nnew".to_vec(), 1);
        drop(parser.replace_storage_source(replacement).unwrap());
        assert_eq!(read_body(&mut parser.parse_message().unwrap()), b"new");
        drop(parser);
        assert_eq!(observed.drops.load(Ordering::SeqCst), 0);
        assert_eq!(read_body(&mut first), b"one");
        drop(first);
        assert_eq!(observed.drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn empty_encoded_bodies_match_copying_framing() {
    for encoding in ["base64", "quoted-printable", "x-uuencode"] {
        for ending in ["\r\n", "\r\n\r\n"] {
            let input = format!("Content-Transfer-Encoding: {encoding}{ending}");
            let mut copied = MimeParser::new(input.as_bytes()).parse_message().unwrap();
            let mut expected = Vec::new();
            copied.write_to(&mut expected).unwrap();
            let (source, observed) = Source::new(input.as_bytes().to_vec(), 1);
            let mut parser = MimeParser::with_storage(source, MimeContentStorage::Source).unwrap();
            let mut message = parser.parse_message().unwrap();
            let Some(MimeEntity::Part(part)) = message.body() else {
                panic!("expected leaf body");
            };
            let MimeContentStream::Source(range) = part.content().get_ref() else {
                panic!("source storage must retain a range even for empty content");
            };
            assert_eq!(range.end_boundary(), Some(range.start_boundary()));
            let reads = observed.reads.load(Ordering::SeqCst);
            drop(parser);
            let mut output = Vec::new();
            message.write_to(&mut output).unwrap();
            assert_eq!(output, expected);
            assert_eq!(output, input.as_bytes());
            assert_eq!(observed.reads.load(Ordering::SeqCst), reads);
        }
    }
}
