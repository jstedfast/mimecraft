// Adapted from MimeKit's MessageDispositionNotification.cs and MessageFeedbackReport.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, Cursor, Read, Seek},
    num::NonZeroUsize,
};

use crate::{HeaderList, MimeContent, MimePartWriteOptions, encodings::NewLineFormat};

pub(crate) fn read<R: Read + Seek>(
    content: &mut MimeContent<R>,
    max_decoded_bytes: usize,
    max_header_length: NonZeroUsize,
) -> io::Result<HeaderList> {
    // Decode the whole body so trailing data cannot evade the byte budget or
    // hide I/O failures, even though only the first header block is returned.
    let bytes = content.decode_to_vec_with_limit(max_decoded_bytes)?;
    HeaderList::load_with_max_header_length(&mut bytes.as_slice(), max_header_length)
}

pub(crate) fn write(
    fields: &HeaderList,
    format: NewLineFormat,
) -> io::Result<MimeContent<Cursor<Vec<u8>>>> {
    let newline = MimePartWriteOptions {
        new_line_format: format,
        ..Default::default()
    }
    .validate()?;
    let mut bytes = Vec::new();
    fields.write_terminated(&mut bytes, format)?;
    // MimeKit's HeaderList remembers its separator. Detached lists do not, so
    // terminate the field block explicitly, including when there are no fields.
    bytes.extend_from_slice(newline);
    Ok(MimeContent::new(Cursor::new(bytes)))
}
