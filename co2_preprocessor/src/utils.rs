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
///
/// Bytes `>= 0x80` are treated as continuing a UTF-8 sequence so that multibyte
/// identifier characters are absorbed whole by callers that scan byte-by-byte (e.g. the
/// `'` lifetime/char-literal disambiguation and preprocessor-directive name scanning).
/// Full XID validation for UTF-8 and universal-character-name identifier characters is
/// performed by [`ident_start_len`] / [`ident_cont_len`].
#[inline(always)]
pub fn is_ident_start_byte(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80
}

/// Check if a byte can continue a C identifier (ASCII alphanumeric, underscore, or dollar sign).
/// GCC extension: '$' is allowed in identifiers (-fdollars-in-identifiers, on by default).
///
/// See [`is_ident_start_byte`] for the treatment of non-ASCII bytes.
#[inline(always)]
pub fn is_ident_cont_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Whether a single character is permitted at the start or in the continuation of a C
/// identifier, per C23 §6.4.2.1 (XID_Start / XID_Continue derived properties), plus the
/// traditional ASCII letters/digits, underscore, and the GCC `$` extension.
#[inline(always)]
pub(crate) fn char_allowed(ch: char, start: bool) -> bool {
    if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
        // Digits may not start an identifier.
        return if start { !ch.is_ascii_digit() } else { true };
    }
    if start {
        unicode_ident::is_xid_start(ch)
    } else {
        unicode_ident::is_xid_continue(ch)
    }
}

/// Decode the UTF-8 character beginning at `pos`, returning it and its byte length.
///
/// Returns `None` if `pos` is out of range or does not begin a valid UTF-8 sequence.
pub(crate) fn decode_utf8_char(bytes: &[u8], pos: usize) -> Option<(char, usize)> {
    let rest = bytes.get(pos..)?;
    let ch = std::str::from_utf8(rest).ok()?.chars().next()?;
    Some((ch, ch.len_utf8()))
}

/// Produce a diagnostic message for a character encountered where a C23 identifier
/// character was expected but is not permitted (C23 §6.4.2.1 / UAX #31).
pub(crate) fn invalid_ident_char_message(ch: char) -> String {
    match ch {
        '\u{200D}' => "identifier cannot begin with U+200D ZERO WIDTH JOINER".to_string(),
        '\u{200C}' => "identifier cannot begin with U+200C ZERO WIDTH NON-JOINER".to_string(),
        '\u{00A0}' => "U+00A0 NO-BREAK SPACE is not permitted in an identifier".to_string(),
        '\u{2003}' => "U+2003 EM SPACE is not permitted in an identifier".to_string(),
        '\u{FEFF}' => {
            "U+FEFF ZERO WIDTH NO-BREAK SPACE is not permitted in an identifier".to_string()
        }
        '\u{200E}' => "U+200E LEFT-TO-RIGHT MARK is not permitted in an identifier".to_string(),
        '\u{200F}' => "U+200F RIGHT-TO-LEFT MARK is not permitted in an identifier".to_string(),
        // XID_Continue but not XID_Start: a combining mark (and similar) may appear inside
        // an identifier but cannot begin one.
        _ if unicode_ident::is_xid_continue(ch) && !unicode_ident::is_xid_start(ch) => {
            "identifier cannot begin with a combining character".to_string()
        }
        _ => "invalid identifier".to_string(),
    }
}

/// Parse a universal character name `\uXXXX` or `\UXXXXXXXX` at `pos` in `bytes`.
///
/// Returns the decoded `char` and the number of bytes consumed (6 or 10) when the escape is
/// well-formed, or `None` otherwise.
pub(crate) fn parse_ucn(bytes: &[u8], pos: usize) -> Option<(char, usize)> {
    if bytes.get(pos) != Some(&b'\\') {
        return None;
    }
    let kind = *bytes.get(pos + 1)?;
    let (digits, len) = match kind {
        b'u' => (4, 6),
        b'U' => (8, 10),
        _ => return None,
    };
    if pos + len > bytes.len() {
        return None;
    }
    let mut value: u32 = 0;
    for i in 0..digits {
        let b = bytes[pos + 2 + i];
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        value = value * 16 + u32::from(d);
    }
    char::from_u32(value).map(|ch| (ch, len))
}

/// Return the byte length of the identifier-start character at `pos`, or `None` if the byte
/// at `pos` does not begin a valid C23 identifier-start (ASCII letter/`_`/`$`, a UTF-8
/// encoded XID_Start character, or a universal character name designating one).
#[inline]
pub fn ident_start_len(bytes: &[u8], pos: usize) -> Option<usize> {
    ident_char_len(bytes, pos, true)
}

/// Return the byte length of the identifier-continuation character at `pos`, or `None` if the
/// byte at `pos` does not begin a valid C23 identifier-continuation character (ASCII
/// alphanumeric/`_`/`$`, a UTF-8 encoded XID_Continue character, or a universal character
/// name designating one).
#[inline]
pub fn ident_cont_len(bytes: &[u8], pos: usize) -> Option<usize> {
    ident_char_len(bytes, pos, false)
}

pub(crate) fn ident_char_len(bytes: &[u8], pos: usize, start: bool) -> Option<usize> {
    let b = *bytes.get(pos)?;
    if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
        // Digits may not start an identifier.
        return if start && b.is_ascii_digit() {
            None
        } else {
            Some(1)
        };
    }
    // Universal character name, e.g. \u03C0 (π) or \U0001F600 (😀).
    if b == b'\\' {
        let (ch, len) = parse_ucn(bytes, pos)?;
        return if char_allowed(ch, start) {
            Some(len)
        } else {
            None
        };
    }
    // UTF-8 multibyte character.
    if b >= 0x80 {
        let rest = bytes.get(pos..)?;
        let ch = std::str::from_utf8(rest).ok()?.chars().next()?;
        return if char_allowed(ch, start) {
            Some(ch.len_utf8())
        } else {
            None
        };
    }
    None
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
