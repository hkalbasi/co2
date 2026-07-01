//! Full C preprocessor implementation.
//!
//! Struct definition, core preprocessing pipeline, directive dispatch,
//! and public configuration API. Predefined macros and target configuration
//! live in `predefined_macros`, pragma handling in `pragmas`, and text
//! processing (comment stripping, line joining) in `text_processing`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU32, Ordering},
};

use co2_ast::{FileId, Rich, Span, Spanned, Token};

use super::builtin_macros::define_builtin_macros;
use super::conditionals::{ConditionalStack, evaluate_condition};
use super::macro_defs::{MacroDef, MacroTable, macro_def_from_parts, parse_define};
use super::macro_token::{resolve_params, tokenize_macro_body};
use super::text_processing::{LogicalSlice, split_first_word, strip_line_comment};
use super::tokenizer;
use super::utils::{is_ident_cont, is_ident_start};

use crate::SourceFile;

/// Maximum number of newlines to accumulate while joining lines for unbalanced
/// parentheses in macro arguments. Prevents runaway accumulation when a source
/// file has a genuinely unbalanced parenthesis. Must be large enough to handle
/// real-world macro calls that span many lines (e.g., QEMU's qapi-introspect.c
/// has a single QLIT_QLIST() macro invocation spanning ~32000 lines).
const MAX_PENDING_NEWLINES: usize = 100_000;

#[derive(Debug, Clone)]
pub struct PreprocessorDiagnostic {
    pub file: String,
    pub range: Range<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
struct PendingExpansion {
    text: String,
    slices: Vec<LogicalSlice>,
}

impl PendingExpansion {
    fn is_empty(&self) -> bool {
        self.slices.is_empty()
    }

    fn clear(&mut self) {
        self.text.clear();
        self.slices.clear();
    }

    fn push(&mut self, slice: &LogicalSlice) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(&slice.text);
        self.slices.push(slice.clone());
    }

    fn push_gap(&mut self, slice: &LogicalSlice) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.slices.push(LogicalSlice {
            text: String::new(),
            source_boundaries: vec![slice.source_offset(0)],
            terminator: slice.terminator,
        });
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn line_count(&self) -> usize {
        self.slices.len()
    }

    fn joined_source(&self) -> (String, Vec<usize>) {
        let mut text = String::new();
        let mut boundaries = Vec::new();
        for (idx, slice) in self.slices.iter().enumerate() {
            if idx == 0 {
                text.push_str(&slice.text);
                boundaries.extend_from_slice(&slice.source_boundaries);
                continue;
            }
            text.push('\n');
            boundaries.push(self.slices[idx - 1].terminator);
            text.push_str(&slice.text);
            boundaries.extend_from_slice(&slice.source_boundaries[1..]);
        }
        if boundaries.is_empty() {
            boundaries.push(0);
        }
        (text, boundaries)
    }
}

pub struct Preprocessor {
    pub(super) macros: MacroTable,
    conditionals: ConditionalStack,
    pub(super) includes: Vec<String>,
    pub(super) filename: String,
    pub(super) errors: Vec<PreprocessorDiagnostic>,
    /// Collected preprocessor warnings (e.g., #warning directives).
    pub(super) warnings: Vec<PreprocessorDiagnostic>,
    /// Include search paths (from -I flags)
    pub(super) include_paths: Vec<PathBuf>,
    /// Quote include paths (from -iquote flags), searched only for #include "..."
    pub(super) quote_include_paths: Vec<PathBuf>,
    /// System include paths explicitly added (from -isystem flags)
    pub(super) isystem_include_paths: Vec<PathBuf>,
    /// System include paths (default search paths)
    pub(super) system_include_paths: Vec<PathBuf>,
    /// Files currently being processed (for recursion detection)
    pub(super) include_stack: Vec<PathBuf>,
    /// Files that have been included with #pragma once
    pub(super) pragma_once_files: HashSet<PathBuf>,
    /// Declarations to inject into the output (from #include processing).
    pub(super) pending_injections: Vec<String>,
    /// Stack for #pragma push_macro / pop_macro.
    /// Maps macro name -> stack of saved definitions (None = was undefined).
    pub(super) macro_save_stack: HashMap<String, Vec<Option<MacroDef>>>,
    /// Cache for include path resolution.
    /// Maps (include_path, is_system, current_dir_key) to the resolved filesystem path.
    /// This avoids repeated `stat()` calls when the same header is included from
    /// multiple locations with the same include search path configuration.
    /// The current_dir_key is the parent directory of the including file for quoted
    /// includes (since resolution depends on it), or empty for system includes.
    pub(super) include_resolve_cache: HashMap<(String, bool, PathBuf), Option<PathBuf>>,
    /// Include guard detection: maps file paths to their guard macro names.
    ///
    /// After preprocessing an included file, we scan the raw source to detect if
    /// the entire file is wrapped in a classic include guard pattern:
    ///   #ifndef GUARD_MACRO
    ///   #define GUARD_MACRO
    ///   ...
    ///   #endif
    ///
    /// On subsequent #include of the same file, if the guard macro is still defined,
    /// we skip re-processing entirely (same optimization as GCC/Clang).
    pub(super) include_guard_macros: HashMap<PathBuf, String>,
    /// Reusable set for directive-level macro expansion (handle_if, handle_elif,
    /// #error). Avoids allocating a new set per directive.
    directive_expanding: HashSet<String>,

    // ── Single-pass token output accumulators ──────────────────────────
    /// Concatenated raw preprocessed text (no normalization), for raw_src.
    pub(super) raw_text: String,
    /// Final token stream with spans resolved to original source positions.
    pub(super) output_tokens: Vec<Spanned<Token>>,
    /// Source file registry for diagnostics.
    pub(super) source_files: HashMap<FileId, SourceFile>,
    /// Path → FileId lookup.
    pub(super) file_index: HashMap<PathBuf, FileId>,
    /// Rewrite boundary map for the main source file.
    pub(super) main_rewrite_boundaries: Vec<usize>,
    /// Tokenizer warnings, accumulated during emission.
    pub(super) lexer_warnings: Vec<Rich<'static, String, Span>>,
    /// Tokenizer errors, accumulated during emission.
    pub(super) lexer_errors: Vec<Rich<'static, String, Span>>,
}

impl Preprocessor {
    pub fn new() -> Self {
        let mut pp = Self {
            macros: MacroTable::new(),
            conditionals: ConditionalStack::new(),
            includes: Vec::new(),
            filename: String::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            include_paths: Vec::new(),
            quote_include_paths: Vec::new(),
            isystem_include_paths: Vec::new(),
            system_include_paths: Vec::new(),
            include_stack: Vec::new(),
            pragma_once_files: HashSet::new(),
            pending_injections: Vec::new(),
            macro_save_stack: HashMap::new(),
            include_resolve_cache: HashMap::new(),
            include_guard_macros: HashMap::new(),
            directive_expanding: HashSet::new(),
            raw_text: String::new(),
            output_tokens: Vec::new(),
            source_files: HashMap::new(),
            file_index: HashMap::new(),
            main_rewrite_boundaries: Vec::new(),
            lexer_warnings: Vec::new(),
            lexer_errors: Vec::new(),
        };
        pp.define_predefined_macros();
        define_builtin_macros(&mut pp.macros);
        pp
    }

    /// Preprocess the main source file and return the final PreprocessedSource.
    pub fn preprocess(&mut self, source: &str, boundaries: &[usize]) -> crate::PreprocessedSource {
        self.main_rewrite_boundaries = boundaries.to_vec();
        self.preprocess_source(source, false);

        if let Some(range) = self.conditionals.unclosed_range() {
            self.errors.push(PreprocessorDiagnostic {
                file: self.filename.clone(),
                range,
                message: "Unterminated conditional directive".to_string(),
            });
        }

        // Emit accumulated lexer diagnostics
        self.emit_lexer_diagnostics();

        let main_file_idx = self.ensure_file(Path::new(&self.filename.clone()));

        crate::PreprocessedSource {
            raw_src: Arc::<str>::from(std::mem::take(&mut self.raw_text)),
            tokens: Arc::new(std::mem::take(&mut self.output_tokens)),
            main_file_idx,
            files: Arc::new(std::mem::take(&mut self.source_files)),
            main_rewrite_boundaries: Arc::new(std::mem::take(&mut self.main_rewrite_boundaries)),
        }
    }

    pub(super) fn preprocess_included(&mut self, source: &str) {
        self.preprocess_source(source, true);
    }

    fn preprocess_source(&mut self, source: &str, is_include: bool) {
        let saved_conditionals = is_include.then(|| std::mem::take(&mut self.conditionals));
        let mut pending = PendingExpansion::default();
        let mut expanding = HashSet::new();

        for slice in Self::logical_slices(source) {
            let trimmed = slice.text.trim();
            let is_directive = trimmed.starts_with('#') && !trimmed.starts_with("#[");
            let is_conditional_directive = if is_directive {
                let after_hash = trimmed[1..].trim_start();
                after_hash.starts_with("if")
                    || after_hash.starts_with("elif")
                    || after_hash.starts_with("else")
                    || after_hash.starts_with("endif")
            } else {
                false
            };
            let is_state_only_directive = if is_directive && !is_conditional_directive {
                let after_hash = trimmed[1..].trim_start();
                after_hash.starts_with("define") || after_hash.starts_with("undef")
            } else {
                false
            };

            if !pending.is_empty()
                && (is_conditional_directive || (is_include && is_state_only_directive))
            {
                if is_conditional_directive || self.conditionals.is_active() {
                    self.process_directive(&slice);
                }
                self.emit_blank_slice(&slice);
                continue;
            }

            let process_directive = is_directive && (!is_include || pending.is_empty());

            if process_directive {
                let after_hash = trimmed[1..].trim_start();
                if !pending.is_empty()
                    && (!is_include
                        && (after_hash.starts_with("define")
                            || after_hash.starts_with("undef")
                            || after_hash.starts_with("include")))
                {
                    self.flush_pending(&mut pending, &mut expanding);
                }

                let had_content = self.process_directive(&slice);
                if !had_content && is_include {
                    self.emit_blank_slice(&slice);
                }

                if !is_include && !self.pending_injections.is_empty() {
                    for decl in std::mem::take(&mut self.pending_injections) {
                        self.emit_synthetic_text(decl);
                    }
                }

                if !is_include {
                    self.emit_blank_slice(&slice);
                }
            } else if self.conditionals.is_active() {
                self.accumulate_and_expand(&slice, &mut pending, &mut expanding);
            } else if pending.is_empty() {
                self.emit_blank_slice(&slice);
            } else {
                pending.push_gap(&slice);
            }
        }

        self.flush_pending(&mut pending, &mut expanding);

        if let Some(saved) = saved_conditionals {
            self.conditionals = saved;
        }
    }

    fn accumulate_and_expand(
        &mut self,
        slice: &LogicalSlice,
        pending: &mut PendingExpansion,
        expanding: &mut HashSet<String>,
    ) {
        if pending.is_empty() {
            if Self::has_unbalanced_parens(&slice.text)
                || self.ends_with_funclike_macro(&slice.text)
            {
                pending.push(slice);
                return;
            }

            let expanded = self.macros.expand_line_reuse(&slice.text, expanding);
            if self.ends_with_funclike_macro(&expanded) {
                pending.push(slice);
            } else {
                self.emit_text_from_slice(slice, expanded);
                self.emit_newline();
            }
            return;
        }

        let needs_more = Self::has_unbalanced_parens(pending.text());
        pending.push(slice);

        if pending.line_count() > MAX_PENDING_NEWLINES {
            self.flush_pending(pending, expanding);
            return;
        }

        if needs_more {
            let expanded = self.macros.expand_line_reuse(pending.text(), expanding);
            if !Self::has_unbalanced_parens(pending.text())
                && !self.ends_with_funclike_macro(&expanded)
            {
                self.emit_pending_output(pending, expanded);
                pending.clear();
            }
            return;
        }

        if Self::has_unbalanced_parens(pending.text()) {
            return;
        }

        let expanded = self.macros.expand_line_reuse(pending.text(), expanding);
        if !self.ends_with_funclike_macro(&expanded) {
            self.emit_pending_output(pending, expanded);
            pending.clear();
        }
    }

    fn flush_pending(&mut self, pending: &mut PendingExpansion, expanding: &mut HashSet<String>) {
        if pending.is_empty() {
            return;
        }
        let expanded = self.macros.expand_line_reuse(pending.text(), expanding);
        self.emit_pending_output(pending, expanded);
        pending.clear();
    }

    fn emit_pending_output(&mut self, pending: &PendingExpansion, expanded: String) {
        let (source_text, source_boundaries) = pending.joined_source();
        self.emit_tokenized_with_remap(expanded, source_text, source_boundaries);
        for _slice in &pending.slices {
            self.emit_newline();
        }
    }

    fn emit_blank_slice(&mut self, _slice: &LogicalSlice) {
        self.emit_newline();
    }

    fn emit_text_from_slice(&mut self, slice: &LogicalSlice, raw: String) {
        self.emit_tokenized_with_remap(raw, slice.text.clone(), slice.source_boundaries.clone());
    }

    fn emit_newline(&mut self) {
        self.raw_text.push('\n');
    }

    /// Wrapper: remap `source_boundaries` for the main file, then emit.
    fn emit_tokenized_with_remap(
        &mut self,
        raw: String,
        source_text: String,
        source_boundaries: Vec<usize>,
    ) {
        if raw.is_empty() {
            return;
        }
        let sb: Vec<usize> = if self.current_path() == Path::new(&self.filename)
            && !self.main_rewrite_boundaries.is_empty()
        {
            source_boundaries
                .iter()
                .map(|offset| {
                    self.main_rewrite_boundaries
                        .get(*offset)
                        .copied()
                        .unwrap_or(*self.main_rewrite_boundaries.last().unwrap_or(&0))
                })
                .collect()
        } else {
            source_boundaries
        };
        self.emit_tokenized_chunk(&raw, &source_text, &sb);
    }

    /// Check if a line ends with an identifier that is a defined function-like macro.
    /// This is used to detect cases where the macro arguments '(' might be on the next line.
    fn ends_with_funclike_macro(&self, line: &str) -> bool {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            return false;
        }
        // Extract the last identifier from the line
        let bytes = trimmed.as_bytes();
        let end = bytes.len();
        // Walk backwards to find end of last identifier
        if !is_ident_cont(bytes[end - 1] as char) {
            return false;
        }
        let mut start = end - 1;
        while start > 0 && is_ident_cont(bytes[start - 1] as char) {
            start -= 1;
        }
        // Check that the identifier starts with a valid start character
        if !is_ident_start(bytes[start] as char) {
            return false;
        }
        let ident = &trimmed[start..end];
        // Check if this identifier is a defined function-like macro
        if let Some(mac) = self.macros.get(ident) {
            mac.is_function_like
        } else {
            false
        }
    }

    /// Set the filename for __FILE__ and __BASE_FILE__ macros and set as the base include directory.
    pub fn set_filename(&mut self, filename: &str) {
        self.filename = filename.to_string();
        self.macros.set_file(format!("\"{filename}\""));
        // __BASE_FILE__ always expands to the main input file name,
        // unlike __FILE__ which changes during #include processing.
        self.macros.define(macro_def_from_parts(
            "__BASE_FILE__".to_string(),
            false,
            Vec::new(),
            false,
            false,
            format!("\"{filename}\""),
        ));
        // Push the file path onto the include stack for relative includes.
        // Use make_absolute (not canonicalize) to preserve symlinks, matching GCC
        // behavior: #include "..." searches relative to the directory where the
        // including file was found (through symlinks), not the symlink target.
        let path = PathBuf::from(filename);
        let abs = super::includes::make_absolute(&path);
        self.include_stack.push(abs);
    }

    /// Get preprocessing errors.
    pub fn errors(&self) -> &[PreprocessorDiagnostic] {
        &self.errors
    }

    /// Get preprocessing warnings.
    pub fn warnings(&self) -> &[PreprocessorDiagnostic] {
        &self.warnings
    }

    /// Get the current source file name for diagnostic purposes.
    /// Returns the top of the include stack (the file currently being preprocessed),
    /// falling back to `self.filename` for the main translation unit.
    pub(super) fn current_file(&self) -> String {
        self.current_path().display().to_string()
    }

    pub(super) fn current_path(&self) -> PathBuf {
        self.include_stack
            .last()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(&self.filename))
    }

    /// Define a macro from a command-line -D flag.
    /// Takes a name and value (e.g., name="FOO", value="1").
    pub fn define_macro(&mut self, name: &str, value: &str) {
        self.macros.define(macro_def_from_parts(
            name.to_string(),
            false,
            Vec::new(),
            false,
            false,
            value.to_string(),
        ));
    }

    /// Undefine a macro by name.
    pub fn undefine_macro(&mut self, name: &str) {
        self.macros.undefine(name);
    }

    /// Add an include search path for #include directives (-I flag).
    /// Adds regardless of whether the directory currently exists.
    pub fn add_include_path(&mut self, path: &str) {
        self.include_paths.push(PathBuf::from(path));
    }

    /// Add a quote-only include search path (-iquote flag).
    /// These paths are searched only for `#include "file"`, not `#include <file>`.
    /// Searched after current directory, before -I paths.
    pub fn add_quote_include_path(&mut self, path: &str) {
        self.quote_include_paths.push(PathBuf::from(path));
    }

    /// Add a system include path (-isystem flag).
    /// These paths are searched after -I paths, before default system paths.
    pub fn add_system_include_path(&mut self, path: &str) {
        self.isystem_include_paths.push(PathBuf::from(path));
    }

    /// Process a force-included file (-include flag). This preprocesses the file content
    /// as if it were #include'd at the very beginning of the main source file.
    /// All #define directives in the file take effect, and the preprocessed output
    /// is discarded (macros/typedefs persist in the preprocessor state).
    pub fn preprocess_force_include(&mut self, content: &str, resolved_path: &str) {
        let resolved = PathBuf::from(resolved_path);

        // Check for #pragma once
        if self.pragma_once_files.contains(&resolved) {
            return;
        }

        // Check for include guard
        if let Some(guard) = self.include_guard_macros.get(&resolved)
            && self.macros.is_defined(guard)
        {
            return;
        }

        // Push onto include stack
        self.include_stack.push(resolved.clone());

        // Save and set __FILE__ (uses set_file to avoid full MacroDef allocation)
        let old_file = self
            .macros
            .get_file_body()
            .map(std::string::ToString::to_string);
        self.macros.set_file(format!("\"{}\"", resolved.display()));

        // Preprocess the included content (accumulated directly into self)
        self.preprocess_included(content);

        // Restore __FILE__
        if let Some(old) = old_file {
            self.macros.set_file(old);
        }

        // Pop include stack
        self.include_stack.pop();
    }

    fn process_directive(&mut self, slice: &LogicalSlice) -> bool {
        let hash_pos = slice.text.find('#').unwrap_or(0);
        let after_hash_raw = &slice.text[hash_pos + 1..];
        let after_hash = after_hash_raw.trim_start();
        let keyword_offset = hash_pos + 1 + (after_hash_raw.len() - after_hash.len());
        let after_hash = strip_line_comment(after_hash);
        let after_hash_ref = after_hash.as_ref();
        let (keyword, rest) = split_first_word(after_hash_ref);
        let mut rest_offset = keyword_offset + keyword.len();
        while rest_offset < slice.text.len()
            && slice.text.as_bytes()[rest_offset].is_ascii_whitespace()
        {
            rest_offset += 1;
        }

        let (keyword, rest, rest_offset) = if keyword.starts_with("include<")
            || keyword.starts_with("include\"")
        {
            (
                "include",
                &after_hash_ref["include".len()..],
                keyword_offset + "include".len(),
            )
        } else if keyword.starts_with("include_next<") || keyword.starts_with("include_next\"") {
            (
                "include_next",
                &after_hash_ref["include_next".len()..],
                keyword_offset + "include_next".len(),
            )
        } else {
            (keyword, rest, rest_offset)
        };
        match keyword {
            "ifdef" | "ifndef" | "if" if !self.conditionals.is_active() => {
                self.conditionals.push_if(
                    false,
                    self.slice_range(slice, keyword_offset, keyword_offset + keyword.len().max(1)),
                );
                return false;
            }
            "elif" => {
                self.handle_elif(rest);
                return false;
            }
            "else" => {
                self.conditionals.handle_else();
                return false;
            }
            "endif" => {
                self.conditionals.handle_endif();
                return false;
            }
            _ if !self.conditionals.is_active() => return false,
            _ => {}
        }

        match keyword {
            "include" => {
                return self.handle_include(
                    rest,
                    self.slice_range(slice, rest_offset, rest_offset + rest.len().max(1)),
                );
            }
            "include_next" => {
                return self.handle_include_next(
                    rest,
                    self.slice_range(slice, rest_offset, rest_offset + rest.len().max(1)),
                );
            }
            "define" => self.handle_define(rest),
            "undef" => self.handle_undef(rest),
            "ifdef" => self.handle_ifdef(
                rest,
                false,
                self.slice_range(slice, keyword_offset, keyword_offset + keyword.len().max(1)),
            ),
            "ifndef" => self.handle_ifdef(
                rest,
                true,
                self.slice_range(slice, keyword_offset, keyword_offset + keyword.len().max(1)),
            ),
            "if" => self.handle_if(
                rest,
                self.slice_range(slice, keyword_offset, keyword_offset + keyword.len().max(1)),
            ),
            "pragma" => {
                if let Some(raw) = self.handle_pragma(rest) {
                    self.emit_synthetic_text(raw);
                    return true;
                }
                return false;
            }
            "error" => {
                let expanded = self
                    .macros
                    .expand_line_reuse(rest, &mut self.directive_expanding);
                self.errors.push(PreprocessorDiagnostic {
                    file: self.current_file(),
                    range: self.slice_range(
                        slice,
                        keyword_offset,
                        keyword_offset + keyword.len().max(1),
                    ),
                    message: format!("#error {expanded}"),
                });
            }
            "warning" => {
                self.warnings.push(PreprocessorDiagnostic {
                    file: self.current_file(),
                    range: self.slice_range(
                        slice,
                        keyword_offset,
                        keyword_offset + keyword.len().max(1),
                    ),
                    message: format!("#warning {rest}"),
                });
            }
            _ => {}
        }

        false
    }

    fn slice_range(&self, slice: &LogicalSlice, start: usize, end: usize) -> Range<usize> {
        let text_start = start.min(slice.text.len());
        let text_end = end.min(slice.text.len()).max(text_start);
        let start = slice.source_offset(text_start);
        let end = slice.source_offset(text_end).max(start);
        start..end
    }

    fn handle_define(&mut self, rest: &str) {
        if let Some(mut def) = parse_define(rest) {
            if def.name == "offsetof" && def.is_function_like && def.params == ["type", "field"] {
                def.body = "__builtin_offsetof(type, field)".to_string();
                let raw = tokenize_macro_body(&def.body);
                def.tokenized_body = resolve_params(&raw, &def.params, def.is_variadic);
                def.has_stringify_or_paste = false;
            }
            self.macros.define(def);
        }
    }

    fn handle_undef(&mut self, rest: &str) {
        let name = rest.split_whitespace().next().unwrap_or("");
        if !name.is_empty() {
            self.macros.undefine(name);
        }
    }

    fn handle_ifdef(&mut self, rest: &str, negate: bool, position: Range<usize>) {
        let name = rest.split_whitespace().next().unwrap_or("");
        let defined = self.macros.is_defined(name);
        let condition = if negate { !defined } else { defined };
        self.conditionals.push_if(condition, position);
    }

    fn handle_if(&mut self, expr: &str, position: Range<usize>) {
        // First resolve `defined(X)` and `__has_*()` before macro expansion
        let resolved = self.resolve_defined_in_expr(expr);
        // Expand macros in the resolved expression (reuse directive_expanding set)
        let expanded = self
            .macros
            .expand_line_reuse(&resolved, &mut self.directive_expanding);
        // Resolve again after macro expansion, in case macros expanded to
        // __has_attribute(), __has_builtin(), __has_include(), etc.
        let expanded = self.resolve_defined_in_expr(&expanded);
        // Replace any remaining identifiers with 0 (standard C behavior for #if)
        let final_expr = Self::replace_remaining_idents_with_zero(&expanded);
        let condition = evaluate_condition(&final_expr, &self.macros);
        self.conditionals.push_if(condition, position);
    }

    fn handle_elif(&mut self, expr: &str) {
        let resolved = self.resolve_defined_in_expr(expr);
        let expanded = self
            .macros
            .expand_line_reuse(&resolved, &mut self.directive_expanding);
        // Resolve again after macro expansion (same reason as handle_if)
        let expanded = self.resolve_defined_in_expr(&expanded);
        let final_expr = Self::replace_remaining_idents_with_zero(&expanded);
        let condition = evaluate_condition(&final_expr, &self.macros);
        self.conditionals.handle_elif(condition);
    }

    // ── Single-pass emit: normalize-free tokenization & accumulation ──

    /// Emit preprocessed text as tokens directly, accumulating into self.
    /// `source_boundaries` must already be remapped to original positions.
    fn emit_tokenized_chunk(&mut self, raw: &str, source_text: &str, source_boundaries: &[usize]) {
        if should_skip_file(&self.current_path()) {
            for &b in raw.as_bytes() {
                if b == b'\n' {
                    self.raw_text.push('\n');
                }
            }
            return;
        }

        let file = self.current_path();
        let file_idx = self.ensure_file(&file);

        let (mut chunk_tokens, warnings, errors) = tokenizer::tokenize_with_diagnostics(raw);
        Self::filter_gnu_extensions(&mut chunk_tokens);

        let is_macro_expanded = raw != source_text;
        let change = is_macro_expanded.then(|| changed_range(raw, source_text, source_boundaries));
        // Tokenize source_text once and reuse for all macro-token span lookups
        // in this chunk, avoiding repeated retokenization per token.
        let source_tokens = is_macro_expanded.then(|| tokenizer::tokenize(source_text));

        for (token, tok_start, tok_end) in chunk_tokens {
            let token_range = if let Some(change) = change
                .as_ref()
                .filter(|c| tok_start >= c.raw.start && tok_end <= c.raw.end)
            {
                let st = source_tokens.as_ref().unwrap();
                macro_token_source_span(
                    &token,
                    &raw[tok_start..tok_end],
                    st,
                    source_text,
                    source_boundaries,
                    change.source.clone(),
                )
            } else {
                let source_start =
                    map_pos_to_source(source_boundaries, source_text, raw, tok_start);
                let source_end = map_pos_to_source(source_boundaries, source_text, raw, tok_end);
                source_start..source_end
            };
            self.output_tokens
                .push((token, Span::from_parts(file_idx, token_range)));
        }

        let map_diag = |range: Range<usize>| -> Range<usize> {
            let start = map_pos_to_source(source_boundaries, source_text, raw, range.start);
            let end = map_pos_to_source(source_boundaries, source_text, raw, range.end);
            start..end.max(start + 1)
        };
        for w in warnings {
            let r = map_diag(w.range);
            self.lexer_warnings
                .push(Rich::custom(Span::from_parts(file_idx, r), w.message));
        }
        for e in errors {
            let r = map_diag(e.range);
            self.lexer_errors
                .push(Rich::custom(Span::from_parts(file_idx, r), e.message));
        }

        self.raw_text.push_str(raw);
    }

    /// Emit synthetic text (from pragma synthetic tokens or pending injections).
    fn emit_synthetic_text(&mut self, raw: String) {
        if raw.is_empty() {
            return;
        }
        let source_boundaries = vec![0usize; raw.len() + 1];
        self.emit_tokenized_chunk(&raw, &raw, &source_boundaries);
    }

    /// Emit accumulated lexer diagnostics (warnings + errors).
    fn emit_lexer_diagnostics(&self) {
        if self.lexer_warnings.is_empty() && self.lexer_errors.is_empty() {
            return;
        }

        let files: HashMap<FileId, (String, Arc<str>)> = self
            .source_files
            .iter()
            .map(|(id, file)| (*id, (file.path.display().to_string(), file.source.clone())))
            .collect();
        co2_ast::set_source_map(Arc::new(crate::PreprocessorSourceMap {
            files: Arc::new(files),
        }));

        if !self.lexer_warnings.is_empty() {
            co2_ast::emit_warnings(self.lexer_warnings.clone());
        }
        if !self.lexer_errors.is_empty() {
            co2_ast::emit_errors_and_terminate(self.lexer_errors.clone());
        }
    }

    /// Ensure a source file is registered, returning its FileId.
    pub(super) fn ensure_file(&mut self, path: &Path) -> FileId {
        if let Some(idx) = self.file_index.get(path).copied() {
            return idx;
        }
        let source = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read source file {}: {e}", path.display()));
        let idx = global_file_id(path);
        self.source_files.insert(
            idx,
            SourceFile {
                path: path.to_path_buf(),
                source: Arc::<str>::from(source),
            },
        );
        self.file_index.insert(path.to_path_buf(), idx);
        idx
    }
}

impl Default for Preprocessor {
    fn default() -> Self {
        Self::new()
    }
}

// ── Token-level GNU extension filter ─────────────────────────────────

/// Filter GNU extensions from the token stream produced by the tokenizer.
///
/// Operates on `(Token, start, end)` tuples (start/end are byte offsets in
/// the raw preprocessed text).  These functions do NOT change the positions:
/// filtering only *removes* tokens and optionally *replaces* them.
impl Preprocessor {
    fn filter_gnu_extensions(tokens: &mut Vec<(Token, usize, usize)>) {
        let mut i = 0;
        while i < tokens.len() {
            let (ref token, start, end) = tokens[i];
            match token {
                Token::Ident(name) if name == "__attribute__" || name == "__attribute" => {
                    // skip attribute call: __attribute__ (( ... ))
                    // or __attribute (( ... ))
                    if let Some(body_end) = Self::skip_balanced_parens(tokens, i + 1) {
                        // Check if body contains __transparent_union__
                        let has_transparent = tokens[i + 1..body_end]
                            .iter()
                            .any(|(t, _, _)| {
                                matches!(t, Token::Ident(n) if n == "__transparent_union__")
                            });
                        if has_transparent {
                            tokens[i] = (Token::TransparentUnionAttr, start, end);
                            let _ = tokens.drain(i + 1..body_end);
                            i += 1;
                        } else {
                            let _ = tokens.drain(i..body_end);
                            // i stays the same — next token shifted to position i
                        }
                        continue;
                    }
                }
                Token::Ident(name) if name == "__asm__" || name == "__asm" => {
                    // skip asm call: __asm__ ( "...string..." )
                    if let Some(body_end) = Self::skip_balanced_parens(tokens, i + 1) {
                        let _ = tokens.drain(i..body_end);
                        continue;
                    }
                }
                Token::Ident(name)
                    if name == "__extension__" || name == "_Complex" || name == "_Noreturn" =>
                {
                    let _ = tokens.remove(i);
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// Skip a balanced-parentheses group starting at index `start`.
    /// Returns the index *after* the closing `)` (exclusive end).
    /// Returns None if unbalanced.
    fn skip_balanced_parens(tokens: &[(Token, usize, usize)], start: usize) -> Option<usize> {
        let mut depth: i32 = 0;
        let mut i = start;
        while i < tokens.len() {
            match tokens[i].0 {
                Token::LParen => depth += 1,
                Token::RParen => {
                    depth -= 1;
                    if depth < 0 {
                        return None; // unmatched closing paren
                    }
                    if depth == 0 {
                        return Some(i + 1); // past the closing paren
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None // unbalanced
    }
}

// ── Span-mapping utilities (moved from lib.rs, no normalization layer) ──

#[derive(Debug, Clone)]
struct ChangedRange {
    raw: Range<usize>,
    source: Range<usize>,
}

fn changed_range(raw: &str, source_text: &str, source_boundaries: &[usize]) -> ChangedRange {
    let prefix = source_text
        .bytes()
        .zip(raw.bytes())
        .take_while(|(source, expanded)| source == expanded)
        .count();
    let suffix = source_text[prefix..]
        .bytes()
        .rev()
        .zip(raw[prefix.min(raw.len())..].bytes().rev())
        .take_while(|(source, expanded)| source == expanded)
        .count();
    let start = prefix.min(source_text.len());
    let mut end = source_text.len().saturating_sub(suffix).max(start);
    let raw_start = prefix.min(raw.len());
    let raw_end = raw.len().saturating_sub(suffix).max(raw_start);
    let source_bytes = source_text.as_bytes();
    if source_bytes[start..end].contains(&b'(') {
        while end < source_bytes.len() && source_bytes[end] == b')' {
            end += 1;
        }
    }
    let source_start = source_boundaries
        .get(start.min(source_boundaries.len().saturating_sub(1)))
        .copied()
        .unwrap_or_else(|| *source_boundaries.last().unwrap_or(&0));
    let source_end = source_boundaries
        .get(end.min(source_boundaries.len().saturating_sub(1)))
        .copied()
        .unwrap_or_else(|| *source_boundaries.last().unwrap_or(&source_start))
        .max(source_start);
    ChangedRange {
        raw: raw_start..raw_end,
        source: source_start..source_end,
    }
}

/// Map a byte offset in tokenized (raw) text to the corresponding byte offset
/// in the original source text. Uses linear interpolation for macro-expanded
/// regions.
fn raw_to_source_text_pos(raw_offset: usize, raw: &str, source_text: &str) -> usize {
    if raw == source_text {
        return raw_offset.min(source_text.len());
    }
    if raw_offset >= raw.len() {
        return source_text.len();
    }
    let source_len = source_text.len();
    let raw_len = raw.len();
    let prefix = source_text
        .bytes()
        .zip(raw.bytes())
        .take_while(|(s, r)| s == r)
        .count();
    if raw_offset <= prefix {
        return raw_offset;
    }
    let suffix = source_text
        .bytes()
        .rev()
        .zip(raw.bytes().rev())
        .take_while(|(s, r)| s == r)
        .count();
    if raw_offset > raw_len - suffix {
        let adj = source_len - (raw_len - raw_offset);
        return adj.min(source_len);
    }
    let exp_raw = raw_len - prefix - suffix;
    let exp_src = source_len - prefix - suffix;
    if exp_raw == 0 {
        return prefix;
    }
    let pos = raw_offset - prefix;
    let scaled = pos * exp_src / exp_raw;
    (prefix + scaled).min(source_len)
}

/// Map a raw-text position through source boundaries to the original source
/// file position (no normalization layer).
fn map_pos_to_source(
    source_boundaries: &[usize],
    source_text: &str,
    raw: &str,
    offset: usize,
) -> usize {
    let source_pos = raw_to_source_text_pos(offset, raw, source_text);
    source_boundaries
        .get(source_pos.min(source_boundaries.len().saturating_sub(1)))
        .copied()
        .unwrap_or_else(|| *source_boundaries.last().unwrap_or(&0))
}

/// Try to map a macro-expanded token's span back to the original source
/// position by matching against pre-tokenized source text.
/// `source_tokens` must be the cached result of `tokenizer::tokenize(source_text)`.
fn macro_token_source_span(
    token: &Token,
    token_text: &str,
    source_tokens: &[(Token, usize, usize)],
    source_text: &str,
    source_boundaries: &[usize],
    default_range: Range<usize>,
) -> Range<usize> {
    if !matches!(token, Token::Ident(_)) {
        return default_range;
    }

    let mut matches = source_tokens
        .iter()
        .filter(|(_, start, end)| source_text[*start..*end] == *token_text)
        .filter_map(|(_, start, end)| {
            let start = *start;
            let end = *end;
            let source_start = source_boundaries
                .get(start.min(source_boundaries.len().saturating_sub(1)))
                .copied()?;
            let source_end = source_boundaries
                .get(end.min(source_boundaries.len().saturating_sub(1)))
                .copied()
                .unwrap_or(source_start);
            (source_start >= default_range.start && source_end <= default_range.end)
                .then_some(source_start..source_end)
        });

    let Some(first) = matches.next() else {
        return generated_identifier_source_span(
            token_text,
            source_tokens,
            source_text,
            source_boundaries,
            default_range,
        );
    };
    if matches.next().is_some() {
        default_range
    } else {
        first
    }
}

fn generated_identifier_source_span(
    token_text: &str,
    source_tokens: &[(Token, usize, usize)],
    source_text: &str,
    source_boundaries: &[usize],
    default_range: Range<usize>,
) -> Range<usize> {
    let mut matches = source_tokens
        .iter()
        .filter_map(|(source_token, start, end)| {
            let start = *start;
            let end = *end;
            match source_token {
                Token::Ident(source_ident)
                    if token_text != source_ident.as_str()
                        && token_text.ends_with(source_ident.as_str()) =>
                {
                    let source_start = source_boundaries
                        .get(start.min(source_boundaries.len().saturating_sub(1)))
                        .copied()?;
                    let source_end = source_boundaries
                        .get(end.min(source_boundaries.len().saturating_sub(1)))
                        .copied()
                        .unwrap_or(source_start);
                    (source_start >= default_range.start && source_end <= default_range.end)
                        .then_some((start, end))
                }
                _ => None,
            }
        });

    let Some((start, end)) = matches.next() else {
        return default_range;
    };
    if matches.next().is_some() {
        return default_range;
    }

    source_macro_call_around(source_text, source_boundaries, start, end).unwrap_or(default_range)
}

fn source_macro_call_around(
    source_text: &str,
    source_boundaries: &[usize],
    start: usize,
    end: usize,
) -> Option<Range<usize>> {
    let bytes = source_text.as_bytes();
    let mut open = start;
    while open > 0 && bytes[open - 1].is_ascii_whitespace() {
        open -= 1;
    }
    if open == 0 || bytes[open - 1] != b'(' {
        return source_span_for_source_text_range(source_boundaries, start..end);
    }
    open -= 1;

    let mut call_start = open;
    while call_start > 0 && bytes[call_start - 1].is_ascii_whitespace() {
        call_start -= 1;
    }
    let ident_end = call_start;
    while call_start > 0 {
        let previous = bytes[call_start - 1];
        if !(previous.is_ascii_alphanumeric() || previous == b'_') {
            break;
        }
        call_start -= 1;
    }
    if call_start == ident_end {
        return source_span_for_source_text_range(source_boundaries, start..end);
    }

    let mut depth = 0usize;
    let mut close = open;
    while close < bytes.len() {
        match bytes[close] {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close += 1;
                    break;
                }
            }
            _ => {}
        }
        close += 1;
    }
    source_span_for_source_text_range(source_boundaries, call_start..close)
}

fn source_span_for_source_text_range(
    source_boundaries: &[usize],
    range: Range<usize>,
) -> Option<Range<usize>> {
    let source_start = source_boundaries
        .get(range.start.min(source_boundaries.len().saturating_sub(1)))
        .copied()?;
    let source_end = source_boundaries
        .get(range.end.min(source_boundaries.len().saturating_sub(1)))
        .copied()
        .unwrap_or(source_start);
    Some(source_start..source_end)
}

// ── File helpers ─────────────────────────────────────────────────────

fn should_skip_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("stdatomic.h" | "tgmath.h")
    )
}

fn global_file_id(path: &Path) -> FileId {
    static FILE_IDS: OnceLock<Mutex<HashMap<PathBuf, FileId>>> = OnceLock::new();
    static NEXT_FILE_ID: AtomicU32 = AtomicU32::new(0);

    let mut guard = FILE_IDS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    if let Some(file_id) = guard.get(path).copied() {
        return file_id;
    }

    let file_id = FileId::from(NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed) as usize);
    guard.insert(path.to_path_buf(), file_id);
    file_id
}
