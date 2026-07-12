//! Token representation and tokenization for macro definition bodies.
//!
//! Macro bodies are tokenized once at `#define` time. The tokenized form is stored
//! in `MacroDef` and used during expansion for `#` (stringification) and `##` (paste)
//! operations instead of re-scanning the raw body text at each expansion.

use super::utils::{ident_cont_len, ident_start_len, is_ident_start_byte};

/// A token within a macro definition body.
///
/// Unlike the full parser token type (`co2_ast::Token`), this representation
/// treats all keywords as plain identifiers. This is essential because macro
/// parameters can have names that happen to be C keywords.
///
/// After construction, [`resolve_params`] replaces `Ident` tokens that match
/// parameter names with `Param(idx)` or `VaArgs`, eliminating the need for
/// repeated linear scans during expansion.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MacroBodyToken {
    Ident(String),
    Number(String),
    StringLit(String),
    CharLit(String),
    Hash,
    HashHash,
    LParen,
    RParen,
    Comma,
    /// Pre-resolved parameter reference. The `usize` is the index in the
    /// macro's `params` list. Set by [`resolve_params`].
    Param(usize),
    /// Pre-resolved reference to `__VA_ARGS__`.
    VaArgs,
    /// Any other text (operators, punctuation, whitespace).
    Other(String),
}

/// Tokenize a macro body string into a sequence of `MacroBodyToken` values.
///
/// The input text has already been through logical line extraction
/// (backslash-newline and comment stripping handled upstream), so this
/// tokenizer does not need to handle those preprocessing steps.
pub(crate) fn tokenize_macro_body(text: &str) -> Vec<MacroBodyToken> {
    let mut tokens = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        if b == b'"' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(MacroBodyToken::StringLit(text[start..i].to_string()));
            continue;
        }

        if b == b'\'' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'\'' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(MacroBodyToken::CharLit(text[start..i].to_string()));
            continue;
        }

        if ident_start_len(bytes, i).is_some() {
            let start = i;
            i += ident_start_len(bytes, i).unwrap();
            while let Some(cl) = ident_cont_len(bytes, i) {
                i += cl;
            }
            tokens.push(MacroBodyToken::Ident(text[start..i].to_string()));
            continue;
        }

        if b == b'#' && i + 1 < len && bytes[i + 1] == b'#' {
            tokens.push(MacroBodyToken::HashHash);
            i += 2;
            continue;
        }

        if b == b'#' {
            tokens.push(MacroBodyToken::Hash);
            i += 1;
            continue;
        }

        if b == b'(' {
            tokens.push(MacroBodyToken::LParen);
            i += 1;
            continue;
        }
        if b == b')' {
            tokens.push(MacroBodyToken::RParen);
            i += 1;
            continue;
        }
        if b == b',' {
            tokens.push(MacroBodyToken::Comma);
            i += 1;
            continue;
        }

        if b.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            tokens.push(MacroBodyToken::Number(text[start..i].to_string()));
            continue;
        }

        if b < 0x80 {
            // Coalesce runs of same-category bytes (whitespace or non-whitespace)
            // into a single Other token. This avoids one allocation per byte
            // for sequences like operators, whitespace runs, or punctuation.
            let is_ws = b.is_ascii_whitespace();
            let start = i;
            i += 1;
            while i < len {
                let c = bytes[i];
                if c == b'"'
                    || c == b'\''
                    || is_ident_start_byte(c)
                    || c == b'#'
                    || c == b'('
                    || c == b')'
                    || c == b','
                    || c.is_ascii_digit()
                    || c >= 0x80
                    || c.is_ascii_whitespace() != is_ws
                {
                    break;
                }
                i += 1;
            }
            tokens.push(MacroBodyToken::Other(text[start..i].to_string()));
        } else {
            let ch = text[i..].chars().next().unwrap();
            tokens.push(MacroBodyToken::Other(ch.to_string()));
            i += ch.len_utf8();
        }
    }

    tokens
}

/// Resolve `Ident` tokens that match parameter names into `Param(idx)` or `VaArgs`.
///
/// Called once at macro definition time. After resolution, `handle_stringify_and_paste`
/// can use the pre-resolved indices directly instead of doing `params.iter().position()`
/// linear scans during expansion.
pub(crate) fn resolve_params(
    tokens: &[MacroBodyToken],
    params: &[String],
    is_variadic: bool,
) -> Vec<MacroBodyToken> {
    tokens
        .iter()
        .map(|t| match t {
            MacroBodyToken::Ident(s) if s == "__VA_ARGS__" && is_variadic => MacroBodyToken::VaArgs,
            MacroBodyToken::Ident(s) => {
                if let Some(idx) = params.iter().position(|p| p == s) {
                    MacroBodyToken::Param(idx)
                } else {
                    t.clone()
                }
            }
            _ => t.clone(),
        })
        .collect()
}

/// Render a `MacroBodyToken` back to its text representation.
///
/// # Panics
/// Panics if called on `Param` or `VaArgs` — those are resolved at definition
/// time and must be handled by the caller before reaching this fallback.
pub(crate) fn token_text(token: &MacroBodyToken) -> &str {
    match token {
        MacroBodyToken::Ident(s)
        | MacroBodyToken::Number(s)
        | MacroBodyToken::StringLit(s)
        | MacroBodyToken::CharLit(s)
        | MacroBodyToken::Other(s) => s,
        MacroBodyToken::Hash => "#",
        MacroBodyToken::HashHash => "##",
        MacroBodyToken::LParen => "(",
        MacroBodyToken::RParen => ")",
        MacroBodyToken::Comma => ",",
        MacroBodyToken::Param(_) | MacroBodyToken::VaArgs => {
            unreachable!("Param/VaArgs must be handled before token_text")
        }
    }
}
