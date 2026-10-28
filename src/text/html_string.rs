// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{fmt, mem, ops::Deref};

use compact_str::CompactString;

const INLINE_CAPACITY: usize = size_of::<CompactString>();

/// Immutable token contents, avoiding allocations for short text and values.
#[derive(Clone, Eq, Hash, PartialEq)]
pub(super) struct HtmlString(CompactString);

impl HtmlString {
    pub(super) fn take(text: &mut String) -> Self {
        if text.len() <= INLINE_CAPACITY {
            // Copy short contents inline, retaining the scanner's scratch
            // allocation. Longer contents transfer their buffer instead.
            let value = Self(CompactString::new(text.as_str()));
            text.clear();
            value
        } else {
            Self(CompactString::from(mem::take(text)))
        }
    }
}

impl From<String> for HtmlString {
    fn from(text: String) -> Self {
        Self(CompactString::from(text))
    }
}

impl From<&str> for HtmlString {
    fn from(text: &str) -> Self {
        Self(CompactString::new(text))
    }
}

impl Deref for HtmlString {
    type Target = str;

    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for HtmlString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

#[cfg(test)]
mod tests {
    use std::hash::{DefaultHasher, Hash, Hasher};

    use super::*;

    #[test]
    fn contents_are_independent_of_construction_and_storage() {
        for text in [
            String::new(),
            "x".repeat(INLINE_CAPACITY - 1),
            "x".repeat(INLINE_CAPACITY),
            "x".repeat(INLINE_CAPACITY + 1),
            format!("{}日", "x".repeat(INLINE_CAPACITY - 3)),
            format!("{}日", "x".repeat(INLINE_CAPACITY - 2)),
            "\0\r\n日本語".into(),
            "日".repeat(INLINE_CAPACITY),
        ] {
            let borrowed = HtmlString::from(text.as_str());
            let owned = HtmlString::from(text.clone());
            let mut scratch = text.clone();
            let taken = HtmlString::take(&mut scratch);
            assert!(scratch.is_empty());
            scratch.push_str("reused buffer");
            assert_eq!(&*borrowed, text);
            assert_eq!(borrowed, owned);
            assert_eq!(borrowed, taken);
            assert_eq!(borrowed, borrowed.clone());
            assert_eq!(borrowed.0.is_heap_allocated(), text.len() > INLINE_CAPACITY);
            assert_eq!(taken.0.is_heap_allocated(), text.len() > INLINE_CAPACITY);
            assert_eq!(format!("{borrowed:?}"), format!("{text:?}"));
            let hash = |value: &HtmlString| {
                let mut hasher = DefaultHasher::new();
                value.hash(&mut hasher);
                hasher.finish()
            };
            assert_eq!(hash(&borrowed), hash(&taken));
        }
    }

    #[test]
    fn taking_short_contents_reuses_scratch_allocation() {
        let mut scratch = String::with_capacity(128);
        scratch.push_str("short");
        let capacity = scratch.capacity();
        let value = HtmlString::take(&mut scratch);
        assert_eq!(&*value, "short");
        assert!(scratch.is_empty());
        assert_eq!(scratch.capacity(), capacity);
        assert!(!value.0.is_heap_allocated());
        assert_eq!(size_of::<HtmlString>(), size_of::<String>());
        assert_eq!(size_of::<Option<HtmlString>>(), size_of::<String>());
    }

    #[test]
    fn taking_long_contents_transfers_allocation() {
        let mut scratch = "long".repeat(INLINE_CAPACITY);
        let pointer = scratch.as_ptr();
        let value = HtmlString::take(&mut scratch);
        assert!(scratch.is_empty());
        assert_eq!(value.as_ptr(), pointer);
        assert!(value.0.is_heap_allocated());
    }

    #[test]
    fn tokenizer_and_owned_tokens_remain_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<super::super::HtmlTokenizer>();
        assert_send_sync::<super::super::HtmlToken>();
    }
}
