use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU32, Ordering},
};

use chumsky::span::Span as _;
mod builtin_macros;
mod conditionals;
mod expr_eval;
mod includes;
mod macro_defs;
mod pipeline;
mod pragmas;
mod predefined_macros;
mod text_processing;
mod utils;

use co2_ast::FileId;
use co2_ast::{Rich, SourceMap, Span};
use pipeline::{PreprocessOutput, Preprocessor};

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub source: Arc<str>,
}

#[derive(Clone, Debug)]
struct OutputChunkMap {
    file_idx: FileId,
    raw_chunk: Arc<str>,
    normalized_chunk: Arc<str>,
    normalized_to_raw: Arc<Vec<usize>>,
    source_chunk: Arc<str>,
    source_boundaries: Arc<Vec<usize>>,
    output_range: Range<usize>,
}

#[derive(Clone, Debug)]
pub struct MappedSpan {
    pub file_idx: FileId,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug)]
pub struct PreprocessedSource {
    pub normalized: Arc<str>,
    pub main_file_idx: FileId,
    files: Arc<HashMap<FileId, SourceFile>>,
    output_chunks: Arc<Vec<OutputChunkMap>>,
    chunk_offsets: Arc<Vec<usize>>,
    main_rewrite_boundaries: Arc<Vec<usize>>,
}

impl PreprocessedSource {
    pub fn files(&self) -> &HashMap<FileId, SourceFile> {
        &self.files
    }

    pub fn real_span(&self, start: usize, end: usize) -> co2_ast::Span {
        let end_lookup = if end > start { end - 1 } else { end };
        let start_chunk_idx = chunk_index_for_offset(&self.chunk_offsets, start);
        let end_chunk_idx = chunk_index_for_offset(&self.chunk_offsets, end_lookup);
        if let Some(start_chunk_idx) = start_chunk_idx
            && let Some(end_chunk_idx) = end_chunk_idx
            && start_chunk_idx == end_chunk_idx
            && let Some(chunk) = self.output_chunks.get(start_chunk_idx)
        {
            let mapped = chunk.normalized_range_to_source(
                start.saturating_sub(chunk.output_range.start),
                end.saturating_sub(chunk.output_range.start),
            );
            return co2_ast::Span {
                start: mapped.start,
                end: mapped.end,
                context: chunk.file_idx,
            };
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
        let chunk_idx = chunk_index_for_offset(&self.chunk_offsets, output_offset)?;
        let chunk = self.output_chunks.get(chunk_idx)?;
        let mapped = chunk
            .normalized_offset_to_source(output_offset.saturating_sub(chunk.output_range.start));
        Some(MappedSpan {
            file_idx: chunk.file_idx,
            start: mapped,
            end: mapped,
        })
    }
}

impl OutputChunkMap {
    fn normalized_offset_to_source(&self, normalized_offset: usize) -> usize {
        let raw_offset = self
            .normalized_to_raw
            .get(normalized_offset.min(self.normalized_chunk.len()))
            .copied()
            .unwrap_or(self.raw_chunk.len());
        let source_offset = if self.raw_chunk.as_ref() == self.source_chunk.as_ref() {
            raw_offset.min(self.source_chunk.len())
        } else {
            map_text_offset(
                self.source_chunk.as_ref(),
                self.raw_chunk.as_ref(),
                raw_offset,
            )
        };
        self.source_boundary(source_offset)
    }

    fn normalized_range_to_source(&self, start: usize, end: usize) -> Range<usize> {
        let start = start.min(self.normalized_chunk.len());
        let end = end.min(self.normalized_chunk.len()).max(start);
        let raw_range = if self.normalized_chunk.as_ref() == self.raw_chunk.as_ref() {
            start.min(self.raw_chunk.len())..end.min(self.raw_chunk.len())
        } else {
            self.normalized_offset_to_raw(start)..self.normalized_offset_to_raw(end)
        };
        let source_range = if self.raw_chunk.as_ref() == self.source_chunk.as_ref() {
            raw_range.start.min(self.source_chunk.len())..raw_range.end.min(self.source_chunk.len())
        } else {
            map_text_range(
                self.source_chunk.as_ref(),
                self.raw_chunk.as_ref(),
                raw_range.start,
                raw_range.end,
            )
        };
        let start = self.source_boundary(source_range.start);
        let end = self.source_boundary(source_range.end).max(start);
        start..end
    }

    fn normalized_offset_to_raw(&self, offset: usize) -> usize {
        self.normalized_to_raw
            .get(offset.min(self.normalized_chunk.len()))
            .copied()
            .unwrap_or(self.raw_chunk.len())
    }

    fn source_boundary(&self, source_offset: usize) -> usize {
        self.source_boundaries
            .get(source_offset.min(self.source_chunk.len()))
            .copied()
            .unwrap_or(*self.source_boundaries.last().unwrap_or(&0))
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

    let Ok(input_bytes) = fs::read_to_string(&input) else {
        panic!(
            "Failed to read {}. Ensure you have starting point file in your crate.",
            input.display()
        );
    };
    let input_source = rewrite_main_source_for_preprocess(&input_bytes);
    let preprocessed = preprocessor.preprocess(&input_source.text);
    let source = build_preprocessed_source(&input, preprocessed, &input_source.boundaries);
    emit_preprocessor_diagnostics(&source, preprocessor.warnings(), preprocessor.errors());
    source
}

fn emit_preprocessor_diagnostics(
    preprocessed: &PreprocessedSource,
    warnings: &[pipeline::PreprocessorDiagnostic],
    errors: &[pipeline::PreprocessorDiagnostic],
) {
    if warnings.is_empty() && errors.is_empty() {
        return;
    }

    let files = preprocessed
        .files()
        .iter()
        .map(|(id, file)| (*id, (file.path.display().to_string(), file.source.clone())))
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
    diagnostics: &[pipeline::PreprocessorDiagnostic],
) -> Vec<Rich<'static, String, Span>> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            Rich::custom(
                preprocessor_diagnostic_span(preprocessed, diagnostic),
                diagnostic.message.clone(),
            )
        })
        .collect()
}

fn preprocessor_diagnostic_span(
    preprocessed: &PreprocessedSource,
    diagnostic: &pipeline::PreprocessorDiagnostic,
) -> Span {
    let wanted_path = absolute_path(Path::new(&diagnostic.file));
    let Some((file_id, file)) = preprocessed
        .files()
        .iter()
        .find(|(_, file)| file.path == wanted_path)
    else {
        return Span::new(preprocessed.main_file_idx, 0..0);
    };

    let remap: &[usize] = if *file_id == preprocessed.main_file_idx {
        preprocessed.main_rewrite_boundaries.as_ref()
    } else {
        &[]
    };
    let start = if remap.is_empty() {
        diagnostic.range.start
    } else {
        remap
            .get(diagnostic.range.start)
            .copied()
            .unwrap_or(*remap.last().unwrap_or(&0))
    }
    .min(file.source.len());
    let mut end = if remap.is_empty() {
        diagnostic.range.end
    } else {
        remap
            .get(diagnostic.range.end)
            .copied()
            .unwrap_or(*remap.last().unwrap_or(&0))
    }
    .min(file.source.len())
    .max(start);
    if end == start && start < file.source.len() {
        end += 1;
    }
    Span::new(*file_id, start..end)
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
        fs::write(
            &input,
            "#warning keep going\nint main(void) { return 0; }\n",
        )
        .unwrap();

        let mut preprocessor = Preprocessor::new();
        configure_preprocessor(&mut preprocessor, &input, &[]);
        let input_source = rewrite_main_source_for_preprocess(&fs::read_to_string(&input).unwrap());
        let preprocessed = preprocessor.preprocess(&input_source.text);
        let source = build_preprocessed_source(&input, preprocessed, &input_source.boundaries);
        let diagnostics = map_preprocessor_diagnostics(&source, preprocessor.warnings());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].to_string(), "#warning keep going");

        let span = *diagnostics[0].span();
        let file = source.files().get(&span.context).unwrap();
        assert_eq!(&file.source[span.start..span.end], "warning");

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn preprocesses_prefixed_char_literals_and_object_like_token_paste() {
        let temp_dir = std::env::temp_dir().join(format!(
            "co2-preprocessor-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let input = temp_dir.join("prefixed-char-and-paste.c");
        fs::write(
            &input,
            "#define PLUS_EQ + ## =\n#if L'A' != 65\n#error bad wide char\n#endif\nint main(void) { int x = 1; x PLUS_EQ 2; return x != 3; }\n",
        )
        .unwrap();

        let mut preprocessor = Preprocessor::new();
        configure_preprocessor(&mut preprocessor, &input, &[]);
        let input_source = rewrite_main_source_for_preprocess(&fs::read_to_string(&input).unwrap());
        let preprocessed = preprocessor.preprocess(&input_source.text);
        let raw = preprocessed
            .chunks
            .iter()
            .map(|chunk| chunk.raw.as_str())
            .collect::<String>();

        assert!(preprocessor.errors().is_empty());
        assert!(raw.contains("x += 2;"));
        assert!(!raw.contains("##"));

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn handles_pending_ifdef_else_endif_in_inactive_branch() {
        let temp_dir = std::env::temp_dir().join(format!(
            "co2-preprocessor-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let input = temp_dir.join("pending-ifdef.c");
        fs::write(
            &input,
            "int check(void) {\n    return (1 &&\n#ifdef FOO\n        0\n#else\n        1\n#endif\n    );\n}\n",
        )
        .unwrap();

        let mut preprocessor = Preprocessor::new();
        configure_preprocessor(&mut preprocessor, &input, &[]);
        let input_source = rewrite_main_source_for_preprocess(&fs::read_to_string(&input).unwrap());
        let preprocessed = preprocessor.preprocess(&input_source.text);
        let raw = preprocessed
            .chunks
            .iter()
            .map(|chunk| chunk.raw.as_str())
            .collect::<String>();

        assert!(preprocessor.errors().is_empty());
        assert!(raw.contains("return (1 &&"));
        assert!(raw.contains("        1\n"));
        assert!(!raw.contains("        0\n"));

        let _ = fs::remove_dir_all(temp_dir);
    }
}

fn discover_system_include_paths() -> Vec<PathBuf> {
    let Ok(output) = std::process::Command::new("gcc")
        .args(["-E", "-Wp,-v", "-"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };

    if !output.status.success() {
        return Vec::new();
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut paths = Vec::new();
    let mut in_system_section = false;

    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.contains("#include <...> search starts here:") {
            in_system_section = true;
            continue;
        }
        if trimmed == "End of search list." {
            break;
        }
        if in_system_section && !trimmed.is_empty() {
            paths.push(PathBuf::from(trimmed));
        }
    }

    paths
}

fn configure_preprocessor(preprocessor: &mut Preprocessor, input: &Path, cpp_args: &[String]) {
    let input_str = input.to_string_lossy().into_owned();
    preprocessor.set_filename(&input_str);
    configure_target(preprocessor);

    let nostdinc = cpp_args.iter().any(|arg| arg == "-nostdinc");
    if !nostdinc {
        preprocessor.system_include_paths = discover_system_include_paths();
    }

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
                let content = fs::read_to_string(&resolved).unwrap_or_else(|e| {
                    panic!("failed to read force include {}: {e}", resolved.display())
                });
                preprocessor.preprocess_force_include(&content, &resolved.to_string_lossy());
            }
            "-isystem" => {
                i += 1;
                preprocessor
                    .add_system_include_path(cpp_args.get(i).expect("missing -isystem value"));
            }
            "-iquote" => {
                i += 1;
                preprocessor
                    .add_quote_include_path(cpp_args.get(i).expect("missing -iquote value"));
            }
            "-nostdinc" => {}
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

fn build_preprocessed_source(
    input: &Path,
    preprocessed: PreprocessOutput,
    main_rewrite_boundaries: &[usize],
) -> PreprocessedSource {
    let mut files = HashMap::<FileId, SourceFile>::new();
    let mut file_index = HashMap::<PathBuf, FileId>::new();
    let main_file_idx = ensure_source_file(input, &mut files, &mut file_index);

    let mut normalized = String::new();
    let mut output_chunks = Vec::new();
    let mut chunk_offsets = Vec::new();

    for mut chunk in preprocessed.chunks {
        if chunk.file == input {
            chunk.source_boundaries = chunk
                .source_boundaries
                .into_iter()
                .map(|offset| {
                    main_rewrite_boundaries
                        .get(offset)
                        .copied()
                        .unwrap_or(*main_rewrite_boundaries.last().unwrap_or(&0))
                })
                .collect();
        }
        let file_idx = ensure_source_file(&chunk.file, &mut files, &mut file_index);
        let mapped = if should_skip_file_contents(&chunk.file) {
            keep_newlines_only(&chunk.raw)
        } else {
            normalize_preprocessed_chunk(&chunk.raw)
        };
        if mapped.text.is_empty() {
            continue;
        }
        let output_start = normalized.len();
        normalized.push_str(&mapped.text);
        let output_end = normalized.len();
        chunk_offsets.push(output_start);
        output_chunks.push(OutputChunkMap {
            file_idx,
            raw_chunk: Arc::<str>::from(chunk.raw),
            normalized_chunk: Arc::<str>::from(mapped.text),
            normalized_to_raw: Arc::new(mapped.boundaries),
            source_chunk: Arc::<str>::from(chunk.source_text),
            source_boundaries: Arc::new(chunk.source_boundaries),
            output_range: output_start..output_end,
        });
    }

    if let Some(path) = std::env::var_os("CO2_DUMP_PREPROCESSED") {
        let _ = fs::write(path, &normalized);
    }

    PreprocessedSource {
        normalized: Arc::<str>::from(normalized),
        main_file_idx,
        files: Arc::new(files),
        output_chunks: Arc::new(output_chunks),
        chunk_offsets: Arc::new(chunk_offsets),
        main_rewrite_boundaries: Arc::new(main_rewrite_boundaries.to_vec()),
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

    let source = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read source file {}: {e}", path.display()));
    let idx = global_file_id(path);
    files.insert(
        idx,
        SourceFile {
            path: path.to_path_buf(),
            source: Arc::<str>::from(source),
        },
    );
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

fn rewrite_main_source_for_preprocess(source: &str) -> MappedText {
    let prefix = "#define __CO2__ 1\n";
    let mut rewritten = String::from(prefix);
    let mut boundaries = vec![0; prefix.len() + 1];
    let mut line_start = 0usize;

    for raw_line in source.split_inclusive('\n') {
        let has_newline = raw_line.ends_with('\n');
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line_end = line_start + line.len();
        if line.trim_start().starts_with('#') && !line.trim_start().starts_with("#[") {
            rewrite_hidden_macros_in_directive(line, line_start, &mut rewritten, &mut boundaries);
        } else {
            rewritten.push_str(line);
            for idx in line_start + 1..=line_end {
                boundaries.push(idx);
            }
        }
        if has_newline {
            rewritten.push('\n');
            boundaries.push(line_end + 1);
        }
        line_start += raw_line.len();
    }

    MappedText {
        text: rewritten,
        boundaries,
    }
}

fn rewrite_hidden_macros_in_directive(
    line: &str,
    absolute_start: usize,
    rewritten: &mut String,
    boundaries: &mut Vec<usize>,
) {
    let mut i = 0usize;
    while i < line.len() {
        let replacement = if line[i..].starts_with("__GNUC__") {
            Some(("__CO2_HIDDEN_GNUC__", "__GNUC__".len()))
        } else if line[i..].starts_with("__clang__") {
            Some(("__CO2_HIDDEN_CLANG__", "__clang__".len()))
        } else {
            None
        };
        if let Some((replacement, consumed)) = replacement {
            rewritten.push_str(replacement);
            boundaries.extend(std::iter::repeat_n(
                absolute_start + i + consumed,
                replacement.len(),
            ));
            i += consumed;
        } else {
            let c = line[i..].chars().next().unwrap();
            let consumed = c.len_utf8();
            rewritten.push(c);
            i += consumed;
            for _ in 0..consumed {
                boundaries.push(absolute_start + i);
            }
        }
    }
}

fn should_skip_file_contents(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("stdatomic.h" | "tgmath.h")
    )
}

#[derive(Debug)]
struct MappedText {
    text: String,
    boundaries: Vec<usize>,
}

fn normalize_preprocessed_chunk(chunk: &str) -> MappedText {
    let chunk = strip_balanced_call_mapped(chunk, "__we_dont_need_this_just_create_a_new_mapped__");
    let chunk = strip_gnu_attributes_mapped(&chunk);
    let chunk = strip_gnu_asm_annotations_mapped(&chunk);
    strip_extension_keywords_mapped(&chunk)
}

fn keep_newlines_only(chunk: &str) -> MappedText {
    let mut text = String::new();
    let mut boundaries = vec![0];
    for (idx, byte) in chunk.bytes().enumerate() {
        if byte == b'\n' {
            text.push('\n');
            boundaries.push(idx + 1);
        }
    }
    MappedText { text, boundaries }
}

fn chunk_index_for_offset(offsets: &[usize], offset: usize) -> Option<usize> {
    if offsets.is_empty() {
        return None;
    }
    match offsets.binary_search(&offset) {
        Ok(i) => Some(i),
        Err(i) => Some(i.saturating_sub(1)),
    }
}

fn map_text_offset(source_text: &str, mapped_text: &str, output_offset: usize) -> usize {
    let output_offset = output_offset.min(mapped_text.len());
    let span_end = expand_token_end(mapped_text, output_offset);
    let span_start = shrink_token_start(mapped_text, output_offset, span_end);
    let needle = mapped_text.get(span_start..span_end).unwrap_or("").trim();
    if !needle.is_empty() {
        let mut matches = source_text.match_indices(needle);
        if let Some((idx, _)) = matches.next()
            && matches.next().is_none()
        {
            return idx + output_offset.saturating_sub(span_start);
        }
    }
    output_offset.min(source_text.len())
}

fn map_text_range(
    source_text: &str,
    mapped_text: &str,
    output_start: usize,
    output_end: usize,
) -> Range<usize> {
    let output_start = output_start.min(mapped_text.len());
    let output_end = output_end.min(mapped_text.len()).max(output_start);
    if output_start == output_end {
        let point = map_text_offset(source_text, mapped_text, output_start);
        return point..point;
    }

    let needle = mapped_text
        .get(output_start..output_end)
        .unwrap_or("")
        .trim();
    if !needle.is_empty() {
        let mut matches = source_text.match_indices(needle);
        if let Some((idx, _)) = matches.next()
            && matches.next().is_none()
        {
            return idx..(idx + needle.len());
        }
    }

    // When the exact needle isn't in the source (e.g. it was synthesised by a
    // token-pasting macro like `field_##x`), try progressively shorter
    // `_`-split suffix components.  If a suffix appears exactly once in the
    // source text and is surrounded by non-identifier characters (so it really
    // is a whole identifier), expand the match to include any enclosing macro
    // call of the form `IDENT(...)`.
    if needle.bytes().all(is_ident_continue)
        && let Some(first_under) = needle.find('_')
    {
        let mut suffix_start = first_under + 1;
        while suffix_start < needle.len() {
            let suffix = &needle[suffix_start..];
            let is_whole_ident = |pos: usize| {
                let bytes = source_text.as_bytes();
                let before_ok = pos == 0 || !is_ident_continue(bytes[pos - 1]);
                let after_ok = pos + suffix.len() >= source_text.len()
                    || !is_ident_continue(bytes[pos + suffix.len()]);
                before_ok && after_ok
            };
            let mut it = source_text
                .match_indices(suffix)
                .filter(|&(pos, _)| is_whole_ident(pos));
            if let Some((idx, _)) = it.next()
                && it.next().is_none()
            {
                return expand_to_macro_call(source_text, idx..idx + suffix.len());
            }
            match suffix.find('_') {
                Some(next) => suffix_start += next + 1,
                None => break,
            }
        }
    }

    source_token_range(source_text, output_start, output_end)
}

/// Given a range covering an identifier in `source_text`, expand it to cover
/// the enclosing `MACRO_NAME(...)` call if the identifier is immediately
/// preceded by `(` which is itself preceded by an identifier.
fn expand_to_macro_call(source_text: &str, range: Range<usize>) -> Range<usize> {
    let bytes = source_text.as_bytes();
    let start = range.start;

    if start > 0 && bytes[start - 1] == b'(' {
        let paren_pos = start - 1;
        if paren_pos > 0 && is_ident_continue(bytes[paren_pos - 1]) {
            let mut macro_start = paren_pos;
            while macro_start > 0 && is_ident_continue(bytes[macro_start - 1]) {
                macro_start -= 1;
            }
            // Find the matching ')' for the '(' at paren_pos.
            let mut depth = 1usize;
            let mut pos = start;
            while pos < bytes.len() && depth > 0 {
                match bytes[pos] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                pos += 1;
            }
            return macro_start..pos;
        }
    }

    range
}

fn source_token_range(source_text: &str, output_start: usize, _output_end: usize) -> Range<usize> {
    let bytes = source_text.as_bytes();
    if bytes.is_empty() {
        return 0..0;
    }

    let mut anchor = output_start.min(bytes.len().saturating_sub(1));
    if !is_ident_continue(bytes[anchor]) && anchor > 0 && is_ident_continue(bytes[anchor - 1]) {
        anchor -= 1;
    }
    if !is_ident_continue(bytes[anchor]) {
        // No identifier at the anchor position — we can't reliably map this token to a
        // source location. Return an empty range at the end of the source text so that
        // any correctly-mapped tokens in the same expression keep their positions and the
        // span union computed by chumsky stays valid (start ≤ end).
        let end = bytes.len();
        return end..end;
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

fn strip_gnu_attributes_mapped(src: &MappedText) -> MappedText {
    let src = compose_boundaries(
        strip_gnu_attribute_mapped(src.text.as_str(), "__attribute__"),
        &src.boundaries,
    );
    compose_boundaries(
        strip_gnu_attribute_mapped(src.text.as_str(), "__attribute"),
        &src.boundaries,
    )
}

fn strip_gnu_asm_annotations_mapped(src: &MappedText) -> MappedText {
    let src = compose_boundaries(
        strip_balanced_call_mapped(src.text.as_str(), "__asm__"),
        &src.boundaries,
    );
    compose_boundaries(
        strip_balanced_call_mapped(src.text.as_str(), "__asm"),
        &src.boundaries,
    )
}

fn strip_balanced_call_mapped(src: &str, keyword: &str) -> MappedText {
    strip_balanced_call_mapped_with(src, keyword, |_| None)
}

fn strip_gnu_attribute_mapped(src: &str, keyword: &str) -> MappedText {
    strip_balanced_call_mapped_with(src, keyword, |call| {
        call.contains("__transparent_union__")
            .then_some("__co2_transparent_union_attr")
    })
}

fn strip_balanced_call_mapped_with(
    src: &str,
    keyword: &str,
    replacement: impl Fn(&str) -> Option<&'static str>,
) -> MappedText {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut boundaries = vec![0];
    let mut i = 0usize;
    while i < bytes.len() {
        if matches_keyword(src, i, keyword) {
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
                if let Some(replacement) = replacement(&src[i..j]) {
                    out.push(' ');
                    boundaries.push(i);
                    out.push_str(replacement);
                    for _ in replacement.bytes() {
                        boundaries.push(i);
                    }
                    out.push(' ');
                    boundaries.push(j);
                } else {
                    out.push(' ');
                    boundaries.push(j);
                }
                i = j;
                continue;
            }
        }
        let c = src[i..].chars().next().unwrap();
        let consumed = c.len_utf8();
        out.push(c);
        i += consumed;
        for _ in 0..consumed {
            boundaries.push(i);
        }
    }
    MappedText {
        text: out,
        boundaries,
    }
}

fn compose_boundaries(mapped: MappedText, prior: &[usize]) -> MappedText {
    MappedText {
        text: mapped.text,
        boundaries: mapped
            .boundaries
            .into_iter()
            .map(|offset| {
                prior
                    .get(offset)
                    .copied()
                    .unwrap_or(*prior.last().unwrap_or(&0))
            })
            .collect(),
    }
}

fn matches_keyword(src: &str, start: usize, kw: &str) -> bool {
    if !src.is_char_boundary(start) {
        return false;
    }
    let bytes = src.as_bytes();
    src[start..].starts_with(kw)
        && (start == 0 || !is_ident_continue(bytes[start - 1]))
        && !bytes
            .get(start + kw.len())
            .is_some_and(|b| is_ident_continue(*b))
}

fn strip_extension_keywords_mapped(src: &MappedText) -> MappedText {
    fn is_ident_start(b: u8) -> bool {
        b.is_ascii_alphabetic() || b == b'_'
    }
    fn is_ident_continue_local(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let bytes = src.text.as_bytes();
    let mut out = String::with_capacity(src.text.len());
    let mut boundaries = vec![0];
    let mut i = 0usize;
    let mut in_string = false;
    let mut in_char = false;
    while i < bytes.len() {
        if in_string {
            let c = src.text[i..].chars().next().unwrap();
            let consumed = c.len_utf8();
            out.push(c);
            i += consumed;
            for _ in 0..consumed {
                boundaries.push(i);
            }
            if c == '\\' && i < bytes.len() {
                let c2 = src.text[i..].chars().next().unwrap();
                let consumed2 = c2.len_utf8();
                out.push(c2);
                i += consumed2;
                for _ in 0..consumed2 {
                    boundaries.push(i);
                }
            } else if c == '"' {
                in_string = false;
            }
        } else if in_char {
            let c = src.text[i..].chars().next().unwrap();
            let consumed = c.len_utf8();
            out.push(c);
            i += consumed;
            for _ in 0..consumed {
                boundaries.push(i);
            }
            if c == '\\' && i < bytes.len() {
                let c2 = src.text[i..].chars().next().unwrap();
                let consumed2 = c2.len_utf8();
                out.push(c2);
                i += consumed2;
                for _ in 0..consumed2 {
                    boundaries.push(i);
                }
            } else if c == '\'' {
                in_char = false;
            }
        } else if bytes[i] == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            boundaries.push(i);
        } else if bytes[i] == b'\'' {
            in_char = true;
            out.push('\'');
            i += 1;
            boundaries.push(i);
        } else if is_ident_start(bytes[i]) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue_local(bytes[i]) {
                i += 1;
            }
            let ident = &src.text[start..i];
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
                boundaries.push(i);
            } else {
                out.push_str(ident);
                for offset in (start + 1)..=i {
                    boundaries.push(offset);
                }
            }
        } else {
            let c = src.text[i..].chars().next().unwrap();
            let consumed = c.len_utf8();
            out.push(c);
            i += consumed;
            for _ in 0..consumed {
                boundaries.push(i);
            }
        }
    }
    compose_boundaries(
        MappedText {
            text: out,
            boundaries,
        },
        &src.boundaries,
    )
}
