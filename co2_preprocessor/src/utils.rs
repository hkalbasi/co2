//! Shared utility functions for the preprocessor module.
//!
//! Provides both char-oriented and byte-oriented helpers for scanning C source text.
//! The byte-oriented variants (`*_byte`, `*_bytes`) are preferred in hot paths since
//! they avoid the overhead of `Vec<char>` allocation. The char-oriented versions are
//! retained for code that still operates on `&[char]`.

/// Check if a character can start a C identifier.
/// GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default).
pub fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

/// Check if a character can continue a C identifier.
/// GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default).
pub fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Check if a byte can start a C identifier (ASCII letter, underscore, or dollar sign).
/// GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default).
#[inline(always)]
pub fn is_ident_start_byte(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

/// Check if a byte can continue a C identifier (ASCII alphanumeric, underscore, or dollar sign).
/// GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default).
#[inline(always)]
pub fn is_ident_cont_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Extract a `&str` slice from `bytes[start..end]`.
///
/// C identifiers consist only of ASCII letters, digits, underscores, and dollar signs,
/// which are all valid single-byte UTF-8 characters.
#[inline(always)]
pub fn bytes_to_str(bytes: &[u8], start: usize, end: usize) -> &str {
    std::str::from_utf8(&bytes[start..end]).expect("bytes_to_str: input is not valid UTF-8")
}

/// Return the length of a C string/character literal encoding prefix at `start`.
///
/// Recognizes the standard prefixes `u8`, `u`, `U`, and `L` when they are
/// immediately followed by a matching literal quote.
#[inline(always)]
pub fn literal_prefix_len(bytes: &[u8], start: usize) -> usize {
    match bytes.get(start..) {
        Some([b'u', b'8', b'"', ..]) => 2,
        Some([b'u' | b'U' | b'L', b'"' | b'\'', ..]) => 1,
        _ => 0,
    }
}

/// Skip past a string or character literal in a byte slice, starting at position `i`.
/// Returns the position after the closing quote. Handles backslash escapes.
/// Byte-oriented version for code that processes `&[u8]` directly.
pub fn skip_literal_bytes(bytes: &[u8], start: usize, quote: u8) -> usize {
    let len = bytes.len();
    let mut i = start + 1; // skip opening quote
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            i += 2;
        } else if bytes[i] == quote {
            return i + 1;
        } else {
            i += 1;
        }
    }
    i
}

/// Copy a string or character literal from a byte slice into a byte buffer result.
/// Returns the position after the closing quote. Handles backslash escapes.
/// Copies raw bytes without any char conversion to preserve UTF-8 sequences.
pub fn copy_literal_bytes_raw(
    bytes: &[u8],
    start: usize,
    quote: u8,
    result: &mut Vec<u8>,
) -> usize {
    let len = bytes.len();
    result.push(bytes[start]); // opening quote
    let mut i = start + 1;
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            if bytes[i + 1] == b'\n' {
                // \<newline>: C phase 2 line continuation, skip both
                i += 2;
            } else if bytes[i + 1] == b'\r' && i + 2 < len && bytes[i + 2] == b'\n' {
                // \<CR><LF>: line continuation, skip all three
                i += 3;
            } else if bytes[i + 1] == b'\\' {
                // \\ followed by something: could be \\<newline>
                result.push(b'\\');
                if i + 2 < len && bytes[i + 2] == b'\n' {
                    // \\<newline>: first \ survives, second \ spliced with newline
                    i += 3;
                } else if i + 2 < len
                    && bytes[i + 2] == b'\r'
                    && i + 3 < len
                    && bytes[i + 3] == b'\n'
                {
                    i += 4;
                } else {
                    // \\<other>: literal backslash escape, output second backslash too
                    result.push(b'\\');
                    i += 2;
                }
            } else {
                // any other escape sequence: output both bytes
                result.push(bytes[i]);
                result.push(bytes[i + 1]);
                i += 2;
            }
        } else if bytes[i] == quote {
            result.push(bytes[i]);
            return i + 1;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    i
}

/// Copy a string or character literal from a byte slice into a String result.
/// Returns the position after the closing quote. Handles backslash escapes.
/// Copies the entire literal as a single `&str` slice for efficiency, rather than
/// pushing individual bytes.
pub fn copy_literal_bytes_to_string(
    bytes: &[u8],
    start: usize,
    quote: u8,
    result: &mut String,
) -> usize {
    let len = bytes.len();
    // Find the end of the literal first, then copy as a single &str slice
    // (the common case), avoiding per-byte push.
    let mut i = start + 1; // skip opening quote
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            i += 2; // skip escape sequence
        } else if bytes[i] == quote {
            i += 1; // include closing quote
            // Source text is valid UTF-8 and we copy a contiguous substring.
            let slice =
                std::str::from_utf8(&bytes[start..i]).expect("literal copy produced non-UTF8");
            result.push_str(slice);
            return i;
        } else {
            i += 1;
        }
    }
    // Unterminated literal - copy what we have
    let slice = std::str::from_utf8(&bytes[start..i]).expect("literal copy produced non-UTF8");
    result.push_str(slice);
    i
}
