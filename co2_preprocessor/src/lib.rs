use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[allow(unused)]
#[path = "../../temp_ai/claudes-c-compiler/src/common/encoding.rs"]
pub mod common_encoding;
#[allow(unused)]
#[path = "../../temp_ai/claudes-c-compiler/src/common/fx_hash.rs"]
pub mod common_fx_hash;
#[allow(unused)]
#[path = "../../temp_ai/claudes-c-compiler/src/common/source.rs"]
pub mod common_source;
#[allow(unused)]
#[path = "../../temp_ai/claudes-c-compiler/src/frontend/preprocessor/mod.rs"]
mod ccc_preprocessor;

pub mod common {
    pub use crate::common_encoding as encoding;
    pub use crate::common_fx_hash as fx_hash;
    pub use crate::common_source as source;
}

pub mod frontend {
    pub mod sema {
        pub mod builtins {
            pub fn is_builtin(_name: &str) -> bool {
                false
            }
        }
    }

    pub mod preprocessor {
        pub(crate) use crate::ccc_preprocessor::*;
    }
}

use ccc_preprocessor::pipeline::Preprocessor;

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub source: Arc<str>,
    line_offsets: Arc<Vec<usize>>,
}

#[derive(Clone, Debug)]
struct OutputLineMap {
    file_idx: usize,
    source_line: usize,
    normalized_line: Arc<str>,
}

#[derive(Clone, Debug)]
pub struct MappedSpan {
    pub file_idx: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug)]
pub struct DiagnosticSpan {
    pub file_name: String,
    pub source: Arc<str>,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug)]
pub struct PreprocessedSource {
    pub normalized: Arc<str>,
    pub main_file_idx: usize,
    files: Arc<Vec<SourceFile>>,
    output_lines: Arc<Vec<OutputLineMap>>,
    normalized_line_offsets: Arc<Vec<usize>>,
}

impl PreprocessedSource {
    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    pub fn map_span(&self, span: co2_ast::Span) -> Option<MappedSpan> {
        if self.output_lines.is_empty() {
            return None;
        }

        let start = self.map_offset(span.start)?;
        let end = self.map_offset(span.end)?;
        if start.file_idx == end.file_idx {
            return Some(MappedSpan {
                file_idx: start.file_idx,
                start: start.start.min(end.end),
                end: start.end.max(end.end),
            });
        }

        Some(start)
    }

    pub fn diagnostic_span(&self, span: co2_ast::Span) -> Option<DiagnosticSpan> {
        let mapped = self.map_span(span)?;
        let file = self.files.get(mapped.file_idx)?;
        Some(DiagnosticSpan {
            file_name: file.path.display().to_string(),
            source: file.source.clone(),
            start: mapped.start,
            end: mapped.end.min(file.source.len()),
        })
    }

    fn map_offset(&self, output_offset: usize) -> Option<MappedSpan> {
        let line_idx = line_index_for_offset(&self.normalized_line_offsets, output_offset)?;
        let line = self.output_lines.get(line_idx)?;
        let output_line_start = *self.normalized_line_offsets.get(line_idx)?;
        let output_col = output_offset.saturating_sub(output_line_start);
        let file = self.files.get(line.file_idx)?;
        let source_range = line_range(file, line.source_line)?;
        let source_line = &file.source[source_range.clone()];

        if source_line.is_empty() {
            return Some(MappedSpan {
                file_idx: line.file_idx,
                start: source_range.start,
                end: source_range.start,
            });
        }

        let normalized_line = line.normalized_line.as_ref();
        let mapped_col = map_column(source_line, normalized_line, output_col);
        let mapped = source_range.start + mapped_col.min(source_line.len());
        Some(MappedSpan {
            file_idx: line.file_idx,
            start: mapped,
            end: mapped,
        })
    }
}

pub fn preprocess(input: &Path, cpp_args: &[String]) -> PreprocessedSource {
    let input = absolute_path(input);
    let mut preprocessor = Preprocessor::new();
    configure_preprocessor(&mut preprocessor, &input, cpp_args);

    let input_bytes = fs::read(&input).expect("failed to read C input for preprocessing");
    let input_source = rewrite_main_source_for_preprocess(&common::encoding::bytes_to_string(input_bytes));
    let preprocessed = preprocessor.preprocess(&input_source);
    if let Some(err) = preprocessor.errors().first() {
        panic!(
            "preprocessor error at {}:{}:{}: {}",
            err.file, err.line, err.col, err.message
        );
    }

    build_preprocessed_source(&input, preprocessed)
}

fn configure_preprocessor(preprocessor: &mut Preprocessor, input: &Path, cpp_args: &[String]) {
    let input_str = input.to_string_lossy().into_owned();
    preprocessor.set_filename(&input_str);
    configure_target(preprocessor);
    add_discovered_system_include_paths(preprocessor);

    let mut i = 0usize;
    while i < cpp_args.len() {
        let arg = &cpp_args[i];
        match arg.as_str() {
            "-I" => {
                i += 1;
                preprocessor.add_include_path(cpp_args.get(i).expect("missing -I value"));
            }
            "-D" => {
                i += 1;
                define_macro(preprocessor, cpp_args.get(i).expect("missing -D value"));
            }
            "-U" => {
                i += 1;
                preprocessor.undefine_macro(cpp_args.get(i).expect("missing -U value"));
            }
            "-include" => {
                i += 1;
                let include_path = cpp_args.get(i).expect("missing -include value");
                let resolved = resolve_force_include(input, include_path);
                let content = common::encoding::bytes_to_string(
                    fs::read(&resolved).unwrap_or_else(|e| {
                        panic!("failed to read force include {}: {e}", resolved.display())
                    }),
                );
                preprocessor.preprocess_force_include(&content, &resolved.to_string_lossy());
            }
            "-isystem" => {
                i += 1;
                preprocessor.add_system_include_path(cpp_args.get(i).expect("missing -isystem value"));
            }
            "-iquote" => {
                i += 1;
                preprocessor.add_quote_include_path(cpp_args.get(i).expect("missing -iquote value"));
            }
            "-nostdinc" => {
            }
            "-undef" => {}
            _ if arg.starts_with("-I") && arg.len() > 2 => {
                preprocessor.add_include_path(&arg[2..]);
            }
            _ if arg.starts_with("-D") && arg.len() > 2 => {
                define_macro(preprocessor, &arg[2..]);
            }
            _ if arg.starts_with("-U") && arg.len() > 2 => {
                preprocessor.undefine_macro(&arg[2..]);
            }
            _ if arg.starts_with("-isystem") && arg.len() > "-isystem".len() => {
                preprocessor.add_system_include_path(&arg["-isystem".len()..]);
            }
            _ if arg.starts_with("-iquote") && arg.len() > "-iquote".len() => {
                preprocessor.add_quote_include_path(&arg["-iquote".len()..]);
            }
            _ => {}
        }
        i += 1;
    }
}

fn configure_target(preprocessor: &mut Preprocessor) {
    match std::env::consts::ARCH {
        "x86_64" => preprocessor.set_target("x86_64"),
        "x86" | "i686" => preprocessor.set_target("i686"),
        "aarch64" => preprocessor.set_target("aarch64"),
        "riscv64" => preprocessor.set_target("riscv64"),
        _ => {}
    }
    preprocessor.set_sse_macros(false);
}

fn add_discovered_system_include_paths(preprocessor: &mut Preprocessor) {
    for pattern_root in [
        "/usr/lib/gcc",
        "/usr/local/lib/gcc",
        "/usr/lib/clang",
    ] {
        let root = Path::new(pattern_root);
        let Ok(first_level) = fs::read_dir(root) else {
            continue;
        };
        for entry in first_level.flatten() {
            let path = entry.path();
            if pattern_root.ends_with("/clang") {
                let include = path.join("include");
                if include.is_dir() {
                    preprocessor.add_system_include_path(&include.to_string_lossy());
                }
                continue;
            }
            let Ok(second_level) = fs::read_dir(&path) else {
                continue;
            };
            for sub in second_level.flatten() {
                for include_dir in [sub.path().join("include"), sub.path().join("include-fixed")] {
                    if include_dir.is_dir() {
                        preprocessor.add_system_include_path(&include_dir.to_string_lossy());
                    }
                }
            }
        }
    }
}

fn define_macro(preprocessor: &mut Preprocessor, raw: &str) {
    match raw.split_once('=') {
        Some((name, value)) => preprocessor.define_macro(name, value),
        None => preprocessor.define_macro(raw, "1"),
    }
}

fn resolve_force_include(input: &Path, include: &str) -> PathBuf {
    let path = PathBuf::from(include);
    if path.is_absolute() {
        return path;
    }
    absolute_path(&input.parent().unwrap_or_else(|| Path::new(".")).join(path))
}

fn build_preprocessed_source(input: &Path, preprocessed: String) -> PreprocessedSource {
    let mut files = Vec::<SourceFile>::new();
    let mut file_index = HashMap::<PathBuf, usize>::new();
    let main_file_idx = ensure_source_file(input, &mut files, &mut file_index);

    let mut normalized = String::new();
    let mut output_lines = Vec::new();
    let mut current_file = input.to_path_buf();
    let mut current_line = 1usize;

    for line in preprocessed.lines() {
        if let Some((path, line_no)) = parse_line_marker(line) {
            current_file = absolutize_marker_path(input, path);
            current_line = line_no;
            ensure_source_file(&current_file, &mut files, &mut file_index);
            continue;
        }

        let normalized_line = if should_skip_file_contents(&current_file) {
            String::new()
        } else if line.trim_start().starts_with('#') {
            String::new()
        } else {
            normalize_preprocessed_line(line)
        };
        normalized.push_str(&normalized_line);
        normalized.push('\n');

        let file_idx = ensure_source_file(&current_file, &mut files, &mut file_index);
        output_lines.push(OutputLineMap {
            file_idx,
            source_line: mapped_source_line_index(input, &current_file, current_line),
            normalized_line: Arc::<str>::from(normalized_line),
        });
        current_line += 1;
    }

    if let Some(path) = std::env::var_os("CO2_DUMP_PREPROCESSED") {
        let _ = fs::write(path, &normalized);
    }

    PreprocessedSource {
        normalized_line_offsets: Arc::new(compute_line_offsets(&normalized)),
        normalized: Arc::<str>::from(normalized),
        main_file_idx,
        files: Arc::new(files),
        output_lines: Arc::new(output_lines),
    }
}

fn ensure_source_file(
    path: &Path,
    files: &mut Vec<SourceFile>,
    file_index: &mut HashMap<PathBuf, usize>,
) -> usize {
    if let Some(idx) = file_index.get(path).copied() {
        return idx;
    }

    let source = common::encoding::bytes_to_string(
        fs::read(path).unwrap_or_else(|e| panic!("failed to read source file {}: {e}", path.display())),
    );
    let idx = files.len();
    files.push(SourceFile {
        path: path.to_path_buf(),
        line_offsets: Arc::new(compute_line_offsets(&source)),
        source: Arc::<str>::from(source),
    });
    file_index.insert(path.to_path_buf(), idx);
    idx
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .expect("failed to resolve current dir")
        .join(path)
}

fn absolutize_marker_path(main_input: &Path, path: &str) -> PathBuf {
    let marker = PathBuf::from(path);
    if marker.is_absolute() {
        return marker;
    }
    let main_input = absolute_path(main_input);
    main_input
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(marker)
}

fn rewrite_main_source_for_preprocess(source: &str) -> String {
    let mut rewritten = String::from("#define __CO2__ 1\n");
    for line in source.lines() {
        if line.trim_start().starts_with('#') {
            rewritten.push_str(
                &line
                    .replace("__GNUC__", "__CO2_HIDDEN_GNUC__")
                    .replace("__clang__", "__CO2_HIDDEN_CLANG__"),
            );
        } else {
            rewritten.push_str(line);
        }
        rewritten.push('\n');
    }
    rewritten
}

fn should_skip_file_contents(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("stdatomic.h" | "tgmath.h")
    )
}

fn mapped_source_line_index(main_input: &Path, current_file: &Path, current_line: usize) -> usize {
    if current_file == main_input {
        current_line.saturating_sub(2)
    } else {
        current_line.saturating_sub(1)
    }
}

fn parse_line_marker(line: &str) -> Option<(&str, usize)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let rest = trimmed[1..].trim_start();
    let digits_len = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits_len == 0 {
        return None;
    }
    let line_no = rest[..digits_len].parse().ok()?;
    let rest = rest[digits_len..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end_quote = rest.find('"')?;
    Some((&rest[..end_quote], line_no))
}

fn normalize_preprocessed_line(line: &str) -> String {
    let line = strip_gnu_attributes(line);
    let line = strip_gnu_asm_annotations(&line);
    let line = replace_gnu_typeof_with_usize(&line);
    strip_extension_keywords(&line)
}

fn line_index_for_offset(offsets: &[usize], offset: usize) -> Option<usize> {
    if offsets.is_empty() {
        return None;
    }
    match offsets.binary_search(&offset) {
        Ok(i) => Some(i),
        Err(i) => Some(i.saturating_sub(1)),
    }
}

fn line_range(file: &SourceFile, line_idx: usize) -> Option<Range<usize>> {
    let start = *file.line_offsets.get(line_idx)?;
    let end = if let Some(next) = file.line_offsets.get(line_idx + 1) {
        next.saturating_sub(1)
    } else {
        file.source.len()
    };
    Some(start..end)
}

fn map_column(source_line: &str, normalized_line: &str, output_col: usize) -> usize {
    let output_col = output_col.min(normalized_line.len());
    let span_end = expand_token_end(normalized_line, output_col);
    let span_start = shrink_token_start(normalized_line, output_col, span_end);
    let needle = normalized_line.get(span_start..span_end).unwrap_or("").trim();
    if !needle.is_empty() {
        let mut matches = source_line.match_indices(needle);
        if let Some((idx, _)) = matches.next() {
            if matches.next().is_none() {
                return idx + output_col.saturating_sub(span_start);
            }
        }
    }

    output_col.min(source_line.len())
}

fn shrink_token_start(line: &str, col: usize, end: usize) -> usize {
    let bytes = line.as_bytes();
    let mut start = col.min(bytes.len());
    while start > 0 && is_ident_continue(bytes[start - 1]) {
        start -= 1;
    }
    if start == end {
        start = start.saturating_sub(1);
    }
    start
}

fn expand_token_end(line: &str, col: usize) -> usize {
    let bytes = line.as_bytes();
    let mut end = col.min(bytes.len());
    while end < bytes.len() && is_ident_continue(bytes[end]) {
        end += 1;
    }
    if end == col && end < bytes.len() {
        end += 1;
    }
    end
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn compute_line_offsets(content: &str) -> Vec<usize> {
    let mut result = vec![0];
    for (idx, b) in content.bytes().enumerate() {
        if b == b'\n' {
            result.push(idx + 1);
        }
    }
    result
}

fn strip_gnu_attributes(src: &str) -> String {
    strip_balanced_call(src, "__attribute__")
}

fn strip_gnu_asm_annotations(src: &str) -> String {
    let src = strip_balanced_call(src, "__asm__");
    strip_balanced_call(&src, "__asm")
}

fn strip_balanced_call(src: &str, keyword: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if src[i..].starts_with(keyword) {
            let mut j = i + keyword.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                let mut depth = 0usize;
                while j < bytes.len() {
                    match bytes[j] {
                        b'(' => depth += 1,
                        b')' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                out.push(' ');
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn replace_gnu_typeof_with_usize(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let kw_len = if src[i..].starts_with("__typeof__") {
            Some("__typeof__".len())
        } else if src[i..].starts_with("__typeof") {
            Some("__typeof".len())
        } else if src[i..].starts_with("typeof") {
            Some("typeof".len())
        } else {
            None
        };
        if let Some(kw_len) = kw_len {
            let mut j = i + kw_len;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                let mut depth = 0usize;
                while j < bytes.len() {
                    match bytes[j] {
                        b'(' => depth += 1,
                        b')' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                out.push_str("usize");
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn strip_extension_keywords(src: &str) -> String {
    fn is_ident_start(b: u8) -> bool {
        b.is_ascii_alphabetic() || b == b'_'
    }
    fn is_ident_continue_local(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if is_ident_start(bytes[i]) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue_local(bytes[i]) {
                i += 1;
            }
            let ident = &src[start..i];
            if matches!(
                ident,
                "__extension__"
                    | "__inline"
                    | "__inline__"
                    | "__restrict"
                    | "__restrict__"
                    | "_Complex"
                    | "_Noreturn"
            ) {
                out.push(' ');
            } else {
                out.push_str(ident);
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}
