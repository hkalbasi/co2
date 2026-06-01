//! Macro definitions and expansion logic for the C preprocessor.
//!
//! Supports:
//! - Object-like macros: `#define FOO value`
//! - Function-like macros: `#define MAX(a,b) ((a)>(b)?(a):(b))`
//! - Variadic macros: `#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)`
//! - Stringification: `#param`
//! - Token pasting: `a ## b`
//!
//! Performance: All scanning operates on byte slices (`&[u8]`) to avoid
//! the overhead of `Vec<char>` allocation. Since C preprocessor tokens are
//! ASCII, this is safe and correct. UTF-8 multi-byte sequences in string/char
//! literals are copied verbatim without interpretation.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use super::macro_token::{MacroBodyToken, resolve_params, tokenize_macro_body};
use super::utils::{
    bytes_to_str, copy_literal_bytes_to_string, is_ident_cont_byte, is_ident_start_byte,
    skip_literal_bytes,
};

/// Check if two adjacent bytes would form an unintended multi-character token
/// when concatenated during macro expansion. Returns true if a separating space
/// is needed to prevent accidental token pasting.
fn would_paste_tokens(last: u8, first: u8) -> bool {
    match (last, first) {
        // -- or -=  or -> from separate sources
        (b'-', b'-' | b'=' | b'>')
        // ++ or +=
        | (b'+', b'+' | b'=')
        // << or <= or <:
        | (b'<', b'<' | b'=' | b':' | b'%')
        // >> or >=
        | (b'>', b'>' | b'=')
        // == or ending = followed by =
        | (b'=', b'=') => true,
        _ => false,
    }
}

/// Represents a macro definition.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct MacroDef {
    /// Name of the macro
    pub name: String,
    /// Whether this is a function-like macro
    pub is_function_like: bool,
    /// Parameters for function-like macros
    pub params: Vec<String>,
    /// Whether the macro is variadic (last param is ...)
    pub is_variadic: bool,
    /// Whether the variadic is a named parameter (e.g., `args...` vs `...`).
    /// When true, the last entry in `params` is the variadic param name.
    /// When false, variadic args are accessed via `__VA_ARGS__`.
    pub has_named_variadic: bool,
    /// The replacement body (as raw text)
    pub body: String,
    /// The replacement body tokenized at definition time.
    /// Used by `handle_stringify_and_paste` to avoid re-scanning the body
    /// text for `#` and `##` operators and identifier boundaries.
    pub tokenized_body: Vec<MacroBodyToken>,
    /// Whether the body contains `#` (stringify) or `##` (paste) operators.
    /// When false, `handle_stringify_and_paste` can be skipped.
    pub has_stringify_or_paste: bool,
}

/// Marker byte used to "blue paint" tokens that were suppressed due to
/// self-referential macro expansion (C11 §6.10.3.4).  Prefixed to
/// identifiers during rescanning so they are never re-expanded, then
/// stripped from the final output in `expand_line()`.
const BLUE_PAINT_MARKER: u8 = 0x01;

// (PASTE_PROTECT_START/END at 0x02/0x03 were removed when handle_stringify_and_paste
// and substitute_params were merged into expand_body in Step 3+4.)

/// Strip all blue-paint markers from a string.
/// Used when substituting arguments into ## token paste operations,
/// since the pasted result is a new token that should be rescanned
/// without inheriting blue paint from its operands.
///
/// Returns a `Cow<str>` to avoid allocation when no markers are present
/// (the common case).
fn strip_blue_paint(s: &str) -> std::borrow::Cow<'_, str> {
    if s.as_bytes().contains(&BLUE_PAINT_MARKER) {
        std::borrow::Cow::Owned(s.replace(BLUE_PAINT_MARKER as char, ""))
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

/// Stores all macro definitions and handles expansion.
#[derive(Debug, Clone)]
pub struct MacroTable {
    macros: HashMap<String, MacroDef>,
    /// Counter for the __COUNTER__ built-in macro. Increments on each expansion.
    counter: Cell<usize>,
}

impl MacroTable {
    pub fn new() -> Self {
        Self {
            macros: HashMap::new(),
            counter: Cell::new(0),
        }
    }

    /// Define a new macro.
    pub fn define(&mut self, def: MacroDef) {
        self.macros.insert(def.name.clone(), def);
    }

    /// Undefine a macro.
    pub fn undefine(&mut self, name: &str) {
        self.macros.remove(name);
    }

    /// Check if a macro is defined.
    pub fn is_defined(&self, name: &str) -> bool {
        name == "__COUNTER__" || name == "__LINE__" || self.macros.contains_key(name)
            // These are special preprocessor operators, not real macros.
            // They need to be recognized by #ifdef but are evaluated as
            // special functions in #if expressions by resolve_defined_in_expr().
            || name == "__has_builtin"
            || name == "__has_attribute"
            || name == "__has_feature"
            || name == "__has_extension"
            || name == "__has_include"
            || name == "__has_include_next"
    }

    /// Get a macro definition.
    pub fn get(&self, name: &str) -> Option<&MacroDef> {
        self.macros.get(name)
    }

    /// Set the __FILE__ macro body without allocating a full MacroDef.
    /// If __FILE__ already exists in the table, only its body is updated.
    /// Otherwise, a new entry is created.
    /// This avoids 2 full MacroDef allocations per #include directive.
    pub fn set_file(&mut self, body: String) {
        if let Some(existing) = self.macros.get_mut("__FILE__") {
            existing.body = body;
            existing.tokenized_body = tokenize_macro_body(&existing.body);
            existing.has_stringify_or_paste = has_stringify_or_paste(&existing.tokenized_body);
            // No params to resolve for __FILE__ (object-like)
        } else {
            self.macros.insert(
                "__FILE__".to_string(),
                macro_def_from_parts(
                    "__FILE__".to_string(),
                    false,
                    Vec::new(),
                    false,
                    false,
                    body,
                ),
            );
        }
    }

    /// Get the current __FILE__ macro body.
    /// Returns None if __FILE__ is not defined.
    pub fn get_file_body(&self) -> Option<&str> {
        self.macros.get("__FILE__").map(|m| m.body.as_str())
    }

    /// Expand macros in a line of text.
    /// Returns the expanded text.
    pub fn expand_line(&self, line: &str) -> String {
        let mut expanding = HashSet::new();
        self.expand_line_reuse(line, &mut expanding)
    }

    /// Expand macros in a line of text, reusing the provided `expanding` set.
    /// The set is cleared before use. This avoids allocating a new HashSet
    /// for every line (the previous per-line allocation was a measurable
    /// overhead when preprocessing kernel headers with thousands of lines).
    pub fn expand_line_reuse(&self, line: &str, expanding: &mut HashSet<String>) -> String {
        expanding.clear();
        let result = self.expand_text(line, expanding);
        // Strip blue paint markers from the final output.
        // BLUE_PAINT_MARKER (0x01) prevents re-expansion per C11 §6.10.3.4.
        if result.as_bytes().contains(&BLUE_PAINT_MARKER) {
            result.replace(BLUE_PAINT_MARKER as char, "")
        } else {
            result
        }
    }

    /// Append `expanded` text to `result`, inserting spaces as needed to prevent
    /// accidental token pasting between the end of `result` and the start/end
    /// of `expanded`, and between the end of `expanded` and `next_byte` (the
    /// next byte in the source after the expansion site).
    fn append_with_paste_guard(result: &mut String, expanded: &str, next_byte: Option<u8>) {
        if expanded.is_empty() {
            return;
        }
        // Leading edge: prevent pasting between result's last byte and expansion's first byte
        if !result.is_empty() {
            let last = result.as_bytes()[result.len() - 1];
            let first = expanded.as_bytes()[0];
            if would_paste_tokens(last, first)
                || (is_ident_cont_byte(last) && is_ident_cont_byte(first))
            {
                result.push(' ');
            }
        }
        result.push_str(expanded);
        // Trailing edge: prevent pasting between expansion's last byte and next source byte
        if let Some(next) = next_byte {
            let last_expanded = expanded.as_bytes()[expanded.len() - 1];
            if would_paste_tokens(last_expanded, next) {
                result.push(' ');
            }
        }
    }

    /// After expanding a function-like macro, check if the result ends with a
    /// function-like macro name and the remaining source starts with '('.
    /// If so, expand the trailing macro call, consuming the arguments from the source.
    /// Returns the updated `expanded` text and new source position.
    fn expand_trailing_func_macros(
        &self,
        mut expanded: String,
        bytes: &[u8],
        mut i: usize,
        expanding: &mut HashSet<String>,
    ) -> (String, usize) {
        let len = bytes.len();
        loop {
            let trailing = extract_trailing_ident(&expanded);
            if let Some(ref trail_ident) = trailing
                && !expanding.contains(trail_ident.as_str())
                && let Some(trail_mac) = self.macros.get(trail_ident.as_str())
                && trail_mac.is_function_like
            {
                let mut k = i;
                while k < len && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < len && bytes[k] == b'(' {
                    let (trail_args, trail_end) = self.parse_macro_args(bytes, k);
                    i = trail_end;
                    let trail_mac_clone = trail_mac.clone();
                    let trimmed_len = expanded.trim_end().len();
                    let prefix_len = trimmed_len - trail_ident.len();
                    expanded.truncate(prefix_len);
                    let (trail_expanded, _) =
                        self.expand_function_macro(&trail_mac_clone, &trail_args, expanding);
                    expanded.push_str(&trail_expanded);
                    continue;
                }
            }
            break;
        }
        (expanded, i)
    }

    /// After expanding an object-like macro, check if the result is a function-like
    /// macro name and the remaining source starts with '('. If so, expand the
    /// function-like macro call, consuming the arguments from the source.
    /// Returns Some((expanded_text, new_pos)) if resolution succeeded, or None.
    fn try_resolve_objlike_to_funclike(
        &self,
        expanded: &str,
        bytes: &[u8],
        i: usize,
        expanding: &mut HashSet<String>,
    ) -> Option<(String, usize)> {
        let len = bytes.len();
        let expanded_trimmed = expanded.trim();
        if expanded_trimmed.is_empty() {
            return None;
        }
        let et_bytes = expanded_trimmed.as_bytes();
        if !is_ident_start_byte(et_bytes[0])
            || !et_bytes.iter().all(|&b| is_ident_cont_byte(b))
            || expanding.contains(expanded_trimmed)
        {
            return None;
        }
        let target_mac = self.macros.get(expanded_trimmed)?;
        if !target_mac.is_function_like {
            return None;
        }
        let mut j = i;
        while j < len && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= len || bytes[j] != b'(' {
            return None;
        }
        let (args, end_pos) = self.parse_macro_args(bytes, j);
        let target_mac_clone = target_mac.clone();
        let (func_expanded, _) = self.expand_function_macro(&target_mac_clone, &args, expanding);
        Some((func_expanded, end_pos))
    }

    /// Recursively expand macros in text, tracking which macros are
    /// currently being expanded to prevent infinite recursion.
    ///
    /// Operates on bytes for performance: avoids allocating Vec<char>.
    fn expand_text(&self, text: &str, expanding: &mut HashSet<String>) -> String {
        let mut result = String::with_capacity(text.len());
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            let b = bytes[i];

            if b == b'"' || b == b'\'' {
                i = copy_literal_bytes_to_string(bytes, i, b, &mut result);
            } else if b == BLUE_PAINT_MARKER {
                i = Self::copy_blue_painted(bytes, i, &mut result);
            } else if is_ident_start_byte(b) {
                i = self.expand_identifier(text, bytes, i, &mut result, expanding);
            } else if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                i = Self::copy_block_comment(bytes, i, &mut result);
            } else if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
                i = Self::copy_line_comment(bytes, i, &mut result);
            } else if b.is_ascii_digit()
                || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit())
            {
                i = Self::copy_ppnumber(bytes, i, &mut result);
            } else if b < 0x80 {
                // Batch consecutive non-special ASCII bytes into a single push_str.
                // This avoids per-byte push overhead for sequences of operators,
                // whitespace, punctuation etc.
                let start = i;
                i += 1;
                while i < len {
                    let c = bytes[i];
                    if c == b'"'
                        || c == b'\''
                        || c == BLUE_PAINT_MARKER
                        || is_ident_start_byte(c)
                        || (c == b'/'
                            && i + 1 < len
                            && (bytes[i + 1] == b'*' || bytes[i + 1] == b'/'))
                        || c.is_ascii_digit()
                        || (c == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit())
                        || c >= 0x80
                    {
                        break;
                    }
                    i += 1;
                }
                let slice = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                result.push_str(slice);
            } else {
                let ch = text[i..].chars().next().unwrap();
                result.push(ch);
                i += ch.len_utf8();
            }
        }

        result
    }

    /// Copy a blue-painted identifier verbatim (never re-expanded).
    /// Uses batch slice copy for the identifier bytes after the marker.
    fn copy_blue_painted(bytes: &[u8], i: usize, result: &mut String) -> usize {
        result.push(BLUE_PAINT_MARKER as char);
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() && is_ident_cont_byte(bytes[j]) {
            j += 1;
        }
        let slice = std::str::from_utf8(&bytes[start..j]).unwrap_or("");
        result.push_str(slice);
        j
    }

    /// Process an identifier: expand macros, handle builtins, or copy verbatim.
    fn expand_identifier(
        &self,
        text: &str,
        bytes: &[u8],
        start: usize,
        result: &mut String,
        expanding: &mut HashSet<String>,
    ) -> usize {
        let len = bytes.len();
        let mut i = start + 1;
        while i < len && is_ident_cont_byte(bytes[i]) {
            i += 1;
        }
        let ident = bytes_to_str(bytes, start, i);

        // Check if this identifier is part of a pp-number (e.g., 1.I, 15.IF, 0x1p2).
        if Self::is_ppnumber_suffix(result.as_bytes(), ident) {
            result.push_str(ident);
            return i;
        }

        // Handle _Pragma("string") operator (C99 §6.10.9).
        if ident == "_Pragma" {
            return Self::skip_pragma(bytes, i, ident, result);
        }

        // Handle __COUNTER__ built-in
        if ident == "__COUNTER__" {
            let val = self.counter.get();
            self.counter.set(val + 1);
            let mut buf = itoa::Buffer::new();
            result.push_str(buf.format(val));
            return i;
        }

        // Byte-only preprocessor mode does not track display rows here.
        // Keep __LINE__ parseable in ordinary code by folding it to a stable
        // positive constant instead of zero.
        if ident == "__LINE__" {
            result.push('1');
            return i;
        }

        // C11 string/char encoding prefixes: u8, u, U, L.
        // When an identifier matches one of these and is immediately followed by
        // a quote ('"' or '\''), it's a string/char literal prefix, not a macro name.
        // The preprocessor must not expand it; let the lexer handle it as a token.
        if i < len
            && (bytes[i] == b'"' || bytes[i] == b'\'')
            && matches!(ident, "u8" | "u" | "U" | "L")
        {
            result.push_str(ident);
            return i;
        }

        // Try macro expansion
        if !expanding.contains(ident)
            && let Some(mac) = self.macros.get(ident)
        {
            return self.expand_macro_invocation(text, bytes, i, ident, mac, result, expanding);
        }

        // Not a macro, or already expanding (blue-painted).
        if expanding.contains(ident) {
            result.push(BLUE_PAINT_MARKER as char);
        }
        result.push_str(ident);
        i
    }

    /// Check if an identifier is a pp-number suffix (e.g., the "I" in "1.I").
    fn is_ppnumber_suffix(result_bytes: &[u8], _ident: &str) -> bool {
        let rlen = result_bytes.len();
        if rlen == 0 {
            return false;
        }
        let prev = result_bytes[rlen - 1];
        if prev.is_ascii_digit() {
            true
        } else if prev == b'.' && rlen >= 2 && result_bytes[rlen - 2].is_ascii_digit() {
            is_ppnumber_context(result_bytes, rlen - 2)
        } else if (prev == b'+' || prev == b'-')
            && rlen >= 3
            && matches!(result_bytes[rlen - 2], b'e' | b'E' | b'p' | b'P')
        {
            is_ppnumber_context(result_bytes, rlen - 3)
        } else {
            false
        }
    }

    /// Skip a _Pragma("...") operator, consuming the parenthesized argument.
    fn skip_pragma(bytes: &[u8], i: usize, ident: &str, result: &mut String) -> usize {
        let len = bytes.len();
        let mut j = i;
        while j < len && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < len && bytes[j] == b'(' {
            let mut depth = 1;
            j += 1;
            while j < len && depth > 0 {
                if bytes[j] == b'(' {
                    depth += 1;
                } else if bytes[j] == b')' {
                    depth -= 1;
                } else if bytes[j] == b'"' || bytes[j] == b'\'' {
                    j = skip_literal_bytes(bytes, j, bytes[j]);
                    continue;
                }
                j += 1;
            }
            return j;
        }
        result.push_str(ident);
        i
    }

    /// Expand a macro invocation (function-like or object-like).
    fn expand_macro_invocation(
        &self,
        _text: &str,
        bytes: &[u8],
        i: usize,
        ident: &str,
        mac: &MacroDef,
        result: &mut String,
        expanding: &mut HashSet<String>,
    ) -> usize {
        let len = bytes.len();
        if mac.is_function_like {
            let mut j = i;
            while j < len && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < len && bytes[j] == b'(' {
                let (args, end_pos) = self.parse_macro_args(bytes, j);
                let mut i = end_pos;
                let (expanded, body_ended_with_func_ident) =
                    self.expand_function_macro(mac, &args, expanding);
                // Per C11 §6.10.3.4, only connect trailing function-like macro
                // identifiers with subsequent source `(` if the substituted body
                // (before rescan) truly ended with that identifier. If the body
                // had tokens after the identifier (e.g., `FOO EMPTY()` from deferred
                // expansion patterns like `#define DEFER(x) x EMPTY()`), those tokens
                // act as a barrier during left-to-right scanning, preventing FOO from
                // connecting with `(` in the source.
                let (expanded, new_i) = if body_ended_with_func_ident {
                    self.expand_trailing_func_macros(expanded, bytes, i, expanding)
                } else {
                    (expanded, i)
                };
                i = new_i;
                let next = if i < len { Some(bytes[i]) } else { None };
                Self::append_with_paste_guard(result, &expanded, next);
                return i;
            }
            result.push_str(ident);
            return i;
        }

        // Object-like macro
        expanding.insert(ident.to_string());
        let expanded = if mac.has_stringify_or_paste {
            let body = self.expand_body(
                &mac.tokenized_body,
                &mac.params,
                &[], // raw args (none for object-like)
                &[], // expanded args (none for object-like)
                mac.is_variadic,
                mac.has_named_variadic,
            );
            self.expand_text(&body, expanding)
        } else {
            self.expand_text(&mac.body, expanding)
        };
        expanding.remove(ident);

        if let Some((func_expanded, end_pos)) =
            self.try_resolve_objlike_to_funclike(&expanded, bytes, i, expanding)
        {
            result.push_str(&func_expanded);
            return end_pos;
        }

        // After expanding an object-like macro, the result may end with a function-like
        // macro name (e.g. `#define i_cmp -c_default_cmp` expands to `-c_default_cmp`).
        // If the remaining source starts with `(`, we need to chain into function-like
        // expansion for the trailing identifier.
        let (expanded, i) = self.expand_trailing_func_macros(expanded, bytes, i, expanding);

        let next = if i < len { Some(bytes[i]) } else { None };
        Self::append_with_paste_guard(result, &expanded, next);
        i
    }

    /// Copy a C-style block comment verbatim.
    /// Uses batch slice copy instead of per-byte push for the comment body.
    fn copy_block_comment(bytes: &[u8], i: usize, result: &mut String) -> usize {
        let len = bytes.len();
        let start = i;
        let mut j = i + 2; // skip /*
        while j + 1 < len && !(bytes[j] == b'*' && bytes[j + 1] == b'/') {
            j += 1;
        }
        if j + 1 < len {
            j += 2; // skip */
        }
        // All comment content is ASCII, safe to copy as str slice
        // SAFETY: bytes came from a valid UTF-8 str, and comments contain only ASCII
        let slice = std::str::from_utf8(&bytes[start..j]).unwrap_or("");
        result.push_str(slice);
        j
    }

    /// Copy a line comment verbatim.
    /// Uses batch slice copy instead of per-byte push.
    fn copy_line_comment(bytes: &[u8], i: usize, result: &mut String) -> usize {
        let len = bytes.len();
        let mut j = i;
        while j < len && bytes[j] != b'\n' {
            j += 1;
        }
        let slice = std::str::from_utf8(&bytes[i..j]).unwrap_or("");
        result.push_str(slice);
        j
    }

    /// Copy a pp-number (numeric literal) verbatim.
    /// Uses batch slice copy instead of per-byte push.
    fn copy_ppnumber(bytes: &[u8], i: usize, result: &mut String) -> usize {
        let len = bytes.len();
        let mut j = i;
        while j < len {
            if bytes[j].is_ascii_alphanumeric()
                || bytes[j] == b'.'
                || bytes[j] == b'_'
                || ((bytes[j] == b'+' || bytes[j] == b'-')
                    && j > i
                    && matches!(bytes[j - 1], b'e' | b'E' | b'p' | b'P'))
            {
                j += 1;
            } else {
                break;
            }
        }
        let slice = std::str::from_utf8(&bytes[i..j]).unwrap_or("");
        result.push_str(slice);
        j
    }

    /// Parse function-like macro arguments from bytes starting at the opening paren.
    /// Returns (args, position after closing paren).
    ///
    /// Optimized to track byte spans directly into the source text, avoiding
    /// per-argument String allocations in the common case where arguments contain
    /// no string/char literals. When all content is plain text, arguments are
    /// extracted as trimmed `&str` slices and converted to String only once.
    fn parse_macro_args(&self, bytes: &[u8], start: usize) -> (Vec<String>, usize) {
        let len = bytes.len();
        let mut i = start + 1; // skip '('
        let mut args: Vec<String> = Vec::new();
        let mut paren_depth = 0;
        // Start position of the current argument in bytes
        let mut arg_start = i;

        while i < len {
            match bytes[i] {
                b'(' => {
                    paren_depth += 1;
                    i += 1;
                }
                b')' => {
                    if paren_depth == 0 {
                        // Extract the argument as a trimmed &str from the byte span
                        let arg_slice = std::str::from_utf8(&bytes[arg_start..i]).unwrap_or("");
                        let trimmed = arg_slice.trim();
                        if !trimmed.is_empty() || !args.is_empty() {
                            args.push(trimmed.to_string());
                        }
                        return (args, i + 1);
                    }
                    paren_depth -= 1;
                    i += 1;
                }
                b',' if paren_depth == 0 => {
                    let arg_slice = std::str::from_utf8(&bytes[arg_start..i]).unwrap_or("");
                    args.push(arg_slice.trim().to_string());
                    i += 1;
                    arg_start = i;
                }
                b'"' | b'\'' => {
                    // Skip past string/char literal (they are part of the argument
                    // text and will be included in the byte span)
                    i = skip_literal_bytes(bytes, i, bytes[i]);
                }
                _ => {
                    i += 1;
                }
            }
        }

        // Unterminated - return what we have
        let arg_slice = std::str::from_utf8(&bytes[arg_start..i]).unwrap_or("");
        let trimmed = arg_slice.trim();
        if !trimmed.is_empty() || !args.is_empty() {
            args.push(trimmed.to_string());
        }
        (args, i)
    }

    /// Expand a function-like macro with given arguments.
    ///
    /// Per C11 §6.10.3.1: Arguments are fully macro-expanded before substitution
    /// into the body. Occurrences adjacent to # or ## use the raw (unexpanded)
    /// argument, handled by handle_stringify_and_paste. After substitution, the
    /// result is rescanned with the current macro name suppressed (§6.10.3.4).
    /// Expand a function-like macro, returning (expanded_text, body_ended_with_func_ident).
    /// The second return value indicates whether the substituted body (before rescan)
    /// ended with a function-like macro identifier. This is needed to correctly implement
    /// C11 §6.10.3.4: the rescan of the replacement list "along with all subsequent
    /// preprocessing tokens of the source file" must be left-to-right. If the body
    /// ended with tokens AFTER a function-like macro name (e.g., `FOO EMPTY()` from
    /// `#define DEFER(x) x EMPTY()`), those tokens act as a barrier preventing the
    /// function-like macro from connecting with `(` in the subsequent source tokens.
    fn expand_function_macro(
        &self,
        mac: &MacroDef,
        args: &[String],
        expanding: &mut HashSet<String>,
    ) -> (String, bool) {
        // Step 1-2: Prescan - expand ALL arguments (C11 §6.10.3.1).
        // Per the standard, arguments adjacent to # or ## use the RAW (unexpanded)
        // form, but that is handled separately by handle_stringify_and_paste (Step 3)
        // which receives the original raw `args`. The expanded_args are used only by
        // substitute_params (Step 4) for non-## occurrences.
        //
        // A parameter can appear both with ## and without ## in the same body
        // (e.g., `#define bitfs(bf, s) __bitf(bf, bf##_##s)`). In that case, the
        // non-## occurrence needs the expanded argument so that commas from macro
        // expansion create additional arguments during rescanning.
        let expanded_args: Vec<String> = args
            .iter()
            .map(|arg| self.expand_text(arg, expanding))
            .collect();

        // Step 3+4: Expand body — handle #/## and substitute params in one pass.
        let body = if mac.has_stringify_or_paste || !mac.params.is_empty() {
            self.expand_body(
                &mac.tokenized_body,
                &mac.params,
                args,
                &expanded_args,
                mac.is_variadic,
                mac.has_named_variadic,
            )
        } else {
            mac.body.clone()
        };

        // Check if the substituted body (before rescan) ends with a function-like
        // macro identifier. Per C11 §6.10.3.4, the rescan processes the replacement
        // list "along with all subsequent preprocessing tokens" left-to-right. If the
        // body ends with `FOO EMPTY()` (from deferred expansion patterns), FOO should
        // NOT connect with `(` from subsequent source tokens because the EMPTY() tokens
        // act as a barrier during left-to-right scanning. Only if the body truly ends
        // with a function-like macro identifier (no tokens after it) should we allow
        // trailing resolution with subsequent source `(`.
        let body_ends_with_func_ident = {
            if let Some(trailing) = extract_trailing_ident(&body) {
                if let Some(tmac) = self.macros.get(trailing.as_str()) {
                    tmac.is_function_like
                } else {
                    false
                }
            } else {
                false
            }
        };

        // Step 5: Rescan with the current macro name suppressed
        expanding.insert(mac.name.clone());
        let result = self.expand_text(&body, expanding);
        expanding.remove(&mac.name);

        // Also check the post-rescan result: token pasting through indirection
        // (e.g., CONCATENATE -> CONCAT2 -> a##b) can produce a function-like macro
        // name that wasn't visible in the pre-rescan body. For example:
        //   #define CONCATENATE(a, b) __CONCAT(a, b)
        //   #define __CONCAT(a, b) a ## b
        //   CONCATENATE(FOO_, COUNT_ARGS(x))(args)
        // After prescan + paste + rescan, result = "FOO_5" which is function-like.
        // We must detect this so expand_trailing_func_macros connects it with "(args)".
        //
        // However, if the trailing identifier was already present in the pre-rescan
        // body as a standalone token (e.g., `FOO EMPTY()` from DEFER patterns), then
        // the tokens after it acted as a barrier per C11 §6.10.3.4 left-to-right
        // scanning. Even if those barrier tokens expanded to nothing during rescan
        // (like EMPTY() -> ""), the identifier should NOT connect with subsequent
        // source `(`. Only identifiers that are NEW after rescan (produced by ##
        // concatenation or other macro expansion) should allow trailing resolution.
        let result_ends_with_func_ident = if body_ends_with_func_ident {
            true
        } else if let Some(trailing) = extract_trailing_ident(&result) {
            if let Some(tmac) = self.macros.get(trailing.as_str()) {
                if tmac.is_function_like {
                    // Only allow if this identifier was NOT already present as a
                    // standalone token in the pre-rescan body. If it was, the
                    // barrier tokens after it were intentional (DEFER pattern).
                    !contains_standalone_ident(&body, trailing.as_str())
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        (result, result_ends_with_func_ident)
    }

    /// Expand a macro body: handle # (stringify), ## (paste), and substitute
    /// standalone parameters (all in one pass using pre-resolved `Param(idx)`
    /// and `VaArgs` tokens).
    fn expand_body(
        &self,
        tokens: &[MacroBodyToken],
        params: &[String],
        args: &[String],          // raw (unexpanded) args for #/##
        expanded_args: &[String], // expanded args for standalone params
        is_variadic: bool,
        has_named_variadic: bool,
    ) -> String {
        let mut result = String::new();
        let len = tokens.len();
        let mut i = 0;
        let mut pending_param_idx: Option<usize> = None;
        let mut pending_va_args: bool = false;

        while i < len {
            match &tokens[i] {
                MacroBodyToken::HashHash => {
                    // ── Token Paste ────────────────────────────────────
                    // Left side: check pending resolved param first
                    if pending_va_args {
                        pending_va_args = false;
                        let va_args_raw = self.get_va_args(params, args);
                        let va_args = strip_blue_paint(&va_args_raw);
                        if va_args.is_empty() {
                            while result.ends_with(' ') || result.ends_with('\t') {
                                result.pop();
                            }
                            if result.ends_with(',') {
                                result.pop();
                            }
                        } else {
                            result.push_str(&va_args);
                        }
                    } else if let Some(idx) = pending_param_idx.take() {
                        self.paste_left_param(
                            &mut result,
                            idx,
                            args,
                            is_variadic,
                            has_named_variadic,
                            params,
                        );
                    } else {
                        // Fallback: trailing ident in result buffer
                        while result.ends_with(' ') || result.ends_with('\t') {
                            result.pop();
                        }
                        if let Some(ref left_ident) = extract_trailing_ident(&result) {
                            if left_ident == "__VA_ARGS__" && is_variadic {
                                let va_args_raw = self.get_va_args(params, args);
                                let va_args = strip_blue_paint(&va_args_raw);
                                let trim_len = result.len() - "__VA_ARGS__".len();
                                result.truncate(trim_len);
                                if va_args.is_empty() {
                                    while result.ends_with(' ') || result.ends_with('\t') {
                                        result.pop();
                                    }
                                    if result.ends_with(',') {
                                        result.pop();
                                    }
                                } else {
                                    result.push_str(&va_args);
                                }
                            } else if let Some(idx) =
                                params.iter().position(|p| p == left_ident.as_str())
                            {
                                let trim_len = result.len() - left_ident.len();
                                result.truncate(trim_len);
                                let arg = args.get(idx).map_or("", std::string::String::as_str);
                                let clean_arg = strip_blue_paint(arg);
                                result.push_str(&clean_arg);
                            }
                        }
                    }

                    i += 1; // skip HashHash

                    // Skip whitespace after ##
                    while i < len && is_whitespace_token(&tokens[i]) {
                        i += 1;
                    }

                    // Right side
                    if i < len {
                        match &tokens[i] {
                            MacroBodyToken::Param(idx) => {
                                self.paste_right_param(
                                    &mut result,
                                    *idx,
                                    args,
                                    len,
                                    i,
                                    is_variadic,
                                    has_named_variadic,
                                    params,
                                );
                            }
                            MacroBodyToken::VaArgs => {
                                let va_args_raw = self.get_va_args(params, args);
                                let va_args = strip_blue_paint(&va_args_raw);
                                if va_args.is_empty() {
                                    while result.ends_with(' ') || result.ends_with('\t') {
                                        result.pop();
                                    }
                                    if result.ends_with(',') {
                                        result.pop();
                                    }
                                } else {
                                    result.push_str(&va_args);
                                }
                            }
                            _ => {
                                let text = super::macro_token::token_text(&tokens[i]);
                                result.push_str(text);
                            }
                        }
                        i += 1;
                    }
                }

                MacroBodyToken::Hash => {
                    // ── Stringification ────────────────────────────────
                    let mut j = i + 1;
                    while j < len && is_whitespace_token(&tokens[j]) {
                        j += 1;
                    }
                    if j < len {
                        match &tokens[j] {
                            MacroBodyToken::Param(idx) => {
                                let arg_str = if is_variadic
                                    && has_named_variadic
                                    && *idx == params.len() - 1
                                {
                                    self.get_named_va_args(*idx, args)
                                } else {
                                    args.get(*idx).cloned().unwrap_or_default()
                                };
                                result.push('"');
                                result.push_str(&stringify_arg(&arg_str));
                                result.push('"');
                                i = j + 1;
                                continue;
                            }
                            MacroBodyToken::VaArgs => {
                                let va_args = self.get_va_args(params, args);
                                result.push('"');
                                result.push_str(&stringify_arg(&va_args));
                                result.push('"');
                                i = j + 1;
                                continue;
                            }
                            _ if let MacroBodyToken::Ident(param_name) = &tokens[j] => {
                                // Not a resolved param, output #name as-is
                                result.push('#');
                                result.push_str(param_name);
                                i = j + 1;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    result.push('#');
                    i += 1;
                }

                MacroBodyToken::Param(idx) => {
                    // ── Standalone param (not in # or ## context) ─────
                    // Look ahead for ## to detect left-side paste
                    let mut lookahead = i + 1;
                    while lookahead < len && is_whitespace_token(&tokens[lookahead]) {
                        lookahead += 1;
                    }
                    if lookahead < len && matches!(tokens[lookahead], MacroBodyToken::HashHash) {
                        pending_param_idx = Some(*idx);
                    } else {
                        let arg = if is_variadic && has_named_variadic && *idx == params.len() - 1 {
                            self.get_named_va_args(*idx, expanded_args)
                        } else {
                            expanded_args.get(*idx).cloned().unwrap_or_default()
                        };
                        let next_byte = tokens.get(i + 1).and_then(|t| {
                            let s = super::macro_token::token_text(t);
                            s.as_bytes().first().copied()
                        });
                        Self::append_with_paste_guard(&mut result, &arg, next_byte);
                    }
                    i += 1;
                }

                MacroBodyToken::VaArgs => {
                    // ── Standalone __VA_ARGS__ (not in # or ## context) ─
                    let mut lookahead = i + 1;
                    while lookahead < len && is_whitespace_token(&tokens[lookahead]) {
                        lookahead += 1;
                    }
                    if lookahead < len && matches!(tokens[lookahead], MacroBodyToken::HashHash) {
                        pending_va_args = true;
                    } else {
                        let va_args = self.get_va_args(params, expanded_args);
                        let next_byte = tokens.get(i + 1).and_then(|t| {
                            let s = super::macro_token::token_text(t);
                            s.as_bytes().first().copied()
                        });
                        Self::append_with_paste_guard(&mut result, &va_args, next_byte);
                    }
                    i += 1;
                }

                MacroBodyToken::Ident(s) => {
                    result.push_str(s);
                    i += 1;
                }

                _ => {
                    let text = super::macro_token::token_text(&tokens[i]);
                    result.push_str(text);
                    i += 1;
                }
            }
        }

        result
    }

    /// Paste the left side: a resolved param immediately before `##`.
    fn paste_left_param(
        &self,
        result: &mut String,
        idx: usize,
        args: &[String],
        is_variadic: bool,
        has_named_variadic: bool,
        params: &[String],
    ) {
        if is_variadic && has_named_variadic && idx == params.len() - 1 {
            let va_args_raw = self.get_named_va_args(idx, args);
            let va_args = strip_blue_paint(&va_args_raw);
            if va_args.is_empty() {
                while result.ends_with(' ') || result.ends_with('\t') {
                    result.pop();
                }
                if result.ends_with(',') {
                    result.pop();
                }
            } else {
                result.push_str(&va_args);
            }
        } else {
            let arg = args.get(idx).map_or("", std::string::String::as_str);
            let clean_arg = strip_blue_paint(arg);
            result.push_str(&clean_arg);
        }
    }

    /// Paste the right side: a resolved param immediately after `##`.
    fn paste_right_param(
        &self,
        result: &mut String,
        idx: usize,
        args: &[String],
        len: usize,
        i: usize,
        is_variadic: bool,
        has_named_variadic: bool,
        params: &[String],
    ) {
        if is_variadic && has_named_variadic && idx == params.len() - 1 {
            let va_args_raw = self.get_named_va_args(idx, args);
            let va_args = strip_blue_paint(&va_args_raw);
            if va_args.is_empty() {
                while result.ends_with(' ') || result.ends_with('\t') {
                    result.pop();
                }
                if result.ends_with(',') {
                    result.pop();
                }
            } else {
                result.push_str(&va_args);
            }
        } else {
            let arg = args.get(idx).map_or("", std::string::String::as_str);
            let clean_arg = strip_blue_paint(arg);
            if clean_arg.is_empty() {
                if i + 1 < len && !result.is_empty() {
                    result.push(' ');
                }
            } else {
                result.push_str(&clean_arg);
            }
        }
    }

    /// Get variadic arguments (__VA_ARGS__) as a comma-separated string.
    fn get_va_args(&self, params: &[String], args: &[String]) -> String {
        let named_count = params.len();
        if args.len() > named_count {
            args[named_count..].join(", ")
        } else {
            String::new()
        }
    }

    /// Get ALL arguments for a named variadic parameter (e.g., `extra...`).
    fn get_named_va_args(&self, param_idx: usize, args: &[String]) -> String {
        if args.len() > param_idx {
            args[param_idx..].join(", ")
        } else {
            String::new()
        }
    }
}

impl Default for MacroTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Check whether a tokenized macro body contains `#` or `##` operators.
fn has_stringify_or_paste(tokens: &[MacroBodyToken]) -> bool {
    tokens
        .iter()
        .any(|t| matches!(t, MacroBodyToken::Hash | MacroBodyToken::HashHash))
}

/// Quick helper for callers that construct a MacroDef.
pub(crate) fn macro_def_from_parts(
    name: String,
    is_function_like: bool,
    params: Vec<String>,
    is_variadic: bool,
    has_named_variadic: bool,
    body: String,
) -> MacroDef {
    let raw_tokens = tokenize_macro_body(&body);
    let tokenized_body = resolve_params(&raw_tokens, &params, is_variadic);
    let has_sp = has_stringify_or_paste(&raw_tokens);
    MacroDef {
        name,
        is_function_like,
        params,
        is_variadic,
        has_named_variadic,
        body,
        tokenized_body,
        has_stringify_or_paste: has_sp,
    }
}

/// Check whether position `pos` in `bytes` is part of a pp-number token.
fn is_ppnumber_context(bytes: &[u8], pos: usize) -> bool {
    let mut j = pos;
    loop {
        let ch = bytes[j];
        if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'.' {
            if j == 0 {
                return ch.is_ascii_digit();
            }
            j -= 1;
        } else if (ch == b'+' || ch == b'-')
            && j >= 1
            && matches!(bytes[j - 1], b'e' | b'E' | b'p' | b'P')
        {
            if j < 2 {
                return false;
            }
            j -= 2;
        } else {
            let start = bytes[j + 1];
            return start.is_ascii_digit()
                || (start == b'.' && j + 2 <= pos && bytes[j + 2].is_ascii_digit());
        }
    }
}

/// Extract the trailing identifier from a string, if it ends with one.
/// Returns Some(ident) if the string ends with an identifier (skipping trailing whitespace).
/// Returns None if the trailing identifier is blue-painted (preceded by \x01 marker).
/// Operates on bytes for performance (no Vec<char> allocation).
fn extract_trailing_ident(s: &str) -> Option<String> {
    let bytes = s.trim_end().as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let end = bytes.len();
    if !is_ident_cont_byte(bytes[end - 1]) {
        return None;
    }
    let mut start = end - 1;
    while start > 0 && is_ident_cont_byte(bytes[start - 1]) {
        start -= 1;
    }
    if !is_ident_start_byte(bytes[start]) {
        return None;
    }
    // Check for blue-paint marker immediately before the identifier
    if start > 0 && bytes[start - 1] == BLUE_PAINT_MARKER {
        return None;
    }
    Some(bytes_to_str(bytes, start, end).to_string())
}

/// Check if a string contains a given identifier as a standalone token
/// (not part of a larger identifier). Used to detect whether a function-like
/// macro name was already present in the pre-rescan body, indicating that
/// barrier tokens after it (like EMPTY()) were intentional and the identifier
/// should not be connected to trailing `(` from subsequent source tokens.
fn contains_standalone_ident(s: &str, ident: &str) -> bool {
    let s_bytes = s.as_bytes();
    let ident_bytes = ident.as_bytes();
    let ident_len = ident_bytes.len();
    if ident_len == 0 || s_bytes.len() < ident_len {
        return false;
    }

    let mut i = 0;
    while i + ident_len <= s_bytes.len() {
        if &s_bytes[i..i + ident_len] == ident_bytes {
            // Check that the character before is not an ident character
            let before_ok = i == 0 || !is_ident_cont_byte(s_bytes[i - 1]);
            // Check that the character after is not an ident character
            let after_ok =
                i + ident_len >= s_bytes.len() || !is_ident_cont_byte(s_bytes[i + ident_len]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Check if a `MacroBodyToken` is purely whitespace text.
fn is_whitespace_token(token: &MacroBodyToken) -> bool {
    match token {
        MacroBodyToken::Other(s) => s.chars().all(|c| c.is_ascii_whitespace()),
        _ => false,
    }
}

/// Stringify a macro argument per C11 6.10.3.2.
/// Operates on bytes to avoid Vec<char> allocation.
fn stringify_arg(arg: &str) -> String {
    let trimmed = arg.trim();
    let mut result = String::new();
    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Skip blue-paint markers (they must not appear in stringified output)
        if bytes[i] == BLUE_PAINT_MARKER {
            i += 1;
            continue;
        }

        // Collapse whitespace sequences to a single space
        if bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' {
            if !result.is_empty() {
                result.push(' ');
            }
            while i < len && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n') {
                i += 1;
            }
            continue;
        }

        // Handle string literals - escape " and \ inside them
        if bytes[i] == b'"' {
            result.push_str("\\\"");
            i += 1;
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    result.push_str("\\\\");
                    i += 1;
                    if i < len {
                        if bytes[i] == b'"' {
                            result.push_str("\\\"");
                        } else if bytes[i] == b'\\' {
                            result.push_str("\\\\");
                        } else {
                            result.push(bytes[i] as char);
                        }
                        i += 1;
                    }
                } else {
                    result.push(bytes[i] as char);
                    i += 1;
                }
            }
            if i < len {
                result.push_str("\\\"");
                i += 1;
            }
            continue;
        }

        // Handle char literals
        if bytes[i] == b'\'' {
            result.push('\'');
            i += 1;
            while i < len && bytes[i] != b'\'' {
                if bytes[i] == b'\\' {
                    result.push_str("\\\\");
                    i += 1;
                    if i < len {
                        if bytes[i] == b'\\' {
                            result.push_str("\\\\");
                        } else if bytes[i] == b'"' {
                            result.push_str("\\\"");
                        } else {
                            result.push(bytes[i] as char);
                        }
                        i += 1;
                    }
                } else if bytes[i] == b'"' {
                    result.push_str("\\\"");
                    i += 1;
                } else {
                    result.push(bytes[i] as char);
                    i += 1;
                }
            }
            if i < len {
                result.push('\'');
                i += 1;
            }
            continue;
        }

        result.push(bytes[i] as char);
        i += 1;
    }

    // Trim trailing space
    if result.ends_with(' ') {
        result.pop();
    }

    result
}

/// Parse a #define directive line and return a MacroDef.
/// The line should be the text after `#define ` (with leading whitespace stripped).
/// Operates on bytes for performance.
pub fn parse_define(line: &str) -> Option<MacroDef> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    // Parse macro name
    if !is_ident_start_byte(bytes[0]) {
        return None;
    }
    while i < len && is_ident_cont_byte(bytes[i]) {
        i += 1;
    }
    let name = bytes_to_str(bytes, 0, i).to_string();

    // Check if function-like (opening paren immediately after name, no space)
    if i < len && bytes[i] == b'(' {
        // Function-like macro
        i += 1; // skip '('
        let mut params = Vec::new();
        let mut is_variadic = false;
        let mut has_named_variadic = false;

        loop {
            while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }

            if i >= len {
                break;
            }

            if bytes[i] == b')' {
                i += 1;
                break;
            }

            if bytes[i] == b'.' && i + 2 < len && bytes[i + 1] == b'.' && bytes[i + 2] == b'.' {
                is_variadic = true;
                i += 3;
                while i < len && bytes[i] != b')' {
                    i += 1;
                }
                if i < len {
                    i += 1;
                }
                break;
            }

            if is_ident_start_byte(bytes[i]) {
                let start = i;
                while i < len && is_ident_cont_byte(bytes[i]) {
                    i += 1;
                }
                let param = bytes_to_str(bytes, start, i).to_string();

                if i + 2 < len && bytes[i] == b'.' && bytes[i + 1] == b'.' && bytes[i + 2] == b'.' {
                    is_variadic = true;
                    has_named_variadic = true;
                    params.push(param);
                    i += 3;
                    while i < len && bytes[i] != b')' {
                        i += 1;
                    }
                    if i < len {
                        i += 1;
                    }
                    break;
                }

                params.push(param);
            }

            while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }

            if i < len && bytes[i] == b',' {
                i += 1;
            }
        }

        // Rest is the body
        let body = if i < len {
            line[i..].trim().to_string()
        } else {
            String::new()
        };

        let raw_tokens = tokenize_macro_body(&body);
        let tokenized_body = resolve_params(&raw_tokens, &params, is_variadic);
        let has_sp = has_stringify_or_paste(&raw_tokens);
        Some(MacroDef {
            name,
            is_function_like: true,
            params,
            is_variadic,
            has_named_variadic,
            body,
            tokenized_body,
            has_stringify_or_paste: has_sp,
        })
    } else {
        // Object-like macro
        let body = if i < len {
            line[i..].trim().to_string()
        } else {
            String::new()
        };

        let raw_tokens = tokenize_macro_body(&body);
        let tokenized_body = resolve_params(&raw_tokens, &[], false);
        let has_sp = has_stringify_or_paste(&raw_tokens);
        Some(MacroDef {
            name,
            is_function_like: false,
            params: Vec::new(),
            is_variadic: false,
            has_named_variadic: false,
            body,
            tokenized_body,
            has_stringify_or_paste: has_sp,
        })
    }
}

/// Inline integer-to-string formatting buffer.
/// Avoids heap allocation for small integers (which __COUNTER__ produces).
mod itoa {
    pub struct Buffer {
        bytes: [u8; 20], // enough for u64::MAX
        len: usize,
    }

    impl Buffer {
        pub fn new() -> Self {
            Buffer {
                bytes: [0u8; 20],
                len: 0,
            }
        }

        pub fn format(&mut self, mut n: usize) -> &str {
            if n == 0 {
                self.bytes[0] = b'0';
                self.len = 1;
                return std::str::from_utf8(&self.bytes[..1])
                    .expect("digit buffer is not valid UTF-8");
            }
            let mut pos = 20;
            while n > 0 {
                pos -= 1;
                self.bytes[pos] = b'0' + (n % 10) as u8;
                n /= 10;
            }
            self.len = 20 - pos;
            // Shift to beginning for simpler return
            self.bytes.copy_within(pos..20, 0);
            // All bytes are ASCII digits (b'0'..=b'9'), which are valid UTF-8.
            std::str::from_utf8(&self.bytes[..self.len]).expect("digit buffer is not valid UTF-8")
        }
    }
}
