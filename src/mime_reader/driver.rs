// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead};

use super::{
    EventSink, MimeEntityParseOptions, MimePosition,
    engine::{MarkerParser, ReadContext, ScanBuffer, TreeEnd, TreeParser},
};

fn drive<R: BufRead + ?Sized, T>(
    reader: &mut R,
    mut parse: impl FnMut(&mut &[u8], bool) -> io::Result<Option<T>>,
) -> io::Result<T> {
    loop {
        let input = match reader.fill_buf() {
            Ok(input) => input,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let length = input.len();
        let mut remaining = input;
        let result = parse(&mut remaining, input.is_empty());
        let consumed = length - remaining.len();
        reader.consume(consumed);
        if let Some(result) = result? {
            return Ok(result);
        }
        debug_assert_eq!(consumed, length);
        debug_assert_ne!(length, 0, "explicit EOF must finish or fail");
    }
}

pub(super) fn read_mbox_marker<R: BufRead + ?Sized, F: EventSink>(
    reader: &mut R,
    options: &MimeEntityParseOptions,
    position: &mut MimePosition,
    buffer: &mut ScanBuffer,
    initial: bool,
    callback: &mut F,
) -> io::Result<Option<usize>> {
    let mut parser = MarkerParser::new(options, *position, buffer, initial)?;
    drive(reader, |input, eof| {
        let result = parser.parse(input, eof, options, buffer, callback);
        *position = parser.position();
        result
    })
}

pub(super) fn read_tree<R: BufRead + ?Sized, F: EventSink>(
    reader: &mut R,
    options: &mut ReadContext<'_>,
    position: &mut MimePosition,
    buffer: &mut ScanBuffer,
    mbox: bool,
    remaining: usize,
    callback: &mut F,
) -> io::Result<TreeEnd> {
    let mut parser = TreeParser::new(options, *position, buffer, mbox, remaining);
    drive(reader, |input, eof| {
        let result = parser.parse(input, eof, options, buffer, callback);
        *position = parser.position();
        result
    })
}
