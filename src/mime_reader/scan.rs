// Adapted from MimeKit's MimeReader.SkipNonBoundaryLines routines.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
#[path = "scan/neon.rs"]
mod simd;
#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
#[path = "scan/x86.rs"]
mod simd;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Scan {
    pub len: usize,
    pub lines: usize,
    // Counts, rather than flags, let the caller exclude a withheld delimiter
    // newline without attributing its format to the preceding body.
    pub dos: usize,
}

impl Scan {
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    #[inline(always)]
    fn push_masks(&mut self, mut lf: u32, mut crlf: u32, candidates: u32) -> bool {
        if candidates != 0 {
            let index = candidates.trailing_zeros();
            // Keep the newline preceding the first candidate, including the
            // last lane. For index 31 the mask must be all ones, not a shift
            // by 32 (MimeKit obtains this through unsigned wraparound).
            let keep = u32::MAX >> (31 - index);
            lf &= keep;
            crlf &= keep;
            self.len += index as usize + 1;
        }
        self.lines += lf.count_ones() as usize;
        self.dos += crlf.count_ones() as usize;
        candidates != 0
    }

    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    fn finish(mut self, input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Self {
        // Like MimeKit's scalar helper, this normally handles only the short
        // SIMD tail. Borrowed Rust slices have no writable newline sentinel.
        let tail = scalar(&input[self.len..], multipart, mbox, cr);
        self.len += tail.len;
        self.lines += tail.lines;
        self.dos += tail.dos;
        self
    }
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(
    not(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    )),
    derive(Default)
)]
pub(super) struct Scanner {
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    accelerator: Option<simd::Accelerator>,
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
impl Default for Scanner {
    fn default() -> Self {
        Self {
            accelerator: simd::Accelerator::detect(),
        }
    }
}

impl Scanner {
    // The caller checks the first line. Only "\n--" and, in mbox mode,
    // "\nFr" can introduce markers; full delimiter validation stays outside
    // this non-validating fast path, as in MimeKit's ScanContent.
    //
    // Unlike MimeKit's complete-line skip, consume an ordinary incomplete
    // final line too, so long lines continue to stream without copying.
    pub(super) fn scan(self, input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Scan {
        // Skip the initial newline-free run with the bulk byte-search routine.
        // No candidates or line counts can occur there; starting the fused
        // scan at its first LF avoids rescanning that run even for long lines.
        let Some(first) = memchr::memchr(b'\n', input) else {
            return Scan {
                len: input.len(),
                ..Scan::default()
            };
        };
        let cr = if first == 0 {
            cr
        } else {
            input[first - 1] == b'\r'
        };
        let mut scanned = self.scan_lines(&input[first..], multipart, mbox, cr);
        scanned.len += first;
        scanned
    }

    fn scan_lines(self, input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Scan {
        #[cfg(all(
            feature = "simd",
            any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
        ))]
        if let Some(accelerator) = self.accelerator {
            return accelerator.scan(input, multipart, mbox, cr);
        }
        scalar(input, multipart, mbox, cr)
    }
}

fn scalar(input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Scan {
    let mut scan = Scan {
        len: input.len(),
        ..Scan::default()
    };
    for index in memchr::memchr_iter(b'\n', input) {
        scan.lines += 1;
        scan.dos += usize::from(if index == 0 {
            cr
        } else {
            input[index - 1] == b'\r'
        });
        let next = &input[index + 1..];
        // A one-byte prefix at a refill/chunk boundary is still ambiguous.
        // Leave it for the line scanner rather than emitting half a marker.
        if (multipart && next.first() == Some(&b'-') && next.get(1).is_none_or(|&b| b == b'-'))
            || (mbox && next.first() == Some(&b'F') && next.get(1).is_none_or(|&b| b == b'r'))
        {
            scan.len = index + 1;
            break;
        }
    }
    scan
}
