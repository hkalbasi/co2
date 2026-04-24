//! Preprocessor conditional expression evaluation.
//!
//! This module handles evaluation of preprocessor conditional expressions (`#if`, `#elif`),
//! including `defined()` operator resolution, `__has_builtin()`, `__has_attribute()`,
//! `__has_include()`, and `__has_include_next()` detection,
//! and replacing undefined identifiers with 0 per the C standard.
//!
//! All scanning operates on byte slices for performance (no Vec<char> allocation).

use super::pipeline::Preprocessor;
use super::utils::{is_ident_start_byte, is_ident_cont_byte, bytes_to_str};

impl Preprocessor {
    /// Replace remaining identifiers (not keywords) with 0 in a #if expression.
    /// Per C standard, after macro expansion, undefined identifiers in #if evaluate to 0.
    pub(super) fn replace_remaining_idents_with_zero(&self, expr: &str) -> String {
        let mut result = String::new();
        let bytes = expr.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            // Skip character literals verbatim
            if bytes[i] == b'\'' {
                result.push(bytes[i] as char);
                i += 1;
                while i < len && bytes[i] != b'\'' {
                    if bytes[i] == b'\\' && i + 1 < len {
                        result.push(bytes[i] as char);
                        i += 1;
                        result.push(bytes[i] as char);
                        i += 1;
                    } else {
                        result.push(bytes[i] as char);
                        i += 1;
                    }
                }
                if i < len && bytes[i] == b'\'' {
                    result.push(bytes[i] as char);
                    i += 1;
                }
                continue;
            }
            // Skip string literals verbatim
            if bytes[i] == b'"' {
                result.push(bytes[i] as char);
                i += 1;
                while i < len && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < len {
                        result.push(bytes[i] as char);
                        i += 1;
                        result.push(bytes[i] as char);
                        i += 1;
                    } else {
                        result.push(bytes[i] as char);
                        i += 1;
                    }
                }
                if i < len && bytes[i] == b'"' {
                    result.push(bytes[i] as char);
                    i += 1;
                }
                continue;
            }
            if bytes[i].is_ascii_digit() {
                // Skip entire number literal
                result.push(bytes[i] as char);
                i += 1;
                if i < len && bytes[i - 1] == b'0' && (bytes[i] == b'x' || bytes[i] == b'X' || bytes[i] == b'b' || bytes[i] == b'B') {
                    result.push(bytes[i] as char);
                    i += 1;
                }
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                    result.push(bytes[i] as char);
                    i += 1;
                }
            } else if is_ident_start_byte(bytes[i]) {
                let start = i;
                i += 1;
                while i < len && is_ident_cont_byte(bytes[i]) {
                    i += 1;
                }
                let ident = bytes_to_str(bytes, start, i);
                if ident == "defined" || ident == "true" || ident == "false" {
                    result.push_str(ident);
                } else {
                    result.push('0');
                }
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        }

        result
    }

    /// Replace `defined(X)`, `defined X`, `__has_builtin(X)`, `__has_attribute(X)`,
    /// `__has_feature(X)`, `__has_extension(X)`, `__has_include(X)`, and
    /// `__has_include_next(X)` with 0 or 1 in a #if expression.
    pub(super) fn resolve_defined_in_expr(&mut self, expr: &str) -> String {
        let mut result = String::new();
        let bytes = expr.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            if is_ident_start_byte(bytes[i]) {
                let start = i;
                i += 1;
                while i < len && is_ident_cont_byte(bytes[i]) {
                    i += 1;
                }
                let ident = bytes_to_str(bytes, start, i);

                if ident == "defined" {
                    while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                        i += 1;
                    }
                    let has_paren = if i < len && bytes[i] == b'(' {
                        i += 1;
                        while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                            i += 1;
                        }
                        true
                    } else {
                        false
                    };

                    let name_start = i;
                    while i < len && is_ident_cont_byte(bytes[i]) {
                        i += 1;
                    }
                    let name = bytes_to_str(bytes, name_start, i);

                    if has_paren {
                        while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                            i += 1;
                        }
                        if i < len && bytes[i] == b')' {
                            i += 1;
                        }
                    }

                    let is_def = self.macros.is_defined(name);
                    result.push_str(if is_def { "1" } else { "0" });
                } else if ident == "__has_builtin" {
                    let val = self.resolve_has_builtin_call_bytes(bytes, &mut i);
                    result.push_str(val);
                } else if ident == "__has_attribute" {
                    let val = self.resolve_has_attribute_call_bytes(bytes, &mut i);
                    result.push_str(val);
                } else if ident == "__has_feature" || ident == "__has_extension" {
                    self.skip_paren_arg_bytes(bytes, &mut i);
                    result.push('0');
                } else if ident == "__has_include" {
                    let val = self.resolve_has_include_call_bytes(bytes, &mut i, false);
                    result.push_str(val);
                } else if ident == "__has_include_next" {
                    let val = self.resolve_has_include_call_bytes(bytes, &mut i, true);
                    result.push_str(val);
                } else {
                    result.push_str(ident);
                }
                continue;
            }

            result.push(bytes[i] as char);
            i += 1;
        }

        result
    }

    /// Parse `(name)` after `__has_builtin` and return "1" or "0" (byte-oriented).
    fn resolve_has_builtin_call_bytes(&self, bytes: &[u8], i: &mut usize) -> &'static str {
        let len = bytes.len();
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b'(' {
            return "0";
        }
        *i += 1;
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        let start = *i;
        while *i < len && is_ident_cont_byte(bytes[*i]) {
            *i += 1;
        }
        let name = bytes_to_str(bytes, start, *i);
        while *i < len && bytes[*i] != b')' {
            *i += 1;
        }
        if *i < len {
            *i += 1;
        }
        if Self::is_supported_builtin(name) { "1" } else { "0" }
    }

    /// Parse `(name)` after `__has_attribute` and return "1" or "0" (byte-oriented).
    fn resolve_has_attribute_call_bytes(&self, bytes: &[u8], i: &mut usize) -> &'static str {
        let len = bytes.len();
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b'(' {
            return "0";
        }
        *i += 1;
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        let start = *i;
        while *i < len && is_ident_cont_byte(bytes[*i]) {
            *i += 1;
        }
        let name = bytes_to_str(bytes, start, *i);
        while *i < len && bytes[*i] != b')' {
            *i += 1;
        }
        if *i < len {
            *i += 1;
        }
        if Self::is_supported_attribute(name) { "1" } else { "0" }
    }

    /// Parse `(<header.h>)` or `("header.h")` after `__has_include` / `__has_include_next`
    /// and return "1" or "0" based on whether the header can be found.
    fn resolve_has_include_call_bytes(&mut self, bytes: &[u8], i: &mut usize, is_next: bool) -> &'static str {
        let len = bytes.len();
        // Skip whitespace
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b'(' {
            return "0";
        }
        *i += 1; // skip '('
        // Skip whitespace
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }

        // Determine if system include (<...>) or quoted ("...")
        let (header_name, is_system) = if *i < len && bytes[*i] == b'<' {
            *i += 1; // skip '<'
            let start = *i;
            while *i < len && bytes[*i] != b'>' {
                *i += 1;
            }
            let name = bytes_to_str(bytes, start, *i);
            if *i < len { *i += 1; } // skip '>'
            (name, true)
        } else if *i < len && bytes[*i] == b'"' {
            *i += 1; // skip '"'
            let start = *i;
            while *i < len && bytes[*i] != b'"' {
                *i += 1;
            }
            let name = bytes_to_str(bytes, start, *i);
            if *i < len { *i += 1; } // skip closing '"'
            (name, false)
        } else {
            // Fallback: try to read as identifier (e.g. macro-expanded argument)
            let start = *i;
            while *i < len && bytes[*i] != b')' {
                *i += 1;
            }
            let name = bytes_to_str(bytes, start, *i).trim();
            (name, false)
        };

        // Skip to closing paren
        while *i < len && bytes[*i] != b')' {
            *i += 1;
        }
        if *i < len { *i += 1; } // skip ')'

        if header_name.is_empty() {
            return "0";
        }

        // Use the preprocessor's include path resolution
        let found = if is_next {
            let current_file = self.include_stack.last().cloned();
            self.resolve_include_next_path(header_name, current_file.as_ref()).is_some()
        } else {
            self.resolve_include_path(header_name, is_system).is_some()
        };

        if found { "1" } else { "0" }
    }

    /// Skip a parenthesized argument (byte-oriented).
    fn skip_paren_arg_bytes(&self, bytes: &[u8], i: &mut usize) {
        let len = bytes.len();
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i < len && bytes[*i] == b'(' {
            *i += 1;
            let mut depth = 1;
            while *i < len && depth > 0 {
                if bytes[*i] == b'(' {
                    depth += 1;
                } else if bytes[*i] == b')' {
                    depth -= 1;
                }
                *i += 1;
            }
        }
    }

    /// Check if a builtin function name is supported by this compiler.
    /// Delegates to the sema builtin registry (BUILTIN_MAP + atomic builtins)
    /// and adds a few names handled as special syntax rather than function calls.
    pub(super) fn is_supported_builtin(name: &str) -> bool {
        // Check the canonical builtin registry (covers BUILTIN_MAP, __builtin_choose_expr,
        // __builtin_unreachable, __builtin_trap, and all __atomic_*/__sync_* builtins)
        if crate::frontend::sema::builtins::is_builtin(name) {
            return true;
        }
        // These are handled as special syntax (dedicated AST nodes or parser keywords)
        // rather than through the normal builtin function call path, so they are not
        // in the sema builtin registry but are still supported.
        matches!(name,
            "__builtin_va_arg" |              // Special token (BuiltinVaArg)
            "__builtin_types_compatible_p" |  // Special AST node (BuiltinTypesCompatibleP)
            "__builtin_offsetof"              // Predefined macro
        )
    }

    /// Check if an attribute is supported by this compiler.
    pub(super) fn is_supported_attribute(name: &str) -> bool {
        matches!(name,
            "aligned" | "__aligned__" |
            "packed" | "__packed__" |
            "unused" | "__unused__" |
            "used" | "__used__" |
            "weak" | "__weak__" |
            "alias" | "__alias__" |
            "section" | "__section__" |
            "visibility" | "__visibility__" |
            "deprecated" | "__deprecated__" |
            "noreturn" | "__noreturn__" |
            "noinline" | "__noinline__" |
            "always_inline" | "__always_inline__" |
            "constructor" | "__constructor__" |
            "destructor" | "__destructor__" |
            "format" | "__format__" |
            "warn_unused_result" | "__warn_unused_result__" |
            "nonnull" | "__nonnull__" |
            "const" | "__const__" |
            "pure" | "__pure__" |
            "cold" | "__cold__" |
            "hot" | "__hot__" |
            "malloc" | "__malloc__" |
            "sentinel" | "__sentinel__" |
            "may_alias" | "__may_alias__" |
            "transparent_union" | "__transparent_union__" |
            "error" | "__error__" |
            "warning" | "__warning__" |
            "cleanup" | "__cleanup__" |
            "fallthrough" | "__fallthrough__" |
            "flatten" | "__flatten__" |
            "nonstring" | "__nonstring__" |
            "uninitialized" | "__uninitialized__" |
            "annotate" | "__annotate__" |
            "no_instrument_function" | "__no_instrument_function__" |
            "alloc_size" | "__alloc_size__" |
            "format_arg" | "__format_arg__" |
            "no_sanitize" | "__no_sanitize__" |
            "no_sanitize_address" | "__no_sanitize_address__" |
            "no_sanitize_thread" | "__no_sanitize_thread__" |
            "no_sanitize_undefined" | "__no_sanitize_undefined__" |
            "noclone" | "__noclone__" |
            "optimize" | "__optimize__" |
            "target" | "__target__" |
            "assume_aligned" | "__assume_aligned__" |
            "returns_nonnull" | "__returns_nonnull__" |
            "externally_visible" | "__externally_visible__" |
            "artificial" | "__artificial__" |
            "leaf" | "__leaf__" |
            "access" | "__access__" |
            "fd_arg" | "__fd_arg__" |
            "tls_model" | "__tls_model__"
        )
    }
}
