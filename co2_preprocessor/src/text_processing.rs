//! Text processing utilities for the preprocessor.
//!
//! This module now exposes byte-origin logical slices instead of line-number maps.

use super::pipeline::Preprocessor;
use super::utils::{copy_literal_bytes_raw, skip_literal_bytes};

#[derive(Clone, Debug)]
pub(super) struct LogicalSlice {
    pub(super) text: String,
    pub(super) source_boundaries: Vec<usize>,
    pub(super) terminator: usize,
}

impl LogicalSlice {
    pub(super) fn source_offset(&self, text_offset: usize) -> usize {
        self.source_boundaries
            .get(text_offset.min(self.text.len()))
            .copied()
            .unwrap_or(self.terminator)
    }

    pub(super) fn newline_boundaries(&self) -> Vec<usize> {
        vec![self.source_offset(self.text.len()), self.terminator]
    }
}

impl Preprocessor {
    /// Check if a line has unbalanced parentheses, indicating a multi-line
    /// macro invocation that needs to be joined with subsequent lines.
    /// Skips string/char literals and line comments.
    pub(super) fn has_unbalanced_parens(line: &str) -> bool {
        let mut depth: i32 = 0;
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            match bytes[i] {
                b'"' | b'\'' => {
                    i = skip_literal_bytes(bytes, i, bytes[i]);
                }
                b'(' => {
                    depth += 1;
                    i += 1;
                }
                b')' => {
                    depth -= 1;
                    i += 1;
                }
                b'/' if i + 1 < len && bytes[i + 1] == b'/' => break,
                _ => {
                    i += 1;
                }
            }
        }

        depth > 0
    }

    /// Apply line splicing and comment stripping while keeping byte-boundary
    /// information back to the original source.
    pub(super) fn logical_slices(source: &str) -> Vec<LogicalSlice> {
        let bytes = source.as_bytes();
        let len = bytes.len();
        let mut slices = Vec::new();
        let mut out = Vec::new();
        let mut boundaries = vec![0usize];
        let mut line_start = 0usize;
        let mut i = 0usize;

        while i < len {
            match bytes[i] {
                b'%' if i + 3 < len
                    && bytes[i + 1] == b':'
                    && bytes[i + 2] == b'%'
                    && bytes[i + 3] == b':' =>
                {
                    out.push(b'#');
                    boundaries.push(i + 2);
                    out.push(b'#');
                    i += 4;
                    boundaries.push(i);
                }
                b'%' if i + 1 < len && bytes[i + 1] == b':' => {
                    out.push(b'#');
                    i += 2;
                    boundaries.push(i);
                }
                b'<' if i + 1 < len && bytes[i + 1] == b':' => {
                    out.push(b'[');
                    i += 2;
                    boundaries.push(i);
                }
                b':' if i + 1 < len && bytes[i + 1] == b'>' => {
                    out.push(b']');
                    i += 2;
                    boundaries.push(i);
                }
                b'<' if i + 1 < len && bytes[i + 1] == b'%' => {
                    out.push(b'{');
                    i += 2;
                    boundaries.push(i);
                }
                b'%' if i + 1 < len && bytes[i + 1] == b'>' => {
                    out.push(b'}');
                    i += 2;
                    boundaries.push(i);
                }
                b'"' | b'\'' => {
                    let start = i;
                    i = copy_literal_bytes_raw(bytes, i, bytes[i], &mut out);
                    for idx in start..i {
                        boundaries.push(idx + 1);
                    }
                }
                b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                    if i + 2 < len && matches!(bytes[i + 2], b'/' | b'!') {
                        while i < len && bytes[i] != b'\n' {
                            out.push(bytes[i]);
                            i += 1;
                            boundaries.push(i);
                        }
                    } else {
                        i += 2;
                        while i < len && bytes[i] != b'\n' {
                            i += 1;
                        }
                    }
                }
                b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                    let comment_start = i;
                    i += 2;
                    while i < len {
                        if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                    out.push(b' ');
                    boundaries.push(i.min(len));
                    if boundaries.len() == 2 && boundaries[0] == 0 {
                        boundaries[0] = comment_start;
                    }
                }
                b'\\' => {
                    let mut j = i + 1;
                    while j < len && matches!(bytes[j], b' ' | b'\t' | b'\r') {
                        j += 1;
                    }
                    if j < len && bytes[j] == b'\n' {
                        i = j + 1;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                        boundaries.push(i);
                    }
                }
                b'\n' => {
                    slices.push(LogicalSlice {
                        text: String::from_utf8(std::mem::take(&mut out))
                            .expect("logical slice must stay utf-8"),
                        source_boundaries: std::mem::take(&mut boundaries),
                        terminator: i + 1,
                    });
                    i += 1;
                    line_start = i;
                    boundaries.push(line_start);
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                    boundaries.push(i);
                }
            }
        }

        if !out.is_empty() || line_start < len {
            slices.push(LogicalSlice {
                text: String::from_utf8(out).expect("logical slice must stay utf-8"),
                source_boundaries: boundaries,
                terminator: len,
            });
        }

        slices
    }
}

/// Strip a // comment from a directive line, but not inside string literals.
/// Returns a `Cow<str>` to avoid allocation when no comment is found (the
/// common case). Only allocates a new String when a `//` comment is present
/// and needs to be stripped.
pub(super) fn strip_line_comment(line: &str) -> std::borrow::Cow<'_, str> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        match bytes[i] {
            b'"' | b'\'' => {
                i = skip_literal_bytes(bytes, i, bytes[i]);
            }
            b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                return std::borrow::Cow::Owned(line[..i].trim_end().to_string());
            }
            _ => i += 1,
        }
    }

    std::borrow::Cow::Borrowed(line)
}

/// Split a string into the first word and the rest.
/// For preprocessor directives, '(' is also a word boundary so that
/// `#if(expr)` is correctly parsed as keyword="if", rest="(expr)".
pub(super) fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    if let Some(pos) = s.find(|c: char| c.is_whitespace() || c == '(') {
        if s.as_bytes()[pos] == b'(' {
            (&s[..pos], &s[pos..])
        } else {
            (&s[..pos], s[pos..].trim())
        }
    } else {
        (s, "")
    }
}
