use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU32, Ordering},
};

use chumsky::span::Span as _;
pub mod common;
pub mod frontend;

use frontend::preprocessor::pipeline::Preprocessor;
use co2_ast::FileId;
use co2_ast::{Rich, Span, SourceMap};

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub source: Arc<str>,
    line_offsets: Arc<Vec<usize>>,
}

#[derive(Clone, Debug)]
struct OutputLineMap {
    file_idx: FileId,
    source_line: usize,
    normalized_line: Arc<str>,
}

#[derive(Clone, Debug)]
pub struct MappedSpan {
    pub file_idx: FileId,
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
    pub main_file_idx: FileId,
    files: Arc<HashMap<FileId, SourceFile>>,
    output_lines: Arc<Vec<OutputLineMap>>,
    normalized_line_offsets: Arc<Vec<usize>>,
}

impl PreprocessedSource {
    pub fn files(&self) -> &HashMap<FileId, SourceFile> {
        &self.files
    }

    pub fn real_span(&self, start: usize, end: usize) -> co2_ast::Span {
        let end_lookup = if end > start { end - 1 } else { end };
        let start_line_idx = line_index_for_offset(&self.normalized_line_offsets, start);
        let end_line_idx = line_index_for_offset(&self.normalized_line_offsets, end_lookup);
        if let Some(start_line_idx) = start_line_idx
            && let Some(end_line_idx) = end_line_idx
            && start_line_idx == end_line_idx
            && let Some(line) = self.output_lines.get(start_line_idx)
        {
            let output_line_start = self.normalized_line_offsets[start_line_idx];
            let file = &self.files[&line.file_idx];
            if let Some(source_range) = line_range(file, line.source_line) {
                let source_line = &file.source[source_range.clone()];
                let normalized_line = line.normalized_line.as_ref();
                let mapped = map_range(
                    source_line,
                    normalized_line,
                    start.saturating_sub(output_line_start),
                    end.saturating_sub(output_line_start),
                );
                return co2_ast::Span {
                    start: source_range.start + mapped.start,
                    end: source_range.start + mapped.end,
                    context: line.file_idx,
                };
            }
        }

        let start_mapped = self.map_offset(start);
        let end_mapped = self.map_offset(end);

        if let Some(start) = start_mapped.as_ref()
            && let Some(end) = end_mapped.as_ref()
            && start.file_idx == end.file_idx
        {
            return co2_ast::Span {
                start: start.start,
                end: end.end.max(start.start),
                context: start.file_idx,
            };
        }

        if let Some(start) = start_mapped.as_ref() {
            return co2_ast::Span {
                start: start.start,
                end: start.end,
                context: start.file_idx,
            };
        }

        co2_ast::Span {
            start,
            end,
            context: self.main_file_idx,
        }
    }

    fn map_offset(&self, output_offset: usize) -> Option<MappedSpan> {
        let line_idx = line_index_for_offset(&self.normalized_line_offsets, output_offset)?;
        let line = self.output_lines.get(line_idx)?;
        let output_line_start = *self.normalized_line_offsets.get(line_idx)?;
        let output_col = output_offset.saturating_sub(output_line_start);
        let file = self.files.get(&line.file_idx)?;
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

struct PreprocessorSourceMap {
    files: Arc<HashMap<FileId, (String, Arc<str>)>>,
}

impl SourceMap for PreprocessorSourceMap {
    fn get_file_info(&self, id: FileId) -> Option<(String, Arc<str>)> {
        self.files.get(&id).cloned()
    }
}

pub fn preprocess(input: &Path, cpp_args: &[String]) -> PreprocessedSource {
    let input = absolute_path(input);
    let mut preprocessor = Preprocessor::new();
    configure_preprocessor(&mut preprocessor, &input, cpp_args);

    let input_bytes = fs::read(&input).expect("failed to read C input for preprocessing");
    let input_source = rewrite_main_source_for_preprocess(&common::encoding::bytes_to_string(input_bytes));
    let preprocessed = preprocessor.preprocess(&input_source);
    let source = build_preprocessed_source(&input, preprocessed);
    emit_preprocessor_diagnostics(&source, preprocessor.warnings(), preprocessor.errors());
    source
}

fn emit_preprocessor_diagnostics(
    preprocessed: &PreprocessedSource,
    warnings: &[frontend::preprocessor::pipeline::PreprocessorDiagnostic],
    errors: &[frontend::preprocessor::pipeline::PreprocessorDiagnostic],
) {
    if warnings.is_empty() && errors.is_empty() {
        return;
    }

    let files = preprocessed
        .files()
        .iter()
        .map(|(id, file)| {
            (
                *id,
                (file.path.display().to_string(), file.source.clone()),
            )
        })
        .collect();
    co2_ast::set_source_map(Arc::new(PreprocessorSourceMap {
        files: Arc::new(files),
    }));

    let warnings = map_preprocessor_diagnostics(preprocessed, warnings);
    if !warnings.is_empty() {
        co2_ast::emit_warnings(warnings);
    }

    let errors = map_preprocessor_diagnostics(preprocessed, errors);
    if !errors.is_empty() {
        co2_ast::emit_errors_and_terminate(errors);
    }
}

fn map_preprocessor_diagnostics(
    preprocessed: &PreprocessedSource,
    diagnostics: &[frontend::preprocessor::pipeline::PreprocessorDiagnostic],
) -> Vec<Rich<'static, String, Span>> {
    diagnostics
        .iter()
        .map(|diagnostic| Rich::custom(preprocessor_diagnostic_span(preprocessed, diagnostic), diagnostic.message.clone()))
        .collect()
}

fn preprocessor_diagnostic_span(
    preprocessed: &PreprocessedSource,
    diagnostic: &frontend::preprocessor::pipeline::PreprocessorDiagnostic,
) -> Span {
    let wanted_path = absolute_path(Path::new(&diagnostic.file));
    let Some((file_id, file)) = preprocessed
        .files()
        .iter()
        .find(|(_, file)| file.path == wanted_path)
    else {
        return Span::new(preprocessed.main_file_idx, 0..0);
    };

    let source_line = if *file_id == preprocessed.main_file_idx {
        diagnostic.line.saturating_sub(2)
    } else {
        diagnostic.line.saturating_sub(1)
    };
    let Some(line_range) = line_range(file, source_line) else {
        return Span::new(*file_id, 0..0);
    };

    let line = &file.source[line_range.clone()];
    let start_in_line = diagnostic.col.saturating_sub(1).min(line.len());
    let start = line_range.start + start_in_line;
    let end = line
        .get(start_in_line..)
        .and_then(first_preprocessor_token_len)
        .map(|len| start + len)
        .unwrap_or(start);
    let end = if end == start && start < line_range.end {
        start + 1
    } else {
        end
    };

    Span::new(*file_id, start..end.min(line_range.end))
}

fn first_preprocessor_token_len(src: &str) -> Option<usize> {
    let len = src
        .char_indices()
        .take_while(|(_, ch)| !ch.is_whitespace())
        .last()
        .map(|(idx, ch)| idx + ch.len_utf8())?;
    Some(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn warning_diagnostic_maps_to_warning_token() {
        let temp_dir = std::env::temp_dir().join(format!(
            "co2-preprocessor-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let input = temp_dir.join("warning.c");
        fs::write(&input, "#warning keep going\nint main(void) { return 0; }\n").unwrap();

        let mut preprocessor = Preprocessor::new();
        configure_preprocessor(&mut preprocessor, &input, &[]);
        let input_bytes = fs::read(&input).unwrap();
        let input_source =
            rewrite_main_source_for_preprocess(&common::encoding::bytes_to_string(input_bytes));
        let preprocessed = preprocessor.preprocess(&input_source);
        let source = build_preprocessed_source(&input, preprocessed);
        let diagnostics = map_preprocessor_diagnostics(&source, preprocessor.warnings());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].to_string(), "#warning keep going");

        let span = *diagnostics[0].span();
        let file = source.files().get(&span.context).unwrap();
        assert_eq!(&file.source[span.start..span.end], "warning");

        let _ = fs::remove_dir_all(temp_dir);
    }
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
    let mut files = HashMap::<FileId, SourceFile>::new();
    let mut file_index = HashMap::<PathBuf, FileId>::new();
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
    files: &mut HashMap<FileId, SourceFile>,
    file_index: &mut HashMap<PathBuf, FileId>,
) -> FileId {
    if let Some(idx) = file_index.get(path).copied() {
        return idx;
    }

    let source = common::encoding::bytes_to_string(
        fs::read(path).unwrap_or_else(|e| panic!("failed to read source file {}: {e}", path.display())),
    );
    let idx = global_file_id(path);
    files.insert(idx, SourceFile {
        path: path.to_path_buf(),
        line_offsets: Arc::new(compute_line_offsets(&source)),
        source: Arc::<str>::from(source),
    });
    file_index.insert(path.to_path_buf(), idx);
    idx
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
    let cwd_resolved = absolute_path(&marker);
    if cwd_resolved.exists() {
        return cwd_resolved;
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

fn map_range(source_line: &str, normalized_line: &str, output_start: usize, output_end: usize) -> Range<usize> {
    let output_start = output_start.min(normalized_line.len());
    let output_end = output_end.min(normalized_line.len()).max(output_start);
    if output_start == output_end {
        let point = map_column(source_line, normalized_line, output_start);
        return point..point;
    }

    let needle = normalized_line.get(output_start..output_end).unwrap_or("").trim();
    if !needle.is_empty() {
        let mut matches = source_line.match_indices(needle);
        if let Some((idx, _)) = matches.next() {
            if matches.next().is_none() {
                return idx..(idx + needle.len());
            }
        }
    }

    source_token_range(source_line, output_start, output_end)
}

fn source_token_range(source_line: &str, output_start: usize, output_end: usize) -> Range<usize> {
    let bytes = source_line.as_bytes();
    if bytes.is_empty() {
        return 0..0;
    }

    let mut anchor = output_start.min(bytes.len().saturating_sub(1));
    if !is_ident_continue(bytes[anchor]) && anchor > 0 && is_ident_continue(bytes[anchor - 1]) {
        anchor -= 1;
    }
    if !is_ident_continue(bytes[anchor]) {
        return output_start.min(bytes.len())..output_end.min(bytes.len());
    }

    let mut start = anchor;
    while start > 0 && is_ident_continue(bytes[start - 1]) {
        start -= 1;
    }

    let mut end = anchor + 1;
    while end < bytes.len() && is_ident_continue(bytes[end]) {
        end += 1;
    }
    start..end
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
