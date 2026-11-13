// Adapted from MimeKit's MimeBoundaryIndex.cs and MimeReader.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::BTreeMap, ops::Deref, sync::OnceLock};

use super::{Frame, boundary_line};

const INDEX_THRESHOLD: usize = 4;

#[derive(Default)]
pub(super) struct Frames {
    pub entries: Vec<Frame>,
    // Frame index and maximum marker length through this active ancestor.
    active: Vec<(usize, usize)>,
    #[allow(clippy::box_collection)] // Keep the inline parser state compact.
    markers: OnceLock<Box<BTreeMap<Vec<u8>, Vec<usize>>>>,
}

impl Deref for Frames {
    type Target = [Frame];

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl Frames {
    pub fn has_boundary(&self) -> bool {
        !self.active.is_empty()
    }

    pub fn max_boundary_length(&self) -> usize {
        self.active.last().map_or(0, |&(_, length)| length)
    }

    pub fn push(&mut self, frame: Frame) {
        let index = self.entries.len();
        if let Frame::Multipart {
            boundary: Some(boundary),
            closed: false,
            ..
        } = &frame
        {
            let length = self.max_boundary_length().max(boundary.len() + 4);
            self.active.push((index, length));
        }
        self.entries.push(frame);
        if self.active.last().is_none_or(|&(last, _)| last != index) {
            return;
        }
        if let Some(markers) = self.markers.get_mut() {
            let Frame::Multipart {
                boundary: Some(boundary),
                ..
            } = &self.entries[index]
            else {
                unreachable!("active multipart has a boundary");
            };
            if let Some(indices) = markers.get_mut(boundary.as_bytes()) {
                indices.push(index);
            } else {
                markers.insert(boundary.as_bytes().to_vec(), vec![index]);
            }
        }
    }

    fn remove_boundary(&mut self) {
        let index = self.entries.len() - 1;
        if self.active.last().is_none_or(|&(last, _)| last != index) {
            return;
        }
        self.active.pop();
        if self.active.len() == INDEX_THRESHOLD {
            self.markers.take();
        } else if let Some(markers) = self.markers.get_mut() {
            let Frame::Multipart {
                boundary: Some(boundary),
                ..
            } = &self.entries[index]
            else {
                unreachable!("active multipart has a boundary");
            };
            let indices = markers
                .get_mut(boundary.as_bytes())
                .expect("indexed boundary");
            indices.pop();
            if indices.is_empty() {
                markers.remove(boundary.as_bytes());
            }
        }
    }

    pub fn pop(&mut self) -> Option<Frame> {
        if self.entries.is_empty() {
            return None;
        }
        self.remove_boundary();
        self.entries.pop()
    }

    pub fn close_boundary(&mut self) {
        self.remove_boundary();
        let Some(Frame::Multipart { closed, .. }) = self.entries.last_mut() else {
            unreachable!("a matched boundary belongs to a multipart");
        };
        *closed = true;
    }

    pub fn find(&self, line: &[u8]) -> Option<(usize, bool)> {
        // Reject ordinary lines before consulting ancestors. Embedded messages
        // without multipart boundaries must not turn this into a depth walk.
        let marker = line.strip_prefix(b"--")?;
        let &(nearest, _) = self.active.last()?;
        let matches = |index| {
            let Frame::Multipart {
                boundary: Some(boundary),
                ..
            } = &self.entries[index]
            else {
                unreachable!("active multipart has a boundary");
            };
            boundary_line(line, boundary).map(|close| (index, close))
        };
        if let Some(found) = matches(nearest) {
            return Some(found);
        }
        if self.active.len() <= INDEX_THRESHOLD {
            // A bounded walk avoids index overhead for shallow multiparts.
            return self
                .active
                .iter()
                .rev()
                .skip(1)
                .find_map(|&(index, _)| matches(index));
        }
        // Even deep trees often match only their nearest boundary. Build the
        // ancestor index only when that fast path actually misses.
        let markers = self.markers.get_or_init(|| {
            let mut markers: BTreeMap<Vec<u8>, Vec<usize>> = BTreeMap::new();
            for &(index, _) in &self.active {
                let Frame::Multipart {
                    boundary: Some(boundary),
                    ..
                } = &self.entries[index]
                else {
                    unreachable!("active multipart has a boundary");
                };
                markers
                    .entry(boundary.as_bytes().to_vec())
                    .or_default()
                    .push(index);
            }
            Box::new(markers)
        });
        // Strip transport whitespace once, rather than once per ancestor.
        let length = marker
            .iter()
            .rposition(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
            .map_or(0, |i| i + 1);
        let marker = &marker[..length];
        let opening = markers
            .get(marker)
            .and_then(|indices| indices.last())
            .copied();
        let closing = marker
            .strip_suffix(b"--")
            .and_then(|marker| markers.get(marker))
            .and_then(|indices| indices.last())
            .copied();
        // A closing marker may also open another boundary. The innermost
        // active match wins, regardless of which form matched.
        if let Some(closing) = closing
            && opening.is_none_or(|opening| closing > opening)
        {
            return Some((closing, true));
        }
        opening.map(|index| (index, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(frames: &Frames) {
        let active: Vec<_> = frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| {
                if let Frame::Multipart {
                    boundary: Some(boundary),
                    closed: false,
                    ..
                } = frame
                {
                    Some((index, boundary))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(frames.has_boundary(), !active.is_empty());
        assert_eq!(
            frames.max_boundary_length(),
            active.iter().map(|(_, b)| b.len() + 4).max().unwrap_or(0)
        );
        for boundary in ["", "a", "a--", "a----", "ab", "a ", "a\t", "missing"] {
            for suffix in ["", "--", " \t\r\n", "-- \t\r\n", "-bad\r\n"] {
                let line = format!("--{boundary}{suffix}");
                let expected = active.iter().rev().find_map(|&(index, boundary)| {
                    boundary_line(line.as_bytes(), boundary).map(|close| (index, close))
                });
                assert_eq!(frames.find(line.as_bytes()), expected, "{line:?}");
            }
        }
        assert_eq!(frames.find(b"ordinary line\r\n"), None);
    }

    #[test]
    fn duplicate_overlapping_boundaries_and_threshold_crossings_match_stack_walk() {
        let mut frames = Frames::default();
        let mut random = 0x324b_f017_aa05_1241u64;
        for _ in 0..3000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            if frames.len() > 40 || random.is_multiple_of(5) {
                frames.pop();
            } else if random.is_multiple_of(7) {
                if matches!(frames.last(), Some(Frame::Multipart { closed: false, .. })) {
                    frames.close_boundary();
                }
            } else if random.is_multiple_of(3) {
                frames.push(Frame::Message { begun: true });
            } else {
                let boundary = ["a", "a--", "a----", "ab", "a ", "a\t", ""][random as usize % 7];
                frames.push(Frame::Multipart {
                    boundary: (!random.is_multiple_of(11)).then(|| boundary.to_owned()),
                    closed: false,
                    digest: false,
                });
            }
            check(&frames);
        }
        while frames.pop().is_some() {
            check(&frames);
        }
        assert!(frames.markers.get().is_none());
    }
}
