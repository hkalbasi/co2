//! Full C preprocessor implementation.
//!
//! Struct definition, core preprocessing pipeline, directive dispatch,
//! and public configuration API. Predefined macros and target configuration
//! live in `predefined_macros`, pragma handling in `pragmas`, and text
//! processing (comment stripping, line joining) in `text_processing`.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

use super::builtin_macros::define_builtin_macros;
use super::conditionals::{ConditionalStack, evaluate_condition};
use super::macro_defs::{MacroDef, MacroTable, parse_define};
use super::text_processing::{LogicalSlice, split_first_word, strip_line_comment};
use super::utils::{is_ident_cont, is_ident_start};

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
pub(crate) struct PreprocessOutput {
    pub(crate) chunks: Vec<PreprocessChunk>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreprocessChunk {
    pub(crate) file: PathBuf,
    pub(crate) raw: String,
    pub(crate) source_text: String,
    pub(crate) source_boundaries: Vec<usize>,
}

impl PreprocessOutput {
    pub(crate) fn push(&mut self, chunk: PreprocessChunk) {
        if !chunk.raw.is_empty() {
            self.chunks.push(chunk);
        }
    }

    pub(crate) fn append(&mut self, mut other: PreprocessOutput) {
        self.chunks.append(&mut other.chunks);
    }

    pub(crate) fn is_blank(&self) -> bool {
        self.chunks
            .iter()
            .all(|chunk| chunk.raw.chars().all(char::is_whitespace))
    }
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
    /// #pragma weak directives: (symbol, optional_alias_target)
    /// - (symbol, None) means "mark symbol as weak"
    /// - (symbol, Some(target)) means "symbol is a weak alias for target"
    pub weak_pragmas: Vec<(String, Option<String>)>,
    /// #pragma redefine_extname directives: (old_name, new_name)
    pub redefine_extname_pragmas: Vec<(String, String)>,
    /// Accumulated output from force-included files (-include).
    /// Prepended to the main source's preprocessed output so that pragma
    /// synthetic tokens (e.g., visibility push/pop) take effect.
    force_include_output: PreprocessOutput,
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
            system_include_paths: Self::default_system_include_paths(),
            include_stack: Vec::new(),
            pragma_once_files: HashSet::new(),
            pending_injections: Vec::new(),
            macro_save_stack: HashMap::new(),
            weak_pragmas: Vec::new(),
            redefine_extname_pragmas: Vec::new(),
            force_include_output: PreprocessOutput::default(),
            include_resolve_cache: HashMap::new(),
            include_guard_macros: HashMap::new(),
            directive_expanding: HashSet::new(),
        };
        pp.define_predefined_macros();
        define_builtin_macros(&mut pp.macros);
        pp
    }

    pub fn preprocess(&mut self, source: &str) -> PreprocessOutput {
        let main_output = self.preprocess_source(source, false);

        if let Some(range) = self.conditionals.unclosed_range() {
            self.errors.push(PreprocessorDiagnostic {
                file: self.filename.clone(),
                range,
                message: "Unterminated conditional directive".to_string(),
            });
        }

        let mut result = std::mem::take(&mut self.force_include_output);
        result.append(main_output);
        result
    }

    pub(super) fn preprocess_included(&mut self, source: &str) -> PreprocessOutput {
        self.preprocess_source(source, true)
    }

    fn preprocess_source(&mut self, source: &str, is_include: bool) -> PreprocessOutput {
        let mut output = PreprocessOutput::default();
        let saved_conditionals = is_include.then(|| std::mem::take(&mut self.conditionals));
        let mut pending = PendingExpansion::default();
        let mut expanding = HashSet::new();

        for slice in Self::logical_slices(source) {
            let trimmed = slice.text.trim();
            let is_directive = trimmed.starts_with('#');
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
                self.emit_blank_slice(&mut output, &slice);
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
                    self.flush_pending(&mut output, &mut pending, &mut expanding);
                }

                if let Some(included_content) = self.process_directive(&slice) {
                    output.append(included_content);
                } else if is_include {
                    self.emit_blank_slice(&mut output, &slice);
                }

                if !is_include && !self.pending_injections.is_empty() {
                    for decl in std::mem::take(&mut self.pending_injections) {
                        self.emit_synthetic_text(&mut output, decl);
                    }
                }

                if !is_include {
                    self.emit_blank_slice(&mut output, &slice);
                }
            } else if self.conditionals.is_active() {
                self.accumulate_and_expand(&slice, &mut pending, &mut output, &mut expanding);
            } else if pending.is_empty() {
                self.emit_blank_slice(&mut output, &slice);
            } else {
                pending.push_gap(&slice);
            }
        }

        self.flush_pending(&mut output, &mut pending, &mut expanding);

        if let Some(saved) = saved_conditionals {
            self.conditionals = saved;
        }

        output
    }

    fn accumulate_and_expand(
        &self,
        slice: &LogicalSlice,
        pending: &mut PendingExpansion,
        output: &mut PreprocessOutput,
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
                self.emit_text_from_slice(output, slice, expanded);
                self.emit_newline(output, slice.newline_boundaries());
            }
            return;
        }

        let needs_more = Self::has_unbalanced_parens(pending.text());
        pending.push(slice);

        if pending.line_count() > MAX_PENDING_NEWLINES {
            self.flush_pending(output, pending, expanding);
            return;
        }

        if needs_more {
            let expanded = self.macros.expand_line_reuse(pending.text(), expanding);
            if !Self::has_unbalanced_parens(pending.text())
                && !self.ends_with_funclike_macro(&expanded)
            {
                self.emit_pending_output(output, pending, expanded);
                pending.clear();
            }
            return;
        }

        if Self::has_unbalanced_parens(pending.text()) {
            return;
        }

        let expanded = self.macros.expand_line_reuse(pending.text(), expanding);
        if !self.ends_with_funclike_macro(&expanded) {
            self.emit_pending_output(output, pending, expanded);
            pending.clear();
        }
    }

    fn flush_pending(
        &self,
        output: &mut PreprocessOutput,
        pending: &mut PendingExpansion,
        expanding: &mut HashSet<String>,
    ) {
        if pending.is_empty() {
            return;
        }
        let expanded = self.macros.expand_line_reuse(pending.text(), expanding);
        self.emit_pending_output(output, pending, expanded);
        pending.clear();
    }

    fn emit_pending_output(
        &self,
        output: &mut PreprocessOutput,
        pending: &PendingExpansion,
        expanded: String,
    ) {
        let (source_text, source_boundaries) = pending.joined_source();
        self.emit_source_text(output, expanded, source_text, source_boundaries);
        for slice in &pending.slices {
            self.emit_newline(output, slice.newline_boundaries());
        }
    }

    fn emit_blank_slice(&self, output: &mut PreprocessOutput, slice: &LogicalSlice) {
        self.emit_newline(output, slice.newline_boundaries());
    }

    fn emit_text_from_slice(
        &self,
        output: &mut PreprocessOutput,
        slice: &LogicalSlice,
        raw: String,
    ) {
        self.emit_source_text(
            output,
            raw,
            slice.text.clone(),
            slice.source_boundaries.clone(),
        );
    }

    fn emit_source_text(
        &self,
        output: &mut PreprocessOutput,
        raw: String,
        source_text: String,
        source_boundaries: Vec<usize>,
    ) {
        if raw.is_empty() {
            return;
        }
        output.push(PreprocessChunk {
            file: self.current_path(),
            raw,
            source_text,
            source_boundaries,
        });
    }

    fn emit_newline(&self, output: &mut PreprocessOutput, source_boundaries: Vec<usize>) {
        output.push(PreprocessChunk {
            file: self.current_path(),
            raw: "\n".to_string(),
            source_text: "\n".to_string(),
            source_boundaries,
        });
    }

    fn emit_synthetic_text(&self, output: &mut PreprocessOutput, raw: String) {
        let base = self.current_offset();
        let len = raw.len();
        output.push(PreprocessChunk {
            file: self.current_path(),
            raw,
            source_text: " ".repeat(len),
            source_boundaries: vec![base; len + 1],
        });
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
        self.macros.define(MacroDef {
            name: "__BASE_FILE__".to_string(),
            is_function_like: false,
            params: Vec::new(),
            is_variadic: false,
            has_named_variadic: false,
            body: format!("\"{filename}\""),
        });
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

    fn current_offset(&self) -> usize {
        0
    }

    /// Define a macro from a command-line -D flag.
    /// Takes a name and value (e.g., name="FOO", value="1").
    pub fn define_macro(&mut self, name: &str, value: &str) {
        self.macros.define(MacroDef {
            name: name.to_string(),
            is_function_like: false,
            params: Vec::new(),
            is_variadic: false,
            has_named_variadic: false,
            body: value.to_string(),
        });
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

        // Preprocess the included content (macros persist; any pragma synthetic tokens
        // like __ccc_visibility_push_hidden are collected and prepended to main output)
        let output = self.preprocess_included(content);
        if !output.is_blank() {
            self.force_include_output.append(output);
            self.force_include_output.push(PreprocessChunk {
                file: self.current_path(),
                raw: "\n".to_string(),
                source_text: "\n".to_string(),
                source_boundaries: vec![self.current_offset(); 2],
            });
        }

        // Restore __FILE__
        if let Some(old) = old_file {
            self.macros.set_file(old);
        }

        // Pop include stack
        self.include_stack.pop();
    }

    fn process_directive(&mut self, slice: &LogicalSlice) -> Option<PreprocessOutput> {
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
                return None;
            }
            "elif" => {
                self.handle_elif(rest);
                return None;
            }
            "else" => {
                self.conditionals.handle_else();
                return None;
            }
            "endif" => {
                self.conditionals.handle_endif();
                return None;
            }
            _ if !self.conditionals.is_active() => return None,
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
                    let mut output = PreprocessOutput::default();
                    self.emit_synthetic_text(&mut output, raw);
                    return Some(output);
                }
                return None;
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

        None
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
}

impl Default for Preprocessor {
    fn default() -> Self {
        Self::new()
    }
}
