// Adapted from MimeKit's MimeTypes.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::HashMap, error::Error, fmt};

use crate::mime_types_data::{EXTENSIONS, MIME_TYPES};

/// Filename MIME-type guesses and preferred extensions, with local overrides.
///
/// Uses MimeKit-derived mappings, including its October 29, 2026 refresh.
/// Legacy reverse aliases and ambiguous filename mappings remain.
/// Does not consult the OS, inspect file contents, access the filesystem, or
/// update from a registry at runtime. A filename guess is not validation of its
/// content.
///
/// Unlike MimeKit's global mutable dictionaries, registrations affect only this
/// instance. Clones retain independent copies of overrides; fresh instances
/// always use the built-in mappings. Both lookup directions ignore ASCII case,
/// while returned values retain their registered spelling.
///
/// ```
/// use mimecraft::MimeTypes;
///
/// let mut types = MimeTypes::new();
/// assert_eq!(types.mime_type("photo.JPG"), "image/jpeg");
/// assert_eq!(types.extension("image/jpeg"), Some(".jpg"));
/// types.register("application/vnd.example", "example")?;
/// assert_eq!(types.mime_type("file.EXAMPLE"), "application/vnd.example");
/// assert_eq!(types.extension("application/vnd.example"), Some(".example"));
/// # Ok::<(), mimecraft::MimeTypeRegistrationError>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct MimeTypes {
    mime_types: HashMap<String, String>,
    extensions: HashMap<String, String>,
}

impl MimeTypes {
    /// Creates an independent view of the built-in mappings, with no overrides.
    pub fn new() -> Self {
        Self::default()
    }

    /// Guesses a MIME-type from the final filename extension.
    ///
    /// Uses only the suffix beginning at the final dot in the last path segment.
    /// Both slash and backslash are separators on all platforms, unlike MimeKit's
    /// platform-dependent `Path.GetExtension`. A leading-dot name such as `.txt`
    /// has an extension; a trailing dot does not. Multiple suffixes use only the
    /// last (`archive.tar.gz` uses `.gz`).
    ///
    /// Unknown extensions, empty names, and names without an extension return
    /// `application/octet-stream`. Does not trim whitespace, strip URI queries
    /// or fragments, percent-decode paths, or validate filename syntax.
    /// Returns a borrowed registered value or a static built-in value.
    pub fn mime_type(&self, file_name: &str) -> &str {
        let name = file_name
            .rsplit(['/', '\\'])
            .next()
            .expect("split has a final segment");
        let Some(dot) = name.rfind('.').filter(|index| *index + 1 < name.len()) else {
            return "application/octet-stream";
        };
        let extension = name[dot..].to_ascii_lowercase();
        self.mime_types
            .get(&extension)
            .map(String::as_str)
            .or_else(|| lookup(MIME_TYPES, &extension))
            .unwrap_or("application/octet-stream")
    }

    /// Returns the preferred dotted extension for an exact MIME-type name.
    ///
    /// Ignores ASCII case but does not trim whitespace, parse Content-Type
    /// parameters, or normalize aliases. Unknown names, including empty input,
    /// return `None`. The preferred-extension table is independent of filename
    /// aliases: a type may have many recognized filename extensions.
    /// Returns a borrowed registered value or a static built-in value.
    pub fn extension(&self, mime_type: &str) -> Option<&str> {
        let key = mime_type.to_ascii_lowercase();
        self.extensions
            .get(&key)
            .map(String::as_str)
            .or_else(|| lookup(EXTENSIONS, &key))
    }

    /// Registers both lookup directions on this instance, adding a dot if needed.
    ///
    /// Overwrites the mapping for this extension and the preferred extension
    /// for this MIME-type. Other aliases and old reverse mappings remain, as in
    /// MimeKit; the dictionaries are not a bijection. For example, registering a
    /// new preferred extension does not remove older recognized filename suffixes.
    ///
    /// Values retain spelling and whitespace; keys ignore ASCII case. Like
    /// MimeKit, only empty strings are rejected: this is not MIME-token, header,
    /// or filesystem-name validation. A multi-dot registered extension cannot
    /// match a filename lookup that considers only its last suffix. Unicode
    /// characters are matched literally, without Unicode case folding.
    ///
    /// # Errors
    ///
    /// Returns [`MimeTypeRegistrationError`] for an empty MIME-type or extension,
    /// leaving both directions unchanged. The MIME-type is checked first.
    pub fn register(
        &mut self,
        mime_type: &str,
        extension: &str,
    ) -> Result<(), MimeTypeRegistrationError> {
        if mime_type.is_empty() {
            return Err(MimeTypeRegistrationError::EmptyMimeType);
        }
        if extension.is_empty() {
            return Err(MimeTypeRegistrationError::EmptyExtension);
        }
        let extension = if extension.starts_with('.') {
            extension.to_owned()
        } else {
            format!(".{extension}")
        };
        // Override each direction independently; do not remove other aliases
        // or infer preferred extensions by reversing the filename table.
        self.mime_types
            .insert(extension.to_ascii_lowercase(), mime_type.to_owned());
        self.extensions
            .insert(mime_type.to_ascii_lowercase(), extension);
        Ok(())
    }
}

fn lookup(table: &[(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table
        .binary_search_by_key(&key, |entry| entry.0)
        .ok()
        .map(|index| table[index].1)
}

/// A MIME-type mapping could not be registered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MimeTypeRegistrationError {
    /// The MIME-type string is empty.
    EmptyMimeType,
    /// The extension string is empty.
    EmptyExtension,
}

impl fmt::Display for MimeTypeRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyMimeType => "cannot register an empty MIME-type",
            Self::EmptyExtension => "cannot register an empty file extension",
        })
    }
}

impl Error for MimeTypeRegistrationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_tables_are_sorted_unique_ascii_keys() {
        assert_eq!(MIME_TYPES.len(), 591);
        assert_eq!(EXTENSIONS.len(), 457);
        for table in [MIME_TYPES, EXTENSIONS] {
            assert!(table.windows(2).all(|rows| rows[0].0 < rows[1].0));
            for &(key, value) in table {
                assert!(key.is_ascii() && value.is_ascii());
                assert_eq!(key, key.to_ascii_lowercase());
                assert!(!key.is_empty() && !value.is_empty());
            }
        }
    }

    #[test]
    fn every_builtin_entry_is_reachable_in_both_ascii_cases() {
        let types = MimeTypes::new();
        for &(extension, expected) in MIME_TYPES {
            assert_eq!(types.mime_type(&format!("file{extension}")), expected);
            assert_eq!(
                types.mime_type(&format!("file{}", extension.to_ascii_uppercase())),
                expected
            );
        }
        for &(mime_type, expected) in EXTENSIONS {
            assert_eq!(types.extension(mime_type), Some(expected));
            assert_eq!(
                types.extension(&mime_type.to_ascii_uppercase()),
                Some(expected)
            );
        }
    }
}
